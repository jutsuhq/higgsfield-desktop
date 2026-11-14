//! Work on finished results: video operations (`upscale_video`, `reframe`, `remove_background`, `dubbing`,
//! `voice_change`) and their voices, the Virality Predictor, and `jobs_wait`.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};
use url::Url;

use super::library::preset_job_id;
use super::{
    Generation, HiggsfieldAccount, ToolRole, credits, no_job_started, object, status_of, unexpected,
};
use crate::core::{GenerationStatus, HiggsfieldError};
use crate::mcp::client::McpTool;
use crate::mcp::json::{self, Object};

/// An upscale's provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoEditProvider {
    Bytedance,
    Topaz,
}

impl VideoEditProvider {
    #[must_use]
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Bytedance => "bytedance",
            Self::Topaz => "topaz",
        }
    }
}

/// What an operation does.
#[derive(Debug, Clone, PartialEq)]
pub enum VideoEditOperation {
    /// `bytedance`: `resolution` "1080p", "2k" or "4k", the source's `width` and `height` (required), `fps` 24, 30
    /// or 60 (`None` for the server's default, 24), `preset` "common", "aigc", "`short_series`", "ugc" or "`old_film`".
    /// `topaz`: `resolution` "1080p" or "2160p", no size, frame rate or preset.
    Upscale {
        resolution: String,
        fps: Option<i64>,
        provider: VideoEditProvider,
        width: Option<i64>,
        height: Option<i64>,
        preset: Option<String>,
    },
    RemoveBackground,
    /// `aspect_ratio` "9:16" and the like. The source's length in seconds (at most 60) and a resolution price the
    /// call, and are sent with it for a source longer than `REFRAME_SHORT_SOURCE`.
    Reframe {
        aspect_ratio: String,
        duration_seconds: f64,
        resolution: String,
    },
    /// `language` is an ISO 639-3 code ("fra").
    Dub {
        language: String,
    },
    /// The pair `list_voices` names a voice by.
    ChangeVoice {
        voice_id: String,
        voice_type: String,
    },
}

impl VideoEditOperation {
    #[must_use]
    pub fn role(&self) -> ToolRole {
        match self {
            Self::Upscale { .. } => ToolRole::UpscaleVideo,
            Self::RemoveBackground => ToolRole::RemoveBackground,
            Self::Reframe { .. } => ToolRole::Reframe,
            Self::Dub { .. } => ToolRole::Dubbing,
            Self::ChangeVoice { .. } => ToolRole::VoiceChange,
        }
    }

    /// The catalog model `generate_video` quotes the operation as, with `get_cost` and the source's whole seconds;
    /// `None` for `reframe`, which its own tool quotes, and for Topaz, which has no cost estimation.
    #[must_use]
    pub fn quote_model(&self) -> Option<&'static str> {
        match self {
            Self::Dub { .. } => Some("dubbing"),
            Self::ChangeVoice { .. } => Some("voice_change"),
            Self::RemoveBackground => Some("video_background_remover"),
            Self::Upscale {
                provider: VideoEditProvider::Bytedance,
                ..
            } => Some("bytedance_video_upscale"),
            Self::Upscale { .. } | Self::Reframe { .. } => None,
        }
    }
}

/// What the tool works on. Every tool takes either under the same key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VideoEditInput {
    /// A finished generation of the account, by its job ID: nothing is uploaded.
    Job(String),
    /// A confirmed upload.
    Media(String),
}

impl VideoEditInput {
    fn id(&self) -> &str {
        match self {
            Self::Job(id) | Self::Media(id) => id,
        }
    }
}

/// One call of a tool that works on an existing video.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoEdit {
    pub operation: VideoEditOperation,
    pub input: VideoEditInput,
}

impl VideoEdit {
    /// The length up to which `reframe` takes the source alone.
    pub const REFRAME_SHORT_SOURCE: f64 = 15.0;

    /// The arguments of the call that runs the operation: `params` with the input under the tool's key, then the
    /// operation's own.
    pub(crate) fn arguments(&self) -> Result<Object, HiggsfieldError> {
        let id = self.input.id();
        let params = match &self.operation {
            VideoEditOperation::Upscale {
                resolution,
                fps,
                provider,
                width,
                height,
                preset,
            } => {
                let mut params = object(
                    json!({ "video_id": id, "provider": provider.raw_value(), "resolution": resolution }),
                );
                if let Some(fps) = fps {
                    params.insert("fps".into(), (*fps).into());
                }
                if let Some(preset) = preset {
                    params.insert("preset".into(), preset.clone().into());
                }
                if let Some(width) = width {
                    params.insert("width".into(), (*width).into());
                }
                if let Some(height) = height {
                    params.insert("height".into(), (*height).into());
                }
                params
            }
            VideoEditOperation::RemoveBackground => object(json!({ "media_id": id, "media_type": "video" })),
            VideoEditOperation::Reframe {
                aspect_ratio,
                duration_seconds,
                resolution,
            } => {
                let mut params = object(
                    json!({ "medias": [{ "role": "video", "value": id }], "aspect_ratio": aspect_ratio }),
                );
                if *duration_seconds > Self::REFRAME_SHORT_SOURCE {
                    params.insert("duration_seconds".into(), number(*duration_seconds)?);
                    params.insert("resolution".into(), resolution.clone().into());
                }
                params
            }
            VideoEditOperation::Dub { language } => {
                object(json!({ "video_id": id, "target_language": language }))
            }
            VideoEditOperation::ChangeVoice { voice_id, voice_type } => {
                object(json!({ "video_id": id, "voice_id": voice_id, "voice_type": voice_type }))
            }
        };
        Ok(object(json!({ "params": params })))
    }

    /// The arguments of `reframe`'s price question; `None` for the other operations. `reframe` is priced by the
    /// source's length and the resolution alone, so a quote names no video.
    pub(crate) fn quote_arguments(operation: &VideoEditOperation) -> Option<Result<Object, HiggsfieldError>> {
        let VideoEditOperation::Reframe {
            duration_seconds,
            resolution,
            ..
        } = operation
        else {
            return None;
        };
        Some(number(*duration_seconds).map(|duration| {
            object(json!({ "params": { "get_cost": true, "duration_seconds": duration, "resolution": resolution } }))
        }))
    }

    /// The arguments of `generate_video`'s price question for an operation (`quote_model`), `None` without such a
    /// model. `duration` is whole seconds, at least 1. An upscale also sends its `resolution` and `fps`: not in the
    /// schema, but the live server prices by them.
    pub(crate) fn generate_quote_arguments(operation: &VideoEditOperation, duration: f64) -> Option<Object> {
        let model = operation.quote_model()?;
        if duration <= 0.0 {
            return None;
        }
        let seconds = json::count(duration.round().max(1.0))?;
        let mut params = object(json!({ "model": model, "get_cost": true, "duration": seconds }));
        if let VideoEditOperation::Upscale { resolution, fps, .. } = operation {
            params.insert("resolution".into(), resolution.clone().into());
            if let Some(fps) = fps {
                params.insert("fps".into(), (*fps).into());
            }
        }
        Some(object(json!({ "params": params })))
    }
}

fn number(value: f64) -> Result<Value, HiggsfieldError> {
    json::number(value)
        .ok_or_else(|| HiggsfieldError::validation("The request has a value that can't be sent"))
}

/// The keys a tool's `params` object declares, over every `oneOf` variant.
pub(crate) fn params_argument_names(tool: &McpTool) -> HashSet<String> {
    let Some(params) =
        json::get(&tool.input_schema, "properties").and_then(|properties| json::get(properties, "params"))
    else {
        return HashSet::new();
    };
    std::iter::once(params)
        .chain(json::array(params, "oneOf").into_iter().flatten())
        .filter_map(|variant| json::get(variant, "properties").and_then(Value::as_object))
        .flat_map(|properties| properties.keys().cloned())
        .collect()
}

/// One voice `voice_change` can take.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Voice {
    pub id: String,
    /// "preset" or "element", sent back with the ID as it came.
    pub kind: String,
    pub name: String,
    /// An https sample; `None` when the answer names none.
    pub preview_url: Option<Url>,
}

/// One page of voices, in the server's order.
#[derive(Debug, Clone, PartialEq)]
pub struct VoicePage {
    pub items: Vec<Voice>,
    /// Opaque: sent back unchanged for the next page. `None` on the last page.
    pub next_cursor: Option<Value>,
}

impl VoicePage {
    const LIST_KEYS: [&'static str; 4] = ["voices", "items", "data", "results"];

    /// `None` when no list is found.
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        let items = Self::LIST_KEYS
            .iter()
            .find_map(|key| json::array(value, key))
            .or_else(|| value.as_array())?;
        let cursor = json::get(value, "next_cursor").or_else(|| json::get(value, "nextCursor"));
        // A last page may still carry a cursor; `has_more` says it is the last.
        let next_cursor = cursor
            .filter(|cursor| !cursor.is_null() && !json::is_bool(value, "has_more", false))
            .cloned();
        Some(Self {
            items: items.iter().filter_map(Voice::from_value).collect(),
            next_cursor,
        })
    }
}

impl Voice {
    const ID_KEYS: [&'static str; 3] = ["voice_id", "voiceId", "id"];
    const TYPE_KEYS: [&'static str; 3] = ["voice_type", "voiceType", "type"];
    const PREVIEW_KEYS: [&'static str; 4] = ["preview_url", "previewUrl", "sample_url", "audio_url"];
    /// What a voice is when the answer doesn't say: the built-in kind.
    const DEFAULT_TYPE: &'static str = "preset";

    fn from_value(value: &Value) -> Option<Self> {
        let id = Self::ID_KEYS
            .iter()
            .find_map(|key| json::string(value, key))
            .filter(|id| !id.is_empty())?;
        let name = super::AccountImageModel::NAME_KEYS
            .iter()
            .find_map(|key| json::string(value, key))
            .filter(|name| !name.is_empty())
            .unwrap_or(id);
        Some(Self {
            id: id.to_owned(),
            kind: Self::TYPE_KEYS
                .iter()
                .find_map(|key| json::string(value, key))
                .unwrap_or(Self::DEFAULT_TYPE)
                .to_owned(),
            name: name.to_owned(),
            preview_url: Self::PREVIEW_KEYS
                .iter()
                .find_map(|key| json::media_url(value, key)),
        })
    }
}

/// What the Virality Predictor said about a video.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ViralityPrediction {
    /// `scores.overall_score`, 0–100.
    pub overall: Option<f64>,
    /// `scores.viral_potential`, 0–100.
    pub viral_potential: Option<f64>,
    /// `scores.hook_score`, 0–100, over the first 3 seconds.
    pub hook: Option<f64>,
    /// `scores.brain_engagement`, 0–100.
    pub engagement: Option<f64>,
    /// `scores.sustain`, 0–100.
    pub sustain: Option<f64>,
    /// `scores.peak_second`: when the video reads strongest.
    pub peak_second: Option<f64>,
    /// `global_scores_by_frame`, one reading per frame of `timeline`, on the server's own scale.
    pub per_frame: Vec<f64>,
    /// `score_details.disclaimer`.
    pub disclaimer: Option<String>,
    /// The job's HTML dashboard (`results.rawUrl`, https only).
    pub dashboard_url: Option<Url>,
}

impl ViralityPrediction {
    /// A completed `job_status` `generation`; `None` without `params.analysis.scores`.
    pub(crate) fn from_generation(generation: &Value) -> Option<Self> {
        let analysis = json::get(generation, "params").and_then(|params| json::get(params, "analysis"));
        let scores = analysis
            .and_then(|analysis| json::get(analysis, "scores"))
            .filter(|scores| scores.is_object())?;
        let percent = |key: &str| {
            json::num(scores, key)
                .filter(|value| value.is_finite())
                .map(|value| value.clamp(0.0, 100.0))
        };
        Some(Self {
            overall: percent("overall_score"),
            viral_potential: percent("viral_potential"),
            hook: percent("hook_score"),
            engagement: percent("brain_engagement"),
            sustain: percent("sustain"),
            peak_second: json::num(scores, "peak_second").filter(|value| value.is_finite() && *value >= 0.0),
            per_frame: analysis
                .and_then(|analysis| json::array(analysis, "global_scores_by_frame"))
                .into_iter()
                .flatten()
                .filter_map(Value::as_f64)
                .filter(|value| value.is_finite())
                .collect(),
            disclaimer: analysis
                .and_then(|analysis| json::get(analysis, "score_details"))
                .and_then(|details| json::string(details, "disclaimer"))
                .map(str::to_owned),
            dashboard_url: json::get(generation, "results")
                .and_then(|results| json::media_url(results, "rawUrl")),
        })
    }
}

/// The job one `virality_predictor` call started.
#[derive(Debug, Clone, PartialEq)]
pub struct ViralityJob {
    pub id: String,
    /// How long to wait between polls, in seconds, when the answer says (`poll_interval`).
    pub poll_interval: Option<f64>,
}

/// Where a Virality Predictor job stands.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum ViralityStatus {
    Running,
    /// `None` when the completed job has no `params.analysis.scores`.
    Completed(Option<ViralityPrediction>),
    Failed {
        message: Option<String>,
    },
}

/// A `jobs_wait` answer: the statuses of up to `HiggsfieldAccount::JOBS_WAIT_LIMIT` jobs from one long-poll.
///
/// Live answers of 2026-10-15 have the `outputSchema`'s shape: `{jobs: [{index, job_id, status, type, model,
/// result_url}], summary, all_terminal, timed_out, poll_after_seconds}`. Other shapes are still read tolerantly: the
/// jobs as an array under `jobs`, `results`, `items`, `data` or `generations`, each with its ID in `job_id`, `id` or
/// `jobId`, or as an object of those by job ID; each job's status as `job_status` gives it, in the job itself or its
/// `generation`, with the file in `result_url` or `results.rawUrl`. A job is left out when its ID wasn't asked for,
/// it has no `status` string, its lookup failed (`lookup_failed`), or it is completed without an https file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JobsWait {
    /// By job ID.
    pub statuses: HashMap<String, GenerationStatus>,
    /// When the server asks to be asked again, in seconds; `None` when it doesn't say.
    pub poll_after_seconds: Option<f64>,
}

impl JobsWait {
    const LIST_KEYS: [&'static str; 5] = ["jobs", "results", "items", "data", "generations"];
    const ID_KEYS: [&'static str; 3] = ["job_id", "id", "jobId"];
    /// The status of a job the server couldn't look up, for now or for good.
    const LOOKUP_FAILED: &'static str = "lookup_failed";

    pub(crate) fn from_answer(answer: &Value, asked: &HashSet<String>) -> Self {
        let mut statuses = HashMap::new();
        for (id, job) in Self::jobs(answer) {
            if !asked.contains(&id) {
                continue;
            }
            let generation = json::get(job, "generation")
                .filter(|generation| generation.is_object())
                .unwrap_or(job);
            let Some(text) = json::string(generation, "status") else {
                continue;
            };
            if text == Self::LOOKUP_FAILED {
                continue;
            }
            if let Some(status) = Self::status(generation, json::get(job, "error")) {
                statuses.insert(id, status);
            }
        }
        let poll_after_seconds =
            json::num(answer, "poll_after_seconds").filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
        Self {
            statuses,
            poll_after_seconds,
        }
    }

    /// As `job_status` reads it, with the file in `result_url` when `results.rawUrl` is missing.
    fn status(job: &Value, error: Option<&Value>) -> Option<GenerationStatus> {
        let raw_url = json::get(job, "results").and_then(|results| json::get(results, "rawUrl"));
        if json::string(job, "status") == Some("completed") && raw_url.is_none() {
            return json::media_url(job, "result_url")
                .map(|video_url| GenerationStatus::Completed { video_url });
        }
        status_of(job, error)
    }

    /// Each job of the answer with its ID.
    fn jobs(answer: &Value) -> Vec<(String, &Value)> {
        for key in Self::LIST_KEYS {
            if let Some(list) = json::array(answer, key) {
                return list
                    .iter()
                    .filter_map(|job| {
                        Self::ID_KEYS
                            .iter()
                            .find_map(|key| json::string(job, key))
                            .map(|id| (id.to_owned(), job))
                    })
                    .collect();
            }
            if let Some(by_id) = json::get(answer, key).and_then(Value::as_object) {
                return by_id
                    .iter()
                    .filter(|(_, job)| job.is_object())
                    .map(|(id, job)| (id.clone(), job))
                    .collect();
            }
        }
        Vec::new()
    }

    /// The `jobs` argument: each ID with its place in `ids` as the `index` the schema requires.
    pub(crate) fn arguments(ids: &[String]) -> Object {
        let jobs: Vec<Value> = ids
            .iter()
            .enumerate()
            .map(|(index, id)| json!({ "job_id": id, "index": index }))
            .collect();
        object(json!({ "jobs": jobs }))
    }
}

impl HiggsfieldAccount {
    /// How many jobs one `jobs_wait` call takes (its schema's `maxItems`).
    pub const JOBS_WAIT_LIMIT: usize = 12;

    /// One `jobs_wait` long-poll of `ids`, which the server holds for up to 15 s. `None` when the account has no
    /// `jobs_wait` whose schema takes `jobs`, or `ids` is empty or longer than `JOBS_WAIT_LIMIT`. Read-only and free.
    pub async fn wait_for_jobs(&self, ids: &[String]) -> Result<Option<JobsWait>, HiggsfieldError> {
        if ids.is_empty()
            || ids.len() > Self::JOBS_WAIT_LIMIT
            || self
                .look_up(ToolRole::JobsWait, Self::OPTIONAL_TOOL_RECHECK)
                .await?
                .is_none()
        {
            return Ok(None);
        }
        let answer = self.call(ToolRole::JobsWait, JobsWait::arguments(ids)).await?;
        Ok(Some(JobsWait::from_answer(
            &answer,
            &ids.iter().cloned().collect(),
        )))
    }

    /// `HiggsfieldError::ToolUnavailable` when the account has no tool for `operation`.
    pub async fn check_tool(&self, operation: &VideoEditOperation) -> Result<(), HiggsfieldError> {
        match self
            .look_up(operation.role(), Self::OPTIONAL_TOOL_RECHECK)
            .await?
        {
            Some(_) => Ok(()),
            None => Err(HiggsfieldError::ToolUnavailable(
                operation.role().raw_value().to_owned(),
            )),
        }
    }

    /// The price in credits; submits nothing. `reframe` asks its own tool, when its schema still declares
    /// `get_cost`; the others ask `generate_video` for their catalog model and the source's `duration` in seconds.
    /// `None`, without a call, with neither, and `None` when the answer names no price or the question is refused.
    pub async fn operation_cost(
        &self,
        operation: &VideoEditOperation,
        duration: Option<f64>,
    ) -> Result<Option<f64>, HiggsfieldError> {
        if let Some(arguments) = VideoEdit::quote_arguments(operation) {
            let declares_cost = self
                .look_up(operation.role(), Self::OPTIONAL_TOOL_RECHECK)
                .await?
                .is_some_and(|tool| params_argument_names(&tool).contains("get_cost"));
            if !declares_cost {
                return Ok(None);
            }
            let value = self.call(operation.role(), arguments?).await?;
            return Ok(credits(json::get(&value, "cost")));
        }
        let Some(arguments) = duration
            .filter(|duration| *duration > 0.0)
            .and_then(|duration| VideoEdit::generate_quote_arguments(operation, duration))
        else {
            return Ok(None);
        };
        match self.call(ToolRole::Generate, arguments).await {
            Ok(value) => Ok(credits(json::get(&value, "cost"))),
            // A model the quote doesn't take: the price is shown after the run.
            Err(HiggsfieldError::Validation { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Not idempotent, like `generate`: no tool takes an idempotency key. The job ID is read like
    /// `execute_preset`'s; a reply that names no job and gives no reason is `Server { status: 200 }`, as it may
    /// have started one. The result is read with `status`.
    pub async fn run_edit(&self, edit: &VideoEdit) -> Result<Generation, HiggsfieldError> {
        let value = self.call(edit.operation.role(), edit.arguments()?).await?;
        let id = preset_job_id(&value).ok_or_else(|| no_job_started(&value))?;
        Ok(Generation::new(
            id,
            json::string(&value, "request_id").map(str::to_owned),
            credits(json::get(&value, "cost")),
        ))
    }

    /// A page of the voices `voice_change` takes: built-in ones and the account's own. `size` is 1–100.
    pub async fn voices(&self, cursor: Option<&Value>, size: i64) -> Result<VoicePage, HiggsfieldError> {
        let mut arguments = object(json!({ "size": size.clamp(1, 100) }));
        super::insert_cursor(&mut arguments, cursor);
        let value = self.call(ToolRole::ListVoices, arguments).await?;
        VoicePage::from_value(&value)
            .ok_or_else(|| HiggsfieldError::validation("Unexpected result from the list_voices tool"))
    }

    /// The fixed `params.model` the Virality Predictor requires.
    const VIRALITY_MODEL: &'static str = "virality_predictor";

    /// `action: "create"` on a finished generation, named by its job ID as the video.
    pub(crate) fn virality_arguments(job_id: &str) -> Object {
        object(json!({
            "action": "create",
            "params": { "model": Self::VIRALITY_MODEL, "medias": [{ "role": "video", "id": job_id }] },
        }))
    }

    /// Starts scoring a finished generation; free, so a resend costs nothing. `ToolUnavailable`, without a call,
    /// when the account has no `virality_predictor`; a refusal (`isError`, or an answer with an `error` or without a
    /// `job_id`) is `Validation`.
    pub async fn predict_virality(&self, job_id: &str) -> Result<ViralityJob, HiggsfieldError> {
        if self
            .look_up(ToolRole::ViralityPredictor, Self::OPTIONAL_TOOL_RECHECK)
            .await?
            .is_none()
        {
            return Err(HiggsfieldError::ToolUnavailable(
                ToolRole::ViralityPredictor.raw_value().to_owned(),
            ));
        }
        let value = self
            .call(ToolRole::ViralityPredictor, Self::virality_arguments(job_id))
            .await?;
        let Some(id) = json::string(&value, "job_id").filter(|id| !id.is_empty()) else {
            return Err(HiggsfieldError::validation(
                json::string(&value, "error").unwrap_or("Unexpected result from the virality_predictor tool"),
            ));
        };
        Ok(ViralityJob {
            id: id.to_owned(),
            poll_interval: json::num(&value, "poll_interval")
                .filter(|seconds| seconds.is_finite() && *seconds >= 0.0),
        })
    }

    /// `job_status` of a Virality Predictor job, with its readings once it completed.
    pub async fn virality_status(&self, job_id: &str) -> Result<ViralityStatus, HiggsfieldError> {
        let value = self
            .call(ToolRole::Status, object(json!({ "jobId": job_id })))
            .await?;
        let generation = json::get(&value, "generation").ok_or_else(|| unexpected(ToolRole::Status))?;
        Ok(virality_status_of(generation, json::get(&value, "error")))
    }
}

/// A completed job reads its analysis rather than requiring `rawUrl` as a video does; any other status reads as
/// `status` does, a rejection or cancel as a failure.
pub(crate) fn virality_status_of(generation: &Value, error: Option<&Value>) -> ViralityStatus {
    if json::string(generation, "status") == Some("completed") {
        return ViralityStatus::Completed(ViralityPrediction::from_generation(generation));
    }
    match status_of(generation, error) {
        Some(GenerationStatus::Failed { message }) => ViralityStatus::Failed { message },
        Some(GenerationStatus::Rejected(_) | GenerationStatus::Canceled) => {
            ViralityStatus::Failed { message: None }
        }
        _ => ViralityStatus::Running,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Moderation;

    /// `jobs_wait` answers in the shape of the tool's `outputSchema` (catalog snapshot of 2026-10-15).
    fn answer() -> Value {
        json!({
            "jobs": [
                {
                    "index": 0, "job_id": "job-1", "status": "completed", "type": "video", "model": "seedance_2_5",
                    "result_url": "https://cdn.example/1.mp4", "thumbnail_url": "https://cdn.example/1.jpg",
                },
                {"index": 1, "job_id": "job-2", "status": "in_progress"},
                {"index": 2, "job_id": "job-3", "status": "failed", "error": "Bad input"},
            ],
            "summary": {"total": 3, "completed": 1, "failed": 1, "active": 1, "errors": 0},
            "all_terminal": false, "poll_after_seconds": 5, "timed_out": true,
        })
    }

    fn asked() -> HashSet<String> {
        ["job-1", "job-2", "job-3"].map(str::to_owned).into()
    }

    fn statuses(entries: Vec<(&str, GenerationStatus)>) -> HashMap<String, GenerationStatus> {
        entries
            .into_iter()
            .map(|(id, status)| (id.to_owned(), status))
            .collect()
    }

    fn completed(url: &str) -> GenerationStatus {
        GenerationStatus::Completed {
            video_url: Url::parse(url).unwrap(),
        }
    }

    #[test]
    fn the_schemas_shape_is_read() {
        let expected = JobsWait {
            statuses: statuses(vec![
                ("job-1", completed("https://cdn.example/1.mp4")),
                ("job-2", GenerationStatus::InProgress),
                (
                    "job-3",
                    GenerationStatus::Failed {
                        message: Some("Bad input".into()),
                    },
                ),
            ]),
            poll_after_seconds: Some(5.0),
        };
        assert_eq!(JobsWait::from_answer(&answer(), &asked()), expected);
    }

    #[test]
    fn other_shapes_are_read_too() {
        let nested = json!({"results": [
            {"job_id": "job-1", "generation": {"status": "nsfw"}}, {"jobId": "job-2", "status": "queued"},
            {"id": "job-3", "status": "completed", "results": {"rawUrl": "https://cdn.example/3.mp4"}},
        ]});
        let expected = statuses(vec![
            ("job-1", GenerationStatus::Rejected(Moderation::ExplicitContent)),
            ("job-2", GenerationStatus::Queued),
            ("job-3", completed("https://cdn.example/3.mp4")),
        ]);
        assert_eq!(JobsWait::from_answer(&nested, &asked()).statuses, expected);

        let by_id = json!({"jobs": {"job-1": {"status": "canceled"}, "job-9": {"status": "queued"}}});
        let expected = statuses(vec![("job-1", GenerationStatus::Canceled)]);
        assert_eq!(JobsWait::from_answer(&by_id, &asked()).statuses, expected);
    }

    #[test]
    fn jobs_that_cant_be_read_are_left_out() {
        let answer = json!({
            "jobs": [
                {"job_id": "job-1", "status": "completed"},
                {"job_id": "job-2", "state": "completed"},
                {"job_id": "job-3", "status": "completed", "result_url": "http://cdn.example/3.mp4"},
                {"job_id": "job-4", "status": "queued"},
                {"status": "queued"},
            ],
            "poll_after_seconds": -1,
        });
        assert_eq!(JobsWait::from_answer(&answer, &asked()), JobsWait::default());
    }

    /// The server couldn't look these up, for now or for good: `job_status` is asked instead.
    #[test]
    fn failed_lookups_are_left_out() {
        let answer = json!({
            "jobs": [
                {"index": 0, "job_id": "job-1", "status": "lookup_failed", "retryable": true},
                {"index": 1, "job_id": "job-2", "status": "lookup_failed", "retryable": false, "error": "Not found"},
                {"index": 2, "job_id": "job-3", "status": "queued"},
            ],
            "all_terminal": false,
        });
        let expected = statuses(vec![("job-3", GenerationStatus::Queued)]);
        assert_eq!(JobsWait::from_answer(&answer, &asked()).statuses, expected);
    }

    #[test]
    fn junk_reads_as_nothing() {
        for answer in [
            Value::Null,
            json!("done"),
            json!(42),
            json!([]),
            json!({}),
            json!({"jobs": "job-1"}),
            json!({"jobs": [1, 2]}),
            json!({"status": "completed"}),
            json!({"poll_after_seconds": "5"}),
            json!({"all_terminal": "yes"}),
        ] {
            assert_eq!(
                JobsWait::from_answer(&answer, &asked()),
                JobsWait::default(),
                "{answer}"
            );
        }
    }

    /// `ViralityTests.aCompletedJobWithoutScoresHasNoPrediction`.
    #[test]
    fn a_completed_virality_job_without_scores_has_no_prediction() {
        let generation = json!({
            "id": "7d2e9b41-0c3a-4f6e-8b12-5a9c0d1e2f34", "type": "image", "status": "completed",
            "model": "brain_activity", "params": {"prompt": "", "analysis": null},
            "results": {"rawUrl": "https://cdn.example/virality/hf_20261015_job.html"},
        });
        assert_eq!(
            virality_status_of(&generation, None),
            ViralityStatus::Completed(None)
        );
    }

    /// `ViralityTests.aFailedOrRejectedJobFails`.
    #[test]
    fn a_failed_or_rejected_virality_job_fails() {
        let failed = json!({"status": "failed", "error": "Video too long"});
        assert_eq!(
            virality_status_of(&failed, None),
            ViralityStatus::Failed {
                message: Some("Video too long".into())
            }
        );
        assert_eq!(
            virality_status_of(&json!({"status": "nsfw"}), None),
            ViralityStatus::Failed { message: None }
        );
        assert_eq!(
            virality_status_of(&json!({"status": "queued"}), None),
            ViralityStatus::Running
        );
    }
}
