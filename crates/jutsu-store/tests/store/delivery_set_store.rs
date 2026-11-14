//! `DeliverySetStoreTests`: frozen sets, and edits brought back as new versions.

use std::path::Path;

use jutsu_core::{
    DeliveryFolder, DeliveryPlan, DeliveryResultStatus, DeliveryScope, DeliverySet, DeliverySetError,
    JobState, Material, MaterialError, MaterialInput, MaterialInputRole, MaterialKind, MaterialReference,
    MaterialVersion, OutputMedia, PixelSize, SavedImage,
};
use jutsu_store::{Store, StoreError};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::common::{at, file_url, generation, job, memory_store, write_file};

/// A saved image whose file holds `contents`.
async fn saved_image(store: &Store, folder: &Path, name: &str, contents: &str) -> Material {
    let file = write_file(&folder.join(format!("{name}.png")), contents);
    let image = SavedImage {
        id: Uuid::new_v4(),
        file_url: file,
        prompt: "a lighthouse".to_owned(),
        model: "soul".to_owned(),
        aspect_ratio: Some("1:1".to_owned()),
        created_at: at(1),
        remote_url: None,
        is_starred: false,
        title: None,
    };
    store
        .save_image(
            &image,
            None,
            Some(generation("a lighthouse", "soul", Some("{}"))),
            &[],
            None,
        )
        .await
        .unwrap()
}

fn new_set(id: Uuid, name: &str, materials: &[Material]) -> DeliverySet {
    DeliverySet::new(
        id,
        name,
        OffsetDateTime::now_utc(),
        DeliverySet::items_from(materials),
        DeliverySet::default_outputs(),
    )
}

fn exported(set: &DeliverySet, folder: &Path) -> DeliverySet {
    let mut set = set.clone();
    let tasks = DeliveryPlan::tasks(&set, DeliveryScope::All, |_| false);
    set.record(
        tasks
            .iter()
            .map(|task| task.result(DeliveryResultStatus::Done, None))
            .collect(),
        DeliveryFolder {
            url: file_url(folder),
        },
        at(100),
    );
    set
}

fn parent(source: MaterialReference) -> MaterialInput {
    MaterialInput {
        source,
        role: MaterialInputRole::Parent,
        position: 0,
    }
}

#[tokio::test]
async fn a_set_exported_yesterday_still_holds_its_versions_after_new_edits() {
    let folder = tempfile::tempdir().unwrap();
    let store = memory_store().await;
    let material = saved_image(&store, folder.path(), "hero", "original").await;
    let set = exported(
        &new_set(Uuid::new_v4(), "Client", std::slice::from_ref(&material)),
        folder.path(),
    );
    store.save_delivery_set(&set).await.unwrap();
    let edit = write_file(&folder.path().join("hero edited.png"), "edited");

    let edited = store
        .add_edited_version(material.reference(), edit, None, true, OffsetDateTime::now_utc())
        .await
        .unwrap();
    let stored = store.delivery_sets().await.unwrap().remove(0);

    assert_ne!(edited.current.id, material.current.id);
    let references: Vec<_> = stored.items().iter().map(|item| item.reference).collect();
    assert_eq!(references, [material.reference()]);
    assert!(stored.is_frozen());
    assert_eq!(stored, set);
}

#[tokio::test]
async fn a_frozen_set_refuses_other_versions_and_unfreezing() {
    let folder = tempfile::tempdir().unwrap();
    let store = memory_store().await;
    let first = saved_image(&store, folder.path(), "one", "original").await;
    let second = saved_image(&store, folder.path(), "two", "original").await;
    let set = exported(&new_set(Uuid::new_v4(), "Client", &[first]), folder.path());
    store.save_delivery_set(&set).await.unwrap();

    let swapped = new_set(set.id, &set.name, &[second]);
    assert!(matches!(
        store.save_delivery_set(&swapped).await,
        Err(StoreError::DeliverySet(DeliverySetError::Frozen))
    ));
    let unfrozen = DeliverySet::new(
        set.id,
        &set.name,
        set.created_at,
        set.items().to_vec(),
        set.outputs.clone(),
    );
    assert!(matches!(
        store.save_delivery_set(&unfrozen).await,
        Err(StoreError::DeliverySet(DeliverySetError::Frozen))
    ));

    let mut noted = set.clone();
    noted.note = "Approved by Anna".to_owned();
    noted.review_link = "https://review.example/42".to_owned();
    store.save_delivery_set(&noted).await.unwrap();
    assert_eq!(store.delivery_sets().await.unwrap(), [noted]);
}

#[tokio::test]
async fn an_unexported_set_can_change_its_versions() {
    let folder = tempfile::tempdir().unwrap();
    let store = memory_store().await;
    let first = saved_image(&store, folder.path(), "one", "original").await;
    let second = saved_image(&store, folder.path(), "two", "original").await;
    let mut set = new_set(Uuid::new_v4(), "Draft", std::slice::from_ref(&first));
    store.save_delivery_set(&set).await.unwrap();

    set.set_items(DeliverySet::items_from(&[first, second])).unwrap();
    store.save_delivery_set(&set).await.unwrap();

    assert_eq!(store.delivery_sets().await.unwrap()[0].items().len(), 2);
    store.delete_delivery_set(set.id).await.unwrap();
    assert!(store.delivery_sets().await.unwrap().is_empty());
}

#[tokio::test]
async fn an_edit_brought_back_becomes_a_new_version_linked_to_its_source() {
    let folder = tempfile::tempdir().unwrap();
    let store = memory_store().await;
    let material = saved_image(&store, folder.path(), "hero", "original").await;
    let source = material.reference();
    let original_file = material.current.file_url.clone().unwrap();
    let edit = write_file(&folder.path().join("hero edited.png"), "edited in Photoshop");

    store
        .add_edited_version(source, edit.clone(), None, true, at(50))
        .await
        .unwrap();
    let stored = store.material(material.id).await.unwrap().unwrap();

    assert_eq!(stored.versions().len(), 2);
    assert_eq!(stored.current.file_url.as_ref(), Some(&edit));
    assert_eq!(stored.current.inputs, [parent(source)]);
    let generation = stored.current.generation.as_ref().unwrap();
    assert_eq!(generation.prompt, "a lighthouse");
    assert_eq!(generation.parameters, None);
    assert_eq!(*stored.versions()[0], material.current);
    let contents = std::fs::read_to_string(original_file.to_file_path().unwrap()).unwrap();
    assert_eq!(contents, "original");
    assert_eq!(
        store.made_from(stored.reference()).await.unwrap(),
        [parent(source)]
    );
    assert_eq!(store.used_in(source).await.unwrap(), [stored.reference()]);
}

#[tokio::test]
async fn a_version_made_on_this_computer_carries_its_own_length_and_size() {
    let folder = tempfile::tempdir().unwrap();
    let store = memory_store().await;
    let mut done = job(JobState::Done);
    done.output_media = Some(OutputMedia {
        duration: 5.0,
        pixel_size: PixelSize::new(854, 480),
    });
    let original = write_file(&folder.path().join("rooftop.mp4"), "original");
    let mut version = MaterialVersion::new(Uuid::new_v4(), OffsetDateTime::now_utc());
    version.file_url = Some(original.clone());
    version.video = Some(Box::new(done.clone()));
    let material = store
        .save_material(Material::new(
            done.id,
            MaterialKind::Video,
            OffsetDateTime::now_utc(),
            version,
        ))
        .await
        .unwrap();
    let slow = write_file(&folder.path().join("rooftop slow 2x.mp4"), "slow");
    let media = OutputMedia {
        duration: 10.0,
        pixel_size: PixelSize::new(854, 480),
    };

    let stored = store
        .add_edited_version(
            material.reference(),
            slow.clone(),
            Some(media),
            true,
            OffsetDateTime::now_utc(),
        )
        .await
        .unwrap();

    let video = stored.current.video.as_ref().unwrap();
    assert_eq!(video.output_media, Some(media));
    assert_eq!(video.output_file.as_ref(), Some(&slow));
    assert_eq!(stored.current.pixel_size, Some(media.pixel_size));
    assert_eq!(stored.current.inputs, [parent(material.reference())]);
    let first_media = stored.versions()[0].video.as_ref().unwrap().output_media.unwrap();
    assert!((first_media.duration - 5.0).abs() < f64::EPSILON);
    let contents = std::fs::read_to_string(original.to_file_path().unwrap()).unwrap();
    assert_eq!(contents, "original");
}

#[tokio::test]
async fn an_edit_of_a_missing_version_is_refused() {
    let folder = tempfile::tempdir().unwrap();
    let store = memory_store().await;
    let material = saved_image(&store, folder.path(), "hero", "original").await;
    let missing = MaterialReference::new(material.id, Uuid::new_v4());

    let result = store
        .add_edited_version(
            missing,
            file_url(folder.path().join("x.png")),
            None,
            true,
            OffsetDateTime::now_utc(),
        )
        .await;
    assert!(matches!(
        result,
        Err(StoreError::Material(MaterialError::MissingMaterial))
    ));
    assert_eq!(
        store
            .material(material.id)
            .await
            .unwrap()
            .unwrap()
            .versions()
            .len(),
        1
    );
}
