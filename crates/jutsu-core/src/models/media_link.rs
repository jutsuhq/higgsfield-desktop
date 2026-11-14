//! Web links to a video or image that Create downloads and uses like a picked file.

use url::Url;

use crate::support::string_enum;
use crate::support::text::{is_whitespace_or_newline, trim, trim_whitespace};
use crate::support::url_ext::{deleting_path_extension, last_path_component, url_extension};

string_enum! {
    /// Whether a link leads to a video or an image.
    pub enum MediaLinkKind {
        /// A video.
        Video = "video",
        /// An image.
        Image = "image",
    }
}

/// Why a link can't be used.
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum MediaLinkError {
    /// The text isn't a web address.
    #[error("notALink")]
    NotALink,
    /// `http:` or another scheme: only `https:` is downloaded.
    #[error("notHTTPS")]
    NotHttps,
    /// The link leads to a page or a file that isn't a video or image.
    #[error("notMedia")]
    NotMedia {
        /// The media type it served, when it said.
        content_type: Option<String>,
    },
    /// A video where an image was asked for, or the other way round.
    #[error("wrongKind")]
    WrongKind {
        /// What was asked for.
        expected: MediaLinkKind,
    },
    /// The file is larger than allowed.
    #[error("tooLarge")]
    TooLarge {
        /// The limit.
        max_bytes: i64,
    },
    /// The server answered with this status.
    #[error("unavailable")]
    Unavailable {
        /// The HTTP status.
        status: i64,
    },
    /// The network failed.
    #[error("network")]
    Network,
    /// The disk is full.
    #[error("notEnoughSpace")]
    NotEnoughSpace,
}

/// A web link to a video or image that Create downloads and uses like a picked file.
pub struct MediaLink;

const GENERIC_TYPES: [&str; 3] = ["", "application/octet-stream", "binary/octet-stream"];

const MIME_TYPES: [(&str, MediaLinkKind, &str); 16] = [
    ("video/mp4", MediaLinkKind::Video, "mp4"),
    ("video/quicktime", MediaLinkKind::Video, "mov"),
    ("video/x-m4v", MediaLinkKind::Video, "m4v"),
    ("video/webm", MediaLinkKind::Video, "webm"),
    ("video/x-matroska", MediaLinkKind::Video, "mkv"),
    ("video/mpeg", MediaLinkKind::Video, "mpg"),
    ("video/3gpp", MediaLinkKind::Video, "3gp"),
    ("image/jpeg", MediaLinkKind::Image, "jpg"),
    ("image/jpg", MediaLinkKind::Image, "jpg"),
    ("image/png", MediaLinkKind::Image, "png"),
    ("image/webp", MediaLinkKind::Image, "webp"),
    ("image/gif", MediaLinkKind::Image, "gif"),
    ("image/heic", MediaLinkKind::Image, "heic"),
    ("image/heif", MediaLinkKind::Image, "heif"),
    ("image/tiff", MediaLinkKind::Image, "tiff"),
    ("image/bmp", MediaLinkKind::Image, "bmp"),
];

const EXTENSION_KINDS: [(&str, MediaLinkKind); 18] = [
    ("mp4", MediaLinkKind::Video),
    ("mov", MediaLinkKind::Video),
    ("m4v", MediaLinkKind::Video),
    ("webm", MediaLinkKind::Video),
    ("mkv", MediaLinkKind::Video),
    ("mpg", MediaLinkKind::Video),
    ("mpeg", MediaLinkKind::Video),
    ("3gp", MediaLinkKind::Video),
    ("jpg", MediaLinkKind::Image),
    ("jpeg", MediaLinkKind::Image),
    ("png", MediaLinkKind::Image),
    ("webp", MediaLinkKind::Image),
    ("gif", MediaLinkKind::Image),
    ("heic", MediaLinkKind::Image),
    ("heif", MediaLinkKind::Image),
    ("tif", MediaLinkKind::Image),
    ("tiff", MediaLinkKind::Image),
    ("bmp", MediaLinkKind::Image),
];

impl MediaLink {
    /// The link in pasted or typed text: one `https:` URL with a host, surrounding spaces ignored.
    pub fn parse(text: &str) -> Result<Url, MediaLinkError> {
        let trimmed = trim(text);
        if trimmed.is_empty() || trimmed.contains(is_whitespace_or_newline) {
            return Err(MediaLinkError::NotALink);
        }
        let url = Url::parse(trimmed).map_err(|_| MediaLinkError::NotALink)?;
        match url.scheme() {
            "https" => {}
            "http" => return Err(MediaLinkError::NotHttps),
            _ => return Err(MediaLinkError::NotALink),
        }
        // The WHATWG parser skips extra slashes ("https:///clip.mp4" gets the host "clip.mp4"); Foundation reads no
        // host there, so the text itself has to name one.
        let names_host = trimmed.split_once(':').is_some_and(|(_, rest)| {
            rest.strip_prefix("//").is_some_and(|authority| {
                authority
                    .chars()
                    .next()
                    .is_some_and(|c| !matches!(c, '/' | '\\' | '?' | '#'))
            })
        });
        if !names_host || url.host_str().is_none_or(str::is_empty) {
            return Err(MediaLinkError::NotALink);
        }
        Ok(url)
    }

    /// What a download is, from its `Content-Type`, and the extension its file gets. A generic binary type (or
    /// none) falls back to the extension of `url`, so a storage host that labels everything
    /// `application/octet-stream` still works. Pages (`text/html`, e.g. a YouTube watch page) and anything else
    /// are refused.
    pub fn classify(
        content_type: Option<&str>,
        url: &Url,
    ) -> Result<(MediaLinkKind, String), MediaLinkError> {
        let mime = mime_type(content_type);
        if let Some((_, kind, ext)) = MIME_TYPES.iter().find(|(name, _, _)| *name == mime) {
            return Ok((*kind, (*ext).to_owned()));
        }
        let path_extension = url_extension(url).to_lowercase();
        if let Some((_, kind)) = EXTENSION_KINDS.iter().find(|(ext, _)| *ext == path_extension)
            && (GENERIC_TYPES.contains(&mime.as_str()) || mime.starts_with(&format!("{}/", kind.raw_value())))
        {
            return Ok((*kind, path_extension));
        }
        Err(MediaLinkError::NotMedia {
            content_type: (!mime.is_empty()).then_some(mime),
        })
    }

    /// Refuses a download whose `Content-Type` says it isn't an image, like `classify`, but takes any `image/`
    /// type and leaves a generic binary type (or none) to the decoder: for a picture that is only shown, never kept
    /// or sent.
    pub fn check_image(content_type: Option<&str>) -> Result<(), MediaLinkError> {
        let mime = mime_type(content_type);
        if mime.starts_with("image/") || GENERIC_TYPES.contains(&mime.as_str()) {
            Ok(())
        } else {
            Err(MediaLinkError::NotMedia {
                content_type: Some(mime),
            })
        }
    }

    /// The downloaded file's name: the link's last path component without its extension, or its host.
    pub fn base_name(url: &Url) -> String {
        let last = last_path_component(url);
        let name = deleting_path_extension(&last);
        if name.is_empty() || name == "/" {
            return url.host_str().unwrap_or("Download").to_owned();
        }
        name.to_owned()
    }

    /// Files a provider can take by link as they are, without the local conversion an upload goes through: the
    /// REST API's input types (`video/mp4`; JPEG, PNG, WebP, GIF).
    pub fn is_linkable(file: &Url, kind: MediaLinkKind) -> bool {
        let ext = url_extension(file).to_lowercase();
        match kind {
            MediaLinkKind::Video => ext == "mp4",
            MediaLinkKind::Image => ["jpg", "jpeg", "png", "webp", "gif"].contains(&ext.as_str()),
        }
    }
}

/// The media type of a `Content-Type` header, without its parameters and lowercased; "" for none.
fn mime_type(content_type: Option<&str>) -> String {
    content_type
        .and_then(|header| header.split(';').find(|part| !part.is_empty()))
        .map(|first| trim_whitespace(first).to_lowercase())
        .unwrap_or_default()
}
