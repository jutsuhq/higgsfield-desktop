//! `DeliverySetRecord`, and edits brought back as new versions.

use jutsu_core::{
    DeliverySet, DeliverySetError, Material, MaterialError, MaterialGeneration, MaterialInputRole,
    MaterialReference, MaterialVersion, OutputMedia,
};
use sqlx::sqlite::SqliteRow;
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::catalog::{find_version, input, material_in, update};
use crate::error::StoreError;
use crate::store::{Store, StoreChange, nanos, text};

const TABLE: &str = "delivery_sets";

fn decode(row: &SqliteRow) -> Result<DeliverySet, StoreError> {
    let id = text(row, "id")?;
    let set: DeliverySet = serde_json::from_str(&text(row, "set_data")?)
        .map_err(|error| StoreError::damaged(TABLE, &id, error))?;
    if set.id.to_string() != id {
        return Err(MaterialError::InvalidRecord.into());
    }
    Ok(set)
}

impl Store {
    /// Every delivery set, newest first. A damaged entry is reported, never omitted.
    pub async fn delivery_sets(&self) -> Result<Vec<DeliverySet>, StoreError> {
        let rows = sqlx::query("SELECT id, set_data FROM delivery_sets ORDER BY created_at DESC")
            .fetch_all(self.pool())
            .await?;
        rows.iter().map(decode).collect()
    }

    /// Saves a new set or changes a stored one. Once the stored set is frozen its versions can't change and it
    /// can't be unfrozen: either fails with `DeliverySetError::Frozen` and leaves the record as it was.
    pub async fn save_delivery_set(&self, set: &DeliverySet) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        let connection = &mut *write.transaction;
        let existing = sqlx::query("SELECT id, set_data FROM delivery_sets WHERE id = ?")
            .bind(set.id.to_string())
            .fetch_optional(&mut *connection)
            .await?;
        let data = serde_json::to_string(set).map_err(StoreError::encode("delivery set"))?;
        if let Some(row) = existing {
            let stored = decode(&row)?;
            let references =
                |set: &DeliverySet| set.items().iter().map(|item| item.reference).collect::<Vec<_>>();
            if stored.is_frozen() && (!set.is_frozen() || references(set) != references(&stored)) {
                return Err(DeliverySetError::Frozen.into());
            }
            sqlx::query("UPDATE delivery_sets SET name = ?, created_at = ?, set_data = ? WHERE id = ?")
                .bind(&set.name)
                .bind(nanos(set.created_at))
                .bind(data)
                .bind(set.id.to_string())
                .execute(&mut *connection)
                .await?;
        } else {
            sqlx::query("INSERT INTO delivery_sets (id, name, created_at, set_data) VALUES (?, ?, ?, ?)")
                .bind(set.id.to_string())
                .bind(&set.name)
                .bind(nanos(set.created_at))
                .bind(data)
                .execute(&mut *connection)
                .await?;
        }
        write.commit(&[StoreChange::DeliverySets]).await
    }

    /// Removes the set of `id`; nothing when it isn't stored.
    pub async fn delete_delivery_set(&self, id: Uuid) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        sqlx::query("DELETE FROM delivery_sets WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *write.transaction)
            .await?;
        write.commit(&[StoreChange::DeliverySets]).await
    }

    /// Adds `file`, an edit of `source` already in the output folder, as the new current version of the same
    /// material, linked to that exact version as its parent. The source version and its file are untouched, and so
    /// is every delivery set that holds it.
    /// - `media`: the file's length and size when known, as an on-device Enhance run reads them back; the version
    ///   then carries them rather than the source's.
    /// - `relists`: whether a material taken off the Library lists again, as an edit brought back from another app
    ///   does; `false` refuses it with `MaterialError::NotListed`, for a run on this computer whose item was
    ///   deleted.
    ///
    /// Returns the material with its new version.
    pub async fn add_edited_version(
        &self,
        source: MaterialReference,
        file: Url,
        media: Option<OutputMedia>,
        relists: bool,
        date: OffsetDateTime,
    ) -> Result<Material, StoreError> {
        let mut write = self.begin().await?;
        let connection = &mut *write.transaction;
        let mut material = material_in(&mut *connection, source.material_id)
            .await?
            .ok_or(MaterialError::MissingMaterial)?;
        if !relists && material.is_hidden {
            return Err(MaterialError::NotListed.into());
        }
        let original = find_version(&material, source.version_id).ok_or(MaterialError::MissingMaterial)?;
        // The prompt and model say what the picture shows; the parameters would claim the edit can be generated.
        let generation = original.generation.map(|generation| MaterialGeneration {
            prompt: generation.prompt,
            model: generation.model,
            parameters: None,
            title: None,
        });
        let mut video = original.video;
        if let Some(job) = video.as_mut() {
            job.output_file = Some(file.clone());
            if let Some(media) = media {
                job.output_media = Some(media);
            }
        }
        let mut version = MaterialVersion::new(Uuid::new_v4(), date);
        version.file_url = Some(file);
        version.generation = generation;
        version.inputs = vec![input(source, MaterialInputRole::Parent, 0)];
        version.aspect_ratio = media.and_then(|media| media.pixel_size.aspect_label());
        version.pixel_size = media.map(|media| media.pixel_size);
        version.video = video;
        material.add_version(version)?;
        // An edit brought back lists the material again, like saving a file does.
        material.is_hidden = false;
        update(connection, &material, false).await?;
        write.commit(&[StoreChange::Materials]).await?;
        Ok(material)
    }
}
