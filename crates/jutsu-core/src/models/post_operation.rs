//! Jobs that work on an existing result instead of generating one.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::models::Cost;
use crate::support::Seconds;
use crate::support::codable::{Empty, Unlabeled};
use crate::support::string_enum;

/// A job that works on an existing result instead of generating one (Upscale, Reframe, Remove Background, Dub,
/// Change Voice). The job's `source` is the result's file and `Job::parent_id` the result it was made from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PostOperation {
    /// What it does.
    pub action: PostAction,
    /// The result's job ID on the account that made it. The operation names the result by it instead of uploading
    /// the file; `None` uploads `JobSpec::source`, as does a server that refuses the ID.
    #[serde(default, rename = "sourceJobID", skip_serializing_if = "Option::is_none")]
    pub source_job_id: Option<String>,
}

/// What an operation does. Stored jobs name the case, so a case is never renamed; a new one leaves old jobs
/// decodable. Coded as Swift codes the enum: `{"upscale":{"_0":{…}}}`, `{"removeBackground":{}}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "specta", specta(type = PostActionRepr))]
#[serde(into = "PostActionRepr", from = "PostActionRepr")]
pub enum PostAction {
    /// A larger, possibly smoother result.
    Upscale(UpscaleOptions),
    /// Keeps the subject on a black background. Live on 2026-10-15 the result was an H.264 MP4 without alpha or
    /// audio, the source's size and length, so nothing in it is transparent.
    RemoveBackground,
    /// A new canvas of another aspect ratio around the whole result, its new edges filled in.
    Reframe(ReframeAspect),
    /// The speech translated into another language and lip-synced.
    Dub(DubLanguage),
    /// The speaker's voice replaced by another one, keeping the timing.
    ChangeVoice(VoiceChoice),
}

/// The stored shape of a `PostAction`.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", rename = "PostAction")]
#[allow(missing_docs, reason = "each case mirrors `PostAction`")]
pub enum PostActionRepr {
    Upscale(Unlabeled<UpscaleOptions>),
    RemoveBackground(Empty),
    Reframe(Unlabeled<ReframeAspect>),
    Dub(Unlabeled<DubLanguage>),
    ChangeVoice(Unlabeled<VoiceChoice>),
}

impl From<PostAction> for PostActionRepr {
    fn from(action: PostAction) -> Self {
        match action {
            PostAction::Upscale(value) => Self::Upscale(Unlabeled { value }),
            PostAction::RemoveBackground => Self::RemoveBackground(Empty {}),
            PostAction::Reframe(value) => Self::Reframe(Unlabeled { value }),
            PostAction::Dub(value) => Self::Dub(Unlabeled { value }),
            PostAction::ChangeVoice(value) => Self::ChangeVoice(Unlabeled { value }),
        }
    }
}

impl From<PostActionRepr> for PostAction {
    fn from(repr: PostActionRepr) -> Self {
        match repr {
            PostActionRepr::Upscale(Unlabeled { value }) => Self::Upscale(value),
            PostActionRepr::RemoveBackground(_) => Self::RemoveBackground,
            PostActionRepr::Reframe(Unlabeled { value }) => Self::Reframe(value),
            PostActionRepr::Dub(Unlabeled { value }) => Self::Dub(value),
            PostActionRepr::ChangeVoice(Unlabeled { value }) => Self::ChangeVoice(value),
        }
    }
}

string_enum! {
    /// The kind of an operation.
    pub enum PostActionKind {
        /// Upscale.
        Upscale = "upscale",
        /// Reframe.
        Reframe = "reframe",
        /// Remove Background.
        RemoveBackground = "removeBackground",
        /// Dub.
        Dub = "dub",
        /// Change Voice.
        ChangeVoice = "changeVoice",
    }
}

impl PostAction {
    /// Its kind.
    pub fn kind(&self) -> PostActionKind {
        match self {
            Self::Upscale(_) => PostActionKind::Upscale,
            Self::RemoveBackground => PostActionKind::RemoveBackground,
            Self::Reframe(_) => PostActionKind::Reframe,
            Self::Dub(_) => PostActionKind::Dub,
            Self::ChangeVoice(_) => PostActionKind::ChangeVoice,
        }
    }

    /// Whether Higgsfield prices the operation only once it has run: a Topaz upscale has no cost preflight, so its
    /// charge is read from the account's credit history when it finishes (`GenerationProvider::charge`).
    pub fn is_priced_after_run(&self) -> bool {
        matches!(self, Self::Upscale(options) if options.engine == UpscaleEngine::Topaz)
    }

    /// Whether the result gets the source's sound back after download, as a generation does (`AudioPlan`): Remove
    /// Background's result has no audio track (live, 2026-10-15). Dub and Change Voice make their own sound, and
    /// what Upscale and Reframe return hasn't been seen, so those keep the result as it came.
    pub fn restores_source_sound(&self) -> bool {
        *self == Self::RemoveBackground
    }
}

/// How an upscale runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct UpscaleOptions {
    /// The size.
    pub target: UpscaleTarget,
    /// The frame rate.
    pub frame_rate: UpscaleFrameRate,
    /// The upscaler.
    pub engine: UpscaleEngine,
    /// The standard upscaler's tuning for the source material; Topaz takes none. A job stored before the preset
    /// was sent ran with the server's default, `common`, and reads as one.
    #[serde(default = "UpscalePreset::common")]
    pub preset: UpscalePreset,
}

impl Default for UpscaleOptions {
    /// 1080p at 24 fps on the standard upscaler, tuned for AI-generated video.
    fn default() -> Self {
        Self {
            target: UpscaleTarget::P1080,
            frame_rate: UpscaleFrameRate::Fps24,
            engine: UpscaleEngine::Standard,
            preset: UpscalePreset::Aigc,
        }
    }
}

string_enum! {
    /// The standard upscaler's `preset`: what kind of video it tunes itself for. The price doesn't depend on it.
    pub enum UpscalePreset {
        /// AI-generated video, which every result here is; the default.
        Aigc = "aigc",
        /// The server's own default.
        Common = "common",
        /// User-generated video.
        Ugc = "ugc",
        /// Short series.
        ShortSeries = "short_series",
        /// Old film.
        OldFilm = "old_film",
    }
}

impl UpscalePreset {
    /// The server's default, for jobs stored without a preset.
    pub const fn common() -> Self {
        Self::Common
    }
}

string_enum! {
    /// The size an upscale makes.
    pub enum UpscaleTarget {
        /// 1080p.
        P1080 = "1080p",
        /// 2K.
        K2 = "2k",
        /// 4K.
        K4 = "4k",
    }
}

/// The frame rate an upscale makes, coded as its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "specta", specta(type = i64))]
#[serde(into = "i64", try_from = "i64")]
pub enum UpscaleFrameRate {
    /// Kept for jobs stored before the catalog was known: nothing is sent, so the upscaler uses its default,
    /// 24 fps. Not offered (`OFFERED`).
    Keep,
    /// 24 fps.
    Fps24,
    /// 30 fps.
    Fps30,
    /// Costs twice as much as 30 fps or less.
    Fps60,
}

impl UpscaleFrameRate {
    /// Every rate, in order.
    pub const ALL: [Self; 4] = [Self::Keep, Self::Fps24, Self::Fps30, Self::Fps60];
    /// The rates offered.
    pub const OFFERED: [Self; 3] = [Self::Fps24, Self::Fps30, Self::Fps60];

    /// Frames per second; 0 for `Keep`.
    pub fn raw_value(self) -> i64 {
        match self {
            Self::Keep => 0,
            Self::Fps24 => 24,
            Self::Fps30 => 30,
            Self::Fps60 => 60,
        }
    }
}

impl From<UpscaleFrameRate> for i64 {
    fn from(rate: UpscaleFrameRate) -> Self {
        rate.raw_value()
    }
}

impl TryFrom<i64> for UpscaleFrameRate {
    type Error = String;

    fn try_from(raw: i64) -> Result<Self, Self::Error> {
        Self::ALL
            .into_iter()
            .find(|rate| rate.raw_value() == raw)
            .ok_or_else(|| format!("No frame rate {raw}"))
    }
}

string_enum! {
    /// The upscaler.
    pub enum UpscaleEngine {
        /// Higgsfield's own upscaler (provider `bytedance`), which needs the source's size and takes a frame rate.
        Standard = "standard",
        /// Topaz Video: 1080p or 4K, at the source's frame rate.
        Topaz = "topaz",
    }
}

impl UpscaleEngine {
    /// The targets the engine offers.
    pub fn targets(self) -> &'static [UpscaleTarget] {
        match self {
            Self::Standard => UpscaleTarget::ALL,
            Self::Topaz => &[UpscaleTarget::P1080, UpscaleTarget::K4],
        }
    }

    /// Whether the engine takes a frame rate and a preset.
    pub fn takes_frame_rate(self) -> bool {
        self == Self::Standard
    }
}

string_enum! {
    /// The canvases Reframe offers. The tool also takes 4:3, 3:4 and 21:9, but not 4:5.
    pub enum ReframeAspect {
        /// 9:16.
        Portrait = "9:16",
        /// 1:1.
        Square = "1:1",
        /// 16:9.
        Landscape = "16:9",
    }
}

impl ReframeAspect {
    /// The longest result Reframe takes, in seconds.
    pub const MAX_SOURCE_LENGTH: Seconds = 60.0;

    /// Width over height.
    pub fn width_over_height(self) -> f64 {
        match self {
            Self::Portrait => 9.0 / 16.0,
            Self::Square => 1.0,
            Self::Landscape => 16.0 / 9.0,
        }
    }
}

string_enum! {
    /// The languages a result can be dubbed into, by the ISO 639-3 code the dubbing tool takes.
    pub enum DubLanguage {
        /// English.
        English = "eng",
        /// Chinese.
        Chinese = "cmn",
        /// French.
        French = "fra",
        /// Hindi.
        Hindi = "hin",
        /// Italian.
        Italian = "ita",
        /// Japanese.
        Japanese = "jpn",
        /// Korean.
        Korean = "kor",
        /// Portuguese.
        Portuguese = "por",
        /// Russian.
        Russian = "rus",
        /// Turkish.
        Turkish = "tur",
        /// Spanish.
        Spanish = "spa",
        /// German.
        German = "deu",
        /// Arabic.
        Arabic = "ara",
        /// Polish.
        Polish = "pol",
        /// Indonesian.
        Indonesian = "ind",
        /// Filipino.
        Filipino = "fil",
        /// Swedish.
        Swedish = "swe",
        /// Finnish.
        Finnish = "fin",
    }
}

/// The voice a result's speaker is changed to, as the provider's voice list names it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct VoiceChoice {
    /// The provider's ID.
    pub id: String,
    /// The provider's kind of voice ("preset", "element"), sent back as it came.
    #[serde(rename = "type")]
    pub kind: String,
    /// What the voice list calls it, for titles.
    pub name: String,
}

impl Cost {
    /// The price of a job no quote or public rate exists for: an operation without a cost preflight (only Reframe
    /// has one), or an account model whose credits per second aren't known before upload. Charged on completion.
    /// Never shown as a number (`Job::price_is_unknown`).
    pub const UNKNOWN_CREDITS: Self = Self::Credits {
        amount: Decimal::ZERO,
        is_estimate: true,
    };
    /// The list price of a REST tariff no public page states (`Pricing::unpriced`), until the server quotes the
    /// uploaded inputs.
    pub const UNKNOWN_USD: Self = Self::Usd {
        amount: Decimal::ZERO,
        is_estimate: true,
    };

    /// `UNKNOWN_CREDITS` or `UNKNOWN_USD`.
    pub fn is_unknown_price(&self) -> bool {
        *self == Self::UNKNOWN_CREDITS || *self == Self::UNKNOWN_USD
    }
}
