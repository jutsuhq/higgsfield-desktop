//! Where the app keeps its files.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jutsu_core::{OutputSubfolder, TemplateFiles};
use url::Url;
use uuid::Uuid;

/// Where the app keeps its files, under the app data folder and the output folder it is given. Every method that
/// touches the disk blocks; call it from `spawn_blocking`. The macOS app's security-scoped bookmarks have no
/// equivalent here: folders are plain paths.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileLayout {
    /// The app's data folder (`Application Support/Jutsu Studio` on macOS).
    pub root: PathBuf,
    /// Results go here unless the user picked another folder.
    pub default_output_folder: PathBuf,
}

impl FileLayout {
    /// The layout under `root`, with results in `default_output_folder`.
    pub fn new(root: impl Into<PathBuf>, default_output_folder: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            default_output_folder: default_output_folder.into(),
        }
    }

    /// The SQLite database.
    pub fn store_path(&self) -> PathBuf {
        self.root.join("default.store")
    }

    /// The Create form between launches; not a cache.
    pub fn create_draft_path(&self) -> PathBuf {
        self.root.join("create-draft.json")
    }

    /// Prepared inputs; safe to delete.
    pub fn sources_folder(&self) -> PathBuf {
        self.root.join("sources")
    }

    /// The caches.
    pub fn cache_folder(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// Preset catalog JSON and previews.
    pub fn preset_cache_folder(&self) -> PathBuf {
        self.cache_folder().join("presets")
    }

    /// The models' limits per account, read again daily.
    pub fn model_constraints_path(&self) -> PathBuf {
        self.cache_folder().join("model-constraints.json")
    }

    /// Motion Library clips picked as sources. Queued jobs read them, so Clear Cache waits for an empty queue.
    pub fn motion_library_folder(&self) -> PathBuf {
        self.root.join("motion-library")
    }

    /// Images and videos the account uploaded, downloaded to be used again; cleared like `motion_library_folder`.
    pub fn account_media_folder(&self) -> PathBuf {
        self.root.join("account-media")
    }

    /// Images pasted into Create from the clipboard, saved to be used as references; cleared like
    /// `motion_library_folder`.
    pub fn pasted_images_folder(&self) -> PathBuf {
        self.root.join("pasted")
    }

    /// Videos and images downloaded from links pasted into Create, one folder per link; cleared like
    /// `pasted_images_folder`.
    pub fn linked_media_folder(&self) -> PathBuf {
        self.root.join("links")
    }

    /// Reference images made with Generate… and picked, one folder per image; cleared like `pasted_images_folder`.
    pub fn generated_images_folder(&self) -> PathBuf {
        self.root.join("generated")
    }

    /// Inputs downloaded on demand or pasted; Clear Cache deletes them.
    pub fn downloaded_input_folders(&self) -> [PathBuf; 5] {
        [
            self.motion_library_folder(),
            self.account_media_folder(),
            self.pasted_images_folder(),
            self.linked_media_folder(),
            self.generated_images_folder(),
        ]
    }

    /// Copies of template reference images; not a cache.
    pub fn templates_folder(&self) -> PathBuf {
        self.root.join("templates")
    }

    /// The folder of one template's copied images.
    pub fn template_folder(&self, template_id: Uuid) -> PathBuf {
        self.templates_folder()
            .join(TemplateFiles::folder_name(template_id))
    }

    /// Working copies of edits made in another app, before they are brought back (`OutsideEdit`).
    pub fn edit_in_folder(&self) -> PathBuf {
        self.root.join("Edit In")
    }

    /// The folder of one kind of result inside the default output folder.
    pub fn output_subfolder(&self, subfolder: OutputSubfolder) -> PathBuf {
        subfolder.path(&self.default_output_folder)
    }

    /// Creates the root, `sources/` and `cache/presets/`.
    pub fn prepare_directories(&self) -> io::Result<()> {
        for folder in [
            self.root.clone(),
            self.sources_folder(),
            self.preset_cache_folder(),
        ] {
            fs::create_dir_all(folder)?;
        }
        Ok(())
    }

    /// The default output folder, created on first use.
    pub fn output_folder(&self) -> io::Result<PathBuf> {
        fs::create_dir_all(&self.default_output_folder)?;
        Ok(self.default_output_folder.clone())
    }

    /// Removes `sources/`, `cache/` and the downloaded inputs only; the store, the results and the template images
    /// are kept, and so are the files in `kept`, such as the ones the Create draft points at, with the folders that
    /// hold them. Links other than files are ignored. Call it only while no job is active: queued jobs read these
    /// files.
    pub fn clear_cache(&self, kept: &[Url]) -> io::Result<()> {
        let kept: HashSet<PathBuf> = kept
            .iter()
            .filter(|url| url.scheme() == "file")
            .filter_map(|url| url.to_file_path().ok())
            .map(|path| comparable_path(&path))
            .collect();
        let mut folders = vec![self.sources_folder(), self.cache_folder()];
        folders.extend(self.downloaded_input_folders());
        for folder in folders {
            if folder.exists() {
                remove(&folder, &kept)?;
            }
        }
        Ok(())
    }

    /// Copies `sources`, which must be readable files, into the template's own folder and returns the copies in
    /// order. Nothing is left behind when a copy fails.
    pub fn copy_template_images(&self, sources: &[Url], template_id: Uuid) -> io::Result<Vec<Url>> {
        let folder = self.template_folder(template_id);
        fs::create_dir_all(&folder)?;
        let copied = sources
            .iter()
            .zip(TemplateFiles::file_names(sources))
            .map(|(source, name)| {
                let from = source
                    .to_file_path()
                    .map_err(|()| io::Error::new(io::ErrorKind::InvalidInput, "not a file"))?;
                let copy = folder.join(name);
                fs::copy(from, &copy)?;
                Url::from_file_path(&copy)
                    .map_err(|()| io::Error::new(io::ErrorKind::InvalidInput, "not absolute"))
            })
            .collect::<io::Result<Vec<Url>>>();
        if copied.is_err() {
            let _ = fs::remove_dir_all(&folder);
        }
        copied
    }

    /// Deletes a template's copied images.
    pub fn remove_template_images(&self, template_id: Uuid) -> io::Result<()> {
        let folder = self.template_folder(template_id);
        if !folder.exists() {
            return Ok(());
        }
        fs::remove_dir_all(folder)
    }

    /// Deletes the image folders of templates that no longer exist.
    pub fn prune_template_images(&self, keeping: &HashSet<Uuid>) -> io::Result<()> {
        let templates = self.templates_folder();
        if !templates.exists() {
            return Ok(());
        }
        let mut names = Vec::new();
        for entry in fs::read_dir(&templates)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with('.') && entry.file_type()?.is_dir() {
                names.push(name);
            }
        }
        for name in TemplateFiles::orphaned_folders(&names, keeping) {
            fs::remove_dir_all(templates.join(name))?;
        }
        Ok(())
    }
}

/// Removes `item` whole when none of `kept` is it or inside it; otherwise everything in it except those.
fn remove(item: &Path, kept: &HashSet<PathBuf>) -> io::Result<()> {
    let path = comparable_path(item);
    if kept.contains(&path) {
        return Ok(());
    }
    if !kept.iter().any(|kept| kept.starts_with(&path)) {
        return if fs::symlink_metadata(item)?.is_dir() {
            fs::remove_dir_all(item)
        } else {
            fs::remove_file(item)
        };
    }
    for entry in fs::read_dir(item)? {
        remove(&entry?.path(), kept)?;
    }
    Ok(())
}

/// `path` with symbolic links resolved, so a file and its folder compare as paths. The part that doesn't exist yet
/// is kept as it is.
fn comparable_path(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut rest = Vec::new();
    loop {
        if let Ok(resolved) = fs::canonicalize(existing) {
            return rest.iter().rev().fold(resolved, |path, part| path.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                existing = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}
