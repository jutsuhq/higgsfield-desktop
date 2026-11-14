//! Catalog writes from outside the engine: joined videos, account history refreshes and saved images.

use jutsu_core::{
    Cost, HistoryPage, Job, JobState, LocalImage, Material, MaterialGeneration, MaterialInputRole,
    MaterialKind, MaterialOrigin, MaterialRemote, MaterialVersion, ProviderKind, SavedImage,
};
use rust_decimal::Decimal;
use sqlx::SqliteConnection;
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::catalog::{input, input_material, material_in, parent_input, upsert};
use crate::error::StoreError;
use crate::jobs::job_id_by_remote_key;
use crate::store::{Store, StoreChange};

async fn set_star(connection: &mut SqliteConnection, id: Uuid, starred: bool) -> Result<(), StoreError> {
    sqlx::query("UPDATE materials SET is_starred = ? WHERE id = ?")
        .bind(starred)
        .bind(id.to_string())
        .execute(connection)
        .await?;
    Ok(())
}

impl Store {
    /// Adds `output`, a local join of `original`'s result, as a derived material: not another paid generation or
    /// an execution job.
    pub async fn save_joined_video(&self, original: &Job, output: Url) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        let connection = &mut *write.transaction;
        let id = Uuid::new_v4();
        let now = OffsetDateTime::now_utc();
        let mut job = Job::new(
            id,
            original.spec.clone(),
            original.provider_kind,
            Cost::credits(Decimal::ZERO, false),
            Uuid::new_v4(),
            now,
        );
        job.owner.clone_from(&original.owner);
        job.parent_id = Some(original.id);
        job.state = JobState::Done;
        job.output_file = Some(output.clone());
        job.finished_at = Some(now);
        let mut relations = vec![parent_input(&mut *connection, original.id).await?];
        if let Some(source) = &original.spec.source {
            let reference = input_material(
                &mut *connection,
                &source.file_url,
                MaterialKind::Video,
                source.material,
            )
            .await?;
            relations.push(input(reference, MaterialInputRole::SourceVideo, 0));
        }
        let mut version = MaterialVersion::new(id, now);
        version.file_url = Some(output);
        version.inputs = relations;
        version.video = Some(Box::new(job));
        upsert(connection, Material::new(id, MaterialKind::Video, now, version)).await?;
        write.commit(&[StoreChange::Materials]).await
    }

    /// Lists a page of the account's history in the catalog, with the same identities as the materials generated
    /// and downloaded here: an item already known keeps its ID, files, versions and culling, and takes its star
    /// from the account's like. Returns the page with each item's `job_id` set to its material.
    pub async fn cache_history(
        &self,
        page: HistoryPage,
        owner: Option<&str>,
    ) -> Result<HistoryPage, StoreError> {
        let mut write = self.begin().await?;
        let connection = &mut *write.transaction;
        let mut page = page;
        let owner = owner.map(str::to_owned);
        for item in &mut page.items {
            let remote = account_remote(owner.clone(), &item.id);
            let running = job_id_by_remote_key(&mut *connection, &remote.key()).await?;
            if let Some(known) = material_by_remote(&mut *connection, &remote).await? {
                item.job_id = known;
            } else if let Some(running) = running {
                item.job_id = running;
            }
            let job = Job::importing(item, owner.clone(), item.created_at);
            let mut version = MaterialVersion::new(item.job_id, item.created_at);
            version.remote_url = Some(item.output_url.clone());
            version.generation = Some(generation(&item.prompt, item.model.raw_value()));
            version.video = Some(Box::new(job));
            let material = listed(
                item.job_id,
                MaterialKind::Video,
                remote,
                item.created_at,
                item.is_liked,
                version,
            );
            let saved = upsert(&mut *connection, material).await?;
            set_star(&mut *connection, saved.id, item.is_liked).await?;
            item.job_id = saved.id;
        }
        for item in &mut page.images {
            let remote = account_remote(owner.clone(), &item.id);
            if let Some(known) = material_by_remote(&mut *connection, &remote).await? {
                item.job_id = known;
            }
            let mut version = MaterialVersion::new(item.job_id, item.created_at);
            version.remote_url = Some(item.output_url.clone());
            version.generation = Some(generation(&item.prompt, &item.model));
            version.aspect_ratio = item.aspect_ratio();
            version.pixel_size = item.pixel_size;
            let material = listed(
                item.job_id,
                MaterialKind::Image,
                remote,
                item.created_at,
                item.is_liked,
                version,
            );
            let saved = upsert(&mut *connection, material).await?;
            set_star(&mut *connection, saved.id, item.is_liked).await?;
            item.job_id = saved.id;
        }
        write.commit(&[StoreChange::Materials]).await?;
        Ok(page)
    }

    /// Registers an image and its real inputs together; a file-only import keeps no invented generation metadata.
    pub async fn save_image(
        &self,
        image: &SavedImage,
        remote: Option<MaterialRemote>,
        generation: Option<MaterialGeneration>,
        inputs: &[LocalImage],
        variant_group_id: Option<Uuid>,
    ) -> Result<Material, StoreError> {
        let mut write = self.begin().await?;
        let connection = &mut *write.transaction;
        let mut relations = Vec::with_capacity(inputs.len());
        for (position, local) in (0_i64..).zip(inputs) {
            let reference = input_material(
                &mut *connection,
                &local.file_url,
                MaterialKind::Image,
                local.material,
            )
            .await?;
            relations.push(input(reference, MaterialInputRole::Reference, position));
        }
        let mut version = MaterialVersion::new(image.id, image.created_at);
        version.file_url = Some(image.file_url.clone());
        version.remote_url.clone_from(&image.remote_url);
        version.generation = generation;
        version.inputs = relations;
        version.aspect_ratio.clone_from(&image.aspect_ratio);
        let mut material = Material::new(image.id, MaterialKind::Image, image.created_at, version);
        material.origin = if remote.is_some() {
            MaterialOrigin::Higgsfield
        } else {
            MaterialOrigin::Local
        };
        material.remote = remote;
        material.is_starred = image.is_starred;
        material.variant_group_id = variant_group_id;
        let saved = upsert(connection, material).await?;
        write.commit(&[StoreChange::Materials]).await?;
        Ok(saved)
    }
}

fn account_remote(owner: Option<String>, id: &str) -> MaterialRemote {
    MaterialRemote {
        provider: ProviderKind::Account,
        owner,
        id: id.to_owned(),
        output: 0,
    }
}

fn generation(prompt: &str, model: &str) -> MaterialGeneration {
    MaterialGeneration {
        prompt: prompt.to_owned(),
        model: model.to_owned(),
        parameters: None,
        title: None,
    }
}

/// An account listing as a material.
fn listed(
    id: Uuid,
    kind: MaterialKind,
    remote: MaterialRemote,
    created_at: OffsetDateTime,
    is_starred: bool,
    version: MaterialVersion,
) -> Material {
    let mut material = Material::new(id, kind, created_at, version);
    material.origin = MaterialOrigin::Higgsfield;
    material.remote = Some(remote);
    material.is_starred = is_starred;
    material
}

async fn material_by_remote(
    connection: &mut SqliteConnection,
    remote: &MaterialRemote,
) -> Result<Option<Uuid>, StoreError> {
    let id: Option<String> = sqlx::query_scalar("SELECT id FROM materials WHERE remote_key = ? LIMIT 1")
        .bind(remote.key())
        .fetch_optional(&mut *connection)
        .await?;
    let Some(id) = id else { return Ok(None) };
    let id = crate::store::uuid("materials", &id)?;
    // Decoded as Swift's `material(remote:)` does: a damaged entry fails the refresh.
    Ok(material_in(connection, id).await?.map(|material| material.id))
}
