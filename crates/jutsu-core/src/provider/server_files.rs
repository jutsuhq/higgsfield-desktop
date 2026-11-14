//! Local names for files the provider serves.

use std::path::{Component, Path, PathBuf};

use url::Url;

use crate::rules::OutputNamePattern;
use crate::support::url_ext::url_extension;

/// Local names for files the provider serves: each name safe as one path component, the extension from an
/// allowlist, and the result checked to stay inside its folder.
pub struct ServerFileName;

impl ServerFileName {
    /// Extensions a downloaded video may keep.
    pub const VIDEO_EXTENSIONS: [&'static str; 2] = ["mp4", "mov"];
    /// Extensions a downloaded image may keep.
    pub const IMAGE_EXTENSIONS: [&'static str; 5] = ["jpg", "jpeg", "png", "heic", "webp"];

    /// `folder/<names[0]>/…/<names.last>.<ext>`, where `ext` is `url`'s extension when `allowed` holds it and
    /// otherwise the first of `allowed`. `None` when the path would leave `folder`.
    pub fn file(folder: &Path, names: &[String], extension_of: &Url, allowed: &[&str]) -> Option<PathBuf> {
        let (last, rest) = names.split_last()?;
        let fallback = allowed.first()?;
        let mut file = folder.to_path_buf();
        for name in rest {
            file.push(Self::component(name));
        }
        let given = url_extension(extension_of).to_lowercase();
        let ext = if allowed.contains(&given.as_str()) {
            given.as_str()
        } else {
            fallback
        };
        file.push(format!("{}.{ext}", Self::component(last)));
        Self::is_inside(&file, folder).then_some(file)
    }

    /// `name` as one path component: no separators, no leading dots, never empty.
    pub fn component(name: &str) -> String {
        let sanitized = OutputNamePattern::sanitize(&name.replace('\\', "-"));
        if sanitized.is_empty() {
            "Untitled".to_owned()
        } else {
            sanitized
        }
    }

    /// Whether `file` lies inside `folder`, both read lexically (`.` and `..` resolved without the disk).
    pub fn is_inside(file: &Path, folder: &Path) -> bool {
        let root = normalized(folder);
        let path = normalized(file);
        path != root && path.starts_with(&root)
    }
}

/// `path` with `.` dropped and `..` taking off the component before it, as `standardizedFileURL` does.
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}
