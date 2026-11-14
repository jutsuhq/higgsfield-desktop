//! The parts of Foundation's `URL` and `NSString` path handling the rules rely on.

use std::path::{Path, PathBuf};

use url::Url;

/// The last path component of `url`, percent-decoded, as `URL.lastPathComponent`.
pub fn last_path_component(url: &Url) -> String {
    let raw = url.path_segments().and_then(|mut segments| {
        let mut last = None;
        for segment in segments.by_ref() {
            if !segment.is_empty() {
                last = Some(segment);
            }
        }
        last
    });
    let raw = raw.unwrap_or("");
    percent_decode(raw)
}

/// Decodes `%XX` escapes; a broken escape stays as written.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            out.push(high * 16 + low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

/// The extension of a file name, as `URL.pathExtension`: what follows the last dot, empty when the name starts with
/// its only dot or ends with a dot.
pub fn path_extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(0) | None => "",
        Some(dot) => &name[dot + 1..],
    }
}

/// A file name without its extension, as `URL.deletingPathExtension().lastPathComponent`.
pub fn deleting_path_extension(name: &str) -> &str {
    let ext = path_extension(name);
    if ext.is_empty() {
        name.strip_suffix('.')
            .filter(|stem| !stem.is_empty())
            .unwrap_or(name)
    } else {
        &name[..name.len() - ext.len() - 1]
    }
}

/// The file name of `path` as text, empty when it has none.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The extension of `url`'s last path component.
pub fn url_extension(url: &Url) -> String {
    path_extension(&last_path_component(url)).to_owned()
}

/// `path` with `name` in place of its last component.
pub fn sibling(path: &Path, name: &str) -> PathBuf {
    path.parent()
        .map_or_else(|| PathBuf::from(name), |folder| folder.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_follow_foundation() {
        assert_eq!(path_extension("clip.MOV"), "MOV");
        assert_eq!(path_extension(".hidden"), "");
        assert_eq!(path_extension("a.tar.gz"), "gz");
        assert_eq!(deleting_path_extension("a.tar.gz"), "a.tar");
        assert_eq!(deleting_path_extension("name"), "name");
        let url = Url::parse("file:///tmp/My%20Clip.mov").unwrap();
        assert_eq!(last_path_component(&url), "My Clip.mov");
        assert_eq!(url_extension(&url), "mov");
    }
}
