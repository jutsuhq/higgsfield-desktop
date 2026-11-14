//! `MaterialVersionsStoreTests`: versions as the catalog stores them, for Compare with Original and Delete from
//! Library. A material's first version has the material's own ID. The Swift tests go on to build the compare pairs
//! and the files to delete with `Presentation` (`MaterialLibrary`, `MaterialVersions`), which isn't ported yet;
//! these assert what the store gives those rules.

use std::path::Path;

use jutsu_core::{DeliverySet, JobState, MaterialError, SavedImage};
use jutsu_store::StoreError;
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::common::{at, generation, job, memory_store, write_file};

fn file(folder: &Path, name: &str) -> Url {
    write_file(&folder.join(name), name)
}

fn file_names(urls: &[Option<&Url>]) -> Vec<String> {
    urls.iter()
        .map(|url| {
            url.unwrap()
                .path_segments()
                .unwrap()
                .next_back()
                .unwrap()
                .replace("%20", " ")
        })
        .collect()
}

fn lighthouse(folder: &Path) -> SavedImage {
    SavedImage {
        id: Uuid::new_v4(),
        file_url: file(folder, "lighthouse.png"),
        prompt: "a lighthouse".to_owned(),
        model: "soul".to_owned(),
        aspect_ratio: Some("1:1".to_owned()),
        created_at: at(1),
        remote_url: None,
        is_starred: false,
        title: None,
    }
}

#[tokio::test]
async fn a_finished_video_with_a_new_version_compares_with_its_original() {
    let folder = tempfile::tempdir().unwrap();
    let store = memory_store().await;
    let mut done = job(JobState::Done);
    done.output_file = Some(file(folder.path(), "rooftop.mp4"));
    store.save_job(&done).await.unwrap();
    let original = store.material(done.id).await.unwrap().unwrap();

    store
        .add_edited_version(
            original.reference(),
            file(folder.path(), "rooftop 4x.mp4"),
            None,
            true,
            OffsetDateTime::now_utc(),
        )
        .await
        .unwrap();
    let material = store.material(done.id).await.unwrap().unwrap();

    assert_eq!(material.versions()[0].id, original.current.id);
    assert_eq!(original.current.id, done.id);
    let videos: Vec<Option<&Url>> = material
        .versions()
        .iter()
        .map(|version| {
            version
                .video
                .as_ref()
                .and_then(|video| video.output_file.as_ref())
        })
        .collect();
    assert_eq!(file_names(&videos), ["rooftop.mp4", "rooftop 4x.mp4"]);
    assert_ne!(material.versions()[0].id, material.current.id);
}

#[tokio::test]
async fn a_saved_image_with_a_new_version_compares_with_its_original() {
    let folder = tempfile::tempdir().unwrap();
    let store = memory_store().await;
    let saved = store
        .save_image(
            &lighthouse(folder.path()),
            None,
            Some(generation("a lighthouse", "soul", None)),
            &[],
            None,
        )
        .await
        .unwrap();

    let material = store
        .add_edited_version(
            saved.reference(),
            file(folder.path(), "lighthouse 4x.png"),
            None,
            true,
            OffsetDateTime::now_utc(),
        )
        .await
        .unwrap();

    assert_eq!(material.versions()[0].id, saved.current.id);
    let files: Vec<Option<&Url>> = material
        .versions()
        .iter()
        .map(|version| version.file_url.as_ref())
        .collect();
    assert_eq!(file_names(&files), ["lighthouse.png", "lighthouse 4x.png"]);
}

#[tokio::test]
async fn a_version_a_delivery_set_holds_stays_on_delete() {
    let folder = tempfile::tempdir().unwrap();
    let store = memory_store().await;
    let mut done = job(JobState::Done);
    done.output_file = Some(file(folder.path(), "rooftop.mp4"));
    store.save_job(&done).await.unwrap();
    let original = store.material(done.id).await.unwrap().unwrap();
    let set = DeliverySet::new(
        Uuid::new_v4(),
        "Client",
        OffsetDateTime::now_utc(),
        DeliverySet::items_from(std::slice::from_ref(&original)),
        DeliverySet::default_outputs(),
    );
    store.save_delivery_set(&set).await.unwrap();
    let now = OffsetDateTime::now_utc();
    let once = store
        .add_edited_version(
            original.reference(),
            file(folder.path(), "rooftop 4x.mp4"),
            None,
            true,
            now,
        )
        .await
        .unwrap();
    let twice = store
        .add_edited_version(
            once.reference(),
            file(folder.path(), "rooftop 4x 60fps.mp4"),
            None,
            true,
            now,
        )
        .await
        .unwrap();

    let held: Vec<_> = store.delivery_sets().await.unwrap()[0]
        .items()
        .iter()
        .map(|item| item.reference)
        .collect();
    assert_eq!(held, [original.reference()]);
    let earlier: Vec<Option<&Url>> = twice
        .previous_versions()
        .iter()
        .map(|version| version.file_url.as_ref())
        .collect();
    assert_eq!(file_names(&earlier), ["rooftop.mp4", "rooftop 4x.mp4"]);
}

#[tokio::test]
async fn a_run_on_this_computer_does_not_bring_back_a_deleted_item() {
    let folder = tempfile::tempdir().unwrap();
    let store = memory_store().await;
    let saved = store
        .save_image(&lighthouse(folder.path()), None, None, &[], None)
        .await
        .unwrap();
    store.remove_materials(&[saved.id]).await.unwrap();

    let result = store
        .add_edited_version(
            saved.reference(),
            file(folder.path(), "lighthouse 4x.png"),
            None,
            false,
            OffsetDateTime::now_utc(),
        )
        .await;
    assert!(matches!(
        result,
        Err(StoreError::Material(MaterialError::NotListed))
    ));
    let stored = store.material(saved.id).await.unwrap().unwrap();
    assert!(stored.is_hidden);
    assert_eq!(stored.versions().len(), 1);
}
