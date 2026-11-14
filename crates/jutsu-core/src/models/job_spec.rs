//! Everything needed to run one generation.

use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::models::{
    AspectRatio, AudioOption, CharacterOrientation, CinemaControls, LocalAudio, LocalImage, Mode, ModeLimits,
    ModelConstraintTable, PostOperation, Resolution, SeedanceBitrate, SourceClip, VideoControl, VideoModel,
};
use crate::provider::RemoteInputs;
use crate::support::Seconds;

use crate::support::text::{
    char_count, is_control, is_format, is_newline, is_whitespace_or_newline, prefix_chars, trim_whitespace,
};
use crate::support::url_ext::{deleting_path_extension, last_path_component};

/// Everything needed to run one generation.
///
/// Jobs stored before `audio` existed decode as `Original`, before `model` as Genjutsu, before
/// `keepsVideoBackground` with the image's background, before `referencesAreFrames` with references, before
/// `usesUnlimited` paid in credits, before `operation` as generations, before `variants` as one, before
/// `cinemaControls` with none set, before `title` named after their source, and before `speech` without one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct JobSpec {
    /// The mode.
    pub mode: Mode,
    /// Only Genjutsu serves every mode (`VideoModel::serves`).
    pub model: VideoModel,
    /// `None` for a clip made from the prompt and images alone (`VideoModel::makes_clips_without_source`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceClip>,
    /// Ordered; the API keeps the order.
    pub references: Vec<LocalImage>,
    /// Restyle only.
    #[serde(rename = "presetID", skip_serializing_if = "Option::is_none")]
    pub preset_id: Option<Uuid>,
    /// Empty when unused: the API rejects `null`.
    pub prompt: String,
    /// One the model renders at in the mode (`VideoModel::resolutions`): `new` and decoding move another to the
    /// nearest one (`fit_resolution`).
    pub resolution: Resolution,
    /// A draft.
    pub is_draft: bool,
    /// Applied locally after download; the API never sees it.
    pub audio: AudioOption,
    /// The source and references as the account connection already holds them, in the same order: the job
    /// sends these media IDs instead of preparing and uploading the files, and the whole source is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_inputs: Option<RemoteInputs>,
    /// Kling only: the result keeps the source video's background instead of the reference image's.
    pub keeps_video_background: bool,
    /// Seconds a clip made without a source runs, or the seconds Extend adds; `None` otherwise, where the source's
    /// selection sets the length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<Seconds>,
    /// The frame of a clip made without a source; `None` with a source. For a clip from frames (`uses_frames`),
    /// the start frame's shape, set when the job is queued.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<AspectRatio>,
    /// Seedance only; `None` leaves the server's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<SeedanceBitrate>,
    /// Kling only; `None` leaves the server's default, the video's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_orientation: Option<CharacterOrientation>,
    /// Cinema Studio only; the director picks each control it leaves out.
    pub cinema_controls: CinemaControls,
    /// Seedance without a source: `references` are the start frame and, when there are two, the end frame
    /// (`uses_frames`), rather than images to draw from.
    pub references_are_frames: bool,
    /// Paid from the plan's unlimited allowance (the account's `use_unlim`) rather than credits. The engine sets
    /// it when the job is queued and clears it when the provider says the job cost credits after all; the daily
    /// cap leaves such a job out.
    pub uses_unlimited: bool,
    /// An operation on an existing result (`source`) rather than a generation; `None` for a generation and in
    /// jobs stored before operations existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<PostOperation>,
    /// How many variants of this same request one submit makes, 1…`MAXIMUM_VARIANTS`: the account's `count`. The
    /// submit's answer becomes that many jobs, and each of them carries 1 from then on, so a retry of one makes
    /// one. 1 in jobs stored before variants existed.
    pub variants: i64,
    /// Seedance on the account: the completed native draft (`RemoteJob::is_native_draft`) this final finalizes at
    /// 1080p by its job ID (`draft_job_id`), instead of rendering the inputs again. `None` for any other job.
    #[serde(rename = "draftJobID", skip_serializing_if = "Option::is_none")]
    pub draft_job_id: Option<String>,
    /// What the job is called in lists instead of its source's file name (`name`): an agent's `title`, trimmed and
    /// cut to `MAX_TITLE_LENGTH` (`title_from`). `None` for the person's own jobs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The recording the characters say, on a model that takes one (`VideoControl::Speech`); `None` for any
    /// other job.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech: Option<LocalAudio>,
}

/// The stored form, with the defaults of jobs stored by older versions.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredJobSpec {
    mode: Mode,
    #[serde(default = "VideoModel::genjutsu")]
    model: VideoModel,
    #[serde(default)]
    source: Option<SourceClip>,
    references: Vec<LocalImage>,
    #[serde(default, rename = "presetID")]
    preset_id: Option<Uuid>,
    prompt: String,
    resolution: Resolution,
    is_draft: bool,
    #[serde(default)]
    audio: AudioOption,
    #[serde(default)]
    remote_inputs: Option<RemoteInputs>,
    #[serde(default)]
    keeps_video_background: bool,
    #[serde(default)]
    length: Option<Seconds>,
    #[serde(default)]
    aspect_ratio: Option<AspectRatio>,
    #[serde(default)]
    bitrate: Option<SeedanceBitrate>,
    #[serde(default)]
    character_orientation: Option<CharacterOrientation>,
    #[serde(default)]
    cinema_controls: CinemaControls,
    #[serde(default)]
    references_are_frames: bool,
    #[serde(default)]
    uses_unlimited: bool,
    #[serde(default)]
    operation: Option<PostOperation>,
    #[serde(default = "one")]
    variants: i64,
    #[serde(default, rename = "draftJobID")]
    draft_job_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    speech: Option<LocalAudio>,
}

const fn one() -> i64 {
    1
}

impl<'de> Deserialize<'de> for JobSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = StoredJobSpec::deserialize(deserializer)?;
        let mut spec = Self {
            mode: stored.mode,
            model: stored.model,
            source: stored.source,
            references: stored.references,
            preset_id: stored.preset_id,
            prompt: stored.prompt,
            resolution: stored.resolution,
            is_draft: stored.is_draft,
            audio: stored.audio,
            remote_inputs: stored.remote_inputs,
            keeps_video_background: stored.keeps_video_background,
            length: stored.length,
            aspect_ratio: stored.aspect_ratio,
            bitrate: stored.bitrate,
            character_orientation: stored.character_orientation,
            cinema_controls: stored.cinema_controls,
            references_are_frames: stored.references_are_frames,
            uses_unlimited: stored.uses_unlimited,
            operation: stored.operation,
            variants: stored.variants,
            draft_job_id: stored.draft_job_id,
            title: stored.title,
            speech: stored.speech,
        };
        // A generation stored at a resolution its mode doesn't render at, such as a Genjutsu Lite Object Swap at
        // 480p from before modes had their own, is the job it is sent as, so the Queue, the Library, the estimate
        // and the cap read the same. An operation keeps the result's resolution.
        if spec.operation.is_none() {
            spec.fit_resolution();
        }
        Ok(spec)
    }
}

impl JobSpec {
    /// The most variants one submit makes (`generate_video`'s `count`).
    pub const MAXIMUM_VARIANTS: i64 = 4;
    /// The longest name made from a prompt, in characters.
    pub const MAX_PROMPT_NAME_LENGTH: usize = 40;
    /// The longest `title`, in characters: what a Queue row or a Library card shows of it.
    pub const MAX_TITLE_LENGTH: usize = 80;

    /// A job of `mode` on `model` with every other input at its default; `resolution` is moved to the nearest one
    /// the model renders at in the mode.
    pub fn new(mode: Mode, model: VideoModel, source: Option<SourceClip>, resolution: Resolution) -> Self {
        let resolution = model.fitted(resolution, mode);
        Self {
            mode,
            model,
            source,
            references: Vec::new(),
            preset_id: None,
            prompt: String::new(),
            resolution,
            is_draft: false,
            audio: AudioOption::Original,
            remote_inputs: None,
            keeps_video_background: false,
            length: None,
            aspect_ratio: None,
            bitrate: None,
            character_orientation: None,
            cinema_controls: CinemaControls::new(),
            references_are_frames: false,
            uses_unlimited: false,
            operation: None,
            variants: 1,
            draft_job_id: None,
            title: None,
            speech: None,
        }
    }

    /// Moves `resolution` to the nearest one `model` renders at in `mode`, as Swift's initializer does.
    pub fn fit_resolution(&mut self) {
        self.resolution = self.model.fitted(self.resolution, self.mode);
    }

    /// The jobs `specs` become once submitted: one per variant.
    pub fn job_count(specs: &[Self]) -> i64 {
        specs.iter().map(|spec| spec.variants.max(1)).sum()
    }

    /// The model makes the clip from a start frame and an optional end frame (Seedance's `image-to-video`): the
    /// first and second reference, on a job without a source, on a model with frames.
    pub fn uses_frames(&self) -> bool {
        self.references_are_frames
            && self.model.declares(VideoControl::Frames)
            && self.source.is_none()
            && !self.mode.is_video_operation()
    }

    /// The inputs `model` takes in `mode`, by `constraints` where they say; for `uses_frames` a start frame and at
    /// most an end frame, or the start alone (`VideoModelDescriptor::frame_images`). A job with a source,
    /// references or an end frame keeps to the model's window for them (`VideoModel::duration`).
    pub fn limits(&self, constraints: &ModelConstraintTable) -> ModeLimits {
        let mut limits = self.model.limits(self.mode, self.source.is_some(), constraints);
        if self.uses_frames() {
            limits.reference_images = self.model.descriptor().frame_images();
        }
        let with_references =
            self.source.is_some() || self.references.len() > usize::from(self.uses_frames());
        limits.duration = self.model.duration(limits.duration, self.mode, with_references);
        limits
    }

    /// The REST endpoint the job runs on with an API key (`VideoModel::endpoint_id`).
    pub fn endpoint_id(&self) -> String {
        self.model.endpoint_id(
            self.mode,
            self.resolution,
            self.source.is_some() || !self.references.is_empty(),
            self.uses_frames(),
        )
    }

    /// Seconds the result runs: the part Extend adds, else the source selection, else the length asked for; on a
    /// model that voices a speech, the speech and the model's tail after it (`VideoModelDescriptor::speech_tail`).
    pub fn duration(&self) -> Seconds {
        if self.mode == Mode::Extend {
            return self.length.unwrap_or(0.0);
        }
        if let Some(speech) = &self.speech
            && self.model.declares(VideoControl::Speech)
        {
            return speech.duration + self.model.descriptor().speech_tail;
        }
        self.source
            .as_ref()
            .map(SourceClip::selected_duration)
            .or(self.length)
            .unwrap_or(0.0)
    }

    /// What the job is called in lists: its `title`, else the source file's name without its extension; for a
    /// clip made without a source, the start of its prompt, else the model.
    pub fn name(&self) -> String {
        if let Some(title) = &self.title {
            return title.clone();
        }
        if let Some(source) = &self.source {
            return deleting_path_extension(&last_path_component(&source.file_url)).to_owned();
        }
        let first_line = self
            .prompt
            .split(is_newline)
            .find(|line| !line.is_empty())
            .unwrap_or("");
        let prompt = trim_whitespace(first_line);
        if prompt.is_empty() {
            return self.model.title();
        }
        if char_count(prompt) > Self::MAX_PROMPT_NAME_LENGTH {
            format!(
                "{}…",
                trim_whitespace(prefix_chars(prompt, Self::MAX_PROMPT_NAME_LENGTH))
            )
        } else {
            prompt.to_owned()
        }
    }

    /// `text` as a `title`: one line with single spaces, trimmed, cut to `MAX_TITLE_LENGTH`; `None` when nothing is
    /// left. An agent sends it, and it names rows, cards and files: control characters (tabs, newlines, NUL) read
    /// as spaces, and format characters, which would reorder or hide what is shown (bidi overrides such as U+202E,
    /// zero-width spaces), are dropped, except the joiner inside emoji.
    pub fn title_from(text: Option<&str>) -> Option<String> {
        let text = text?;
        let cleaned: String = text
            .chars()
            .filter(|c| !(is_format(*c) && *c != '\u{200D}'))
            .map(|c| {
                if is_control(c) || matches!(c, '\u{2028}' | '\u{2029}') {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        let line = cleaned
            .split(is_whitespace_or_newline)
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if line.is_empty() {
            return None;
        }
        Some(trim_whitespace(prefix_chars(&line, Self::MAX_TITLE_LENGTH)).to_owned())
    }
}
