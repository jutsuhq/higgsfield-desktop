use serde_json::Value;
use url::Url;

use crate::mcp::error::McpError;
use crate::mcp::json::{self, Object};

/// What an uploaded file is to the generation. The server renames `video` and `image` to these for the
/// `VideoRequest` models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaRole {
    Video,
    Image,
    /// Seedance's start and end frames, sent with `mode` `omni_reference`.
    StartFrame,
    EndFrame,
    /// A speech recording, as the models that voice one list it in `medias[].roles`.
    Audio,
    /// Intone's speakers, the one or two people who say the speech.
    Character,
}

impl MediaRole {
    #[must_use]
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Video => "video_references",
            Self::Image => "image_references",
            Self::StartFrame => "start_image",
            Self::EndFrame => "end_image",
            Self::Audio => "audio_references",
            Self::Character => "character_images",
        }
    }

    /// History names the inputs `video` and `image`; `generate_video` takes the `_references` names. A start or
    /// end frame keeps its role. A speech recording is `audio_references`, or `audio` by the same rule.
    #[must_use]
    pub fn from_history_role(role: &str) -> Option<Self> {
        Some(match role {
            "video" | "video_references" => Self::Video,
            "image" | "image_references" => Self::Image,
            "start_image" => Self::StartFrame,
            "end_image" => Self::EndFrame,
            "audio" | "audio_references" => Self::Audio,
            "character_images" => Self::Character,
            _ => return None,
        })
    }
}

/// An uploaded file, by its `media_id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VideoMedia {
    pub role: MediaRole,
    pub id: String,
}

impl VideoMedia {
    pub fn new(role: MediaRole, id: impl Into<String>) -> Self {
        Self { role, id: id.into() }
    }
}

/// The `params` of one `generate_video` call. `use_unlim` is always sent, `false` unless `uses_unlimited`: left
/// out, the server submits nothing for a model the allowance covers and answers with `unlim_choice`, a question
/// for the user.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoRequest {
    /// A `generate_video` model, by the ID `models_explore` lists it under. Any ID is one: which models the app runs
    /// is data, so a new account model needs no code here.
    pub model: String,
    /// Left out when empty, as for Seedance's `t2v`.
    pub media: Vec<VideoMedia>,
    /// Left out when empty.
    pub prompt: String,
    /// "480p", "720p" or "1080p", from the model's `resolution` options; Kling takes `mode` instead.
    pub resolution: Option<String>,
    pub mode: Option<String>,
    /// Seconds.
    pub duration: Option<f64>,
    /// Whether the model generates a soundtrack.
    pub generate_audio: Option<bool>,
    /// Kling only: which input the background comes from; left out for the server's default, the image.
    pub background_source: Option<String>,
    /// "16:9", "9:16", "auto"…; a reference video sets the frame otherwise.
    pub aspect_ratio: Option<String>,
    /// Seedance: "standard" (the server's default) or "high".
    pub bitrate_mode: Option<String>,
    /// Seedance's Extend only (`extension_mode`): `EXTEND_FORWARD`.
    pub extension_mode: Option<String>,
    /// `use_unlim`: pay from the plan's unlimited allowance instead of credits.
    pub uses_unlimited: bool,
    /// `count`: how many variants the request makes, 1–4, each its own job. Sent only above 1, and never with
    /// `use_unlim`: the server caps an unlimited request to one generation.
    pub count: i64,
    /// Seedance 2.5's `draft`: a 480p draft that `draft_job_id` can finalize at 1080p within seven days.
    pub draft: bool,
    /// Seedance 2.5's `draft_job_id`: the completed draft job to finalize at 1080p.
    pub draft_job_id: Option<String>,
}

impl VideoRequest {
    /// Genjutsu Motion Transfer.
    pub const MOTION_CONTROL: &'static str = "hf_mult_motion_control";
    /// Genjutsu Object Swap.
    pub const REPLACE_OBJECT: &'static str = "hf_mult_replace_object";
    pub const SEEDANCE_25: &'static str = "seedance_2_5";
    pub const KLING_MOTION_CONTROL: &'static str = "kling3_0_motion_control";

    /// `mode` values, as `models_explore get` names them.
    pub const TEXT_TO_VIDEO: &'static str = "t2v";
    pub const OMNI_REFERENCE: &'static str = "omni_reference";
    /// Seedance's Edit: one reference video changed by the prompt.
    pub const SEEDANCE_EDIT: &'static str = "video_edit";
    /// Seedance's Extend: `duration` is the added part. Needs `extension_mode`.
    pub const SEEDANCE_EXTEND: &'static str = "video_extension";
    /// Extend's `extension_mode`: the new part follows the clip.
    pub const EXTEND_FORWARD: &'static str = "forward";
    pub const KLING_STANDARD: &'static str = "std";
    pub const KLING_PRO: &'static str = "pro";
    /// Kling 4.0's `mode`s.
    pub const KLING4_TEXT_TO_VIDEO: &'static str = "text-to-video";
    pub const KLING4_IMAGE_TO_VIDEO: &'static str = "image-to-video";
    pub const KLING4_REFERENCE_TO_VIDEO: &'static str = "reference-to-video";
    pub const KLING4_VIDEO_EDIT: &'static str = "video-edit";
    /// Gemini Omni Flash 1.2's own `mode`s.
    pub const GEMINI_EDIT: &'static str = "edit";
    pub const GEMINI_EXTEND: &'static str = "extend";
    /// Kling's `background_source`: the output keeps the reference video's background.
    pub const BACKGROUND_FROM_VIDEO: &'static str = "input_video";
    /// What a draft is finalized at; `draft_job_id` takes no other.
    pub const FINALIZE_RESOLUTION: &'static str = "1080p";

    pub fn new(model: impl Into<String>, media: Vec<VideoMedia>) -> Self {
        Self {
            model: model.into(),
            media,
            prompt: String::new(),
            resolution: None,
            mode: None,
            duration: None,
            generate_audio: None,
            background_source: None,
            aspect_ratio: None,
            bitrate_mode: None,
            extension_mode: None,
            uses_unlimited: false,
            count: 1,
            draft: false,
            draft_job_id: None,
        }
    }

    /// The finalize of a completed Seedance 2.5 draft at 1080p: `{model, draft_job_id, resolution: "1080p",
    /// use_unlim: false}` and nothing else.
    pub fn finalize(draft_job_id: impl Into<String>) -> Self {
        let mut request = Self::new(Self::SEEDANCE_25, Vec::new());
        request.resolution = Some(Self::FINALIZE_RESOLUTION.to_owned());
        request.draft_job_id = Some(draft_job_id.into());
        request
    }

    /// With `get_cost`, the server quotes the request instead of submitting it. `McpError::Encoding` for a
    /// duration JSON can't hold.
    pub fn arguments(&self, get_cost: bool) -> Result<Object, McpError> {
        let mut params = Object::new();
        params.insert("model".into(), self.model.clone().into());
        params.insert("use_unlim".into(), self.uses_unlimited.into());
        if !self.media.is_empty() {
            let medias = self
                .media
                .iter()
                .map(|media| serde_json::json!({ "role": media.role.raw_value(), "value": media.id }))
                .collect();
            params.insert("medias".into(), Value::Array(medias));
        }
        let strings = [
            ("resolution", &self.resolution),
            ("mode", &self.mode),
            ("background_source", &self.background_source),
            ("aspect_ratio", &self.aspect_ratio),
            ("bitrate_mode", &self.bitrate_mode),
            ("extension_mode", &self.extension_mode),
            ("draft_job_id", &self.draft_job_id),
        ];
        for (key, value) in strings {
            if let Some(value) = value {
                params.insert(key.into(), value.clone().into());
            }
        }
        if let Some(duration) = self.duration {
            params.insert(
                "duration".into(),
                json::number(duration).ok_or(McpError::Encoding)?,
            );
        }
        if self.draft {
            params.insert("draft".into(), true.into());
        }
        if self.count > 1 && !self.uses_unlimited {
            params.insert("count".into(), self.count.into());
        }
        if let Some(generate_audio) = self.generate_audio {
            params.insert("generate_audio".into(), generate_audio.into());
        }
        if !self.prompt.is_empty() {
            params.insert("prompt".into(), self.prompt.clone().into());
        }
        if get_cost {
            params.insert("get_cost".into(), true.into());
        }
        let mut arguments = Object::new();
        arguments.insert("params".into(), Value::Object(params));
        Ok(arguments)
    }
}

/// A submitted generation.
#[derive(Debug, Clone, PartialEq)]
pub struct Generation {
    /// What `status` takes.
    pub id: String,
    /// The server's trace ID, for support requests.
    pub request_id: Option<String>,
    /// Credits the answer says the job costs (`cost.credits_exact` or `cost.credits`); `None` when it names none.
    pub credits: Option<f64>,
    /// The other jobs the same request started, one per further variant (`VideoRequest::count`), in the answer's
    /// order. Each is polled like `id`; `credits` is the whole request's and isn't repeated for them.
    pub variant_ids: Vec<String>,
}

impl Generation {
    pub(crate) fn new(id: String, request_id: Option<String>, credits: Option<f64>) -> Self {
        Self {
            id,
            request_id,
            credits,
            variant_ids: Vec::new(),
        }
    }
}

/// An uploaded and confirmed file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UploadedMedia {
    /// What `VideoMedia` takes.
    pub id: String,
    pub url: Url,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountBalance {
    pub credits: f64,
    /// `subscription_plan_type`, e.g. "`creator_plus`".
    pub plan: Option<String>,
}
