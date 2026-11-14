//! What the API answers, and the endpoints the app submits to.

use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;
use url::Url;
use uuid::Uuid;

use crate::core::{GenerationStatus, ImageStatus, Moderation, is_https};

/// Where to PUT a file and the URL the API reads it from afterwards.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct UploadTarget {
    /// Presigned, valid about an hour. Never log it.
    pub upload_url: Url,
    pub public_url: Url,
    /// Headers the presigned signature covers, e.g. `x-amz-tagging`; the PUT fails without them.
    pub upload_headers: Option<HashMap<String, String>>,
}

/// Prints without the presigned URL.
impl fmt::Debug for UploadTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadTarget")
            .field("upload_url", &"<redacted>")
            .field("public_url", &self.public_url)
            .field("upload_headers", &self.upload_headers)
            .finish()
    }
}

/// The model endpoints the app submits to, by their endpoint ID. Each takes its own body: `GenerationRequest` for
/// Genjutsu, `SeedanceRequest`, `SeedanceVideoRequest` for Seedance's Edit and Extend, `SeedanceFramesRequest` for
/// its image-to-video, and `KlingMotionRequest`; the image models `SoulImageRequest`, `GrokImageRequest` and
/// `QwenEditRequest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelEndpoint {
    MotionTransfer,
    Restyle,
    ObjectSwap,
    /// Genjutsu Lite's one REST endpoint; the same body as Genjutsu's (`GenerationRequest`).
    GenjutsuLiteMotionTransfer,
    SeedanceTextToVideo,
    SeedanceReferenceToVideo,
    SeedanceImageToVideo,
    SeedanceVideoEdit,
    SeedanceVideoExtend,
    /// Kling's quality is the endpoint, not a field.
    KlingMotionControlStandard,
    KlingMotionControlPro,
    /// Kling O3 from one reference video; its quality is the `mode` field.
    KlingO3VideoReference,
    /// Kling O3's Edit; its quality is the `mode` field.
    KlingO3VideoEdit,
    /// Kling 2.6 Motion Control, the same body as 3.0's (`KlingMotionRequest`).
    Kling26MotionControlStandard,
    Kling26MotionControlPro,
    /// `ReferenceToVideoRequest`.
    Wan30ReferenceToVideo,
    Wan30PrimeReferenceToVideo,
    Wan27ReferenceToVideo,
    CinemaStudio4,
    MinimaxH3ReferenceToVideo,
    /// Kling 4.0's four endpoints, one body (`Kling4Request`).
    Kling4TextToVideo,
    Kling4ImageToVideo,
    Kling4ReferenceToVideo,
    Kling4VideoEdit,
    /// Kling 4.0 Flash's four endpoints, the same body at 720p and 3–20 s, without an end frame.
    Kling4FlashTextToVideo,
    Kling4FlashImageToVideo,
    Kling4FlashReferenceToVideo,
    Kling4FlashVideoEdit,
    SoulV2,
    /// Generates without `image_urls`, edits with them.
    GrokImage2,
    QwenImage3Edit,
}

impl ModelEndpoint {
    pub const ALL: [Self; 31] = [
        Self::MotionTransfer,
        Self::Restyle,
        Self::ObjectSwap,
        Self::GenjutsuLiteMotionTransfer,
        Self::SeedanceTextToVideo,
        Self::SeedanceReferenceToVideo,
        Self::SeedanceImageToVideo,
        Self::SeedanceVideoEdit,
        Self::SeedanceVideoExtend,
        Self::KlingMotionControlStandard,
        Self::KlingMotionControlPro,
        Self::KlingO3VideoReference,
        Self::KlingO3VideoEdit,
        Self::Kling26MotionControlStandard,
        Self::Kling26MotionControlPro,
        Self::Wan30ReferenceToVideo,
        Self::Wan30PrimeReferenceToVideo,
        Self::Wan27ReferenceToVideo,
        Self::CinemaStudio4,
        Self::MinimaxH3ReferenceToVideo,
        Self::Kling4TextToVideo,
        Self::Kling4ImageToVideo,
        Self::Kling4ReferenceToVideo,
        Self::Kling4VideoEdit,
        Self::Kling4FlashTextToVideo,
        Self::Kling4FlashImageToVideo,
        Self::Kling4FlashReferenceToVideo,
        Self::Kling4FlashVideoEdit,
        Self::SoulV2,
        Self::GrokImage2,
        Self::QwenImage3Edit,
    ];

    /// The endpoint ID, which is also its path.
    #[must_use]
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::MotionTransfer => "higgsfield/genjutsu/motion-transfer/v1.0",
            Self::Restyle => "higgsfield/genjutsu/restyle/v1.0",
            Self::ObjectSwap => "higgsfield/genjutsu/object-swap/v1.0",
            Self::GenjutsuLiteMotionTransfer => "higgsfield/genjutsu-light/motion-transfer/v1.0",
            Self::SeedanceTextToVideo => "bytedance/seedance-2.5/text-to-video",
            Self::SeedanceReferenceToVideo => "bytedance/seedance-2.5/reference-to-video",
            Self::SeedanceImageToVideo => "bytedance/seedance-2.5/image-to-video",
            Self::SeedanceVideoEdit => "bytedance/seedance-2.5/video-edit",
            Self::SeedanceVideoExtend => "bytedance/seedance-2.5/video-extend",
            Self::KlingMotionControlStandard => "kling-video/v3/motion-control/std",
            Self::KlingMotionControlPro => "kling-video/v3/motion-control/pro",
            Self::KlingO3VideoReference => "kling-video/o3/video-reference",
            Self::KlingO3VideoEdit => "kling-video/o3/video-edit",
            Self::Kling26MotionControlStandard => "kling-video/motion-control/std",
            Self::Kling26MotionControlPro => "kling-video/motion-control/pro",
            Self::Wan30ReferenceToVideo => "alibaba/wan-3.0/reference-to-video",
            Self::Wan30PrimeReferenceToVideo => "alibaba/wan-3.0-prime/reference-to-video",
            Self::Wan27ReferenceToVideo => "wan/v2.7/reference-to-video",
            Self::CinemaStudio4 => "higgsfield/cinema-studio/4.0",
            Self::MinimaxH3ReferenceToVideo => "minimax/h3/reference-to-video",
            Self::Kling4TextToVideo => "kling-video/v4.0/text-to-video",
            Self::Kling4ImageToVideo => "kling-video/v4.0/image-to-video",
            Self::Kling4ReferenceToVideo => "kling-video/v4.0/reference-to-video",
            Self::Kling4VideoEdit => "kling-video/v4.0/video-edit",
            Self::Kling4FlashTextToVideo => "kling-video/v4.0-flash/text-to-video",
            Self::Kling4FlashImageToVideo => "kling-video/v4.0-flash/image-to-video",
            Self::Kling4FlashReferenceToVideo => "kling-video/v4.0-flash/reference-to-video",
            Self::Kling4FlashVideoEdit => "kling-video/v4.0-flash/video-edit",
            Self::SoulV2 => "higgsfield-ai/soul/v2/standard",
            Self::GrokImage2 => "xai/grok-imagine-image-2.0",
            Self::QwenImage3Edit => "alibaba/qwen-image-3/edit",
        }
    }

    /// The endpoint with this ID.
    #[must_use]
    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|endpoint| endpoint.raw_value() == raw)
    }
}

/// A submitted request as the API answered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmittedRequest {
    pub request_id: String,
    /// `X-Correlation-ID`, for support requests.
    pub correlation_id: Option<String>,
    /// Where the API said to poll and cancel; pass them back to `status` and `cancel`, which check the host.
    pub status_url: Option<Url>,
    pub cancel_url: Option<Url>,
    /// What the submit answer said about the job, when it said anything.
    pub initial_status: Option<GenerationStatus>,
}

/// A Restyle preset.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Preset {
    pub id: Uuid,
    pub name: String,
    pub preview_url: Url,
}

#[derive(Deserialize)]
pub(crate) struct UrlHolder {
    pub url: Option<Url>,
}

/// The submit answer has the shape of a status answer, with the polling and cancel URLs.
#[derive(Deserialize)]
pub(crate) struct SubmitResponse {
    pub request_id: String,
    pub status: Option<String>,
    pub status_url: Option<Url>,
    pub cancel_url: Option<Url>,
    pub video: Option<UrlHolder>,
    pub error: Option<String>,
}

impl SubmitResponse {
    /// `None` when the answer leaves the state to the first poll, including a `completed` without a video URL:
    /// the job is paid for, so the poll fetches the result instead of failing it here.
    pub fn initial_status(&self) -> Option<GenerationStatus> {
        let status = self.status.as_deref()?;
        let video_url = self.video.as_ref().and_then(|video| video.url.clone());
        if status == "completed" && video_url.is_none() {
            return None;
        }
        Some(generation_status(status, video_url, self.error.clone()))
    }
}

#[derive(Deserialize)]
pub(crate) struct StatusResponse {
    pub status: String,
    pub video: Option<UrlHolder>,
    /// Image models answer `images: [{url}]` instead of `video`.
    pub images: Option<Vec<UrlHolder>>,
    pub error: Option<String>,
}

impl StatusResponse {
    pub fn generation_status(&self) -> GenerationStatus {
        generation_status(
            &self.status,
            self.video.as_ref().and_then(|video| video.url.clone()),
            self.error.clone(),
        )
    }

    /// As `generation_status`, for an image model: completed with at least one https image, else failed, since
    /// polling again won't bring one.
    pub fn image_status(&self) -> ImageStatus {
        match self.generation_status() {
            GenerationStatus::Queued => ImageStatus::Queued,
            GenerationStatus::InProgress => ImageStatus::InProgress,
            GenerationStatus::Failed { message } if self.status != "completed" => {
                ImageStatus::Failed { message }
            }
            GenerationStatus::Failed { .. } | GenerationStatus::Completed { .. } => {
                let urls: Vec<Url> = self
                    .images
                    .iter()
                    .flatten()
                    .filter_map(|image| image.url.clone())
                    .filter(is_https)
                    .collect();
                if urls.is_empty() {
                    ImageStatus::Failed {
                        message: Some("Completed without an image URL.".to_owned()),
                    }
                } else {
                    ImageStatus::Completed { image_urls: urls }
                }
            }
            GenerationStatus::Rejected(moderation) => ImageStatus::Rejected(moderation),
            GenerationStatus::Canceled => ImageStatus::Canceled,
        }
    }
}

pub(crate) fn generation_status(
    status: &str,
    video_url: Option<Url>,
    error: Option<String>,
) -> GenerationStatus {
    match status {
        "queued" => GenerationStatus::Queued,
        "completed" => match video_url {
            Some(video_url) => GenerationStatus::Completed { video_url },
            None => GenerationStatus::Failed {
                message: Some("Completed without a video URL.".to_owned()),
            },
        },
        "failed" => GenerationStatus::Failed { message: error },
        "nsfw" => GenerationStatus::Rejected(Moderation::ExplicitContent),
        "canceled" | "cancelled" => GenerationStatus::Canceled,
        // A status added later is most likely another intermediate stage; keep polling it.
        _ => GenerationStatus::InProgress,
    }
}

/// `POST /estimate/{model}`: both amounts are decimal strings.
#[derive(Deserialize)]
pub(crate) struct EstimateResponse {
    #[serde(rename = "credits")]
    _credits: Option<String>,
    pub usd: String,
}

#[derive(Deserialize)]
pub(crate) struct PresetCatalog {
    pub items: Vec<PresetItem>,
}

#[derive(Deserialize)]
pub(crate) struct PresetItem {
    pub id: Uuid,
    pub name: String,
    pub preview_url: Url,
}
