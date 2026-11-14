//! Files on this computer a job takes: the source clip, reference images and a speech recording.
//!
//! The macOS app also stores a security-scoped bookmark of each file so the sandbox lets a later launch read it.
//! Bookmarks have no equivalent here and are not kept; a stored `bookmark` key is ignored when read.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::models::{MaterialReference, PixelSize};
use crate::provider::ServerMedia;
use crate::rules::Validation;
use crate::support::Seconds;
use crate::support::range::ClosedRange;
use crate::support::url_ext::{last_path_component, url_extension};

/// The speech recording a model that voices characters takes (`VideoControl::Speech`): one audio file they say,
/// which sets how long the clip runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LocalAudio {
    /// A file on this computer, or the link of one only the account holds (a generation made elsewhere).
    #[serde(rename = "fileURL")]
    pub file_url: Url,
    /// Seconds the recording runs, read from the file when it was added; 0 when unknown (an imported generation
    /// whose recording wasn't downloaded).
    pub duration: Seconds,
}

impl LocalAudio {
    /// The file's name, as the Create form and the agents show it.
    pub fn name(&self) -> String {
        last_path_component(&self.file_url)
    }
}

/// A reference image.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct LocalImage {
    /// The file.
    #[serde(rename = "fileURL")]
    pub file_url: Url,
    /// The `https:` link the image was downloaded from; `None` for a file the user picked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_origin: Option<Url>,
    /// The prompt the image was generated from in the app (Generate…); `None` for any other image. Templates keep
    /// it with their copies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// The media the provider already keeps of this exact file (an image of an account element), sent instead of
    /// an upload; `None` for any other image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_media: Option<ServerMedia>,
    /// The catalog version it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialReference>,
}

impl LocalImage {
    /// The image at `file_url`, with nothing else known.
    pub fn new(file_url: Url) -> Self {
        Self {
            file_url,
            remote_origin: None,
            prompt: None,
            server_media: None,
            material: None,
        }
    }

    /// The link the image was downloaded from, when a provider can take the file by link as it is.
    pub fn linkable_origin(&self) -> Option<&Url> {
        let origin = self.remote_origin.as_ref()?;
        (self.file_url.scheme() == "file"
            && crate::models::MediaLink::is_linkable(&self.file_url, crate::models::MediaLinkKind::Image))
        .then_some(origin)
    }
}

/// A local source video and the part of it the user picked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SourceClip {
    /// The file.
    #[serde(rename = "fileURL")]
    pub file_url: Url,
    /// Length of the whole file, in seconds.
    pub duration: Seconds,
    /// Its frame.
    pub pixel_size: PixelSize,
    /// Selected range, in seconds from the start of the file.
    pub range: ClosedRange<Seconds>,
    /// The user allowed upscaling a frame that is too small for object swap.
    pub allows_upscale: bool,
    /// The `https:` link the file was downloaded from; `None` for a file the user picked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_origin: Option<Url>,
    /// The catalog version it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialReference>,
}

impl SourceClip {
    /// The part `range` of the file at `file_url`.
    pub fn new(file_url: Url, duration: Seconds, pixel_size: PixelSize, range: ClosedRange<Seconds>) -> Self {
        Self {
            file_url,
            duration,
            pixel_size,
            range,
            allows_upscale: false,
            remote_origin: None,
            material: None,
        }
    }

    /// Seconds selected.
    pub fn selected_duration(&self) -> Seconds {
        self.range.upper() - self.range.lower()
    }

    /// The link the file was downloaded from while the job sends the file exactly as downloaded: the whole of it,
    /// not upscaled, in a type a provider takes by link. `None` when the clip has to be prepared and uploaded.
    pub fn linkable_origin(&self) -> Option<&Url> {
        let origin = self.remote_origin.as_ref()?;
        if self.file_url.scheme() != "file"
            || !crate::models::MediaLink::is_linkable(&self.file_url, crate::models::MediaLinkKind::Video)
        {
            return None;
        }
        let tolerance = 0.01;
        let is_whole = self.range.lower() <= tolerance && self.range.upper() >= self.duration - tolerance;
        let is_upscaled = self.allows_upscale
            && Validation::upscaled_size(self.pixel_size.width, self.pixel_size.height) != self.pixel_size;
        (is_whole && !is_upscaled).then_some(origin)
    }

    /// The file's extension, lowercased.
    pub fn file_extension(&self) -> String {
        url_extension(&self.file_url).to_lowercase()
    }
}
