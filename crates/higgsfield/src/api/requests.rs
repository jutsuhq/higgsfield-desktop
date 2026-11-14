//! Bodies of the model endpoints. The API rejects `null`, so nothing is sent as one: unset fields and, where the
//! schema refuses them, empty arrays are left out. The client writes every body with its keys sorted.

use std::collections::BTreeMap;

use serde::{Serialize, Serializer};
use url::Url;
use uuid::Uuid;

fn one_video<S: Serializer>(url: &Url, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_seq([url])
}

/// Body of all three Genjutsu workflows. The API rejects `null`, so `prompt` and `image_urls` are always
/// sent, and `preset_id` is omitted (not null) outside restyle, whose schema forbids extra keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GenerationRequest {
    pub video_url: Url,
    pub image_urls: Vec<Url>,
    pub prompt: String,
    /// "480p", "720p" or "1080p".
    pub resolution: String,
    /// Restyle only, where it is required.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset_id: Option<Uuid>,
}

/// Body of Seedance 2.5 `text-to-video` (no media) and `reference-to-video`. Both schemas forbid unknown keys and
/// reject `null`, an empty prompt and empty arrays, so those are omitted rather than sent empty. Seedance has no
/// `mode` field: the endpoint is the mode.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SeedanceRequest {
    /// Required by `text-to-video`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub image_urls: Vec<Url>,
    /// The source clip first; `reference-to-video` only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub video_urls: Vec<Url>,
    /// Whole seconds, 4–30.
    pub duration: i64,
    /// "480p", "720p" or "1080p".
    pub resolution: String,
    /// "16:9", "9:16", …; the server's default (16:9) when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    pub generate_audio: bool,
    /// `bitrate_mode`, "standard" or "high"; the server's default (`high`) when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate_mode: Option<String>,
}

/// Body of Seedance 2.5 `image-to-video`: the clip starts on `image_url` and, when given, ends on `end_image_url`.
/// The output's frame follows the start image, so there is no `aspect_ratio`. Like the other Seedance schemas it
/// forbids unknown keys and rejects `null` and an empty prompt, so unset fields are omitted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SeedanceFramesRequest {
    /// Optional here; at least one character when sent.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    /// The start frame.
    pub image_url: Url,
    /// The end frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_image_url: Option<Url>,
    /// Whole seconds, 4–30.
    pub duration: i64,
    /// "480p", "720p" or "1080p".
    pub resolution: String,
    pub generate_audio: bool,
    /// `bitrate_mode`, "standard" or "high"; the server's default (`high`) when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate_mode: Option<String>,
}

/// Body of Seedance 2.5 `video-edit` and `video-extend`. Like the other Seedance schemas they forbid unknown keys and
/// reject `null` and empty arrays, so unset fields are omitted. `video-edit` takes no `duration` (the output follows
/// the source); for `video-extend` it is the length of the new part. `video_urls` and `audio_urls` aren't sent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SeedanceVideoRequest {
    /// Required by both, at least one character.
    pub prompt: String,
    /// The clip to edit or extend.
    pub video_url: Url,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub image_urls: Vec<Url>,
    /// Whole seconds, 4–30; `video-extend` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<i64>,
    /// "480p", "720p" or "1080p".
    pub resolution: String,
    pub generate_audio: bool,
    /// `bitrate_mode`, "standard" or "high"; the server's default (`high`) when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate_mode: Option<String>,
}

/// `keep_original_sound`: whether the result keeps the motion video's sound (server default `yes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum OriginalSound {
    #[serde(rename = "yes")]
    Keep,
    #[serde(rename = "no")]
    Drop,
}

/// `character_orientation`: the character faces as in the image or as in the video (server default `video`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    Image,
    Video,
}

/// Body of Kling 3.0 Motion Control, Standard and Pro alike. There is no `duration` (the output follows the source)
/// and no background parameter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct KlingMotionRequest {
    /// The character.
    pub image_url: Url,
    /// The motion, 3–30 s.
    pub video_url: Url,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    /// The server's default when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_original_sound: Option<OriginalSound>,
    /// The server's default when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_orientation: Option<Orientation>,
}

/// `mode` of Kling O3: Standard or Pro (server default `pro`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum KlingQuality {
    #[serde(rename = "std")]
    Standard,
    #[serde(rename = "pro")]
    Pro,
}

/// Body of Kling O3's `video-reference`: a clip from the prompt (required), one reference video and up to four
/// images. `elements` and `aspect_ratio` are left out, so the server's defaults apply.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct KlingVideoReferenceRequest {
    pub prompt: String,
    /// The one reference video (`video_urls`, exactly one item).
    #[serde(rename = "video_urls", serialize_with = "one_video")]
    pub video_url: Url,
    /// 0–4, in order; left out when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub image_urls: Vec<Url>,
    #[serde(rename = "mode")]
    pub quality: KlingQuality,
    /// Whole seconds of the generated clip, 3–10.
    pub duration: i64,
}

/// Body of Kling O3's `video-edit`: the one source video (3–15.5 s) changed as the prompt (required) says, with up
/// to four images. No `duration`: the output follows the source. `elements` is left out.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct KlingVideoEditRequest {
    pub prompt: String,
    /// The one source video (`video_urls`, exactly one item).
    #[serde(rename = "video_urls", serialize_with = "one_video")]
    pub video_url: Url,
    /// 0–4, in order; left out when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub image_urls: Vec<Url>,
    /// `mode`: `std` or `pro` (server default `pro`); `4k` isn't sent.
    #[serde(rename = "mode")]
    pub quality: KlingQuality,
}

/// Body of the reference-to-video endpoints that take a prompt (required), reference videos and images, a length
/// and a resolution: Wan 3.0 and 3.0 Prime, Wan 2.7, Cinema Studio 4.0 and `MiniMax` H3. Their schemas differ only in
/// the optional fields, so `aspect_ratio`, `generate_audio` and Cinema Studio's creative controls are sent only when
/// set; empty arrays are left out (Wan 2.7 refuses an empty one). Seeds, negative prompts, documents, audio
/// references and `MiniMax`'s `aigc_watermark` aren't sent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ReferenceToVideoRequest {
    pub prompt: String,
    /// The source video, the one item of `video_urls`.
    #[serde(rename = "video_urls", serialize_with = "one_video")]
    pub video_url: Url,
    /// In order; left out when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub image_urls: Vec<Url>,
    /// Whole seconds of the generated clip.
    pub duration: i64,
    /// "480p", "720p", "1080p" or `MiniMax` H3's "2K", from the model's own list.
    pub resolution: String,
    /// One of the endpoint's frames; left out for the server's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    /// Left out for a model without a soundtrack (Wan 2.7).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generate_audio: Option<bool>,
    /// Cinema Studio's creative controls by their field (`camera_model`, `genre`, …), each one of the schema's
    /// values; empty for the other models, and for the controls the director picks.
    #[serde(flatten)]
    pub controls: BTreeMap<String, String>,
}

/// Body of Kling 4.0's endpoints (open.higgsfield.ai, read 2026-10-28), whose schemas share these fields:
/// `text-to-video` (`prompt`*, `aspect_ratio` 21:9, 16:9, 9:16 or 1:1, default 16:9), `image-to-video` (`image_url`*
/// the start frame, `end_image_url`, `prompt` optional, no `aspect_ratio`: the frames set it), `reference-to-video`
/// (`prompt`*, `image_urls` ≤10 and `video_urls` ≤5, at least one of them non-empty, named `@image_1`… and
/// `@video_1`… in the prompt) and `video-edit` (`prompt`*, `video_url`* the clip to edit, with the same references; the
/// clip is `@video_1`). Each takes `duration` 3–30 (the output's, also for an edit) and `resolution` 720p or 1080p;
/// the reference and edit endpoints add `adaptive` to the frames, their default. Unset fields and empty arrays are left
/// out. `elements` (Kling element IDs) and image-to-video's middle frames aren't sent.
///
/// Kling 4.0 Flash's endpoints (`kling-video/v4.0-flash/…`, model pages read 2026-11-11) take the same fields with
/// `duration` 3–20, `resolution` 720p only, no `end_image_url` and no `elements`; their reference and edit
/// `aspect_ratio` defaults to 16:9, so Auto is sent as `adaptive` there too.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Kling4Request {
    /// Left out when blank, which only `image-to-video` allows.
    #[serde(skip_serializing_if = "is_blank")]
    pub prompt: String,
    /// `image-to-video`'s start frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<Url>,
    /// `image-to-video`'s end frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_image_url: Option<Url>,
    /// `video-edit`'s clip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_url: Option<Url>,
    /// References, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub image_urls: Vec<Url>,
    /// Reference videos, in order: the source of a clip made from references.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub video_urls: Vec<Url>,
    /// Whole seconds, 3–30.
    pub duration: i64,
    /// "720p" or "1080p".
    pub resolution: String,
    /// One of the endpoint's frames, or `adaptive`; left out for the server's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
}

impl Kling4Request {
    /// `aspect_ratio` of the reference and edit endpoints that follows the references.
    pub const ADAPTIVE: &'static str = "adaptive";
}

/// A blank prompt is none, as validation reads it; the schemas refuse an empty one.
fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

// Bodies of the image endpoints, from each model's page at docs.higgsfield.ai/docs/models (checked 2026-10-15).
// Every field left out takes the server's documented default.

/// `higgsfield-ai/soul/v2/standard`: text to image, 720p unless asked otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SoulImageRequest {
    pub prompt: String,
    /// One of 9:16, 16:9, 4:3, 3:4, 1:1, 2:3, 3:2; the server's default is 1:1.
    pub aspect_ratio: String,
    /// 1 or 4 only.
    pub batch_size: i64,
}

/// `xai/grok-imagine-image-2.0`: one image, generated from the prompt or, with `image_urls`, an edit of up to 10
/// images. Quality (medium) and resolution (1k) stay at the server's defaults.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct GrokImageRequest {
    pub prompt: String,
    /// `auto`, 1:1, 1:2, 2:1, 3:2, 2:3, 4:3, 3:4, 16:9 or 9:16.
    pub aspect_ratio: String,
    /// Left out when empty: the request then generates rather than edits.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub image_urls: Vec<Url>,
}

/// `alibaba/qwen-image-3/edit`: one edited image from 1–3 input images, 1k.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct QwenEditRequest {
    /// At least one non-blank character.
    pub prompt: String,
    pub image_urls: Vec<Url>,
    /// 1:1, 2:3, 3:2, 3:4, 4:3, 7:9, 9:7, 9:16, 16:9 or 21:9.
    pub aspect_ratio: String,
}
