//! System notifications about jobs (`JobNotification.swift`): which state change posts one, its title, its text, its
//! buttons, what a click opens, and the tracker that makes each change post once.
//!
//! macOS posts exactly these: a job done, failed, rejected or paused for the person. It posts nothing for a finished
//! batch or for low credits, so neither is posted here.

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::PathBuf;

use jutsu_core::{Job, JobState, PauseReason};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::preferences::NotificationPreferences;
use crate::queue::{JobRowModel, QueueJobCommands, format};
use crate::system::tray::TrayOpen;

/// What a notification is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationKind {
    Done,
    Failed,
    Rejected,
    Paused,
}

impl NotificationKind {
    /// `None` for the states that don't notify: work in progress, and canceled or paused by the person.
    pub fn of(state: &JobState) -> Option<Self> {
        match state {
            JobState::Done => Some(Self::Done),
            JobState::Failed { .. } => Some(Self::Failed),
            JobState::Rejected => Some(Self::Rejected),
            JobState::Paused(PauseReason::ByUser) => None,
            JobState::Paused(_) => Some(Self::Paused),
            _ => None,
        }
    }
}

/// Whether the Settings › Notifications switches let `kind` post: done is one switch, the problems the other.
pub fn allows(preferences: NotificationPreferences, kind: NotificationKind) -> bool {
    match kind {
        NotificationKind::Done => preferences.on_done,
        NotificationKind::Failed | NotificationKind::Rejected | NotificationKind::Paused => {
            preferences.on_problems
        }
    }
}

/// The screen a click on the notification opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Destination {
    Queue,
    Library,
}

impl Destination {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Library => "library",
        }
    }

    fn from_raw(value: &str) -> Option<Self> {
        match value {
            "queue" => Some(Self::Queue),
            "library" => Some(Self::Library),
            _ => None,
        }
    }
}

/// A button on the notification. Each runs what the same button does in the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationAction {
    /// Opens the result's page, where its final is priced and rendered.
    RenderFinal,
    ShowInFolder,
    /// Retry, Retry download or Check again, whichever the job's Queue row offers.
    Retry,
}

impl NotificationAction {
    pub const ALL: [Self; 3] = [Self::RenderFinal, Self::ShowInFolder, Self::Retry];

    /// The identifier the system reports back; macOS's raw values.
    pub fn id(self) -> &'static str {
        match self {
            Self::RenderFinal => "renderFinal",
            Self::ShowInFolder => "showInFinder",
            Self::Retry => "retry",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::RenderFinal => "Render Final…",
            Self::ShowInFolder => "Show in File Manager",
            Self::Retry => "Retry",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }
}

/// The set of buttons a notification has; registered with the system once, by identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationCategory {
    DraftResult,
    Result,
    Retryable,
}

impl NotificationCategory {
    pub const ALL: [Self; 3] = [Self::DraftResult, Self::Result, Self::Retryable];

    pub fn id(self) -> &'static str {
        match self {
            Self::DraftResult => "job.draft-result",
            Self::Result => "job.result",
            Self::Retryable => "job.retryable",
        }
    }

    pub fn actions(self) -> &'static [NotificationAction] {
        match self {
            Self::DraftResult => &[NotificationAction::RenderFinal, NotificationAction::ShowInFolder],
            Self::Result => &[NotificationAction::ShowInFolder],
            Self::Retryable => &[NotificationAction::Retry],
        }
    }
}

/// A system notification about one job reaching a state the person has to know about.
#[derive(Debug, Clone, PartialEq)]
pub struct JobNotification {
    pub job_id: Uuid,
    pub kind: NotificationKind,
    /// What happened: "Video ready".
    pub title: String,
    /// Which job: "dance · Motion transfer".
    pub subtitle: String,
    /// Why, or how long it took; empty when there's nothing to add.
    pub body: String,
    pub plays_sound: bool,
    /// The batch the job belongs to; `None` for a single job. macOS groups a batch's notifications by it; the
    /// notification servers here have no such grouping, so it is kept for the payload only.
    pub thread_id: Option<String>,
    /// The finished video, for Show in File Manager.
    pub result_file: Option<PathBuf>,
    /// The notification's buttons; `None` for none.
    pub category: Option<NotificationCategory>,
}

const JOB_ID_KEY: &str = "jobID";
const DESTINATION_KEY: &str = "destination";
const RESULT_PATH_KEY: &str = "resultPath";

impl JobNotification {
    fn new(job: &Job, kind: NotificationKind, plays_sound: bool) -> Self {
        let row = JobRowModel::new(job, job.created_at, None);
        let title = match kind {
            NotificationKind::Done => "Video ready",
            NotificationKind::Failed => "Job failed",
            NotificationKind::Rejected => "Job rejected",
            NotificationKind::Paused => "Job paused",
        };
        let body = [
            row.note.as_ref().map(|note| note.text.clone()),
            (kind == NotificationKind::Done).then(|| generated(job)).flatten(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
        let result_file = (kind == NotificationKind::Done)
            .then(|| job.output_file.as_ref().and_then(|url| url.to_file_path().ok()))
            .flatten();
        let category = match kind {
            NotificationKind::Done if result_file.is_none() => None,
            NotificationKind::Done if job.spec.is_draft => Some(NotificationCategory::DraftResult),
            NotificationKind::Done => Some(NotificationCategory::Result),
            NotificationKind::Failed => QueueJobCommands::new(&row)
                .retry
                .map(|_| NotificationCategory::Retryable),
            NotificationKind::Rejected | NotificationKind::Paused => None,
        };
        Self {
            job_id: job.id,
            kind,
            title: title.to_owned(),
            subtitle: row.title,
            body,
            plays_sound,
            thread_id: job.batch_id.map(|id| id.to_string().to_uppercase()),
            result_file,
            category,
        }
    }

    pub fn destination(&self) -> Destination {
        if self.kind == NotificationKind::Done {
            Destination::Library
        } else {
            Destination::Queue
        }
    }

    /// The text under the title. The notification servers here show a title and a body only, so the job's name
    /// (macOS's subtitle) leads the body.
    pub fn display_body(&self) -> String {
        if self.body.is_empty() {
            self.subtitle.clone()
        } else {
            format!("{}\n{}", self.subtitle, self.body)
        }
    }

    /// The notification's identifier, the same for every notification of one job, so a later state replaces the
    /// earlier banner where the platform can (macOS uses the job ID as the request identifier). The plugin takes an
    /// `i32`: the exclusive or of the job ID's four 32-bit words.
    pub fn notification_id(&self) -> i32 {
        let (words, _) = self.job_id.as_bytes().as_chunks::<4>();
        let folded = words
            .iter()
            .fold(0_u32, |folded, word| folded ^ u32::from_be_bytes(*word));
        i32::from_be_bytes(folded.to_be_bytes())
    }

    /// Stored in the notification, so a click finds the job and its file.
    pub fn user_info(&self) -> HashMap<String, String> {
        let mut info = HashMap::from([
            (JOB_ID_KEY.to_owned(), self.job_id.to_string()),
            (
                DESTINATION_KEY.to_owned(),
                self.destination().raw_value().to_owned(),
            ),
        ]);
        if let Some(file) = &self.result_file {
            info.insert(RESULT_PATH_KEY.to_owned(), file.to_string_lossy().into_owned());
        }
        info
    }
}

/// "Generated in 1 min 40 s": the API side, from submit until it finished; `None` unless both are known.
pub fn generated(job: &Job) -> Option<String> {
    let (submitted_at, charged_at) = (job.submitted_at?, job.charged_at?);
    if charged_at < submitted_at {
        return None;
    }
    Some(format!(
        "Generated in {}",
        format::spoken((charged_at - submitted_at).as_seconds_f64())
    ))
}

/// Where a notification's user info points; `None` for one this app didn't post.
pub fn target<S: BuildHasher>(user_info: &HashMap<String, String, S>) -> Option<(Destination, Uuid)> {
    let id = Uuid::parse_str(user_info.get(JOB_ID_KEY)?).ok()?;
    let destination = Destination::from_raw(user_info.get(DESTINATION_KEY)?)?;
    Some((destination, id))
}

/// What the person did with a notification this app posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationResponse {
    pub destination: Destination,
    pub job_id: Uuid,
    /// The button pressed; `None` for a click on the notification itself.
    pub action: Option<NotificationAction>,
    /// The finished video, for Show in File Manager.
    pub result_file: Option<PathBuf>,
}

/// The identifier the notification plugin reports for a click on the notification itself.
pub const DEFAULT_ACTION_ID: &str = "tap";

impl NotificationResponse {
    /// `None` for a notification this app didn't post, or for a response that is neither a click nor one of its
    /// buttons.
    pub fn new<S: BuildHasher>(user_info: &HashMap<String, String, S>, action_id: &str) -> Option<Self> {
        let (destination, job_id) = target(user_info)?;
        let action = if action_id == DEFAULT_ACTION_ID {
            None
        } else {
            Some(NotificationAction::from_id(action_id)?)
        };
        Some(Self {
            destination,
            job_id,
            action,
            result_file: user_info.get(RESULT_PATH_KEY).map(PathBuf::from),
        })
    }

    /// What the app does (`SystemIntegration.respond`): a click opens the job's screen, a button runs what the same
    /// button does in the app.
    pub fn effect(&self) -> NotificationEffect {
        let id = self.job_id.to_string();
        match self.action {
            None => NotificationEffect::Open(match self.destination {
                Destination::Library => TrayOpen::LibraryJob { id },
                Destination::Queue => TrayOpen::QueueJob { id },
            }),
            // As the card menu's Render Final…: the result's page picks the resolution and quotes the price.
            Some(NotificationAction::RenderFinal) => NotificationEffect::Open(TrayOpen::LibraryJob { id }),
            Some(NotificationAction::ShowInFolder) => match &self.result_file {
                Some(file) => NotificationEffect::Reveal(file.clone()),
                None => NotificationEffect::Open(TrayOpen::QueueJob { id }),
            },
            Some(NotificationAction::Retry) => NotificationEffect::Retry(self.job_id),
        }
    }
}

/// What answering a notification does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationEffect {
    /// Brings the main window forward on this screen.
    Open(TrayOpen),
    /// Shows the file in the file manager; the window stays as it is.
    Reveal(PathBuf),
    /// Opens the job's Queue row and runs the Retry, Retry Download or Check Again it offers now.
    Retry(Uuid),
}

/// The notifications for what changed between two snapshots of the queue. `since` is when observing started: a job
/// missing from `previous` notifies only if it finished after that, so finished jobs loaded on launch stay quiet.
pub fn changes<'a, S: BuildHasher>(
    previous: &HashMap<Uuid, JobState, S>,
    current: impl IntoIterator<Item = &'a Job>,
    since: OffsetDateTime,
    preferences: NotificationPreferences,
) -> Vec<JobNotification> {
    current
        .into_iter()
        .filter_map(|job| {
            let kind = NotificationKind::of(&job.state)?;
            if !allows(preferences, kind) {
                return None;
            }
            if let Some(old) = previous.get(&job.id) {
                if NotificationKind::of(old) == Some(kind) {
                    return None;
                }
            } else if kind == NotificationKind::Paused
                || job.finished_at.is_none_or(|finished| finished < since)
            {
                return None;
            }
            Some(JobNotification::new(job, kind, preferences.plays_sound))
        })
        .collect()
}

/// Remembers the last snapshot of the queue so each change notifies once.
#[derive(Debug, Clone)]
pub struct JobTransitionTracker {
    states: HashMap<Uuid, JobState>,
    since: OffsetDateTime,
}

impl JobTransitionTracker {
    pub fn new(since: OffsetDateTime) -> Self {
        Self {
            states: HashMap::new(),
            since,
        }
    }

    /// The notifications to post for `jobs`, every job the app knows. Progress-only updates return none.
    pub fn update<'a>(
        &mut self,
        jobs: impl IntoIterator<Item = &'a Job> + Clone,
        preferences: NotificationPreferences,
    ) -> Vec<JobNotification> {
        let notifications = changes(&self.states, jobs.clone(), self.since, preferences);
        self.states = jobs.into_iter().map(|job| (job.id, job.state.clone())).collect();
        notifications
    }
}

#[cfg(test)]
#[path = "notifications_tests.rs"]
mod tests;
