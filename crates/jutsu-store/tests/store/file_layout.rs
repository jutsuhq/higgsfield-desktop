//! `FileLayoutTests`.

use std::collections::HashSet;
use std::path::Path;

use jutsu_store::FileLayout;
use url::Url;
use uuid::Uuid;

use crate::common::{file_url, write_file};

fn layout(temp: &Path) -> FileLayout {
    FileLayout::new(temp.join("support"), temp.join("Movies"))
}

fn exists(path: &Path) -> bool {
    path.exists()
}

#[test]
fn layout_paths() {
    let temp = tempfile::tempdir().unwrap();
    let layout = layout(temp.path());
    assert_eq!(layout.store_path().file_name().unwrap(), "default.store");
    assert_eq!(layout.sources_folder(), layout.root.join("sources"));
    assert!(layout.preset_cache_folder().ends_with("cache/presets"));
}

#[test]
fn clear_cache_removes_only_sources_cache_and_downloaded_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let layout = layout(temp.path());
    layout.prepare_directories().unwrap();
    std::fs::write(layout.store_path(), "store").unwrap();
    for folder in layout.downloaded_input_folders() {
        write_file(&folder.join("input.mp4"), "input");
    }
    std::fs::write(layout.sources_folder().join("clip.mp4"), "clip").unwrap();
    std::fs::write(layout.preset_cache_folder().join("catalog.json"), "{}").unwrap();
    let output = layout.output_folder().unwrap();
    std::fs::write(output.join("result.mp4"), "result").unwrap();
    let image = write_file(&temp.path().join("face.png"), "png");
    let copies = layout.copy_template_images(&[image], Uuid::new_v4()).unwrap();

    layout.clear_cache(&[]).unwrap();
    layout.clear_cache(&[]).unwrap();

    assert!(!exists(&layout.sources_folder()));
    assert!(!exists(&layout.cache_folder()));
    assert!(!exists(&layout.motion_library_folder()));
    assert!(!exists(&layout.account_media_folder()));
    assert!(exists(&layout.store_path()));
    assert!(exists(&output.join("result.mp4")));
    assert!(copies.iter().all(|copy| exists(&copy.to_file_path().unwrap())));
}

#[test]
fn clear_cache_keeps_the_files_it_is_told_to_keep() {
    let temp = tempfile::tempdir().unwrap();
    let layout = layout(temp.path());
    layout.prepare_directories().unwrap();
    let pasted = layout.pasted_images_folder().join("paste.png");
    let link = layout.linked_media_folder().join("a/clip.mp4");
    let other_link = layout.linked_media_folder().join("b/clip.mp4");
    let generated = layout.generated_images_folder().join("c/image.png");
    let other_paste = layout.pasted_images_folder().join("old.png");
    for file in [&pasted, &link, &other_link, &generated, &other_paste] {
        write_file(file, "media");
    }
    let elsewhere = file_url(temp.path().join("Desktop/face.png"));
    let remote = Url::parse("https://example.com/a.png").unwrap();

    layout
        .clear_cache(&[file_url(&pasted), file_url(&link), elsewhere, remote])
        .unwrap();

    assert!(exists(&pasted));
    assert!(exists(&link));
    assert!(!exists(&other_paste));
    assert!(!exists(other_link.parent().unwrap()));
    assert!(!exists(&layout.generated_images_folder()));
    assert!(!exists(&layout.sources_folder()));
    assert!(!exists(&layout.motion_library_folder()));
}

#[test]
fn template_images_survive_the_originals_and_are_removed_with_the_template() {
    let temp = tempfile::tempdir().unwrap();
    let layout = layout(temp.path());
    let first = write_file(&temp.path().join("a/face.png"), "one");
    let second = write_file(&temp.path().join("b/face.png"), "two");
    let id = Uuid::new_v4();

    let copies = layout
        .copy_template_images(&[first.clone(), second.clone()], id)
        .unwrap();
    std::fs::remove_file(first.to_file_path().unwrap()).unwrap();
    std::fs::remove_file(second.to_file_path().unwrap()).unwrap();

    let paths: Vec<_> = copies.iter().map(|copy| copy.to_file_path().unwrap()).collect();
    let names: Vec<_> = paths
        .iter()
        .map(|path| path.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(names, ["1-face.png", "2-face.png"]);
    let contents: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect();
    assert_eq!(contents, ["one", "two"]);
    assert!(
        paths
            .iter()
            .all(|path| path.parent().unwrap() == layout.template_folder(id))
    );
    layout.remove_template_images(id).unwrap();
    layout.remove_template_images(id).unwrap();
    assert!(!exists(&layout.template_folder(id)));
}

#[test]
fn failed_copy_leaves_no_folder() {
    let temp = tempfile::tempdir().unwrap();
    let layout = layout(temp.path());
    let id = Uuid::new_v4();
    assert!(
        layout
            .copy_template_images(&[file_url(temp.path().join("missing.png"))], id)
            .is_err()
    );
    assert!(!exists(&layout.template_folder(id)));
}

#[test]
fn prune_removes_only_folders_without_a_template() {
    let temp = tempfile::tempdir().unwrap();
    let layout = layout(temp.path());
    let image = write_file(&temp.path().join("face.png"), "png");
    let kept = Uuid::new_v4();
    let orphan = Uuid::new_v4();
    layout
        .copy_template_images(std::slice::from_ref(&image), kept)
        .unwrap();
    layout.copy_template_images(&[image], orphan).unwrap();

    layout.prune_template_images(&HashSet::from([kept])).unwrap();

    assert!(exists(&layout.template_folder(kept)));
    assert!(!exists(&layout.template_folder(orphan)));
}

#[test]
fn output_folder_is_created_on_demand() {
    let temp = tempfile::tempdir().unwrap();
    let layout = layout(temp.path());
    assert!(!exists(&layout.default_output_folder));
    assert_eq!(layout.output_folder().unwrap(), layout.default_output_folder);
    assert!(exists(&layout.default_output_folder));
}
