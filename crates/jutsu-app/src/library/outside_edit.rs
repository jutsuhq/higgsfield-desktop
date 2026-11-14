//! The round trip through another app (`OutsideEdit.swift`): "Edit in…" opens a working copy of an exact version,
//! never the original, and the edited file comes back (picked with Import as New Version) as a copy in the output
//! folder that becomes the material's new version, linked to the version it was made from.
//!
//! A working copy sits in `<root>/<materialID>_<versionID>_<n>/`, so a file saved there, in place or under another
//! name, still says which version it was made from after a relaunch.

use std::io;
use std::path::{Path, PathBuf};

use jutsu_core::{Material, MaterialReference, unused_file_path};
use uuid::Uuid;

/// Edit in…, on a card's menu.
pub const EDIT_IN_TITLE: &str = "Edit in…";
/// Import as New Version…, on a card's menu.
pub const IMPORT_TITLE: &str = "Import as New Version…";
/// The app picker's message.
pub const EDIT_IN_MESSAGE: &str = "Choose the app to edit a copy in. The original stays as it is; bring the edit back \
     with Import as New Version or by dropping it on the card.";
/// The file picker's message.
pub const IMPORT_MESSAGE: &str = "Choose the edited file. It is copied into the library as a new version.";
pub const EDIT_IN_FAILED: &str = "Can't open a copy to edit";
pub const IMPORT_FAILED: &str = "Can't import the edited file";

/// Copies `file`, the file of `version`, into a folder of its own under `root`; `file` is only read.
pub fn working_copy(file: &Path, version: MaterialReference, root: &Path) -> io::Result<PathBuf> {
    let tag: String = Uuid::new_v4().simple().to_string().chars().take(8).collect();
    let folder = root.join(format!(
        "{}_{}_{tag}",
        uuid_text(version.material_id),
        uuid_text(version.version_id)
    ));
    std::fs::create_dir_all(&folder)?;
    let name = file
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let copy = folder.join(name);
    std::fs::copy(file, &copy)?;
    Ok(copy)
}

/// Swift's `uuidString`: upper case.
fn uuid_text(id: Uuid) -> String {
    id.hyphenated().to_string().to_uppercase()
}

/// The version a working copy under `root` was made from; `None` for any other file.
pub fn source(file: &Path, root: &Path) -> Option<MaterialReference> {
    let folder = file.parent()?;
    if folder.parent()? != root {
        return None;
    }
    let name = folder.file_name()?.to_str()?;
    let parts: Vec<&str> = name.split('_').collect();
    let [material, version, _] = parts.as_slice() else {
        return None;
    };
    Some(MaterialReference {
        material_id: Uuid::parse_str(material).ok()?,
        version_id: Uuid::parse_str(version).ok()?,
    })
}

/// The version an edit of `material` brought back as `file` is linked to: the one its working copy was made from
/// when it is a version of this material, else the current one.
pub fn parent(file: &Path, material: &Material, root: &Path) -> MaterialReference {
    source(file, root)
        .filter(|source| {
            source.material_id == material.id
                && material
                    .versions()
                    .iter()
                    .any(|version| version.id == source.version_id)
        })
        .unwrap_or_else(|| material.reference())
}

/// Copies the edited `file` into `folder` as "<name> edited.<ext>" (" 2", " 3"… when taken). The edited file and
/// every other file in `folder` are left as they are.
pub fn copy_back(file: &Path, folder: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(folder)?;
    let stem = file.file_stem().and_then(|stem| stem.to_str()).unwrap_or("Edit");
    let name = match file.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if !ext.is_empty() => format!("{stem} edited.{ext}"),
        _ => format!("{stem} edited"),
    };
    let copy = unused_file_path(&folder.join(name), Path::exists);
    std::fs::copy(file, &copy)?;
    Ok(copy)
}

/// Whether `file` is one of the material's own version files: a file of the item itself is not an edit.
pub fn is_own_file(file: &Path, material: &Material) -> bool {
    material
        .versions()
        .iter()
        .filter_map(|version| version.file_url.as_ref()?.to_file_path().ok())
        .any(|path| path == file)
}

#[cfg(test)]
mod tests {
    use jutsu_core::{MaterialKind, MaterialVersion};
    use url::Url;

    use super::*;
    use crate::library::fixture::now;

    fn material(path: &Path) -> Material {
        let mut version = MaterialVersion::new(Uuid::new_v4(), now());
        version.file_url = Url::from_file_path(path).ok();
        Material::new(Uuid::new_v4(), MaterialKind::Image, now(), version)
    }

    /// `OutsideEditTests.aWorkingCopySaysWhichVersionItCameFromAndLeavesTheOriginal`.
    #[test]
    fn a_working_copy_says_which_version_it_came_from_and_leaves_the_original() {
        let folder = tempfile::tempdir().unwrap();
        let library = folder.path().join("Images");
        std::fs::create_dir_all(&library).unwrap();
        let original = library.join("hero.png");
        std::fs::write(&original, "original").unwrap();
        let mut material = material(&original);
        let source_ref = material.reference();
        let root = folder.path().join("Edit In");

        let copy = working_copy(&original, source_ref, &root).unwrap();
        std::fs::write(&copy, "edited").unwrap();
        let saved_as = copy.parent().unwrap().join("hero final.tif");
        std::fs::write(&saved_as, "edited").unwrap();
        let mut other = MaterialVersion::new(Uuid::new_v4(), now());
        other.file_url = Url::from_file_path(library.join("other.png")).ok();
        material.add_version(other).unwrap();

        assert_eq!(copy.file_name().unwrap(), "hero.png");
        assert_eq!(source(&copy, &root), Some(source_ref));
        assert_eq!(parent(&saved_as, &material, &root), source_ref);
        assert_eq!(std::fs::read_to_string(&original).unwrap(), "original");
    }

    /// `OutsideEditTests.aFileFromElsewhereIsAnEditOfTheCurrentVersion`.
    #[test]
    fn a_file_from_elsewhere_is_an_edit_of_the_current_version() {
        let material = material(Path::new("/library/hero.png"));
        let other = self::material(Path::new("/library/other.png"));
        let root = Path::new("/tmp/Edit In");
        let reference = other.reference();
        let foreign_copy = root
            .join(format!(
                "{}_{}_abcd1234",
                uuid_text(reference.material_id),
                uuid_text(reference.version_id)
            ))
            .join("other.png");

        assert_eq!(
            parent(Path::new("/home/me/Desktop/hero.png"), &material, root),
            material.reference()
        );
        assert_eq!(parent(&foreign_copy, &material, root), material.reference());
        assert_eq!(source(&foreign_copy, root), Some(other.reference()));
        assert!(is_own_file(Path::new("/library/hero.png"), &material));
        assert!(!is_own_file(Path::new("/library/other.png"), &material));
    }

    /// `OutsideEditTests.theEditIsCopiedBackUnderAFreeName`.
    #[test]
    fn the_edit_is_copied_back_under_a_free_name() {
        let folder = tempfile::tempdir().unwrap();
        let images = folder.path().join("Images");
        std::fs::create_dir_all(&images).unwrap();
        let edited = folder.path().join("hero.png");
        std::fs::write(&edited, "edited").unwrap();
        std::fs::write(images.join("hero edited.png"), "earlier").unwrap();

        let copy = copy_back(&edited, &images).unwrap();

        assert_eq!(copy.file_name().unwrap(), "hero edited 2.png");
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "edited");
        assert_eq!(
            std::fs::read_to_string(images.join("hero edited.png")).unwrap(),
            "earlier"
        );
        assert!(edited.exists());
    }
}
