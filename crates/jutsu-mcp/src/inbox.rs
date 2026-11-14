//! The helper's half of the file handoff (`AgentInbox`): it reads an agent's file and sends its bytes to the app,
//! which stores them in a folder of their own and answers with the copy's path for the command to name.

use jutsu_agent::{AgentError, ErrorCode, UploadCommand, UploadRequest};
use std::{
    env, fs,
    path::{Component, Path, PathBuf},
};

pub struct Inbox {
    /// Relative paths are the agent's, from where it started the helper.
    pub working_directory: PathBuf,
}

impl Inbox {
    pub fn new(working_directory: impl Into<PathBuf>) -> Self {
        Self {
            working_directory: working_directory.into(),
        }
    }

    /// The process's working directory, or the root when it can't be read.
    pub fn current_directory() -> PathBuf {
        env::current_dir().unwrap_or_else(|_| PathBuf::from(std::path::MAIN_SEPARATOR_STR))
    }

    /// The file at `path` and the upload that hands it over, checked before a byte is read.
    pub fn upload(
        &self,
        path: &str,
        request: Option<UploadRequest>,
    ) -> Result<(PathBuf, UploadCommand), AgentError> {
        let source = Self::resolve(path, &self.working_directory);
        let Some(size) = fs::metadata(&source)
            .ok()
            .filter(fs::Metadata::is_file)
            .map(|metadata| metadata.len())
        else {
            return Err(AgentError::new(
                ErrorCode::FILE_REFUSED,
                format!("There is no file at {path}."),
                false,
                Some("Pass the absolute path of a file on this computer."),
            ));
        };
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let command = UploadCommand {
            name,
            size: i64::try_from(size).unwrap_or(i64::MAX),
            request,
        };
        if let Some(problem) = UploadCommand::problem(&command.name, command.size) {
            return Err(AgentError::new(
                ErrorCode::FILE_REFUSED,
                format!("Jutsu Studio can't take {path}: {problem}."),
                false,
                None,
            ));
        }
        Ok((source, command))
    }

    /// `path` as a file on this computer: `~` expanded, relative to `directory`, `.` and `..` resolved, and symlinks
    /// followed as far as the path exists.
    pub fn resolve(path: &str, directory: &Path) -> PathBuf {
        let expanded = expand_tilde(path);
        let joined = if expanded.is_absolute() {
            expanded
        } else {
            directory.join(expanded)
        };
        follow_symlinks(&standardize(&joined))
    }
}

fn home() -> Option<PathBuf> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(variable)
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn expand_tilde(path: &str) -> PathBuf {
    let rest = if path == "~" {
        Some("")
    } else {
        path.strip_prefix("~/").or_else(|| {
            if cfg!(windows) {
                path.strip_prefix("~\\")
            } else {
                None
            }
        })
    };
    match (rest, home()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// `.` dropped and `..` taking off the component before it, without reading the disk.
fn standardize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    result.components().next_back(),
                    None | Some(Component::RootDir | Component::Prefix(_))
                ) {
                    result.pop();
                }
            }
            other => result.push(other),
        }
    }
    result
}

/// The longest part of `path` that exists with its symlinks followed, and the rest as given.
fn follow_symlinks(path: &Path) -> PathBuf {
    for ancestor in path.ancestors() {
        if let Ok(real) = fs::canonicalize(ancestor) {
            let rest = path.strip_prefix(ancestor).unwrap_or(Path::new(""));
            let real = plain(real);
            return if rest.as_os_str().is_empty() {
                real
            } else {
                real.join(rest)
            };
        }
    }
    path.to_path_buf()
}

/// Windows' canonical paths are verbatim (`\\?\C:\…`); agents and the app read the plain form. On macOS the real
/// path of `/tmp` or `/var/folders/…` starts with `/private`, which Foundation's `resolvingSymlinksInPath` drops when
/// the shorter path exists: the macOS helper's paths, and so the fingerprints of requests with files, have it dropped.
fn plain(path: PathBuf) -> PathBuf {
    if cfg!(target_os = "macos")
        && let Ok(rest) = path.strip_prefix("/private")
        && !rest.as_os_str().is_empty()
    {
        let short = Path::new("/").join(rest);
        if short.exists() {
            return short;
        }
    }
    if cfg!(windows) {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\")
            && rest.as_bytes().get(1) == Some(&b':')
        {
            return PathBuf::from(rest);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::{Inbox, standardize};
    use std::path::Path;

    #[test]
    fn a_path_is_standardized_and_made_absolute() {
        assert_eq!(standardize(Path::new("/a/./b/../c")), Path::new("/a/c"));
        assert_eq!(standardize(Path::new("/../a")), Path::new("/a"));
        let folder = tempfile::tempdir().expect("folder");
        let real = super::follow_symlinks(folder.path());
        assert_eq!(Inbox::resolve("x/../y.png", folder.path()), real.join("y.png"));
        // Absolute on Unix only: Windows puts the current drive before it.
        #[cfg(unix)]
        assert_eq!(
            Inbox::resolve("/nonexistent-golden/a.png", folder.path()),
            Path::new("/nonexistent-golden/a.png")
        );
    }

    /// A file in a temporary folder resolves as the macOS helper resolves it, so a request with it keeps the
    /// fingerprint that helper gave it.
    #[cfg(target_os = "macos")]
    #[test]
    fn private_is_dropped_as_foundation_drops_it() {
        let folder = tempfile::tempdir().expect("folder");
        let file = folder.path().join("face.png");
        std::fs::write(&file, b"png").expect("file");
        let resolved = Inbox::resolve(&file.to_string_lossy(), Path::new("/"));
        assert!(!resolved.starts_with("/private"), "{}", resolved.display());
        assert_eq!(std::fs::read(&resolved).expect("same file"), b"png");
        assert_eq!(
            Inbox::resolve("/private/nonexistent-golden/a.png", Path::new("/")),
            Path::new("/private/nonexistent-golden/a.png")
        );
    }
}
