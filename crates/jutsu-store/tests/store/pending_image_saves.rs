//! `PendingImageSavesTests`.

use std::collections::HashSet;

use jutsu_core::PendingImage;
use jutsu_store::PendingImageSaves;
use url::Url;
use uuid::Uuid;

use crate::common::at;

fn pending(name: &str) -> PendingImage {
    PendingImage::new(
        Uuid::new_v4(),
        Url::parse(&format!("https://cdn.example.com/{name}.png")).unwrap(),
        name,
        "Soul",
        None,
        at(0),
    )
}

#[tokio::test]
async fn pending_images_are_kept_until_removed() {
    let folder = tempfile::tempdir().unwrap();
    let index = PendingImageSaves::new(folder.path());
    let first = pending("first");
    let second = pending("second");
    index
        .add_pending(vec![first.clone(), second.clone()])
        .await
        .unwrap();
    index.add_pending(vec![first.clone()]).await.unwrap();
    assert_eq!(index.pending().await.unwrap(), [first.clone(), second.clone()]);

    index.remove_pending(&HashSet::from([first.id])).await.unwrap();
    assert_eq!(
        PendingImageSaves::new(folder.path()).pending().await.unwrap(),
        [second]
    );
    assert!(folder.path().join("pending-images.json").exists());
}

/// The journal is read strictly: a file that doesn't decode fails and is left as it is.
#[tokio::test]
async fn an_unreadable_journal_fails_and_is_kept() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("pending-images.json");
    std::fs::write(&path, "not json").unwrap();
    let index = PendingImageSaves::new(folder.path());

    assert!(index.pending().await.is_err());
    assert!(index.add_pending(vec![pending("new")]).await.is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "not json");
}
