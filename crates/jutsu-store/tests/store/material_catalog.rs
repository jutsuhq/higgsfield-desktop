//! `MaterialCatalogTests`: the Library catalog, its identities, relations, culling and variant primaries.

use std::collections::HashMap;
use std::time::Instant;

use jutsu_core::{
    Culling, HistoryCursor, HistoryImageItem, HistoryPage, JobSpec, JobState, LocalImage, Material,
    MaterialError, MaterialInput, MaterialInputRole, MaterialKind, MaterialRemote, MaterialVersion, Mode,
    ProviderKind, Resolution, VersionAvailability, VideoModel,
};
use jutsu_store::{FileLayout, MaterialQuery, MaterialSort, Store, StoreError};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::common::{file_url, generation, job, memory_store, saved_image};

fn account(owner: &str, id: &str) -> MaterialRemote {
    MaterialRemote {
        provider: ProviderKind::Account,
        owner: Some(owner.to_owned()),
        id: id.to_owned(),
        output: 0,
    }
}

fn version(file: Url) -> MaterialVersion {
    let mut version = MaterialVersion::new(Uuid::new_v4(), OffsetDateTime::now_utc());
    version.file_url = Some(file);
    version
}

fn exists(path: &std::path::Path) -> bool {
    path.exists()
}

#[tokio::test]
async fn image_and_video_survive_reopening_with_metadata_stars_and_versions() {
    let folder = tempfile::tempdir().unwrap();
    let layout = FileLayout::new(folder.path(), folder.path().join("Media"));
    layout.prepare_directories().unwrap();
    let first = Store::open(&layout.store_path()).await.unwrap();
    let saved = saved_image(folder.path(), "image");
    let remote = account("alice", "remote-image");
    let mut material = first
        .save_image(
            &saved,
            Some(remote.clone()),
            Some(generation(&saved.prompt, &saved.model, Some("{}"))),
            &[],
            Some(Uuid::new_v4()),
        )
        .await
        .unwrap();
    let original = material.reference();
    let mut edit = version(file_url(folder.path().join("edit.png")));
    edit.inputs = vec![MaterialInput {
        source: original,
        role: MaterialInputRole::Parent,
        position: 0,
    }];
    material.add_version(edit).unwrap();
    let material = first.save_material(material).await.unwrap();
    first.set_materials_starred(true, &[material.id]).await.unwrap();
    let mut done = job(JobState::Done);
    done.output_file = Some(file_url(folder.path().join("video.mp4")));
    first.save_job(&done).await.unwrap();

    let reopened = Store::open(&layout.store_path()).await.unwrap();
    let restored = reopened.material_by_remote(&remote).await.unwrap().unwrap();
    assert_eq!(restored.id, material.id);
    assert_eq!(restored.reference(), material.reference());
    assert!(restored.is_starred);
    assert_eq!(restored.versions().len(), 2);
    let parameters = restored.previous_versions()[0]
        .generation
        .as_ref()
        .unwrap()
        .parameters
        .clone();
    assert_eq!(parameters, Some(b"{}".to_vec()));
    let made_from: Vec<_> = reopened
        .made_from(restored.reference())
        .await
        .unwrap()
        .iter()
        .map(|input| input.source)
        .collect();
    assert_eq!(made_from, [original]);
    let video = reopened
        .material(done.id)
        .await
        .unwrap()
        .unwrap()
        .current
        .video
        .unwrap();
    assert_eq!(*video, done);
}

#[tokio::test]
async fn remote_identity_deduplicates_across_reloads_and_separates_owners() {
    let folder = tempfile::tempdir().unwrap();
    let catalog = memory_store().await;
    let id = "same-remote-id";
    let page = || HistoryPage {
        items: vec![],
        images: vec![HistoryImageItem {
            id: id.to_owned(),
            job_id: Uuid::new_v4(),
            prompt: String::new(),
            model: "soul".to_owned(),
            created_at: OffsetDateTime::now_utc(),
            is_liked: false,
            output_url: Url::parse("https://example.com/image.png").unwrap(),
            pixel_size: None,
        }],
        next_cursor: None::<HistoryCursor>,
    };
    let first = catalog.cache_history(page(), Some("alice")).await.unwrap();
    let again = catalog.cache_history(page(), Some("alice")).await.unwrap();
    let bob = catalog.cache_history(page(), Some("bob")).await.unwrap();
    assert_eq!(first.images[0].job_id, again.images[0].job_id);
    assert_ne!(first.images[0].job_id, bob.images[0].job_id);
    assert_eq!(catalog.materials().await.unwrap().len(), 2);
    let local = saved_image(folder.path(), "image");
    let identity = account("alice", id);
    let imported = catalog
        .save_image(&local, Some(identity.clone()), None, &[], None)
        .await
        .unwrap();
    assert_eq!(imported.id, first.images[0].job_id);
    catalog.cache_history(page(), Some("alice")).await.unwrap();
    let after_refresh = catalog.material_by_remote(&identity).await.unwrap().unwrap();
    assert_eq!(after_refresh.current.file_url.as_ref(), Some(&local.file_url));
    assert_eq!(after_refresh.reference(), imported.reference());
    assert_eq!(catalog.materials().await.unwrap().len(), 2);
    let mut edited = after_refresh;
    edited
        .add_version(version(file_url(folder.path().join("edited.png"))))
        .unwrap();
    catalog.save_material(edited.clone()).await.unwrap();
    catalog.cache_history(page(), Some("alice")).await.unwrap();
    let refreshed = catalog.material_by_remote(&identity).await.unwrap().unwrap();
    assert_eq!(refreshed.reference(), edited.reference());
    assert_eq!(refreshed.versions().len(), 2);
    assert_eq!(
        refreshed.previous_versions()[0].file_url.as_ref(),
        Some(&local.file_url)
    );
}

#[tokio::test]
async fn enqueue_pins_all_input_versions_and_roles_before_completion() {
    let folder = tempfile::tempdir().unwrap();
    let catalog = memory_store().await;
    let start_image = saved_image(folder.path(), "start");
    let end_image = saved_image(folder.path(), "end");
    let start = catalog
        .save_image(&start_image, None, None, &[], None)
        .await
        .unwrap();
    let mut end = catalog
        .save_image(&end_image, None, None, &[], None)
        .await
        .unwrap();
    let original_end = end.reference();
    let mut job = job(JobState::Preparing);
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        None,
        Resolution::P720,
    );
    spec.references = vec![
        LocalImage::new(start_image.file_url.clone()),
        LocalImage::new(end_image.file_url.clone()),
    ];
    spec.length = Some(5.0);
    spec.references_are_frames = true;
    job.spec = spec;
    catalog.save_job(&job).await.unwrap();
    end.add_version(version(file_url(folder.path().join("new-end.png"))))
        .unwrap();
    let end = catalog.save_material(end).await.unwrap();
    job.state = JobState::Done;
    job.output_file = Some(file_url(folder.path().join("result.mp4")));
    catalog.save_job(&job).await.unwrap();
    let output = catalog.material(job.id).await.unwrap().unwrap();
    let inputs = catalog.made_from(output.reference()).await.unwrap();
    let roles: Vec<_> = inputs.iter().map(|input| input.role).collect();
    assert_eq!(
        roles,
        [MaterialInputRole::StartFrame, MaterialInputRole::EndFrame]
    );
    let sources: Vec<_> = inputs.iter().map(|input| input.source).collect();
    assert_eq!(sources, [start.reference(), original_end]);
    assert_eq!(catalog.used_in(original_end).await.unwrap(), [output.reference()]);
    assert!(catalog.used_in(end.reference()).await.unwrap().is_empty());
}

/// Make Video pins the version it was asked for on the image (`LocalImage::material`): the relation names that
/// version and the role it was given, even when the file at the path is a newer edit or is removed meanwhile.
#[tokio::test]
async fn make_video_records_the_pinned_version_with_its_role_at_enqueue() {
    let folder = tempfile::tempdir().unwrap();
    let catalog = memory_store().await;
    let picked_image = saved_image(folder.path(), "picked");
    let mut picked = catalog
        .save_image(
            &picked_image,
            None,
            Some(generation("A knight", "soul", None)),
            &[],
            None,
        )
        .await
        .unwrap();
    let asked = picked.reference();
    picked
        .add_version(version(picked_image.file_url.clone()))
        .unwrap();
    let picked = catalog.save_material(picked).await.unwrap();
    let mut job = job(JobState::Preparing);
    let mut reference = LocalImage::new(picked_image.file_url.clone());
    reference.material = Some(asked);
    job.spec.references = vec![reference];
    catalog.save_job(&job).await.unwrap();
    catalog.remove_materials(&[picked.id]).await.unwrap();
    job.state = JobState::Done;
    job.output_file = Some(file_url(folder.path().join("made.mp4")));
    catalog.save_job(&job).await.unwrap();
    let video = catalog.material(job.id).await.unwrap().unwrap();
    let images: Vec<MaterialInput> = catalog
        .made_from(video.reference())
        .await
        .unwrap()
        .into_iter()
        .filter(|input| input.role != MaterialInputRole::SourceVideo)
        .collect();
    assert_eq!(
        images,
        [MaterialInput {
            source: asked,
            role: MaterialInputRole::Reference,
            position: 0
        }]
    );
    assert_eq!(catalog.used_in(asked).await.unwrap(), [video.reference()]);
    assert!(catalog.material(picked.id).await.unwrap().unwrap().is_hidden);
}

#[tokio::test]
async fn draft_final_and_joined_output_keep_their_sources_and_variant_siblings() {
    let folder = tempfile::tempdir().unwrap();
    let catalog = memory_store().await;
    let mut draft = job(JobState::Done);
    draft.output_file = Some(file_url(folder.path().join("draft.mp4")));
    draft.spec.is_draft = true;
    draft.variant_group_id = Some(Uuid::new_v4());
    catalog.save_job(&draft).await.unwrap();
    let mut sibling = job(JobState::Done);
    sibling.variant_group_id = draft.variant_group_id;
    sibling.output_file = Some(file_url(folder.path().join("variant.mp4")));
    catalog.save_job(&sibling).await.unwrap();
    let mut last = job(JobState::Done);
    last.parent_id = Some(draft.id);
    last.spec.mode = Mode::Extend;
    last.output_file = Some(file_url(folder.path().join("extend.mp4")));
    catalog.save_job(&last).await.unwrap();
    catalog
        .save_joined_video(&last, file_url(folder.path().join("joined.mp4")))
        .await
        .unwrap();
    let materials = catalog.materials().await.unwrap();
    let find = |id: Uuid| materials.iter().find(|material| material.id == id).unwrap();
    let draft_material = find(draft.id);
    let sibling_material = find(sibling.id);
    let final_material = find(last.id);
    let joined = materials
        .iter()
        .find(|material| {
            material
                .current
                .file_url
                .as_ref()
                .is_some_and(|url| url.path().ends_with("/joined.mp4"))
        })
        .unwrap();
    assert_eq!(draft_material.variant_group_id, sibling_material.variant_group_id);
    assert!(draft_material.versions().len() == 1 && sibling_material.versions().len() == 1);
    let final_inputs = catalog.made_from(final_material.reference()).await.unwrap();
    assert!(
        final_inputs
            .iter()
            .any(|input| input.source == draft_material.reference())
    );
    let joined_inputs = catalog.made_from(joined.reference()).await.unwrap();
    assert!(
        joined_inputs
            .iter()
            .any(|input| input.source == final_material.reference())
    );
    assert_eq!(catalog.job(joined.id).await.unwrap(), None);
}

#[tokio::test]
async fn unavailable_files_and_removed_sources_retain_relations() {
    let folder = tempfile::tempdir().unwrap();
    let catalog = memory_store().await;
    let image = saved_image(folder.path(), "image");
    let input = catalog.save_image(&image, None, None, &[], None).await.unwrap();
    let mut job = job(JobState::Done);
    let mut reference = LocalImage::new(image.file_url.clone());
    reference.material = Some(input.reference());
    job.spec.references = vec![reference];
    job.output_file = Some(file_url(folder.path().join("missing.mp4")));
    catalog.save_job(&job).await.unwrap();
    catalog.remove_materials(&[input.id]).await.unwrap();
    let kept = catalog.material(input.id).await.unwrap().unwrap();
    assert!(kept.is_hidden);
    assert_eq!(
        kept.current.availability(exists),
        VersionAvailability::Unavailable
    );
    assert_eq!(catalog.used_in(input.reference()).await.unwrap().len(), 1);
    let output = catalog.material(job.id).await.unwrap().unwrap();
    assert_eq!(
        output.current.availability(exists),
        VersionAvailability::Unavailable
    );
    assert_eq!(kept.current.generation, None);
    std::fs::write(image.file_url.to_file_path().unwrap(), b"").unwrap();
    let restored = catalog.material(input.id).await.unwrap().unwrap();
    assert_eq!(
        restored.current.availability(exists),
        VersionAvailability::Available
    );
    assert_eq!(catalog.used_in(input.reference()).await.unwrap().len(), 1);
}

#[tokio::test]
async fn removed_materials_are_listed_again_when_saved_again() {
    let folder = tempfile::tempdir().unwrap();
    let catalog = memory_store().await;
    let remote = account("alice", "remote-image");
    let mut account_image = saved_image(folder.path(), "account");
    account_image.remote_url = Some(Url::parse("https://example.com/account.png").unwrap());
    let imported = catalog
        .save_image(&account_image, Some(remote.clone()), None, &[], None)
        .await
        .unwrap();
    let local_image = saved_image(folder.path(), "local");
    let local = catalog
        .save_image(&local_image, None, None, &[], None)
        .await
        .unwrap();
    catalog.remove_materials(&[imported.id, local.id]).await.unwrap();

    // The account copy is listed from the account again; the local one leaves the Library.
    let removed_account = catalog.material(imported.id).await.unwrap().unwrap();
    assert!(!removed_account.is_hidden);
    assert_eq!(removed_account.current.file_url, None);
    assert_eq!(removed_account.current.remote_url, account_image.remote_url);
    assert!(catalog.material(local.id).await.unwrap().unwrap().is_hidden);

    let reimported = catalog
        .save_image(&account_image, Some(remote), None, &[], None)
        .await
        .unwrap();
    assert_eq!(reimported.id, imported.id);
    assert_eq!(
        reimported.current.file_url.as_ref(),
        Some(&account_image.file_url)
    );
    let mut again = saved_image(folder.path(), "local");
    again.id = local.id;
    let resaved = catalog.save_image(&again, None, None, &[], None).await.unwrap();
    assert!(!resaved.is_hidden);
    let listed = catalog
        .materials()
        .await
        .unwrap()
        .iter()
        .filter(|material| !material.is_hidden)
        .count();
    assert_eq!(listed, 2);
}

async fn break_material_data(catalog: &Store, id: Uuid) {
    sqlx::query("UPDATE materials SET material_data = 'broken catalog entry' WHERE id = ?")
        .bind(id.to_string())
        .execute(catalog.pool())
        .await
        .unwrap();
}

async fn stored_data(catalog: &Store, id: Uuid) -> String {
    sqlx::query_scalar("SELECT material_data FROM materials WHERE id = ?")
        .bind(id.to_string())
        .fetch_one(catalog.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn corrupt_catalog_entry_throws_without_replacing_it() {
    let folder = tempfile::tempdir().unwrap();
    let catalog = memory_store().await;
    let saved = catalog
        .save_image(&saved_image(folder.path(), "image"), None, None, &[], None)
        .await
        .unwrap();
    break_material_data(&catalog, saved.id).await;
    assert!(matches!(
        catalog.materials().await,
        Err(StoreError::Damaged { .. })
    ));
    assert!(catalog.save_material(saved.clone()).await.is_err());
    assert_eq!(stored_data(&catalog, saved.id).await, "broken catalog entry");
}

/// Swift fails the image's own write with a date JSON can't hold; here the image's stored entry is damaged. Either
/// way the input registered before the failure is rolled back with it.
#[tokio::test]
async fn failed_image_write_rolls_back_input_registration() {
    let folder = tempfile::tempdir().unwrap();
    let catalog = memory_store().await;
    let image = saved_image(folder.path(), "image");
    catalog.save_image(&image, None, None, &[], None).await.unwrap();
    break_material_data(&catalog, image.id).await;
    let result = catalog
        .save_image(
            &image,
            None,
            None,
            &[LocalImage::new(file_url(folder.path().join("reference.png")))],
            None,
        )
        .await;
    assert!(result.is_err());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM materials")
        .fetch_one(catalog.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(stored_data(&catalog, image.id).await, "broken catalog entry");
    let other = saved_image(folder.path(), "other");
    catalog.save_image(&other, None, None, &[], None).await.unwrap();
    assert_eq!(catalog.material(other.id).await.unwrap().unwrap().id, other.id);
}

#[tokio::test]
async fn culling_and_the_primary_survive_reopening_and_refreshes() {
    let folder = tempfile::tempdir().unwrap();
    let layout = FileLayout::new(folder.path(), folder.path().join("Media"));
    layout.prepare_directories().unwrap();
    let first = Store::open(&layout.store_path()).await.unwrap();
    let group = Uuid::new_v4();
    let mut draft = job(JobState::Done);
    draft.variant_group_id = Some(group);
    draft.output_file = Some(file_url(folder.path().join("first.mp4")));
    first.save_job(&draft).await.unwrap();
    let mut sibling = job(JobState::Done);
    sibling.variant_group_id = Some(group);
    sibling.output_file = Some(file_url(folder.path().join("second.mp4")));
    first.save_job(&sibling).await.unwrap();
    let other = first
        .save_image(&saved_image(folder.path(), "image"), None, None, &[], None)
        .await
        .unwrap();

    let draft_reference = first.material(draft.id).await.unwrap().unwrap().reference();
    first
        .set_culling(&HashMap::from([
            (draft_reference, Culling::Rejected),
            (other.reference(), Culling::Kept),
        ]))
        .await
        .unwrap();
    first.make_variant_primary(sibling.id).await.unwrap();
    first.make_variant_primary(draft.id).await.unwrap();
    // The engine saving the job again must not reset the verdict.
    first.save_job(&draft).await.unwrap();

    let reopened = Store::open(&layout.store_path()).await.unwrap();
    let materials = reopened.materials().await.unwrap();
    let find = |id: Uuid| materials.iter().find(|material| material.id == id).unwrap();
    assert_eq!(find(draft.id).current.culling, Culling::Rejected);
    assert_eq!(find(sibling.id).current.culling, Culling::Unreviewed);
    assert_eq!(find(other.id).current.culling, Culling::Kept);
    assert!(find(draft.id).is_variant_primary);
    assert!(!find(sibling.id).is_variant_primary);
    assert!(matches!(
        reopened.make_variant_primary(other.id).await,
        Err(StoreError::Material(MaterialError::MissingMaterial))
    ));
}

#[tokio::test]
async fn culling_is_per_version() {
    let folder = tempfile::tempdir().unwrap();
    let catalog = memory_store().await;
    let material = catalog
        .save_image(&saved_image(folder.path(), "image"), None, None, &[], None)
        .await
        .unwrap();
    let original = material.reference();
    catalog
        .set_culling(&HashMap::from([(original, Culling::Rejected)]))
        .await
        .unwrap();
    let mut material = catalog.material(material.id).await.unwrap().unwrap();
    material
        .add_version(version(file_url(folder.path().join("edit.png"))))
        .unwrap();
    let material = catalog.save_material(material).await.unwrap();
    assert_eq!(material.current.culling, Culling::Unreviewed);
    catalog
        .set_culling(&HashMap::from([(material.reference(), Culling::Kept)]))
        .await
        .unwrap();
    let restored = catalog.material(material.id).await.unwrap().unwrap();
    let culling: Vec<Culling> = restored
        .versions()
        .iter()
        .map(|version| version.culling)
        .collect();
    assert_eq!(culling, [Culling::Rejected, Culling::Kept]);
}

#[tokio::test]
async fn loads_five_thousand_materials_within_five_seconds() {
    let catalog = memory_store().await;
    for index in 0..5_000 {
        let kind = if index % 2 == 0 {
            MaterialKind::Image
        } else {
            MaterialKind::Video
        };
        let material = Material::new(
            Uuid::new_v4(),
            kind,
            OffsetDateTime::now_utc(),
            version(file_url(format!("/tmp/catalog/{index}.png"))),
        );
        catalog.save_material(material).await.unwrap();
    }
    let start = Instant::now();
    let loaded = catalog.materials().await.unwrap();
    let elapsed = start.elapsed();
    assert_eq!(loaded.len(), 5_000);
    assert!(elapsed.as_secs_f64() < 5.0, "loaded 5,000 entries in {elapsed:?}");
}

/// The store's own query: kind, star, hidden and rejected filters, order and paging.
#[tokio::test]
async fn materials_matching_filters_sorts_and_pages() {
    let folder = tempfile::tempdir().unwrap();
    let catalog = memory_store().await;
    let mut ids = Vec::new();
    for (index, name) in ["a", "b", "c", "d"].iter().enumerate() {
        let mut image = saved_image(folder.path(), name);
        image.created_at = crate::common::at(1_000 + i64::try_from(index).unwrap());
        ids.push(
            catalog
                .save_image(&image, None, None, &[], None)
                .await
                .unwrap()
                .id,
        );
    }
    let mut video = job(JobState::Done);
    video.output_file = Some(file_url(folder.path().join("v.mp4")));
    catalog.save_job(&video).await.unwrap();
    catalog.set_materials_starred(true, &[ids[1]]).await.unwrap();
    catalog.remove_materials(&[ids[2]]).await.unwrap();
    let rejected = catalog.material(ids[3]).await.unwrap().unwrap().reference();
    catalog
        .set_culling(&HashMap::from([(rejected, Culling::Rejected)]))
        .await
        .unwrap();

    let select = async |query: MaterialQuery| -> Vec<Uuid> {
        catalog
            .materials_matching(&query)
            .await
            .unwrap()
            .iter()
            .map(|material| material.id)
            .collect()
    };
    let images = MaterialQuery {
        kind: Some(MaterialKind::Image),
        ..MaterialQuery::default()
    };
    assert_eq!(select(images.clone()).await, [ids[3], ids[1], ids[0]]);
    assert_eq!(
        select(MaterialQuery {
            hide_rejected: true,
            sort: MaterialSort::OldestFirst,
            ..images.clone()
        })
        .await,
        [ids[0], ids[1]]
    );
    assert_eq!(
        select(MaterialQuery {
            include_hidden: true,
            limit: Some(2),
            offset: 1,
            ..images.clone()
        })
        .await,
        [ids[3], ids[2]]
    );
    assert_eq!(
        select(MaterialQuery {
            starred_only: true,
            ..MaterialQuery::default()
        })
        .await,
        [ids[1]]
    );
    assert_eq!(
        select(MaterialQuery {
            kind: Some(MaterialKind::Video),
            ..MaterialQuery::default()
        })
        .await,
        [video.id]
    );
}
