//! `OutputSubfolderTests.swift`. Paths stand in for Swift's directory URLs.

use std::collections::HashSet;
use std::path::Path;

use jutsu_core::*;

fn output() -> &'static Path {
    Path::new("/home/me/Movies/Jutsu Studio")
}

#[test]
fn each_kind_has_a_named_folder_of_its_own_inside_the_output_folder() {
    let folders: Vec<_> = OutputSubfolder::ALL
        .iter()
        .map(|folder| folder.path(output()))
        .collect();
    assert_eq!(
        folders.iter().collect::<HashSet<_>>().len(),
        OutputSubfolder::ALL.len()
    );
    for folder in &folders {
        assert_eq!(folder.parent(), Some(output()));
    }
    assert_eq!(
        OutputSubfolder::Videos.path(output()).file_name().unwrap(),
        "Videos"
    );
    assert_eq!(
        OutputSubfolder::Images.path(output()).file_name().unwrap(),
        "Images"
    );
}
