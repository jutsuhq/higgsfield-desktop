//! `LibraryStarringTests`, `LibraryDeletionTests` and the deletion part of `LibrarySelectionTests`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use jutsu_core::{GenerationLikes, MaterialVersion, PortError, ProviderError, ProviderKind, RemoteJob};
use uuid::Uuid;

use super::{
    Deletion, LibraryStarring, StarError, StarStore, Trash, delete_confirmation, deletion_message,
    earlier_files, image_deletion_message, image_trash_confirmation, trash_files,
};
use crate::library::fixture::{file, history_item, item, minutes_ago, video};
use crate::library::item::LibraryItem;

/// Records every like it is asked for; refuses the IDs in `refused`.
#[derive(Default)]
struct FakeLikes {
    refused: HashSet<String>,
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl GenerationLikes for FakeLikes {
    async fn set_liked(&self, request_id: String, is_liked: bool) -> Result<(), PortError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{request_id} {is_liked}"));
        if self.refused.contains(&request_id) {
            return Err(ProviderError::Server { status: 500 }.into());
        }
        Ok(())
    }
}

#[derive(Default)]
struct FakeStars {
    writes: Mutex<Vec<(Uuid, bool)>>,
}

#[async_trait]
impl StarStore for FakeStars {
    async fn set_starred(&self, is_starred: bool, id: Uuid) -> Result<(), String> {
        self.writes.lock().unwrap().push((id, is_starred));
        Ok(())
    }
}

fn account_item(request_id: &str, owner: Option<&str>, is_starred: bool) -> LibraryItem {
    let mut item = item();
    item.job.provider_kind = ProviderKind::Account;
    item.job.owner = owner.map(str::to_owned);
    item.job.remote = Some(RemoteJob::new(request_id));
    item.is_starred = is_starred;
    item
}

fn starring(likes: &Arc<FakeLikes>) -> LibraryStarring {
    LibraryStarring {
        likes: Some(likes.clone()),
        owner: Some("user_1".to_owned()),
    }
}

#[test]
fn the_like_follows_the_star_only_for_the_connected_accounts_results() {
    let account = starring(&Arc::new(FakeLikes::default()));
    let api_key = LibraryStarring {
        likes: None,
        owner: None,
    };
    let history = LibraryItem::from_history(history_item("h", false));
    assert!(account.syncs_like(&account_item("a", Some("user_1"), false)));
    assert!(!account.syncs_like(&account_item("a", Some("user_2"), false)));
    assert!(account.syncs_like(&history));
    assert!(!account.syncs_like(&item()));
    assert!(!api_key.syncs_like(&account_item("a", Some("user_1"), false)));
    assert!(api_key.can_star(&account_item("a", Some("user_1"), false)));
    assert!(!api_key.can_star(&history));
    assert!(account.can_star(&history));
}

#[tokio::test]
async fn an_accepted_like_keeps_the_star_and_reaches_the_history() {
    let likes = Arc::new(FakeLikes::default());
    let stars = FakeStars::default();
    let item = account_item("a", Some("user_1"), false);
    let mut reported = Vec::new();
    starring(&likes)
        .set_starred(true, &item, &stars, &mut |id, is_liked| {
            reported.push(format!("{id} {is_liked}"));
        })
        .await
        .unwrap();
    assert_eq!(*stars.writes.lock().unwrap(), [(item.id(), true)]);
    assert_eq!(*likes.calls.lock().unwrap(), ["a true"]);
    assert_eq!(reported, ["a true"]);
}

#[tokio::test]
async fn a_refused_like_puts_the_star_back_and_is_returned() {
    let likes = Arc::new(FakeLikes {
        refused: HashSet::from(["a".to_owned()]),
        ..FakeLikes::default()
    });
    let stars = FakeStars::default();
    let item = account_item("a", Some("user_1"), true);
    let mut reported = Vec::new();
    let result = starring(&likes)
        .set_starred(false, &item, &stars, &mut |id, _| reported.push(id.to_owned()))
        .await;
    assert!(matches!(result, Err(StarError::Like(_))));
    assert_eq!(
        *stars.writes.lock().unwrap(),
        [(item.id(), false), (item.id(), true)]
    );
    assert_eq!(reported.len(), 0);
}

#[tokio::test]
async fn a_local_star_never_asks_higgsfield() {
    let likes = Arc::new(FakeLikes::default());
    let stars = FakeStars::default();
    let item = item();
    starring(&likes)
        .set_starred(true, &item, &stars, &mut |_, _| {})
        .await
        .unwrap();
    assert_eq!(*stars.writes.lock().unwrap(), [(item.id(), true)]);
    assert_eq!(likes.calls.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn a_batch_skips_results_already_so_and_stops_at_the_first_refusal() {
    let likes = Arc::new(FakeLikes {
        refused: HashSet::from(["b".to_owned()]),
        ..FakeLikes::default()
    });
    let stars = FakeStars::default();
    let items = [
        account_item("done", Some("user_1"), true),
        account_item("a", Some("user_1"), false),
        account_item("b", Some("user_1"), false),
        account_item("c", Some("user_1"), false),
    ];
    let result = starring(&likes)
        .set_all_starred(true, &items, &stars, &mut |_, _| {})
        .await;
    assert!(result.is_err());
    assert_eq!(*likes.calls.lock().unwrap(), ["a true", "b true"]);
    assert_eq!(
        *stars.writes.lock().unwrap(),
        [
            (items[1].id(), true),
            (items[2].id(), true),
            (items[2].id(), false)
        ]
    );
}

/// Files are there unless listed as missing; moving one listed as stuck fails.
#[derive(Default)]
struct FakeTrash {
    missing: HashSet<PathBuf>,
    stuck: HashSet<PathBuf>,
    trashed: Mutex<Vec<PathBuf>>,
}

impl Trash for FakeTrash {
    fn file_exists(&self, path: &Path) -> bool {
        !self.missing.contains(path)
    }

    fn move_to_trash(&self, path: &Path) -> Result<(), String> {
        if self.stuck.contains(path) {
            return Err("You don't have permission.".to_owned());
        }
        self.trashed.lock().unwrap().push(path.to_owned());
        Ok(())
    }
}

fn path(name: &str) -> PathBuf {
    PathBuf::from(format!("/home/me/Movies/Jutsu Studio/{name}.mp4"))
}

fn result(name: Option<&str>) -> (Uuid, Option<PathBuf>) {
    (Uuid::new_v4(), name.map(path))
}

#[test]
fn everything_deleted_says_nothing() {
    let items = [result(Some("a")), result(None)];
    let trash = FakeTrash::default();
    let deletion = trash_files(&items, &trash, |_| Vec::new());
    assert_eq!(*trash.trashed.lock().unwrap(), [path("a")]);
    assert_eq!(deletion.deleted, [items[0].0, items[1].0]);
    assert_eq!(deletion_message(&deletion, Ok(())), None);
}

#[test]
fn a_file_that_cant_be_trashed_keeps_its_result_and_is_reported() {
    let items = [
        result(Some("kept")),
        result(Some("gone")),
        result(Some("missing")),
    ];
    let trash = FakeTrash {
        stuck: HashSet::from([path("kept")]),
        missing: HashSet::from([path("missing")]),
        ..FakeTrash::default()
    };
    let deletion = trash_files(&items, &trash, |_| Vec::new());
    assert_eq!(deletion.deleted, [items[1].0, items[2].0]);
    assert_eq!(*trash.trashed.lock().unwrap(), [path("gone")]);
    let message = deletion_message(&deletion, Ok(())).unwrap();
    assert_eq!(message.title, "Can't move the file to the Trash");
    assert_eq!(message.message, "You don't have permission.");
}

#[test]
fn several_files_left_are_counted() {
    let items = [result(Some("a")), result(Some("b")), result(Some("c"))];
    let trash = FakeTrash {
        stuck: HashSet::from([path("a"), path("b")]),
        ..FakeTrash::default()
    };
    let deletion = trash_files(&items, &trash, |_| Vec::new());
    assert_eq!(deletion.deleted, [items[2].0]);
    let message = deletion_message(&deletion, Ok(())).unwrap();
    assert_eq!(message.title, "Can't move 2 files to the Trash");
    assert!(message.message.starts_with("They stay in the Library. "));
}

#[test]
fn earlier_versions_go_to_the_trash_with_the_current_file() {
    let items = [result(Some("upscaled")), result(Some("other"))];
    let earlier = path("original");
    let first = items[0].0;
    let trash = FakeTrash::default();
    let deletion = trash_files(&items, &trash, |id| {
        if id == first {
            vec![earlier.clone()]
        } else {
            Vec::new()
        }
    });
    assert_eq!(
        *trash.trashed.lock().unwrap(),
        [earlier.clone(), path("upscaled"), path("other")]
    );
    assert_eq!(deletion.deleted.len(), 2);

    let stuck = FakeTrash {
        stuck: HashSet::from([earlier.clone()]),
        ..FakeTrash::default()
    };
    let kept = trash_files(&items[..1], &stuck, |_| vec![earlier.clone()]);
    assert_eq!(stuck.trashed.lock().unwrap().len(), 0);
    assert_eq!(kept.deleted.len(), 0);
}

#[test]
fn a_failed_save_is_reported_before_any_trash_failure() {
    let deletion = Deletion {
        deleted: vec![Uuid::new_v4()],
        not_trashed: vec!["You don't have permission.".to_owned()],
    };
    let message = deletion_message(&deletion, Err("The disk is full.".to_owned())).unwrap();
    assert_eq!(message.title, "Can't remove the result from the Library");
    assert_eq!(message.message, "The disk is full.");
}

#[test]
fn the_delete_confirmation_names_the_count() {
    assert_eq!(delete_confirmation(1).0, "Delete from library?");
    assert_eq!(delete_confirmation(3).0, "Delete 3 results from library?");
    assert!(
        delete_confirmation(3)
            .1
            .starts_with("The video files and their earlier versions")
    );
}

#[test]
fn earlier_files_are_the_other_versions_the_app_keeps() {
    let mut material = video(1.0);
    let mut original = MaterialVersion::new(Uuid::new_v4(), minutes_ago(2.0));
    original.file_url = Some(file("/home/me/Movies/Jutsu Studio/original.mp4"));
    let mut elsewhere = MaterialVersion::new(Uuid::new_v4(), minutes_ago(2.0));
    elsewhere.file_url = Some(file("/home/me/Desktop/picked.mp4"));
    let current = material.current.clone();
    material.current = original;
    material.add_version(elsewhere).unwrap();
    material.add_version(current).unwrap();
    let files = earlier_files(&material, |path| {
        path.starts_with(crate::testing::absolute("/home/me/Movies/Jutsu Studio"))
    });
    assert_eq!(
        files,
        [crate::testing::absolute(
            "/home/me/Movies/Jutsu Studio/original.mp4"
        )]
    );
}

/// `LibraryView`'s image trash: its own question, and its own title when the Trash refuses.
#[test]
fn images_ask_and_fail_in_their_own_words() {
    let (title, message) = image_trash_confirmation(2);
    assert_eq!(title, "Move 2 images to the Trash?");
    assert_eq!(
        message,
        "Their earlier versions made in Jutsu Studio go too. You can put them back from the Trash in your file manager."
    );
    let deletion = Deletion {
        deleted: Vec::new(),
        not_trashed: vec!["You don't have permission.".to_owned()],
    };
    let failure = image_deletion_message(&deletion, Ok(())).unwrap();
    assert_eq!(failure.title, "Couldn't move the images to the Trash");
    assert_eq!(failure.message, "You don't have permission.");
    assert!(image_deletion_message(&Deletion::default(), Ok(())).is_none());
}

/// `LibraryBulkActionsTests.countsAndFiles` and `starsAllUnlessEveryOneIsStarred`.
#[test]
fn bulk_actions_delete_the_library_ones_and_star_unless_all_are() {
    let entry = |needs_import: bool, can_star: bool, is_starred: bool| super::BulkEntry {
        id: Uuid::new_v4(),
        needs_import,
        can_star,
        is_starred,
        can_render_final: false,
        image: None,
        has_file: !needs_import,
    };
    let starred = entry(false, true, true);
    let plain = entry(false, true, false);
    let remote = entry(true, false, false);
    let bulk = super::BulkActions::new(&[starred.clone(), plain.clone(), remote]);
    assert_eq!(bulk.title, "3 selected");
    assert_eq!(bulk.deletable_ids, [starred.id.to_string(), plain.id.to_string()]);
    assert_eq!(bulk.starrable_ids.len(), 2);
    assert!(bulk.stars_all);
    assert_eq!(bulk.star_title, "Star");
    assert_eq!(bulk.file_ids, [starred.id.to_string(), plain.id.to_string()]);
    assert!(!super::BulkActions::new(&[starred.clone(), starred]).stars_all);
}

/// `MenuCommandsTests`: Unstar when every one is starred; Render Final… for one draft here only.
#[test]
fn the_library_menu_unstars_all_starred_and_renders_one_final() {
    let entry = |is_starred: bool, can_render_final: bool| super::BulkEntry {
        id: Uuid::new_v4(),
        needs_import: false,
        can_star: true,
        is_starred,
        can_render_final,
        image: None,
        has_file: true,
    };
    let draft = entry(true, true);
    let single = super::BulkActions::new(std::slice::from_ref(&draft));
    assert_eq!(single.star_title, "Unstar");
    assert_eq!(single.render_final_id, Some(draft.id.to_string()));
    assert_eq!(
        super::BulkActions::new(&[entry(false, false)]).render_final_id,
        None
    );
    assert_eq!(
        super::BulkActions::new(&[draft.clone(), entry(false, true)]).render_final_id,
        None
    );
    assert_eq!(super::BulkActions::new(&[]).render_final_id, None);
}

/// `LibraryImageSelectionBar`: Compare for two chosen images with one here, picking the ones that can be picked.
#[test]
fn compare_is_offered_for_two_images_with_one_here() {
    use super::ImageFile;
    let entry = |image: Option<ImageFile>| super::BulkEntry {
        id: Uuid::new_v4(),
        needs_import: image == Some(ImageFile::OnHiggsfield),
        can_star: true,
        is_starred: false,
        can_render_final: false,
        image,
        has_file: false,
    };
    let ids = |entries: &[&super::BulkEntry]| {
        entries
            .iter()
            .map(|entry| entry.id.to_string())
            .collect::<Vec<_>>()
    };
    let compare = |entries: &[&super::BulkEntry]| {
        let owned: Vec<super::BulkEntry> = entries.iter().map(|entry| (*entry).clone()).collect();
        super::BulkActions::new(&owned).compare_ids
    };
    let here = entry(Some(ImageFile::Here));
    let other = entry(Some(ImageFile::Here));
    let remote = entry(Some(ImageFile::OnHiggsfield));
    let second_remote = entry(Some(ImageFile::OnHiggsfield));
    let missing = entry(Some(ImageFile::Missing));
    let video = entry(None);
    assert_eq!(compare(&[&here, &other]), ids(&[&here, &other]));
    assert_eq!(compare(&[&remote, &here]), ids(&[&remote, &here]));
    assert_eq!(compare(&[&here, &missing]), ids(&[&here]));
    assert_eq!(compare(&[&remote, &second_remote]), Vec::<String>::new());
    assert_eq!(compare(&[&remote, &missing]), Vec::<String>::new());
    assert_eq!(compare(&[&here, &video]), Vec::<String>::new());
    assert_eq!(compare(&[&here, &other, &remote]), Vec::<String>::new());
    assert_eq!(compare(&[&here]), Vec::<String>::new());
}
