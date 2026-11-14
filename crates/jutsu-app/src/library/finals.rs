//! Render final and Finalize in 1080p of a draft (`ResultFollowUp.swift`, `LibraryItemModel`'s follow-up jobs,
//! `BatchActions.swift`'s `BatchFinals`).

pub use jutsu_core::Resolution;
use jutsu_core::{Cost, Job, JobSpec, JobState, KlingQuality, Pricing, ProviderKind};
use serde::Serialize;
use time::{Duration, OffsetDateTime, UtcOffset};
use uuid::Uuid;

use super::item::LibraryItem;
use crate::queue::format;

/// How long Higgsfield keeps a native draft finalizable.
pub const FINALIZE_WINDOW: Duration = Duration::days(7);
/// The button and menu item of a finalize.
pub const FINALIZE_TITLE: &str = "Finalize in 1080p";
/// The finalize button's tooltip.
pub const FINALIZE_HELP: &str = "Render this draft in 1080p";
/// Render final's button while that final is in the queue.
pub const FINAL_QUEUED: &str = "Final queued · View in Queue";
/// Render Finals for Starred… on a batch.
pub const FINALS_MENU_TITLE: &str = "Render Finals for Starred…";

/// When this native Seedance draft of the account stops being finalizable: seven days after Higgsfield completed it
/// (`charged_at`, else when the job finished here). `None` for any other job.
pub fn finalize_deadline(job: &Job) -> Option<OffsetDateTime> {
    let native = job.remote.as_ref().is_some_and(|remote| remote.is_native_draft);
    if !job.spec.is_draft
        || job.provider_kind != ProviderKind::Account
        || job.state != JobState::Done
        || !native
    {
        return None;
    }
    job.charged_at
        .or(job.finished_at)
        .map(|completed| completed + FINALIZE_WINDOW)
}

/// Finalize in 1080p: the draft's final made by Higgsfield from the draft itself (`JobSpec::draft_job_id`). `None`
/// unless this is a native draft the signed-in account (`owner`) made and its seven days haven't passed at `now`.
pub fn finalize_spec(job: &Job, owner: Option<&str>, now: OffsetDateTime) -> Option<JobSpec> {
    let deadline = finalize_deadline(job)?;
    let owner = owner?;
    if now >= deadline || job.owner.as_deref() != Some(owner) {
        return None;
    }
    let draft_job_id = job.remote.as_ref()?.request_id.clone();
    let mut spec = job.spec.clone();
    spec.resolution = Resolution::P1080;
    spec.is_draft = false;
    spec.variants = 1;
    spec.uses_unlimited = false;
    spec.draft_job_id = Some(draft_job_id);
    Some(spec)
}

/// "Finalize in 1080p until Oct 22, 2026 at 14:05", or "The 1080p finalize ended Oct 22, 2026 at 14:05" once
/// `deadline` is past at `now`.
pub fn finalize_window(deadline: OffsetDateTime, now: OffsetDateTime, offset: UtcOffset) -> String {
    let date = crate::settings::spending::date_at_time(deadline, offset);
    if deadline > now {
        format!("Finalize in 1080p until {date}")
    } else {
        format!("The 1080p finalize ended {date}")
    }
}

/// Only drafts get a final render, once they are in the library.
pub fn can_render_final(item: &LibraryItem) -> bool {
    item.spec().is_draft && item.history_item.is_none()
}

/// What a final can be rendered at. A Kling draft is Standard, and a Standard final would cost the same as the
/// draft, so Pro is the only step up.
pub fn final_resolutions(item: &LibraryItem) -> Vec<Resolution> {
    if item.spec().model.offers_draft() {
        vec![Resolution::P720, Resolution::P1080]
    } else {
        vec![KlingQuality::Pro.resolution()]
    }
}

/// The same inputs at `resolution`, as a final; a resolution the item doesn't offer becomes the highest it does.
pub fn final_spec_at(item: &LibraryItem, resolution: Resolution) -> JobSpec {
    let options = final_resolutions(item);
    let mut spec = item.spec().clone();
    spec.resolution = if options.contains(&resolution) {
        resolution
    } else {
        options.last().copied().unwrap_or(resolution)
    };
    spec.is_draft = false;
    spec
}

/// The final at `resolution`: at 1080p, Higgsfield's finalize of this native draft while the signed-in account can
/// run it, else the same inputs rendered again.
pub fn final_spec(
    item: &LibraryItem,
    resolution: Resolution,
    owner: Option<&str>,
    now: OffsetDateTime,
) -> JobSpec {
    if resolution == Resolution::P1080
        && let Some(finalize) = finalize_spec(&item.job, owner, now)
    {
        return finalize;
    }
    final_spec_at(item, resolution)
}

/// List price of the final for an API key; `None` for an account, whose credits only the provider can estimate.
pub fn final_price(
    item: &LibraryItem,
    resolution: Resolution,
    pricing: Option<&Pricing>,
    connection: Option<ProviderKind>,
) -> Option<Cost> {
    if connection != Some(ProviderKind::ApiKey) {
        return None;
    }
    pricing?.cost(&final_spec_at(item, resolution)).ok()
}

/// One final to queue and the draft it is made from.
#[derive(Debug, Clone, PartialEq)]
pub struct Final {
    pub spec: JobSpec,
    /// The draft's ID: the final's `Job::parent_id`.
    pub parent_id: Uuid,
}

/// Render finals for the starred drafts of a batch: one resolution for all, one price and one cap check for the
/// total.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchFinals {
    /// The starred drafts, in batch order.
    pub drafts: Vec<LibraryItem>,
}

impl BatchFinals {
    pub fn new(items: &[LibraryItem]) -> Self {
        Self {
            drafts: items
                .iter()
                .filter(|item| item.is_starred && can_render_final(item))
                .cloned()
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.drafts.is_empty()
    }

    /// What the one resolution choice offers.
    pub fn resolutions(&self) -> Vec<Resolution> {
        self.drafts.first().map(final_resolutions).unwrap_or_default()
    }

    /// Each draft's final at `resolution`, leaving out a final already in the queue.
    pub fn finals(
        &self,
        resolution: Resolution,
        owner: Option<&str>,
        now: OffsetDateTime,
        is_queued: impl Fn(&JobSpec, Uuid) -> bool,
    ) -> Vec<Final> {
        self.drafts
            .iter()
            .map(|draft| Final {
                spec: final_spec(draft, resolution, owner, now),
                parent_id: draft.id(),
            })
            .filter(|final_| !is_queued(&final_.spec, final_.parent_id))
            .collect()
    }

    /// The finals' combined price: `None` when any is unknown or there are none.
    pub fn total(prices: &[Option<Cost>]) -> Option<Cost> {
        let known: Vec<Cost> = prices.iter().flatten().cloned().collect();
        if known.len() == prices.len() {
            format::total(&known)
        } else {
            None
        }
    }

    /// The combined list price for an API key; `None` for an account.
    pub fn list_price(
        finals: &[Final],
        pricing: Option<&Pricing>,
        connection: Option<ProviderKind>,
    ) -> Option<Cost> {
        if connection != Some(ProviderKind::ApiKey) {
            return None;
        }
        let pricing = pricing?;
        let prices: Vec<Option<Cost>> = finals
            .iter()
            .map(|final_| pricing.cost(&final_.spec).ok())
            .collect();
        Self::total(&prices)
    }

    /// The button: "Render 3 finals".
    pub fn title(count: usize) -> String {
        format!("Render {count} finals")
    }
}

/// One resolution of the final's picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionOption {
    pub value: Resolution,
    /// "720p", or "Pro" for Kling.
    pub title: String,
}

/// The options of a picker, titled after the model.
pub fn resolution_options(resolutions: &[Resolution], item: &LibraryItem) -> Vec<ResolutionOption> {
    resolutions
        .iter()
        .map(|resolution| ResolutionOption {
            value: *resolution,
            title: format::resolution_title(*resolution, &item.spec().model),
        })
        .collect()
}

/// The "Render final" box on a result's page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FinalSection {
    /// The picker's options; one option needs no picker, `detail` names it.
    pub options: Vec<ResolutionOption>,
    pub selected: Resolution,
    pub detail: Option<String>,
    /// The seven-day finalize window of a native draft.
    pub window: Option<String>,
    /// "Render final" or "Finalize in 1080p".
    pub button_title: String,
    pub help: Option<String>,
    /// The price on the button; `None` while not known.
    pub price: Option<String>,
    /// The final at this resolution is in the queue: rendering it again would charge twice.
    pub is_queued: bool,
    pub is_connected: bool,
}

/// Render finals for starred: the sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FinalsSheet {
    /// The batch's title.
    pub subtitle: String,
    pub options: Vec<ResolutionOption>,
    pub selected: Resolution,
    pub detail: Option<String>,
    /// How many finals the button queues; 0 offers View in Queue.
    pub count: i32,
    /// "Render 3 finals".
    pub button_title: String,
    pub price: Option<String>,
    /// "Leaves out 2 already in the Queue."
    pub queued_note: Option<String>,
    pub is_connected: bool,
}

#[cfg(test)]
mod tests {
    use jutsu_core::{HistoryItem, Mode, RemoteInputs, RemoteJob, VideoModel};

    use super::*;
    use crate::library::fixture::{dec, history_url, now};

    fn day() -> Duration {
        Duration::days(1)
    }

    /// A native draft the account `owner-1` made, completed `days_ago` days before `now`.
    fn draft(days_ago: f64, kind: ProviderKind, owner: &str, is_native: bool) -> Job {
        let mut spec = JobSpec::new(
            Mode::MotionTransfer,
            VideoModel::SEEDANCE25,
            None,
            Resolution::P480,
        );
        spec.prompt = "a fox".to_owned();
        spec.is_draft = true;
        spec.variants = 1;
        let mut job = Job::new(
            Uuid::new_v4(),
            spec,
            kind,
            Cost::credits(dec("45"), false),
            Uuid::new_v4(),
            now() - day() * 10,
        );
        job.owner = Some(owner.to_owned());
        job.state = JobState::Done;
        let mut remote = RemoteJob::new("draft-job".to_owned());
        remote.is_native_draft = is_native;
        job.remote = Some(remote);
        job.charged_at = Some(now() - Duration::seconds_f64(days_ago * 86_400.0));
        job.finished_at = Some(now() - Duration::seconds_f64(days_ago * 86_400.0 - 60.0));
        job
    }

    fn native(days_ago: f64) -> Job {
        draft(days_ago, ProviderKind::Account, "owner-1", true)
    }

    /// `FinalizeTests.theWindowIsSevenDaysFromCompletion`.
    #[test]
    fn the_window_is_seven_days_from_completion() {
        let job = native(2.0);
        assert_eq!(finalize_deadline(&job), Some(now() + day() * 5));
        let mut unknown = job.clone();
        unknown.charged_at = None;
        assert_eq!(
            finalize_deadline(&unknown),
            job.finished_at.map(|date| date + day() * 7)
        );
    }

    /// `FinalizeTests.theWindowShowsItsEnd`.
    #[test]
    fn the_window_shows_its_end() {
        let deadline = finalize_deadline(&native(2.0)).unwrap();
        assert!(finalize_window(deadline, now(), UtcOffset::UTC).starts_with("Finalize in 1080p until"));
        assert!(
            finalize_window(deadline, deadline + Duration::seconds(1), UtcOffset::UTC)
                .starts_with("The 1080p finalize ended")
        );
    }

    /// `FinalizeTests.aDraftOfTheAccountWithinSevenDaysIsOffered`.
    #[test]
    fn a_draft_of_the_account_within_seven_days_is_offered() {
        let spec = finalize_spec(&native(6.9), Some("owner-1"), now()).unwrap();
        assert_eq!(spec.draft_job_id.as_deref(), Some("draft-job"));
        assert_eq!(spec.resolution, Resolution::P1080);
        assert!(!spec.is_draft);
        assert_eq!(spec.prompt, "a fox");
    }

    /// `FinalizeTests.otherDraftsAreNotOffered`.
    #[test]
    fn other_drafts_are_not_offered() {
        let cases = [
            (native(7.1), Some("owner-1")),
            (native(2.0), Some("owner-2")),
            (native(2.0), None),
            (draft(2.0, ProviderKind::ApiKey, "owner-1", true), Some("owner-1")),
            (
                draft(2.0, ProviderKind::Account, "owner-1", false),
                Some("owner-1"),
            ),
        ];
        for (job, owner) in cases {
            assert_eq!(finalize_spec(&job, owner, now()), None);
        }
    }

    /// `FinalizeTests.aFinishedFinalIsNoDraftToFinalize`.
    #[test]
    fn a_finished_final_is_no_draft_to_finalize() {
        let mut final_ = native(2.0);
        final_.spec.is_draft = false;
        assert_eq!(finalize_deadline(&final_), None);
    }

    /// `FinalizeTests.theFinalAt1080pIsTheFinalize`.
    #[test]
    fn the_final_at_1080p_is_the_finalize() {
        let item = LibraryItem::new(native(2.0), true);
        let owner = Some("owner-1");
        assert_eq!(
            final_spec(&item, Resolution::P1080, owner, now())
                .draft_job_id
                .as_deref(),
            Some("draft-job")
        );
        assert_eq!(
            final_spec(&item, Resolution::P720, owner, now()),
            final_spec_at(&item, Resolution::P720)
        );
        assert_eq!(
            final_spec(&item, Resolution::P1080, Some("owner-2"), now()),
            final_spec_at(&item, Resolution::P1080)
        );
        assert_eq!(final_spec_at(&item, Resolution::P1080).draft_job_id, None);

        let finals =
            BatchFinals::new(std::slice::from_ref(&item))
                .finals(Resolution::P1080, owner, now(), |_, _| false);
        assert_eq!(
            finals
                .iter()
                .map(|final_| final_.spec.draft_job_id.clone())
                .collect::<Vec<_>>(),
            [Some("draft-job".to_owned())]
        );
        assert_eq!(
            finals.iter().map(|final_| final_.parent_id).collect::<Vec<_>>(),
            [item.id()]
        );
    }

    /// `FinalizeTests.aDraftFromHistoryIsOfferedWithinItsSevenDays`.
    #[test]
    fn a_draft_from_history_is_offered_within_its_seven_days() {
        let mut history = HistoryItem::new(
            "draft-job",
            Mode::MotionTransfer,
            VideoModel::SEEDANCE25,
            now() - day(),
            history_url("draft.mp4"),
            RemoteInputs {
                video: None,
                images: Vec::new(),
                audio: None,
            },
        );
        history.duration = Some(15.0);
        history.resolution = Some(Resolution::P480);
        history.prompt = "a fox".to_owned();
        history.is_native_draft = true;
        let job = Job::importing(&history, Some("owner-1".to_owned()), now());
        assert_eq!(
            finalize_deadline(&job),
            Some(history.created_at + FINALIZE_WINDOW)
        );
        assert_eq!(
            finalize_spec(&job, Some("owner-1"), now()).and_then(|spec| spec.draft_job_id),
            Some("draft-job".to_owned())
        );
        assert_eq!(finalize_spec(&job, Some("owner-1"), now() + day() * 7), None);
        assert!(!can_render_final(&LibraryItem::from_history(history)));
        assert!(can_render_final(&LibraryItem::new(job, false)));
    }

    /// A Kling draft's final is Pro only; another model offers 720p and 1080p, and an unoffered one becomes the top.
    #[test]
    fn the_final_resolutions_follow_the_model() {
        let item = LibraryItem::new(native(2.0), false);
        assert_eq!(final_resolutions(&item), [Resolution::P720, Resolution::P1080]);
        assert_eq!(
            final_spec_at(&item, Resolution::P480).resolution,
            Resolution::P1080
        );
        let mut kling = item.clone();
        kling.job.spec.model = VideoModel::KLING_MOTION_CONTROL;
        assert!(!kling.spec().model.offers_draft());
        assert_eq!(final_resolutions(&kling), [Resolution::P1080]);
    }

    /// `BatchFinals`: starred drafts only, a queued final left out, the price only when every part is known.
    #[test]
    fn batch_finals_take_the_starred_drafts() {
        let starred = LibraryItem::new(native(2.0), true);
        let plain = LibraryItem::new(native(2.0), false);
        let finals = BatchFinals::new(&[starred.clone(), plain]);
        assert_eq!(finals.drafts.len(), 1);
        assert_eq!(
            finals
                .finals(Resolution::P720, None, now(), |_, parent| parent == starred.id())
                .len(),
            0
        );
        assert_eq!(BatchFinals::title(3), "Render 3 finals");
        let four = Cost::credits(dec("4"), false);
        assert_eq!(
            BatchFinals::total(&[Some(four.clone()), Some(four.clone())]),
            Some(Cost::credits(dec("8"), false))
        );
        assert_eq!(BatchFinals::total(&[Some(four), None]), None);
        assert_eq!(BatchFinals::total(&[]), None);
    }
}
