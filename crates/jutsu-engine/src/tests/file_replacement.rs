//! Port of `FileReplacementTests`.

use std::path::PathBuf;

use crate::FileReplacement;

#[test]
fn a_new_name_is_created() {
    let folder = tempfile::tempdir().unwrap();
    let target = folder.path().join("Joined.mp4");
    let finished = folder.path().join("scratch");
    std::fs::write(&finished, b"new").unwrap();

    FileReplacement::move_file(&finished, &target).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"new");
}

#[test]
fn a_finished_file_replaces_the_one_at_the_target() {
    let folder = tempfile::tempdir().unwrap();
    let (finished, target) = (folder.path().join("new"), folder.path().join("old"));
    std::fs::write(&finished, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    FileReplacement::move_file(&finished, &target).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    assert!(!finished.exists());
}

#[test]
fn a_failed_move_keeps_both_files() {
    let folder = tempfile::tempdir().unwrap();
    let missing = folder.path().join("missing");
    let target = folder.path().join("old");
    std::fs::write(&target, b"old").unwrap();
    assert!(FileReplacement::move_file(&missing, &target).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
}

/// Runs, as in Swift, only with `JUTSU_REPLACEMENT_TEST_VOLUME` naming an isolated small volume (a tmpfs in CI, a
/// mounted scratch image on a Mac): without one it checks nothing.
#[test]
fn a_cross_volume_copy_that_runs_out_of_space_preserves_the_old_file() {
    let Some(volume) = std::env::var_os("JUTSU_REPLACEMENT_TEST_VOLUME").map(PathBuf::from) else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir_in(&volume).unwrap();
    let free = fs4::available_space(&volume).unwrap();
    assert!(free < 128 << 20, "Use a small isolated test volume");
    let target = destination.path().join("result");
    std::fs::write(&target, b"old").unwrap();
    let source = folder.path().join("scratch");
    std::fs::File::create(&source)
        .unwrap()
        .set_len(free + (1 << 20))
        .unwrap();

    assert!(FileReplacement::move_file(&source, &target).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    assert!(source.exists());
    std::fs::write(&source, b"new").unwrap();
    FileReplacement::move_file(&source, &target).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    assert!(!source.exists());
}
