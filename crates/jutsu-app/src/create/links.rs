//! Paste URL… on the Create screen (`LinkImport.swift`, `CreateViewLinks.swift`): a web link to a video or image,
//! downloaded first and then taken like a picked file. The macOS text says "on your Mac"; here it says "on this
//! computer".

use jutsu_core::{MediaLinkError, MediaLinkKind, ProviderKind};
use jutsu_engine::MediaDownloadError;
use serde::Serialize;
use url::Url;

use crate::connection::UserMessage;
use crate::queue::format::percent;

/// The Paste URL field's placeholder.
pub const PLACEHOLDER: &str = "https://";
/// Under the field.
pub const EXPLANATION: &str =
    "A direct link to the file, starting with https://. Jutsu Studio downloads it first.";

/// The Paste URL field's title.
pub fn prompt(kind: MediaLinkKind) -> &'static str {
    match kind {
        MediaLinkKind::Video => "Paste a link to a video",
        MediaLinkKind::Image => "Paste a link to an image",
    }
}

/// A link being downloaded, shown with its progress and Cancel in the section it goes to.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LinkDownloadView {
    /// "Downloading from example.com… 42%".
    pub text: String,
    /// The section that shows it: `"video"` (Source) or `"image"` (References).
    pub section: String,
    /// 0…1; `None` while the size is unknown.
    pub fraction: Option<f64>,
}

/// What a finished download says: an alert, or what wasn't added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum LinkOutcome {
    /// Taken; `notice` is the alert about an image that didn't fit the mode's limit.
    Done {
        notice: Option<UserMessage>,
    },
    /// Cancelled, or replaced by another link.
    Cancelled,
    Failed {
        message: UserMessage,
    },
}

/// "Downloading from example.com…", with the whole percent once the size is known.
pub fn download_text(link: &Url, fraction: Option<f64>) -> String {
    let host = link.host_str().map_or_else(|| link.to_string(), str::to_owned);
    match fraction {
        Some(fraction) => format!(
            "Downloading from {host}… {}",
            percent((fraction * 100.0).floor() / 100.0)
        ),
        None => format!("Downloading from {host}…"),
    }
}

pub(crate) fn download_view(
    link: &Url,
    expected: Option<MediaLinkKind>,
    fraction: Option<f64>,
) -> LinkDownloadView {
    LinkDownloadView {
        text: download_text(link, fraction),
        // A download without a kind shows in the Source section.
        section: expected.unwrap_or(MediaLinkKind::Video).raw_value().to_owned(),
        fraction,
    }
}

/// The kind a section asks for: `"video"` or `"image"`; anything else takes either.
pub fn expected_kind(raw: Option<&str>) -> Option<MediaLinkKind> {
    raw.and_then(MediaLinkKind::from_raw)
}

/// The Paste URL field: its title, placeholder and explanation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LinkEntryView {
    pub title: String,
    pub placeholder: String,
    pub explanation: String,
    /// The clipboard's link, or empty.
    pub text: String,
}

/// The field for `kind`, filled with the clipboard's link if it holds one.
pub fn entry(kind: MediaLinkKind, clipboard: &str) -> LinkEntryView {
    LinkEntryView {
        title: prompt(kind).to_owned(),
        placeholder: PLACEHOLDER.to_owned(),
        explanation: EXPLANATION.to_owned(),
        text: prefill(clipboard),
    }
}

/// What the Paste URL field opens with: the clipboard's link, or nothing.
pub fn prefill(clipboard: &str) -> String {
    jutsu_core::MediaLink::parse(clipboard)
        .map(|link| link.to_string())
        .unwrap_or_default()
}

/// Decimal units, as the file manager writes a size: "2 GB".
fn size_text(bytes: i64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "a limit only needs a few digits")]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < 4 {
        value /= 1000.0;
        unit += 1;
    }
    let number = format!("{value:.2}");
    let number = number.trim_end_matches('0').trim_end_matches('.');
    format!("{number} {}", ["bytes", "KB", "MB", "GB", "TB"][unit])
}

/// The alert for a link that couldn't be used.
pub fn link_message(error: &MediaLinkError) -> UserMessage {
    let title = "Can't use this link";
    let (title, message) = match error {
        MediaLinkError::NotALink => (title, "Paste a web address that starts with https://.".to_owned()),
        MediaLinkError::NotHttps => (
            title,
            "Only secure links (https://) are downloaded. Try the same address with https.".to_owned(),
        ),
        MediaLinkError::NotMedia { .. } => (
            "This link isn't a video or an image",
            "It leads to a web page or another kind of file. Use a direct link to the file, such as one ending in \
             .mp4 or .jpg; pages like a YouTube video can't be downloaded."
                .to_owned(),
        ),
        MediaLinkError::WrongKind {
            expected: MediaLinkKind::Video,
        } => (
            "This link isn't a video",
            "Add images under references instead.".to_owned(),
        ),
        MediaLinkError::WrongKind {
            expected: MediaLinkKind::Image,
        } => (
            "This link isn't an image",
            "Add videos as the source instead.".to_owned(),
        ),
        MediaLinkError::TooLarge { max_bytes } => (
            "The file is too large",
            format!(
                "Links to files over {} aren't downloaded. Trim the video first.",
                size_text(*max_bytes)
            ),
        ),
        MediaLinkError::Unavailable { status } if *status == 401 || *status == 403 => (
            "The site doesn't let the app download this file",
            format!(
                "The server answered {status}: it gives the file only to a browser. Open the link in a browser, \
                 save the file, then drop it on the window."
            ),
        ),
        MediaLinkError::Unavailable { status } => (
            "Can't download the file",
            format!("The server answered {status}. Check that the link opens in a browser."),
        ),
        MediaLinkError::Network => (
            "Can't download the file",
            "Check your internet connection and try again.".to_owned(),
        ),
        MediaLinkError::NotEnoughSpace => (
            "Not enough disk space",
            "Free up space on this computer and try again.".to_owned(),
        ),
    };
    UserMessage::new(title, &message, None)
}

/// The alert for a failed download.
pub fn download_message(error: &MediaDownloadError) -> UserMessage {
    match error {
        MediaDownloadError::Link(error) => link_message(error),
        MediaDownloadError::Provider(error) => UserMessage::from_error(
            &jutsu_core::PortError::Provider(error.clone()),
            Some(ProviderKind::Account),
        )
        .unwrap_or_else(|| link_message(&MediaLinkError::Network)),
        MediaDownloadError::Io { message, .. } => UserMessage::new("Can't download the file", message, None),
        // A cancelled download says nothing; its caller drops it first.
        MediaDownloadError::Cancelled => UserMessage::new("Can't download the file", "Try again.", None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link() -> Url {
        Url::parse("https://cdn.example.com/a/clip.mp4").unwrap()
    }

    #[test]
    fn the_row_names_the_host_and_the_whole_percent() {
        assert_eq!(download_text(&link(), None), "Downloading from cdn.example.com…");
        assert_eq!(
            download_text(&link(), Some(0.429)),
            "Downloading from cdn.example.com… 42%"
        );
        assert_eq!(download_view(&link(), None, None).section, "video");
        assert_eq!(
            download_view(&link(), Some(MediaLinkKind::Image), None).section,
            "image"
        );
    }

    #[test]
    fn the_field_opens_with_the_clipboard_link_only() {
        assert_eq!(
            prefill("  https://example.com/a.mp4 "),
            "https://example.com/a.mp4"
        );
        assert_eq!(prefill("hello"), "");
    }

    #[test]
    fn a_refused_link_says_how_to_get_the_file() {
        let message = link_message(&MediaLinkError::Unavailable { status: 403 });
        assert_eq!(message.title, "The site doesn't let the app download this file");
        let message = link_message(&MediaLinkError::Unavailable { status: 404 });
        assert_eq!(
            message.message,
            "The server answered 404. Check that the link opens in a browser."
        );
        let message = link_message(&MediaLinkError::TooLarge {
            max_bytes: 2_000_000_000,
        });
        assert_eq!(
            message.message,
            "Links to files over 2 GB aren't downloaded. Trim the video first."
        );
        assert_eq!(
            link_message(&MediaLinkError::NotEnoughSpace).message,
            "Free up space on this computer and try again."
        );
    }
}
