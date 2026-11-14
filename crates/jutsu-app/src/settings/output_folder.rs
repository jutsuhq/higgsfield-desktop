//! Where results download (`OutputFolder.swift`, `OutputFolderHistory.swift`): the folder picked in Settings or the
//! default one, and the folders picked before it that may still hold results. The macOS app keeps a security-scoped
//! bookmark per folder; here a folder is its plain path, so there is nothing to resolve or keep open.

use std::path::{Component, Path, PathBuf};

use jutsu_core::OutputSubfolder;
use jutsu_store::FileLayout;

use crate::preferences::{Preferences, PreferencesError, PreferencesStore};

/// How a folder is identified: its path with `.` and `..` worked out and without a trailing separator.
pub fn folder_key(path: &Path) -> String {
    let mut parts: Vec<Component<'_>> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if matches!(parts.last(), Some(Component::Normal(_))) => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.iter().collect::<PathBuf>().to_string_lossy().into_owned()
}

/// Whether `path` is the folder `folder` (as `folder_key` gives it) or lies inside it. A sibling whose name starts
/// the same is not inside.
pub fn folder_contains(folder: &str, path: &Path) -> bool {
    Path::new(&folder_key(path)).starts_with(folder)
}

/// Custom output folders that earlier results were saved in, other than the current one, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputFolderHistory {
    folders: Vec<String>,
}

impl OutputFolderHistory {
    /// The history as the preferences keep it.
    pub fn new(stored: Option<Vec<String>>) -> Self {
        Self {
            folders: stored.unwrap_or_default(),
        }
    }

    pub fn folders(&self) -> &[String] {
        &self.folders
    }

    /// For the preferences: `None` once empty, so the key goes.
    pub fn stored(&self) -> Option<Vec<String>> {
        (!self.folders.is_empty()).then(|| self.folders.clone())
    }

    /// Remembers a folder the user switched away from, replacing an older entry for the same folder.
    pub fn add(&mut self, folder: &Path) {
        let key = folder_key(folder);
        self.folders.retain(|known| *known != key);
        self.folders.push(key);
    }

    /// Forgets `folder`, e.g. because it is the current folder again. Whether it was there.
    pub fn remove(&mut self, folder: &Path) -> bool {
        let key = folder_key(folder);
        let count = self.folders.len();
        self.folders.retain(|known| *known != key);
        self.folders.len() != count
    }

    /// Drops the folders among `candidates` that hold none of `files` (results and active jobs' download folders)
    /// and returns them. Folders added after `candidates` was taken are kept: `files` may predate their results.
    pub fn prune(&mut self, candidates: &[String], files: &[PathBuf]) -> Vec<String> {
        let mut dropped = Vec::new();
        self.folders.retain(|folder| {
            let drop = candidates.contains(folder) && !files.iter().any(|file| folder_contains(folder, file));
            if drop {
                dropped.push(folder.clone());
            }
            !drop
        });
        dropped
    }
}

/// The folder results download to now.
pub fn current_output_folder(preferences: &Preferences, layout: &FileLayout) -> PathBuf {
    preferences
        .output_folder
        .as_ref()
        .map_or_else(|| layout.default_output_folder.clone(), PathBuf::from)
}

/// Why the output folder couldn't change.
#[derive(Debug, thiserror::Error)]
pub enum OutputFolderError {
    #[error("Can't use this folder: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Preferences(#[from] PreferencesError),
}

/// Switches to `folder`, or back to the default with `None`, creating its `Videos` and `Images` folders first. The
/// custom folder switched away from joins the history, so its results stay readable and the media server keeps
/// serving them; a folder picked again leaves it. Blocks on the disk: call it from `spawn_blocking`.
pub fn use_output_folder(
    store: &PreferencesStore,
    layout: &FileLayout,
    folder: Option<&Path>,
) -> Result<Preferences, OutputFolderError> {
    let target = folder.map_or_else(|| layout.default_output_folder.clone(), Path::to_path_buf);
    for subfolder in [OutputSubfolder::Videos, OutputSubfolder::Images] {
        std::fs::create_dir_all(subfolder.path(&target))?;
    }
    Ok(store.edit(|preferences| {
        let mut history = OutputFolderHistory::new(preferences.previous_output_folders.take());
        if let Some(current) = preferences.output_folder.take() {
            history.add(Path::new(&current));
        }
        if let Some(folder) = folder {
            history.remove(folder);
        }
        preferences.output_folder = folder.map(folder_key);
        preferences.previous_output_folders = history.stored();
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLDER_A: &str = "/Volumes/Drive/Renders A/";
    const FOLDER_B: &str = "/home/me/Renders B/";

    fn history(folders: &[&str]) -> OutputFolderHistory {
        let mut history = OutputFolderHistory::default();
        for folder in folders {
            history.add(Path::new(folder));
        }
        history
    }

    #[test]
    fn items_in_folder_a_stay_readable_after_switching_to_folder_b() {
        let result = Path::new(FOLDER_A).join("2026-10-14_restyle_1a2b3c4d.mp4");
        let mut history = history(&[FOLDER_A]);
        let relaunched = OutputFolderHistory::new(history.stored());
        assert_eq!(relaunched, history);
        let candidates = history.folders().to_vec();
        assert_eq!(history.prune(&candidates, &[result]), Vec::<String>::new());
        assert_eq!(history.folders(), ["/Volumes/Drive/Renders A"]);
    }

    #[test]
    fn folders_no_result_uses_are_dropped() {
        let mut history = history(&[FOLDER_A, FOLDER_B]);
        let partial_download = Path::new(FOLDER_B).join("nested");
        // A sibling whose name starts like folder A's is not inside it.
        let sibling = PathBuf::from("/Volumes/Drive/Renders A2/result.mp4");
        let candidates = history.folders().to_vec();
        let dropped = history.prune(&candidates, &[sibling, partial_download]);
        assert_eq!(dropped, ["/Volumes/Drive/Renders A"]);
        assert_eq!(history.folders(), ["/home/me/Renders B"]);
    }

    #[test]
    fn a_folder_added_after_the_jobs_were_read_is_not_dropped() {
        let mut history = history(&[FOLDER_A]);
        let candidates = history.folders().to_vec();
        history.add(Path::new(FOLDER_B));
        history.prune(&candidates, &[]);
        assert_eq!(history.folders(), ["/home/me/Renders B"]);
    }

    #[test]
    fn switching_back_to_a_folder_removes_it_from_the_history() {
        let mut history = history(&[FOLDER_A, FOLDER_A]);
        assert_eq!(history.folders().len(), 1);
        assert!(history.remove(Path::new("/Volumes/Drive/Renders A")));
        assert!(!history.remove(Path::new(FOLDER_B)));
        assert_eq!(history.folders(), [] as [String; 0]);
        assert_eq!(history.stored(), None);
    }

    #[test]
    fn folder_keys_drop_dots_and_the_trailing_separator() {
        assert_eq!(folder_key(Path::new("/a/./b/../c/")), "/a/c");
        assert!(folder_contains("/a/c", Path::new("/a/c")));
        assert!(folder_contains("/a/c", Path::new("/a/c/d.mp4")));
        assert!(!folder_contains("/a/c", Path::new("/a/cd/e.mp4")));
    }

    #[test]
    fn switching_folders_creates_subfolders_and_keeps_the_earlier_ones() {
        let temp = tempfile::tempdir().unwrap();
        let layout = FileLayout::new(temp.path().join("data"), temp.path().join("Default"));
        let store = PreferencesStore::open(&temp.path().join("data"));
        let first = temp.path().join("First");
        let second = temp.path().join("Second");

        use_output_folder(&store, &layout, Some(&first)).unwrap();
        assert!(first.join("Videos").is_dir() && first.join("Images").is_dir());
        assert_eq!(current_output_folder(&store.get(), &layout), first);
        assert_eq!(store.get().previous_output_folders, None);

        use_output_folder(&store, &layout, Some(&second)).unwrap();
        assert_eq!(
            store.get().previous_output_folders,
            Some(vec![folder_key(&first)])
        );

        use_output_folder(&store, &layout, Some(&first)).unwrap();
        assert_eq!(
            store.get().previous_output_folders,
            Some(vec![folder_key(&second)])
        );

        let preferences = use_output_folder(&store, &layout, None).unwrap();
        assert_eq!(preferences.output_folder, None);
        assert_eq!(
            current_output_folder(&preferences, &layout),
            layout.default_output_folder
        );
        assert!(layout.default_output_folder.join("Videos").is_dir());
        assert_eq!(
            preferences.previous_output_folders,
            Some(vec![folder_key(&second), folder_key(&first)])
        );
    }

    #[test]
    fn a_folder_that_cant_be_created_changes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file");
        std::fs::write(&file, "x").unwrap();
        let layout = FileLayout::new(temp.path().join("data"), temp.path().join("Default"));
        let store = PreferencesStore::open(&temp.path().join("data"));
        assert!(use_output_folder(&store, &layout, Some(&file)).is_err());
        assert_eq!(store.get().output_folder, None);
    }
}
