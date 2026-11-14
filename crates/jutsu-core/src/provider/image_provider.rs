//! Reference images a connection makes.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::models::Cost;
use crate::ports::PortError;
use crate::provider::{PreparedFile, ProviderKind, Rejection, RemoteAsset, RemoteJob};
use crate::support::codable::Empty;

/// What the input image of an edit may be. Coded as Swift codes the enum: `{"optional":{}}`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "specta", specta(type = ImageModelInputRepr))]
#[serde(into = "ImageModelInputRepr", from = "ImageModelInputRepr")]
pub enum ImageModelInput {
    /// Text to image only.
    #[default]
    None,
    /// Generates from the prompt alone, or edits an image when one is given.
    Optional,
    /// Edits only: an image is required.
    Required,
}

/// The stored shape of an `ImageModelInput`.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", rename = "ImageModelInput")]
#[allow(missing_docs, reason = "each case mirrors `ImageModelInput`")]
pub enum ImageModelInputRepr {
    None(Empty),
    Optional(Empty),
    Required(Empty),
}

impl From<ImageModelInput> for ImageModelInputRepr {
    fn from(input: ImageModelInput) -> Self {
        match input {
            ImageModelInput::None => Self::None(Empty {}),
            ImageModelInput::Optional => Self::Optional(Empty {}),
            ImageModelInput::Required => Self::Required(Empty {}),
        }
    }
}

impl From<ImageModelInputRepr> for ImageModelInput {
    fn from(repr: ImageModelInputRepr) -> Self {
        match repr {
            ImageModelInputRepr::None(_) => Self::None,
            ImageModelInputRepr::Optional(_) => Self::Optional,
            ImageModelInputRepr::Required(_) => Self::Required,
        }
    }
}

/// An image model the connection offers for making reference images.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ImageModel {
    /// The REST endpoint ID, or the account's model ID.
    pub id: String,
    /// Its title.
    pub title: String,
    /// How many images one request can make, ascending; `[1]` when it makes one.
    pub variant_counts: Vec<i64>,
    /// The frames the model takes, as "W:H".
    pub aspect_ratios: Vec<String>,
    /// What the input image of an edit may be.
    pub input: ImageModelInput,
    /// The most input images one request takes, where the model's catalog says; `None` takes one, unless
    /// `unstated_input_limit` is set. 0 with `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_inputs: Option<i64>,
    /// For a model that takes images but whose catalog states no most: how many may be sent anyway,
    /// `INPUT_BOUND`. The provider's own quote then refuses more than the model takes, before anything is sent or
    /// paid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unstated_input_limit: Option<i64>,
    /// The public list price of one image, labelled as an estimate; `None` when none is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_price: Option<Cost>,
}

impl ImageModel {
    /// The most input images any request sends, where a model's catalog states no limit of its own: as many as
    /// Nano Banana Pro takes, the most of the account's models. The same as the agent crate's bound, which agents
    /// are told.
    pub const INPUT_BOUND: i64 = 14;

    /// A text-to-image model making one image per request, with no list price.
    pub fn new(id: impl Into<String>, title: impl Into<String>, aspect_ratios: Vec<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            variant_counts: vec![1],
            aspect_ratios,
            input: ImageModelInput::None,
            max_inputs: None,
            unstated_input_limit: None,
            list_price: None,
        }
    }

    /// Sets the variant counts; an empty list makes one.
    #[must_use]
    pub fn with_variant_counts(mut self, counts: Vec<i64>) -> Self {
        self.variant_counts = if counts.is_empty() { vec![1] } else { counts };
        self
    }

    /// How many input images one request may send: none for a text-only model, else the catalog's limit, else
    /// `unstated_input_limit`, else one.
    pub fn input_limit(&self) -> i64 {
        if self.input == ImageModelInput::None {
            return 0;
        }
        self.max_inputs.or(self.unstated_input_limit).unwrap_or(1).max(1)
    }

    /// The limit is a bound rather than the model's own: its catalog states none, and the provider's quote refuses
    /// what the model doesn't take.
    pub fn input_limit_is_bound(&self) -> bool {
        self.input != ImageModelInput::None
            && self.max_inputs.is_none()
            && self.unstated_input_limit.is_some()
    }

    /// The ratio of `aspect_ratios` closest to `width` × `height`; the first one when the size is unusable.
    pub fn nearest_aspect_ratio(&self, width: i64, height: i64) -> Option<String> {
        if width <= 0 || height <= 0 {
            return self.aspect_ratios.first().cloned();
        }
        let target = (width as f64 / height as f64).ln();
        let mut best: Option<(&String, f64)> = None;
        for ratio in &self.aspect_ratios {
            let Some(shape) = Self::width_over_height(ratio) else {
                continue;
            };
            let distance = (shape.ln() - target).abs();
            if best.is_none_or(|(_, closest)| distance < closest) {
                best = Some((ratio, distance));
            }
        }
        best.map(|(ratio, _)| ratio.clone())
            .or_else(|| self.aspect_ratios.first().cloned())
    }

    /// "16:9" as 1.78; `None` for "auto" or anything that isn't two positive numbers.
    pub fn width_over_height(ratio: &str) -> Option<f64> {
        let sides: Vec<f64> = ratio
            .split(':')
            .filter(|side| !side.is_empty())
            .filter_map(|side| side.parse().ok())
            .collect();
        match sides.as_slice() {
            [width, height] if *width > 0.0 && *height > 0.0 => Some(width / height),
            _ => None,
        }
    }
}

/// One request for reference images.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ImageRequest {
    /// The model's ID.
    pub model: String,
    /// The prompt.
    pub prompt: String,
    /// "W:H".
    pub aspect_ratio: String,
    /// Images the request makes, one of the model's `variant_counts`.
    pub count: i64,
    /// The uploaded images to edit; empty to generate from the prompt.
    #[serde(default)]
    pub input_images: Vec<RemoteAsset>,
}

/// Where a submitted image job is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImageJobStatus {
    /// Still running.
    Pending,
    /// The images, never empty.
    Completed(Vec<Url>),
    /// Failed, with the provider's message when it gave one.
    Failed {
        /// The message.
        message: Option<String>,
    },
    /// Refused by content moderation.
    Rejected(Rejection),
    /// Canceled.
    Canceled,
}

/// Makes reference images: a small service beside `GenerationProvider`, not one of its modes. Its jobs never enter
/// the job engine or the Queue; whoever runs them polls `image_status` and counts their price toward the daily cap
/// (`ImageCharge`).
#[async_trait]
pub trait ImageProvider: Send + Sync {
    /// The connection kind.
    fn kind(&self) -> ProviderKind;
    /// The account or key the jobs belong to (`GenerationProvider::owner`), so they are followed again only there.
    fn owner(&self) -> Option<String>;
    /// The curated models of this connection; empty when it can't make images.
    async fn image_models(&self) -> Result<Vec<ImageModel>, PortError>;
    /// The price of the whole request, the provider's own quote when it gives one.
    async fn image_estimate(&self, request: &ImageRequest) -> Result<Cost, PortError>;
    /// The input of an edit.
    async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError>;
    /// One job per request, or several when the provider makes a job per image.
    async fn submit_images(
        &self,
        request: &ImageRequest,
        idempotency_key: Uuid,
    ) -> Result<Vec<RemoteJob>, PortError>;
    /// Where a submitted job is.
    async fn image_status(&self, job: &RemoteJob) -> Result<ImageJobStatus, PortError>;
    /// Fails when the job can't be cancelled any more.
    async fn cancel_images(&self, job: &RemoteJob) -> Result<(), PortError>;
}
