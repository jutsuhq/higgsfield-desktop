//! What the video, image, jobs and `job_action` commands answer.

use crate::{AgentAmount, AgentElement, AgentExportFile, AgentJobAction, AgentReference, Timestamp, Uuid};
use serde::{Deserialize, Serialize};

/// One piece of work, as an agent follows it with `jobs`: a video job, an image request, an export, or an Enhance run
/// on this computer (`kind`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools, reason = "the wire's fields")]
pub struct AgentJob {
    pub id: Uuid,
    /// preparing, uploading, `waiting_for_slot`, queued, processing, downloading, paused, done, failed, rejected,
    /// canceled; unknown for an ID Jutsu Studio doesn't know.
    pub state: String,
    pub pause_reason: Option<String>,
    pub hold: Option<String>,
    /// Only the person can move it on.
    pub needs_person: bool,
    pub progress: Option<f64>,
    pub price: Option<AgentAmount>,
    pub is_price_exact: bool,
    /// The provider charged, or may have.
    pub charged: bool,
    pub error: Option<String>,
    pub result: Option<String>,
    pub results: Option<Vec<String>>,
    /// video, image, export, `delivery_export`, `on_device`, unknown.
    pub kind: String,
    pub request_id: Option<String>,
    pub client: Option<String>,
    pub mode: Option<String>,
    pub model: Option<String>,
    pub folder: Option<String>,
    pub files: Option<Vec<AgentExportFile>>,
    pub set: Option<Uuid>,
    pub operation: Option<String>,
    pub item: Option<Uuid>,
    pub estimated_seconds: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<f64>,
    pub frame_note: Option<String>,
    pub next: Option<String>,
    pub created_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
}

impl AgentJob {
    /// A job in `state` with nothing else set, of kind video.
    pub fn new(id: Uuid, state: &str, created_at: Option<Timestamp>) -> Self {
        Self {
            id,
            state: state.to_owned(),
            pause_reason: None,
            hold: None,
            needs_person: false,
            progress: None,
            price: None,
            is_price_exact: false,
            charged: false,
            error: None,
            result: None,
            results: None,
            kind: "video".to_owned(),
            request_id: None,
            client: None,
            mode: None,
            model: None,
            folder: None,
            files: None,
            set: None,
            operation: None,
            item: None,
            estimated_seconds: None,
            width: None,
            height: None,
            duration: None,
            frame_note: None,
            next: None,
            created_at,
            finished_at: None,
        }
    }

    /// This job as read again, keeping the `next` an earlier answer gave it while it still waits in the same state for
    /// the same reason.
    #[must_use]
    pub fn keeping_next(&self, earlier: &Self) -> Self {
        let mut job = self.clone();
        if self.next.is_none()
            && self.state == earlier.state
            && self.pause_reason == earlier.pause_reason
            && self.hold == earlier.hold
        {
            job.next.clone_from(&earlier.next);
        }
        job
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCreated {
    pub request_id: String,
    /// The request was known: these are the jobs it queued before, and nothing new was queued.
    pub known: bool,
    pub jobs: Vec<AgentJob>,
    /// The images a new request sent, in order, with the `@ImageN` tags its prompt names them by.
    pub references: Option<Vec<AgentReference>>,
    /// With `wait_seconds`: the wait ran out before every job's price was known or it was held.
    pub timed_out: Option<bool>,
    /// `create_element`: the element it made, which queues no job.
    pub element: Option<AgentElement>,
}

impl AgentCreated {
    pub fn new(request_id: &str, known: bool, jobs: Vec<AgentJob>) -> Self {
        Self {
            request_id: request_id.to_owned(),
            known,
            jobs,
            references: None,
            timed_out: None,
            element: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobs {
    pub jobs: Vec<AgentJob>,
    /// The wait ran out before every job settled; nothing was stopped.
    pub timed_out: bool,
    /// Without ids: pass as `cursor` for the next, older page; `None` on the last one.
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentActed {
    pub action: AgentJobAction,
    /// The job as it is now, for an action on one.
    pub job: Option<AgentJob>,
}
