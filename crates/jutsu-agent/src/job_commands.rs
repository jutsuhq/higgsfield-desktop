//! `jobs` and `job_action`: following the agents' work and acting on it. Their answers are in `job`.

use crate::{AgentError, Uuid, names, values::wire_enum, video::count};
use serde::{Deserialize, Serialize};

/// The state of the named jobs, or of the agents' latest jobs; with `wait_seconds`, once they have all settled or the
/// wait is over, whichever comes first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobsCommand {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<Uuid>,
    /// Without `ids`: how many of the agents' latest jobs, newest first.
    pub limit: Option<i64>,
    #[serde(default, skip_serializing_if = "is_not_positive")]
    pub wait_seconds: i64,
    /// Without `ids`: the `nextCursor` of the page before.
    pub cursor: Option<String>,
}

#[allow(clippy::trivially_copy_pass_by_ref, reason = "serde passes a reference")]
fn is_not_positive(number: &i64) -> bool {
    *number <= 0
}

impl JobsCommand {
    pub const MAX_IDS: i64 = 20;
    pub const MAX_LIMIT: i64 = 50;
    /// The longest wait: under the minute MCP clients usually allow a tool call.
    pub const MAX_WAIT_SECONDS: i64 = 50;

    /// Why these fields can't be a `jobs` call; `None` when they can.
    pub fn problem(
        id_count: i64,
        limit: Option<i64>,
        wait_seconds: i64,
        cursor: Option<&str>,
    ) -> Option<String> {
        if id_count > Self::MAX_IDS {
            return Some(format!("ids takes at most {} job IDs", Self::MAX_IDS));
        }
        if limit.is_some_and(|limit| !(1..=Self::MAX_LIMIT).contains(&limit)) {
            return Some(format!("limit is 1 to {}", Self::MAX_LIMIT));
        }
        if let Some(problem) = Self::wait_problem(wait_seconds) {
            return Some(problem);
        }
        if let Some(cursor) = cursor {
            if id_count != 0 {
                return Some("cursor pages the latest jobs; leave out ids".to_owned());
            }
            if !names::is_valid_cursor(cursor) {
                return Some("cursor is a nextCursor from an earlier answer".to_owned());
            }
        }
        None
    }

    /// Why a wait of `seconds` can't be asked for: `jobs`, `create_video` and the exports take the same.
    pub fn wait_problem(seconds: i64) -> Option<String> {
        (!(0..=Self::MAX_WAIT_SECONDS).contains(&seconds))
            .then(|| format!("wait_seconds is 0 to {}", Self::MAX_WAIT_SECONDS))
    }

    pub(crate) fn check(&self) -> Result<(), AgentError> {
        Self::problem(
            count(self.ids.len()),
            self.limit,
            self.wait_seconds,
            self.cursor.as_deref(),
        )
        .map_or(Ok(()), |problem| Err(AgentError::bad_request(problem)))
    }
}

wire_enum! {
    /// What `job_action` can do. None of them deletes anything.
    pub enum AgentJobAction {
        Cancel = "cancel",
        Retry = "retry",
        RunNext = "run_next",
        PauseQueue = "pause_queue",
        ResumeQueue = "resume_queue",
    }
}

impl AgentJobAction {
    /// The actions on one job, which need its ID; the queue's take none.
    pub fn needs_job(self) -> bool {
        matches!(self, Self::Cancel | Self::Retry | Self::RunNext)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobActionCommand {
    pub action: AgentJobAction,
    pub job: Option<Uuid>,
}

impl JobActionCommand {
    /// Why `action` can't go with a job, or without one; `None` when it can.
    pub fn problem(action: AgentJobAction, has_job: bool) -> Option<String> {
        if action.needs_job() == has_job {
            return None;
        }
        Some(if action.needs_job() {
            format!("{} needs job, a job ID", action.as_str())
        } else {
            format!("{} acts on the whole queue and takes no job", action.as_str())
        })
    }

    pub(crate) fn check(&self) -> Result<(), AgentError> {
        Self::problem(self.action, self.job.is_some())
            .map_or(Ok(()), |problem| Err(AgentError::bad_request(problem)))
    }
}
