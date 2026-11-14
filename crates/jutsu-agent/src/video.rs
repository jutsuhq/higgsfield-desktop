//! The video commands `quote` and `create_video` (`jobs` and `job_action` are in `job_commands`, what they all answer
//! in `job`).

use crate::{
    AgentBudgetUnit, AgentError, Uuid, commands, elements::AgentElementNames, job_commands::JobsCommand,
    names,
};
use serde::{Deserialize, Serialize};

/// An amount in whole units of what the connection pays with: cents with an API key, credits on an account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentAmount {
    pub amount: i64,
    pub unit: AgentBudgetUnit,
}

impl AgentAmount {
    pub fn new(amount: i64, unit: AgentBudgetUnit) -> Self {
        Self { amount, unit }
    }

    /// The largest amount an agent can name: $10 000 or 10 000 000 credits, the most a daily budget can be.
    pub fn maximum(unit: AgentBudgetUnit) -> i64 {
        if unit == AgentBudgetUnit::UsdCents {
            1_000_000
        } else {
            10_000_000
        }
    }

    pub(crate) fn check(&self) -> Result<(), AgentError> {
        let maximum = Self::maximum(self.unit);
        if (0..=maximum).contains(&self.amount) {
            Ok(())
        } else {
            Err(AgentError::bad_request(format!(
                "amount is 0 to {maximum} {}",
                self.unit.as_str()
            )))
        }
    }
}

/// The source clip of a video: a file in the inbox for `create_video`; for `quote` only its length counts, so it may
/// come without a file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentSource {
    /// For `create_video`: the clip's path in the app's inbox, from an upload.
    pub path: Option<String>,
    /// Seconds from the start of the file; 0 when left out.
    pub start: Option<f64>,
    /// Where the job's range ends, in seconds; left out, the whole clip from `start` up to the model's longest.
    pub end: Option<f64>,
}

impl AgentSource {
    /// The longest file any model is asked about: 24 hours.
    pub const MAX_SECONDS: f64 = 86_400.0;

    /// Why these seconds can't be a range; checked before anything formats or computes with them.
    pub fn problem(start: Option<f64>, end: Option<f64>) -> Option<String> {
        for (name, value) in [("start", start), ("end", end)] {
            if let Some(value) = value
                && !(value.is_finite() && (0.0..=Self::MAX_SECONDS).contains(&value))
            {
                return Some(format!("source.{name} is a number of seconds from 0 to 86400"));
            }
        }
        if let (Some(start), Some(end)) = (start, end)
            && end <= start
        {
            return Some("source.end comes after source.start".to_owned());
        }
        None
    }

    pub(crate) fn check(&self) -> Result<(), AgentError> {
        if let Some(problem) = Self::problem(self.start, self.end) {
            return Err(AgentError::bad_request(problem));
        }
        if let Some(path) = &self.path
            && !AgentVideoRequest::is_valid_path(path)
        {
            return Err(AgentError::bad_request("source.path is a file path"));
        }
        Ok(())
    }
}

/// The names agents use for the modes.
pub const MODES: [&str; 5] = ["motion_transfer", "restyle", "object_swap", "edit", "extend"];
pub const AUDIO_OPTIONS: [&str; 4] = ["original", "mute", "fade_out", "generated"];
/// A sanity bound of the protocol only, above every model's own limit.
pub const PROTOCOL_MAX_IMAGES: i64 = 16;
pub const MAX_VARIANTS: i64 = 16;
/// Seconds a clip made without a source may be asked to run, or Extend to add.
pub const MAX_LENGTH: f64 = 600.0;
/// A bound of the protocol only: the app keeps the first 80 characters of a title.
pub const MAX_TITLE_LENGTH: i64 = 1_000;
/// In Unicode scalars, as Create counts a prompt; `create_image` takes the same.
pub const MAX_PROMPT_LENGTH: i64 = 10_000;

/// The video an agent asks for, as `quote` and `create_video` take it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentVideoRequest {
    pub mode: String,
    /// A model ID from `capabilities`; left out, the mode's default model.
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub source: Option<AgentSource>,
    /// For `create_video`: the reference images' paths in the app's inbox, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// For `quote`: how many reference images there will be.
    pub image_count: Option<i64>,
    /// Restyle's style, by its ID.
    pub style: Option<Uuid>,
    pub resolution: Option<String>,
    /// A cheaper, lower resolution preview on a model that has drafts.
    #[serde(default, skip_serializing_if = "commands::is_false")]
    pub draft: bool,
    /// Seconds a clip without a source runs, or Extend adds.
    pub length: Option<f64>,
    /// The frame of a clip without a source: "16:9", "9:16", "1:1"…
    pub aspect_ratio: Option<String>,
    pub audio: Option<String>,
    /// Videos one submit makes, on a connection that offers more than one.
    pub variants: Option<i64>,
    /// The account's reference elements by name: their images go first, before `images`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<String>,
    /// What the Queue, the Library and export names call the job instead of the source's file name.
    pub title: Option<String>,
    /// For `create_video`: the speech recording's path in the app's inbox, on a model that voices one.
    pub speech: Option<String>,
    /// For `quote`: the seconds the speech recording runs.
    pub speech_length: Option<f64>,
}

impl AgentVideoRequest {
    /// A request in `mode` with nothing else set.
    pub fn new(mode: &str) -> Self {
        Self {
            mode: mode.to_owned(),
            ..Self::default()
        }
    }

    /// How many reference images the job has: the files, else the count a quote names.
    pub fn reference_count(&self) -> i64 {
        if self.images.is_empty() {
            self.image_count.unwrap_or(0)
        } else {
            count(self.images.len())
        }
    }

    /// Why the fields can't be a request, before the app's own rules; `None` when they can. Shared by the helper,
    /// which checks before it reads any file, and the app.
    pub fn problem(&self) -> Option<String> {
        if !MODES.contains(&self.mode.as_str()) {
            return Some(format!("mode is one of {}", MODES.join(", ")));
        }
        if self
            .model
            .as_deref()
            .is_some_and(|model| !names::is_valid_model_id(model))
        {
            return Some("model is a model ID from capabilities".to_owned());
        }
        if self
            .prompt
            .as_deref()
            .is_some_and(|prompt| count(names::scalar_count(prompt)) > MAX_PROMPT_LENGTH)
        {
            return Some(format!(
                "prompt is at most {} characters",
                names::spaced(MAX_PROMPT_LENGTH)
            ));
        }
        if self
            .title
            .as_deref()
            .is_some_and(|title| count(names::scalar_count(title)) > MAX_TITLE_LENGTH)
        {
            return Some(format!(
                "title is at most {} characters",
                names::spaced(MAX_TITLE_LENGTH)
            ));
        }
        if count(self.images.len()) > PROTOCOL_MAX_IMAGES {
            return Some(format!(
                "images takes at most {PROTOCOL_MAX_IMAGES} files, more than any model takes"
            ));
        }
        if let Some(path) = self.images.iter().find(|path| !Self::is_valid_path(path)) {
            return Some(format!(
                "images has a path that isn't one: {}",
                names::prefix(path, 80)
            ));
        }
        if self
            .speech
            .as_deref()
            .is_some_and(|speech| !Self::is_valid_path(speech))
        {
            return Some("speech is the path of an audio file".to_owned());
        }
        if self.speech_length.is_some_and(|seconds| !is_length(seconds)) {
            return Some("speech_length is a number of seconds above 0 and up to 600".to_owned());
        }
        if self
            .image_count
            .is_some_and(|images| !(0..=PROTOCOL_MAX_IMAGES).contains(&images))
        {
            return Some(format!("image_count is 0 to {PROTOCOL_MAX_IMAGES}"));
        }
        if let Some(problem) = AgentElementNames::problem(&self.elements) {
            return Some(problem);
        }
        if self.length.is_some_and(|seconds| !is_length(seconds)) {
            return Some("length is a number of seconds above 0 and up to 600".to_owned());
        }
        if self
            .variants
            .is_some_and(|variants| !(1..=MAX_VARIANTS).contains(&variants))
        {
            return Some(format!("variants is 1 to {MAX_VARIANTS}"));
        }
        if self
            .audio
            .as_deref()
            .is_some_and(|audio| !AUDIO_OPTIONS.contains(&audio))
        {
            return Some(format!("audio is one of {}", AUDIO_OPTIONS.join(", ")));
        }
        if self
            .resolution
            .as_deref()
            .is_some_and(|resolution| resolution.len() > 16)
        {
            return Some("resolution is one from capabilities, such as 720p".to_owned());
        }
        if self.aspect_ratio.as_deref().is_some_and(|ratio| ratio.len() > 16) {
            return Some("aspect_ratio is a ratio such as 16:9".to_owned());
        }
        None
    }

    pub(crate) fn check(&self) -> Result<(), AgentError> {
        if let Some(source) = &self.source {
            source.check()?;
        }
        self.problem()
            .map_or(Ok(()), |problem| Err(AgentError::bad_request(problem)))
    }

    pub fn is_valid_path(path: &str) -> bool {
        (1..=4096).contains(&path.len()) && !path.contains('\0')
    }
}

fn is_length(seconds: f64) -> bool {
    seconds.is_finite() && seconds > 0.0 && seconds <= MAX_LENGTH
}

/// A length as the protocol's counts take it.
pub(crate) fn count(length: usize) -> i64 {
    i64::try_from(length).unwrap_or(i64::MAX)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuoteCommand {
    pub video: AgentVideoRequest,
}

impl QuoteCommand {
    pub(crate) fn check(&self) -> Result<(), AgentError> {
        self.video.check()
    }
}

/// Queues one video. The same `request_id` with the same parameters returns the job it queued before, however often
/// it is sent; with other parameters it is refused with `request_conflict`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVideoCommand {
    pub request_id: String,
    /// `AgentFingerprint` of the request as the agent sent it.
    pub fingerprint: String,
    /// The same digest without `max_cost`; `None` from a helper older than it.
    pub parameters: Option<String>,
    pub video: AgentVideoRequest,
    /// The most the agent agrees to pay for the job.
    pub max_cost: AgentAmount,
    /// How long to wait, up to `JobsCommand::MAX_WAIT_SECONDS`, until the job's price is known or it is held.
    #[serde(default, skip_serializing_if = "commands::is_zero")]
    pub wait_seconds: i64,
}

impl CreateVideoCommand {
    pub(crate) fn check(&self) -> Result<(), AgentError> {
        self.video.check()?;
        self.max_cost.check()?;
        commands::check_request_id(&self.request_id)?;
        commands::check_fingerprints(&self.fingerprint, self.parameters.as_deref())?;
        JobsCommand::wait_problem(self.wait_seconds)
            .map_or(Ok(()), |problem| Err(AgentError::bad_request(problem)))
    }
}
