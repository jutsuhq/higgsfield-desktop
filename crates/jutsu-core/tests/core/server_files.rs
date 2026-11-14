//! `ServerFilesTests.swift`. Paths stand in for Swift's file URLs.

use std::path::{Component, Path, PathBuf};

use jutsu_core::*;
use url::Url;

fn folder() -> &'static Path {
    Path::new("/tmp/jutsu/motion-library")
}

fn file(names: &[&str], url: &str, allowed: &[&str]) -> Option<PathBuf> {
    let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    ServerFileName::file(folder(), &names, &Url::parse(url).unwrap(), allowed)
}

fn video(names: &[&str], url: &str) -> Option<PathBuf> {
    file(names, url, &ServerFileName::VIDEO_EXTENSIONS)
}

#[test]
fn plain_names_are_kept() {
    let placed = video(&["preset-1", "Dance Move"], "https://cdn.example/clip.MOV").unwrap();
    assert_eq!(
        placed,
        Path::new("/tmp/jutsu/motion-library/preset-1/Dance Move.mov")
    );
}

#[test]
fn dot_dot_names_stay_inside_the_folder() {
    let placed = video(&["..", "default"], "https://cdn.example/x.store").unwrap();
    assert_eq!(
        placed,
        Path::new("/tmp/jutsu/motion-library/Untitled/default.mp4")
    );
}

#[test]
fn separators_cannot_climb_out() {
    let placed = video(
        &["../../..", "..\\..\\evil", "a/../../b"],
        "https://cdn.example/x.mp4",
    )
    .unwrap();
    assert_eq!(
        placed.parent().and_then(Path::parent).and_then(Path::parent),
        Some(folder())
    );
    assert!(placed.starts_with(folder()) && placed != folder());
    let inner = placed.strip_prefix(folder()).unwrap();
    assert!(
        !inner
            .components()
            .any(|component| component == Component::ParentDir)
    );
}

#[test]
fn extensions_outside_the_allowlist_fall_back() {
    let images = ServerFileName::IMAGE_EXTENSIONS;
    let extension = |url| {
        file(&["id"], url, &images)
            .unwrap()
            .extension()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    };
    assert_eq!(extension("https://cdn.example/a.webp"), "webp");
    assert_eq!(extension("https://cdn.example/a.HEIC"), "heic");
    assert_eq!(extension("https://cdn.example/a.store"), "jpg");
    assert_eq!(extension("https://cdn.example/a"), "jpg");
}

#[test]
fn empty_names_get_a_placeholder() {
    let placed = video(&["  ", "..."], "https://cdn.example/a.mp4");
    assert_eq!(
        placed.as_deref(),
        Some(Path::new("/tmp/jutsu/motion-library/Untitled/Untitled.mp4"))
    );
}

#[test]
fn containment_is_checked() {
    assert!(ServerFileName::is_inside(&folder().join("a.mp4"), folder()));
    assert!(!ServerFileName::is_inside(&folder().join("../a.mp4"), folder()));
    assert!(!ServerFileName::is_inside(folder(), folder()));
    assert!(!ServerFileName::is_inside(
        Path::new("/tmp/jutsu/motion-library-2/a.mp4"),
        folder()
    ));
}
