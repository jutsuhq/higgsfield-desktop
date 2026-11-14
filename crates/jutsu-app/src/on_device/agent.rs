//! An agent's `enhance` that runs on this computer (`AgentOnDeviceRuns.swift`, `AgentOnDeviceRuns+Rules.swift`):
//! refused by the sheet's own rules, run once for its request ID through the request gate, and followed with `jobs`
//! as kind `on_device`. Free, so no `max_cost`, agent budget or hold applies. The runs are `OnDeviceRuns`', the
//! person's too; this keeps which ones agents started, in memory: a relaunch forgets them, as it forgets exports.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use jutsu_agent::{
    AgentAmount, AgentBudgetUnit, AgentEnhancement, AgentError, AgentJob, AgentOnDeviceEngine,
    AgentSmoothMotion, EnhanceCommand, ErrorCode, OnDevice, Timestamp,
};
use jutsu_core::{
    MaterialReference, OnDeviceAction, OnDeviceEngine, OnDeviceEstimate, OnDeviceUpscale,
    OnDeviceUpscaleTarget, PixelSize, SmoothMotion,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::runs::{OnDeviceOutcome, OnDeviceRequest, OnDeviceRunState, OnDeviceRuns};
use crate::agents::wire_id;

/// What `jobs` calls a run on this computer.
pub const KIND: &str = "on_device";

/// What the app read of an item an agent asked to enhance on this computer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentOnDeviceMedia {
    /// A video's size, frame rate and length.
    Video {
        size: PixelSize,
        frame_rate: f64,
        duration: f64,
    },
    /// A video whose file can't be read.
    UnreadableVideo,
    /// An image's size; `None` when it can't be read.
    Image(Option<PixelSize>),
}

/// The item an agent named: its current version, its file, what it is, and where the result goes.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentOnDeviceTarget {
    pub source: MaterialReference,
    pub file: PathBuf,
    pub media: AgentOnDeviceMedia,
    pub destination: PathBuf,
    pub scratch: PathBuf,
}

#[derive(Debug, Clone)]
struct Record {
    request_id: String,
    client: Option<String>,
    estimate: Option<f64>,
}

/// The runs agents started.
pub struct AgentOnDeviceRuns {
    runs: Arc<OnDeviceRuns>,
    records: Mutex<HashMap<Uuid, Record>>,
}

impl std::fmt::Debug for AgentOnDeviceRuns {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentOnDeviceRuns")
            .finish_non_exhaustive()
    }
}

const HIGGSFIELD_UPSCALE: &str = "Upscale it on Higgsfield instead: engine standard, with max_cost (paid).";

impl AgentOnDeviceRuns {
    pub fn new(runs: Arc<OnDeviceRuns>) -> Self {
        Self {
            runs,
            records: Mutex::new(HashMap::new()),
        }
    }

    fn records(&self) -> MutexGuard<'_, HashMap<Uuid, Record>> {
        self.records.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts the run as the sheet would, once `record` kept the request with the run's ID; the request gate around
    /// it answers the same request again with the run it started.
    pub fn start(
        &self,
        fields: &EnhanceCommand,
        client: Option<String>,
        target: &AgentOnDeviceTarget,
        unit: AgentBudgetUnit,
        record: impl FnOnce(&[Uuid]) -> Result<(), AgentError>,
    ) -> Result<AgentJob, AgentError> {
        let action = action(fields, target.media)?;
        if let Some(refusal) = refusal(action, target, &self.runs) {
            return Err(refusal);
        }
        let id = Uuid::new_v4();
        record(&[id])?;
        let request = OnDeviceRequest {
            source: target.source,
            file: target.file.clone(),
            action,
            destination: target.destination.clone(),
            scratch: target.scratch.clone(),
        };
        if !self.runs.start(request, id) {
            return Err(refusal(action, target, &self.runs)
                .unwrap_or_else(|| AgentError::new(ErrorCode::FAILED, "The run didn't start.", true, None)));
        }
        self.records().insert(
            id,
            Record {
                request_id: fields.request_id.clone(),
                client,
                estimate: estimate(action, target.media),
            },
        );
        Ok(self.job(id, unit).unwrap_or_else(|| forgotten(id)))
    }

    // MARK: Following

    /// The runs agents started, for `jobs` without IDs.
    pub fn started(&self) -> Vec<(OffsetDateTime, Uuid)> {
        let ids: Vec<Uuid> = self.records().keys().copied().collect();
        ids.into_iter()
            .filter_map(|id| self.runs.run(id).map(|(run, _)| (run.created_at, id)))
            .collect()
    }

    /// Over, or not known: what `jobs`' wait waits for.
    pub fn is_settled(&self, id: Uuid) -> bool {
        self.runs.run(id).is_none_or(|(_, finished)| finished.is_some())
    }

    /// The run as `jobs` lists it: an agent's or the person's, while this computer knows it.
    pub fn job(&self, id: Uuid, unit: AgentBudgetUnit) -> Option<AgentJob> {
        let (run, finished) = self.runs.run(id)?;
        let record = self.records().get(&id).cloned();
        let mut job = AgentJob::new(wire_id(id), "processing", Some(Timestamp(run.created_at)));
        job.price = Some(AgentAmount::new(0, unit));
        job.is_price_exact = true;
        KIND.clone_into(&mut job.kind);
        job.request_id = record.as_ref().map(|record| record.request_id.clone());
        job.client = record.as_ref().and_then(|record| record.client.clone());
        job.estimated_seconds = record.and_then(|record| record.estimate);
        job.mode = mode(&run.request.action).map(str::to_owned);
        job.operation = Some(operation(&run.request.action).to_owned());
        job.item = Some(wire_id(run.request.source.material_id));
        job.finished_at = finished.as_ref().map(|finished| Timestamp(finished.finished_at));
        match (finished.map(|finished| finished.outcome), run.state) {
            (Some(OnDeviceOutcome::Done(file)), _) => {
                "done".clone_into(&mut job.state);
                job.progress = Some(1.0);
                job.result = Some(file.to_string_lossy().into_owned());
                job.next = Some(
                    "The result is now the item's current version: library_get with its ID shows it and the version \
                     it was made from; frames checks it."
                        .to_owned(),
                );
            }
            (Some(OnDeviceOutcome::Failed(message)), _) => {
                "failed".clone_into(&mut job.state);
                job.error = Some(message);
            }
            (Some(OnDeviceOutcome::Canceled), _) => "canceled".clone_into(&mut job.state),
            (None, OnDeviceRunState::Waiting) => {
                "queued".clone_into(&mut job.state);
                job.next = Some(
                    "Runs on this computer go one at a time; it starts when the one before it is done."
                        .to_owned(),
                );
            }
            (None, OnDeviceRunState::DownloadingModel(fraction)) => {
                "downloading".clone_into(&mut job.state);
                job.progress = Some(fraction);
                job.next = Some(
                    "Jutsu Studio is downloading Real-ESRGAN from GitHub once; the frames follow.".to_owned(),
                );
            }
            (None, OnDeviceRunState::Working(fraction)) => job.progress = Some(fraction),
        }
        Some(job)
    }

    /// Cancels a run an agent started and answers with it as it is now; `None` when `id` isn't a run here.
    pub async fn cancel(&self, id: Uuid, unit: AgentBudgetUnit) -> Result<Option<AgentJob>, AgentError> {
        let Some((_, finished)) = self.runs.run(id) else {
            return Ok(None);
        };
        if !self.records().contains_key(&id) {
            return Err(AgentError::new(
                ErrorCode::NEEDS_PERSON,
                "The person started this run on this computer; only they can cancel it.",
                false,
                Some("Ask the person, or leave it be."),
            ));
        }
        if finished.is_none() {
            self.runs.cancel(id);
            self.runs.over(id).await;
        }
        Ok(self.job(id, unit))
    }

    /// Why `show` can't open the sheet that starts an upscale: this computer has no engine for it.
    pub fn sheet_refusal(&self, reason: Option<String>) -> Option<AgentError> {
        let action = OnDeviceAction::UpscaleVideo(OnDeviceUpscaleTarget::Full);
        (!self.runs.supports(action.engine())).then(|| unavailable(action, reason))
    }

    /// What `capabilities` says of this computer's engines.
    pub fn capabilities(&self) -> OnDevice {
        let video = OnDeviceUpscale::VIDEO_LIMIT;
        let image = OnDeviceUpscale::SCALER_LIMIT;
        OnDevice::new(
            self.engine(OnDeviceEngine::FrameRateConversion, None),
            self.engine(OnDeviceEngine::VideoSuperResolution, Some(video)),
            self.engine(OnDeviceEngine::ImageSuperResolution, Some(image)),
        )
    }

    fn engine(&self, engine: OnDeviceEngine, limit: Option<PixelSize>) -> AgentOnDeviceEngine {
        let model = if !engine.downloads_model() {
            "none"
        } else if self.runs.needs_consent(engine) {
            "needs_consent"
        } else if self
            .runs
            .runs()
            .iter()
            .any(|run| run.request.action.engine() == engine && run.state.is_downloading_model())
        {
            "downloading"
        } else if self.runs.supports(engine) && self.runs.model_is_ready(engine) {
            "ready"
        } else {
            "agreed"
        };
        AgentOnDeviceEngine {
            available: self.runs.supports(engine),
            max_long_side: limit.map(|limit| limit.width),
            max_short_side: limit.map(|limit| limit.height),
            model: model.to_owned(),
        }
    }
}

/// A run's ID this computer no longer knows: a relaunch forgets them.
pub fn forgotten(id: Uuid) -> AgentJob {
    let mut job = AgentJob::new(wire_id(id), "unknown", None);
    job.error = Some("Jutsu Studio forgets a run on this computer when it quits.".to_owned());
    KIND.clone_into(&mut job.kind);
    job.next = Some(
        "If it finished, its result is on the item's current version: call library_get, or subtitles get, with the \
         item's ID. Otherwise send the request again with a new request_id."
            .to_owned(),
    );
    job
}

fn smooth_motion(mode: AgentSmoothMotion) -> SmoothMotion {
    match mode {
        AgentSmoothMotion::Fps60 => SmoothMotion::Fps60,
        AgentSmoothMotion::Slow2x => SmoothMotion::SlowMotion2x,
        AgentSmoothMotion::Slow4x => SmoothMotion::SlowMotion4x,
    }
}

fn agent_motion(motion: SmoothMotion) -> AgentSmoothMotion {
    match motion {
        SmoothMotion::Fps60 => AgentSmoothMotion::Fps60,
        SmoothMotion::SlowMotion2x => AgentSmoothMotion::Slow2x,
        SmoothMotion::SlowMotion4x => AgentSmoothMotion::Slow4x,
    }
}

/// The run the fields name on this item.
pub fn action(fields: &EnhanceCommand, media: AgentOnDeviceMedia) -> Result<OnDeviceAction, AgentError> {
    let is_image = matches!(media, AgentOnDeviceMedia::Image(_));
    match fields.operation {
        AgentEnhancement::SmoothMotion => {
            if is_image {
                return Err(AgentError::new(
                    ErrorCode::INVALID_REQUEST,
                    "smooth_motion runs on a video; this item is an image.",
                    false,
                    Some("Pick a video's Library ID (library_search with kind video)."),
                ));
            }
            let Some(mode) = fields.mode.as_deref().and_then(AgentSmoothMotion::parse) else {
                return Err(AgentError::bad_request(format!(
                    "smooth_motion needs mode: {}",
                    AgentSmoothMotion::names()
                )));
            };
            Ok(OnDeviceAction::SmoothMotion(smooth_motion(mode)))
        }
        AgentEnhancement::Upscale if fields.is_on_device() => {
            let target = match fields.target.as_deref() {
                None => OnDeviceUpscaleTarget::Full,
                Some(name) => OnDeviceUpscaleTarget::from_name(name).ok_or_else(|| {
                    AgentError::bad_request(format!(
                        "upscale with engine mac takes target {}; left out it is 4×",
                        OnDeviceUpscaleTarget::NAMES
                    ))
                })?,
            };
            Ok(if is_image {
                OnDeviceAction::UpscaleImage(target)
            } else {
                OnDeviceAction::UpscaleVideo(target)
            })
        }
        operation => Err(AgentError::bad_request(format!(
            "{} runs on Higgsfield, not on this computer",
            operation.as_str()
        ))),
    }
}

/// Why the sheet wouldn't start `action` on the target now, in the order an agent can act on: the engine, the file,
/// a run already going on the item, and last the person's agreement to the model download, so the person is only
/// asked for a run that can go.
pub fn refusal(
    action: OnDeviceAction,
    target: &AgentOnDeviceTarget,
    runs: &OnDeviceRuns,
) -> Option<AgentError> {
    let item = target.source.material_id;
    if !runs.supports(action.engine()) {
        return Some(unavailable(action, None));
    }
    match (action, target.media) {
        (_, AgentOnDeviceMedia::UnreadableVideo | AgentOnDeviceMedia::Image(None)) => {
            return Some(AgentError::new(
                ErrorCode::FAILED,
                "The item's file can't be read.",
                false,
                Some("Call library_get with its ID to check its file."),
            ));
        }
        (OnDeviceAction::UpscaleVideo(_), AgentOnDeviceMedia::Video { size, .. })
            if !OnDeviceUpscale::offers_video(size) =>
        {
            return Some(too_large(
                "videos",
                size,
                OnDeviceUpscale::VIDEO_LIMIT,
                HIGGSFIELD_UPSCALE,
            ));
        }
        (OnDeviceAction::UpscaleImage(_), AgentOnDeviceMedia::Image(Some(size)))
            if !OnDeviceUpscale::offers_image(size) =>
        {
            return Some(too_large(
                "images",
                size,
                OnDeviceUpscale::SCALER_LIMIT,
                "Pick an image up to that size; Higgsfield's upscale here takes videos only.",
            ));
        }
        (OnDeviceAction::UpscaleVideo(goal), AgentOnDeviceMedia::Video { size, .. })
        | (OnDeviceAction::UpscaleImage(goal), AgentOnDeviceMedia::Image(Some(size)))
            if goal.output(size).is_none() =>
        {
            return Some(unreachable(goal, size));
        }
        (OnDeviceAction::SmoothMotion(motion), AgentOnDeviceMedia::Video { frame_rate, .. })
            if !motion.is_offered(frame_rate) =>
        {
            let offered: Vec<&str> = SmoothMotion::offered(frame_rate)
                .into_iter()
                .map(|motion| agent_motion(motion).as_str())
                .collect();
            #[allow(clippy::cast_possible_truncation, reason = "a frame rate")]
            let rate = frame_rate.round() as i64;
            return Some(AgentError::new(
                ErrorCode::INVALID_REQUEST,
                format!("60fps adds nothing to a clip of 50 frames a second or more; this one has {rate}."),
                false,
                Some(&format!("Use mode {}.", offered.join(" or "))),
            ));
        }
        _ => {}
    }
    if let Some(run) = runs.run_for(item) {
        let (item, run_id) = (wire_id(item), wire_id(run.id));
        return Some(AgentError::new(
            ErrorCode::BUSY,
            format!(
                "Item {item} already has a run on this computer, {run_id} ({}); each run adds a version, so the next \
                 waits for it.",
                operation(&run.request.action)
            ),
            true,
            Some(&format!(
                "Follow it with jobs (ids [{run_id}]) and send enhance again once it is over, or cancel it with \
                 job_action if an agent started it."
            )),
        ));
    }
    if runs.needs_consent(action.engine()) {
        let item = wire_id(item);
        return Some(AgentError::new(
            ErrorCode::NEEDS_PERSON,
            "Nothing was started: Upscale on this computer needs Jutsu Studio to download Real-ESRGAN from GitHub \
             once, and only the person can agree to that. There is no run to follow in jobs until enhance is sent \
             again.",
            true,
            Some(&format!(
                "Call show with item {item} and sheet upscale_on_mac, which opens the sheet where the person agrees \
                 (and may start the upscale there), and tell the person; once they agreed, send enhance again. \
                 capabilities' onDevice model says needs_consent until then."
            )),
        ));
    }
    None
}

/// The engine can't run here; `reason` says why when the engine knows (no Vulkan, no build for this system).
pub fn unavailable(action: OnDeviceAction, reason: Option<String>) -> AgentError {
    match action {
        OnDeviceAction::SmoothMotion(_) => AgentError::new(
            ErrorCode::ENGINE_UNAVAILABLE,
            reason.map_or_else(
                || "Smooth Motion needs ffmpeg, which this computer doesn't have.".to_owned(),
                |reason| format!("Smooth Motion can't run here: {reason}"),
            ),
            false,
            Some("Upscale with engine standard and frame_rate 60 makes 60 fps on Higgsfield."),
        ),
        OnDeviceAction::UpscaleVideo(_) | OnDeviceAction::UpscaleImage(_) => AgentError::new(
            ErrorCode::ENGINE_UNAVAILABLE,
            reason.map_or_else(
                || "Upscale on this computer needs ffmpeg and Real-ESRGAN with Vulkan, which this computer doesn't have."
                    .to_owned(),
                |reason| format!("Upscale on this computer can't run here: {reason}"),
            ),
            false,
            Some(if action.engine() == OnDeviceEngine::VideoSuperResolution {
                HIGGSFIELD_UPSCALE
            } else {
                "Higgsfield's upscale here takes videos only."
            }),
        ),
    }
}

fn too_large(what: &str, size: PixelSize, limit: PixelSize, next: &str) -> AgentError {
    AgentError::new(
        ErrorCode::INVALID_REQUEST,
        format!(
            "Upscale on this computer takes {what} up to {}×{} either way round; this one is {}×{}.",
            limit.width, limit.height, size.width, size.height
        ),
        false,
        Some(next),
    )
}

fn unreachable(target: OnDeviceUpscaleTarget, size: PixelSize) -> AgentError {
    let scale = OnDeviceUpscale::SCALE;
    AgentError::new(
        ErrorCode::INVALID_REQUEST,
        format!(
            "target {} isn't larger than this item, {}×{}, or needs more than 4×: on this computer it upscales to at \
             most {}×{}.",
            target.file_suffix(),
            size.width,
            size.height,
            size.width * scale,
            size.height * scale
        ),
        false,
        Some("Pick a target in between, or leave target out for 4×."),
    )
}

/// The sheet's estimate of how long the run takes.
pub fn estimate(action: OnDeviceAction, media: AgentOnDeviceMedia) -> Option<f64> {
    match (action, media) {
        (
            OnDeviceAction::SmoothMotion(_),
            AgentOnDeviceMedia::Video {
                size,
                frame_rate,
                duration,
            },
        ) => Some(OnDeviceEstimate::smooth_motion(duration, frame_rate, size)),
        (
            OnDeviceAction::UpscaleVideo(_),
            AgentOnDeviceMedia::Video {
                size,
                frame_rate,
                duration,
            },
        ) => Some(OnDeviceEstimate::upscale_video(duration, frame_rate, size)),
        (OnDeviceAction::UpscaleImage(_), AgentOnDeviceMedia::Image(Some(size))) => {
            Some(OnDeviceEstimate::upscale_image(size))
        }
        _ => None,
    }
}

pub fn operation(action: &OnDeviceAction) -> &'static str {
    if matches!(action, OnDeviceAction::SmoothMotion(_)) {
        AgentEnhancement::SmoothMotion.as_str()
    } else {
        AgentEnhancement::Upscale.as_str()
    }
}

pub fn mode(action: &OnDeviceAction) -> Option<&'static str> {
    match action {
        OnDeviceAction::SmoothMotion(motion) => Some(agent_motion(*motion).as_str()),
        _ => None,
    }
}
