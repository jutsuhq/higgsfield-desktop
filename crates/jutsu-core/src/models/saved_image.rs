//! Images made with Generate…

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime, UtcOffset};
use url::Url;
use uuid::Uuid;

use crate::models::{LocalImage, MaterialRemote};
use crate::support::text::{char_count, is_whitespace_or_newline};

/// An image made with Generate…, saved in the output folder's `Images` folder (`OutputSubfolder::Images`) as soon
/// as it is ready. Library › Images lists them; the file is the image, and this keeps what made it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SavedImage {
    /// Its identity.
    pub id: Uuid,
    /// The file.
    pub file_url: Url,
    /// The prompt.
    pub prompt: String,
    /// The image model's name, e.g. "Higgsfield Soul 2.0".
    pub model: String,
    /// "16:9"; `None` when unknown.
    pub aspect_ratio: Option<String>,
    /// When it was made.
    pub created_at: OffsetDateTime,
    /// Where the provider served it; its links expire, so it isn't read again.
    pub remote_url: Option<Url>,
    /// The catalog star, projected into the image card.
    pub is_starred: bool,
    /// An agent's name for it (`MaterialGeneration::title`), which the card shows instead of the prompt.
    pub title: Option<String>,
}

impl SavedImage {
    /// The longest part of the prompt a file name keeps, in characters.
    pub const MAX_TITLE_LENGTH: usize = 48;

    /// "2026-10-15 10.41.03 A knight in silver armor", without the extension: the date first, so the files sort by
    /// when they were made, then the start of the prompt. `offset` stands in for the Mac's time zone.
    pub fn base_name(prompt: &str, created_at: OffsetDateTime, offset: UtcOffset) -> String {
        let local = created_at.to_offset(offset);
        let date = format!(
            "{:04}-{:02}-{:02} {:02}.{:02}.{:02}",
            local.year(),
            u8::from(local.month()),
            local.day(),
            local.hour(),
            local.minute(),
            local.second()
        );
        let mut title = String::new();
        for word in prompt
            .split(is_whitespace_or_newline)
            .filter(|word| !word.is_empty())
        {
            let next = if title.is_empty() {
                word.to_owned()
            } else {
                format!("{title} {word}")
            };
            if char_count(&next) > Self::MAX_TITLE_LENGTH {
                break;
            }
            title = next;
        }
        // No path separators, colons or leading dots: the name stays one Finder-safe component.
        let safe: String = title
            .chars()
            .map(|c| if "/:\\".contains(c) { '-' } else { c })
            .collect();
        let safe = safe.trim_start_matches('.');
        if safe.is_empty() {
            date
        } else {
            format!("{date} {safe}")
        }
    }
}

/// An image that was made, and paid for, but isn't saved into the library yet. Kept until it is, so a download
/// that fails or a quit doesn't lose it while the provider's link still serves it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PendingImage {
    /// Its identity.
    pub id: Uuid,
    /// Where the provider serves it.
    #[serde(rename = "remoteURL")]
    pub remote_url: Url,
    /// The prompt.
    pub prompt: String,
    /// The image model's name.
    pub model: String,
    /// "16:9"; `None` when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    /// When it was made.
    #[serde(with = "crate::support::date::rfc3339")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub created_at: OffsetDateTime,
    /// The account's generation it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<MaterialRemote>,
    /// The exact request, encoded as JSON.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::support::codable::base64_option"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub parameters: Option<Vec<u8>>,
    /// The images it was made from.
    #[serde(default)]
    pub input_images: Vec<LocalImage>,
    /// Its group of alternate generations.
    #[serde(default, rename = "variantGroupID", skip_serializing_if = "Option::is_none")]
    pub variant_group_id: Option<Uuid>,
    /// An agent's name for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl PendingImage {
    /// How long the provider serves a result, so how long saving it again is worth trying.
    pub const LIFETIME: Duration = Duration::days(7);

    /// A pending image with nothing but what made it.
    pub fn new(
        id: Uuid,
        remote_url: Url,
        prompt: impl Into<String>,
        model: impl Into<String>,
        aspect_ratio: Option<String>,
        created_at: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            remote_url,
            prompt: prompt.into(),
            model: model.into(),
            aspect_ratio,
            created_at,
            remote: None,
            parameters: None,
            input_images: Vec::new(),
            variant_group_id: None,
            title: None,
        }
    }
}
