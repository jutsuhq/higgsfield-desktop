//! Score Variants in a batch comparison (`VariantScoring.swift`): who can be scored, the price and its question
//! under the cap, and how a score reads on a cell.

use jutsu_core::{Budget, BudgetDecision, Cost, JobState, ProviderKind, SpendCap, ViralityScore};
use rust_decimal::Decimal;
use serde::Serialize;
use uuid::Uuid;

use super::item::LibraryItem;
use crate::queue::QueueConfirmation;
use crate::queue::format;

/// An operation Higgsfield gives no quote for before it runs (`PostOperationText.unknownPriceNotice`).
pub const UNKNOWN_PRICE_NOTICE: &str =
    "Price unknown — charged on completion. Higgsfield gives no quote for this before it runs.";
/// The agreement to an unknown price (`PostOperationText.unknownPriceConsent`).
pub const UNKNOWN_PRICE_CONSENT: &str = "Charge whatever Higgsfield bills when it finishes";
/// `PostOperationText.waitingForCatalog`.
pub const WAITING_FOR_CATALOG: &str = "Waiting for the account's tool catalog to be confirmed";
/// What the sheet says before a re-score.
pub const RESCORE_NOTICE: &str =
    "Every variant already has a score. Scoring them again replaces those scores.";
/// The model a score's charge is logged under in the image spend log.
pub const MODEL_TITLE: &str = "Virality Predictor";
/// A score answer without a reading.
pub const UNREADABLE: &str =
    "Higgsfield's answer couldn't be read, so no score was kept. It may have been charged.";

/// Whether the comparison shows Score Variants, and whether it can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ViralityAvailability {
    Hidden,
    /// Shown, off for `reason`.
    Disabled {
        reason: String,
    },
    Available,
}

impl ViralityAvailability {
    /// - `has_scorer`: the active connection scores virality; `false` while disconnected or for an API key.
    /// - `offered`: the account's scoring call is confirmed.
    pub fn of(has_scorer: bool, offered: bool) -> Self {
        if !has_scorer {
            Self::Hidden
        } else if offered {
            Self::Available
        } else {
            Self::Disabled {
                reason: WAITING_FOR_CATALOG.to_owned(),
            }
        }
    }
}

/// Why `item` can't be scored by the signed-in account (`owner`); `None` when it can.
pub fn virality_blocker(item: &LibraryItem, owner: Option<&str>) -> Option<&'static str> {
    if item.job.state != JobState::Done || item.output_file().is_none() {
        return Some("Can't be scored: not finished");
    }
    if item.job.provider_kind != ProviderKind::Account {
        return Some("Can't be scored: made with an API key");
    }
    if item.job.owner.as_deref() != owner {
        return Some("Can't be scored: made by another account");
    }
    if item
        .job
        .remote
        .as_ref()
        .is_none_or(|remote| remote.request_id.is_empty())
    {
        return Some("Can't be scored: no account job ID");
    }
    None
}

/// The account job ID the result is scored by; `None` when it can't be.
pub fn virality_job_id(item: &LibraryItem, owner: Option<&str>) -> Option<String> {
    if virality_blocker(item, owner).is_some() {
        return None;
    }
    item.job.remote.as_ref().map(|remote| remote.request_id.clone())
}

/// One result to score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViralityEntry {
    pub item_id: Uuid,
    pub job_id: String,
}

/// Which results Score Variants scores: the ones not scored yet, or, once every one is, all of them again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViralityPlan {
    pub entries: Vec<ViralityEntry>,
    /// Every scorable result already has a score: scoring them again is paid again.
    pub is_rescore: bool,
}

impl ViralityPlan {
    pub fn new(items: &[LibraryItem], owner: Option<&str>) -> Self {
        let scorable: Vec<(&LibraryItem, ViralityEntry)> = items
            .iter()
            .filter_map(|item| {
                virality_job_id(item, owner).map(|job_id| {
                    (
                        item,
                        ViralityEntry {
                            item_id: item.id(),
                            job_id,
                        },
                    )
                })
            })
            .collect();
        let unscored: Vec<ViralityEntry> = scorable
            .iter()
            .filter(|(item, _)| item.job.virality.is_none())
            .map(|(_, entry)| entry.clone())
            .collect();
        let is_rescore = unscored.is_empty() && !scorable.is_empty();
        let entries = if is_rescore {
            scorable.into_iter().map(|(_, entry)| entry).collect()
        } else {
            unscored
        };
        Self { entries, is_rescore }
    }

    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// The toolbar button.
    pub fn button_title(&self) -> &'static str {
        if self.is_rescore {
            "Score Again…"
        } else {
            "Score Variants…"
        }
    }

    /// Why there is nothing to score; `None` when there is.
    pub fn blocker(&self) -> Option<&'static str> {
        self.entries
            .is_empty()
            .then_some("None of these results can be scored")
    }

    /// The price of the whole plan from the price of one result: `Cost::UNKNOWN_CREDITS` when that isn't known.
    pub fn total(&self, each: Option<&Cost>) -> Cost {
        let count = Decimal::from(self.count());
        match each {
            Some(each) if !each.is_unknown_price() => each.with_amount(each.amount() * count),
            _ => Cost::UNKNOWN_CREDITS,
        }
    }

    /// A price of nothing, as the account's Virality Predictor has.
    pub fn is_free(total: &Cost) -> bool {
        total.amount().is_zero() && !total.is_unknown_price()
    }

    /// The price on the Score button: "12 cr", or "Free".
    pub fn price_tag(total: &Cost) -> String {
        if Self::is_free(total) {
            "Free".to_owned()
        } else {
            format::cost_text(total)
        }
    }

    /// "3 videos · 12 cr · Price from Higgsfield", "3 videos · Free", or the unknown price.
    pub fn price_text(&self, total: &Cost) -> String {
        let videos = if self.count() == 1 {
            "1 video".to_owned()
        } else {
            format!("{} videos", self.count())
        };
        if *total == Cost::UNKNOWN_CREDITS {
            return format!("{videos} · {UNKNOWN_PRICE_NOTICE}");
        }
        if Self::is_free(total) {
            return format!("{videos} · Free");
        }
        let amount = format::cost_text(total);
        if total.is_estimate() {
            format!("{videos} · About {amount} · Estimate")
        } else {
            format!("{videos} · {amount} · Price from Higgsfield")
        }
    }

    /// The question to ask before scoring when the price takes today past the cap, or isn't known under one; `None`
    /// to go ahead, always for a free run.
    pub fn cap_confirmation(
        total: &Cost,
        committed_today: Decimal,
        cap: Option<&SpendCap>,
    ) -> Option<QueueConfirmation> {
        let cap = cap?;
        if Self::is_free(total) {
            return None;
        }
        let cap_text = format::amount(cap.amount, cap.unit);
        if *total == Cost::UNKNOWN_CREDITS {
            let today = format::amount(committed_today, cap.unit);
            return Some(QueueConfirmation {
                title: "Price not known yet".to_owned(),
                message: format!(
                    "These scores' price isn't known, so they may take today past your {cap_text} cap. Today is at {today} so far."
                ),
                confirm_title: "Score anyway".to_owned(),
            });
        }
        let BudgetDecision::NeedsConfirmation { total } = Budget::allows(total, committed_today, Some(cap))
        else {
            return None;
        };
        Some(QueueConfirmation {
            title: "Over today's spend cap".to_owned(),
            message: format!(
                "These scores bring today to {} against your {cap_text} cap.",
                format::amount(total, cap.unit)
            ),
            confirm_title: "Score anyway".to_owned(),
        })
    }
}

/// The badge, "72"; `None` when only a report came back.
pub fn badge_text(score: &ViralityScore) -> Option<String> {
    score.score.map(|score| score.to_string())
}

/// The readings the answer named, one per line: "Viral potential: 45", "Hook (0–3 s): 29", "Peak at 12 s".
pub fn details(score: &ViralityScore) -> Vec<String> {
    [
        score
            .viral_potential
            .map(|value| format!("Viral potential: {value}")),
        score.hook.map(|value| format!("Hook (0–3 s): {value}")),
        score.engagement.map(|value| format!("Engagement: {value}")),
        score.sustain.map(|value| format!("Sustain: {value}")),
        score
            .peak_second
            .map(|value| format!("Peak at {}", seconds(value))),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// "Virality score 72 of 100", or the report alone.
pub fn headline(score: &ViralityScore) -> String {
    score.score.map_or_else(
        || "Virality report on Higgsfield".to_owned(),
        |value| format!("Virality score {value} of 100"),
    )
}

/// The badge's hover text: the score out of 100, the readings, then the disclaimer.
pub fn help(score: &ViralityScore) -> String {
    let mut lines = vec![headline(score)];
    lines.extend(details(score));
    lines.extend(score.disclaimer.clone());
    lines.join("\n")
}

/// What a screen reader reads for the badge: everything the hover shows.
pub fn accessibility_label(score: &ViralityScore) -> String {
    help(score).replace('\n', ", ")
}

/// The per-frame readings scaled to 0–1 between their lowest and highest, for a small line; empty with fewer than
/// two, or when they never change.
pub fn curve(score: &ViralityScore) -> Vec<f64> {
    let Some(values) = score.per_frame.as_ref().filter(|values| values.len() > 1) else {
        return Vec::new();
    };
    let low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if high <= low {
        return Vec::new();
    }
    values.iter().map(|value| (value - low) / (high - low)).collect()
}

/// "12 s", "2.5 s".
fn seconds(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0} s")
    } else {
        format!("{rounded:.1} s")
    }
}

/// A cell's score as the window shows it.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ViralityView {
    /// The call for this result is running or waiting its turn.
    pub is_scoring: bool,
    /// "72"; `None` when only a report came back, or with no score.
    pub badge: Option<String>,
    /// The badge's hover text and the popover's lines.
    pub help: Option<String>,
    pub accessibility_label: Option<String>,
    pub headline: Option<String>,
    pub details: Vec<String>,
    pub disclaimer: Option<String>,
    /// The per-frame line, 0–1.
    pub curve: Vec<f64>,
    pub report_url: Option<String>,
    /// Why the last scoring of this result gave nothing.
    pub failure: Option<String>,
    /// Why it can't be scored.
    pub blocker: Option<String>,
}

impl ViralityView {
    pub fn new(item: &LibraryItem, owner: Option<&str>, is_scoring: bool, failure: Option<String>) -> Self {
        let score = item.job.virality.as_ref();
        Self {
            is_scoring,
            badge: score.and_then(badge_text),
            help: score.map(help),
            accessibility_label: score.map(accessibility_label),
            headline: score.map(headline),
            details: score.map(details).unwrap_or_default(),
            disclaimer: score.and_then(|score| score.disclaimer.clone()),
            curve: score.map(curve).unwrap_or_default(),
            report_url: score
                .and_then(|score| score.report_url.as_ref())
                .map(ToString::to_string),
            failure: failure.filter(|_| score.is_none()),
            blocker: virality_blocker(item, owner)
                .filter(|_| score.is_none())
                .map(str::to_owned),
        }
    }
}

#[cfg(test)]
mod tests {
    use jutsu_core::RemoteJob;
    use time::OffsetDateTime;

    use super::*;
    use crate::library::fixture::{dec, file, item};

    const OWNER: &str = "user_1";
    const DISCLAIMER: &str = "Predictive proxy metrics, not guaranteed performance or clinical measures.";

    fn scored_at() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(2_000_000).unwrap()
    }

    fn empty_score() -> ViralityScore {
        ViralityScore {
            score: None,
            viral_potential: None,
            hook: None,
            engagement: None,
            sustain: None,
            peak_second: None,
            per_frame: None,
            disclaimer: None,
            report_url: None,
            scored_at: scored_at(),
        }
    }

    /// The readings of a live run on 2026-10-15.
    fn score() -> ViralityScore {
        ViralityScore {
            score: Some(46),
            viral_potential: Some(45),
            hook: Some(29),
            engagement: Some(39),
            sustain: Some(100),
            peak_second: Some(12.0),
            per_frame: Some(vec![0.2, 0.42, 0.3]),
            disclaimer: Some(DISCLAIMER.to_owned()),
            report_url: Some("https://cdn.example/virality/job.html".parse().unwrap()),
            scored_at: scored_at(),
        }
    }

    /// A finished account result of `owner` with the job ID `request_id`.
    fn scorable(
        request_id: Option<&str>,
        kind: ProviderKind,
        owner: Option<&str>,
        state: JobState,
        virality: Option<ViralityScore>,
    ) -> LibraryItem {
        let mut item = item();
        item.job.provider_kind = kind;
        item.job.owner = owner.map(str::to_owned);
        item.job.state = state;
        item.job.remote = request_id.map(|id| RemoteJob::new(id.to_owned()));
        item.job.output_file = Some(file(&format!("/out/{}.mp4", request_id.unwrap_or("none"))));
        item.job.virality = virality;
        item
    }

    fn account(request_id: &str) -> LibraryItem {
        scorable(
            Some(request_id),
            ProviderKind::Account,
            Some(OWNER),
            JobState::Done,
            None,
        )
    }

    /// `VariantScoringTests.shownOnlyWhereTheConnectionScoresAndOffWhileUnconfirmed`.
    #[test]
    fn shown_only_where_the_connection_scores_and_off_while_unconfirmed() {
        assert_eq!(
            ViralityAvailability::of(false, true),
            ViralityAvailability::Hidden
        );
        assert_eq!(
            ViralityAvailability::of(true, false),
            ViralityAvailability::Disabled {
                reason: "Waiting for the account's tool catalog to be confirmed".to_owned()
            }
        );
        assert_eq!(
            ViralityAvailability::of(true, true),
            ViralityAvailability::Available
        );
    }

    /// `VariantScoringTests.onlyFinishedResultsOfThisAccountWithAJobIDCanBeScored`.
    #[test]
    fn only_finished_results_of_this_account_with_a_job_id_can_be_scored() {
        let owner = Some(OWNER);
        assert_eq!(
            virality_job_id(&account("job-1"), owner).as_deref(),
            Some("job-1")
        );
        let api = scorable(Some("job-1"), ProviderKind::ApiKey, owner, JobState::Done, None);
        assert_eq!(
            virality_blocker(&api, owner),
            Some("Can't be scored: made with an API key")
        );
        let other = scorable(
            Some("job-1"),
            ProviderKind::Account,
            Some("user_2"),
            JobState::Done,
            None,
        );
        assert_eq!(
            virality_blocker(&other, owner),
            Some("Can't be scored: made by another account")
        );
        let none = scorable(None, ProviderKind::Account, owner, JobState::Done, None);
        assert_eq!(
            virality_blocker(&none, owner),
            Some("Can't be scored: no account job ID")
        );
        let failed = scorable(
            Some("job-1"),
            ProviderKind::Account,
            owner,
            JobState::Failed {
                reason: jutsu_core::FailureReason::OutcomeUnknown,
            },
            None,
        );
        assert_eq!(
            virality_blocker(&failed, owner),
            Some("Can't be scored: not finished")
        );
        assert_eq!(virality_job_id(&api, owner), None);
    }

    /// `VariantScoringTests.thePlanSkipsScoredResultsAndRescoresOnlyWhenAllAre`.
    #[test]
    fn the_plan_skips_scored_results_and_rescores_only_when_all_are() {
        let owner = Some(OWNER);
        let scored = scorable(
            Some("job-1"),
            ProviderKind::Account,
            owner,
            JobState::Done,
            Some(score()),
        );
        let fresh = account("job-2");
        let api = scorable(Some("job-3"), ProviderKind::ApiKey, owner, JobState::Done, None);
        let jobs = |plan: &ViralityPlan| {
            plan.entries
                .iter()
                .map(|entry| entry.job_id.clone())
                .collect::<Vec<_>>()
        };

        let plan = ViralityPlan::new(&[scored.clone(), fresh, api.clone()], owner);
        assert_eq!(jobs(&plan), ["job-2"]);
        assert!(!plan.is_rescore);
        assert_eq!(plan.button_title(), "Score Variants…");

        let again = ViralityPlan::new(&[scored, api.clone()], owner);
        assert_eq!(jobs(&again), ["job-1"]);
        assert!(again.is_rescore);
        assert_eq!(again.button_title(), "Score Again…");

        let none = ViralityPlan::new(&[api], owner);
        assert_eq!(none.blocker(), Some("None of these results can be scored"));
        assert!(!none.is_rescore);
    }

    /// `VariantScoringTests.thePriceIsForEveryVideoOrUnknown`.
    #[test]
    fn the_price_is_for_every_video_or_unknown() {
        let plan = ViralityPlan::new(
            &[account("job-1"), account("job-2"), account("job-3")],
            Some(OWNER),
        );
        let quoted = plan.total(Some(&Cost::credits(dec("4"), false)));
        assert_eq!(quoted, Cost::credits(dec("12"), false));
        assert_eq!(
            plan.price_text(&quoted),
            "3 videos · 12 cr · Price from Higgsfield"
        );
        assert_eq!(plan.total(None), Cost::UNKNOWN_CREDITS);
        assert_eq!(plan.total(Some(&Cost::UNKNOWN_CREDITS)), Cost::UNKNOWN_CREDITS);
        assert_eq!(
            plan.price_text(&Cost::UNKNOWN_CREDITS),
            "3 videos · Price unknown — charged on completion. Higgsfield gives no quote for this before it runs."
        );
        let one = ViralityPlan::new(&[account("job-1")], Some(OWNER));
        assert_eq!(
            one.price_text(&Cost::credits(dec("4"), true)),
            "1 video · About 4 cr · Estimate"
        );
    }

    /// `VariantScoringTests.aFreeRunSaysFreeAndNeverAsksAboutTheCap`.
    #[test]
    fn a_free_run_says_free_and_never_asks_about_the_cap() {
        let plan = ViralityPlan::new(&[account("job-1"), account("job-2")], Some(OWNER));
        let free = plan.total(Some(&Cost::credits(Decimal::ZERO, false)));
        assert!(ViralityPlan::is_free(&free));
        assert!(!ViralityPlan::is_free(&Cost::UNKNOWN_CREDITS));
        assert_eq!(plan.price_text(&free), "2 videos · Free");
        assert_eq!(ViralityPlan::price_tag(&free), "Free");
        assert_eq!(ViralityPlan::price_tag(&Cost::credits(dec("4"), false)), "4 cr");
        let cap = SpendCap::credits(100);
        assert!(ViralityPlan::cap_confirmation(&free, dec("150"), Some(&cap)).is_none());
    }

    /// `VariantScoringTests.underACapAnUnknownOrTooHighPriceAsks`.
    #[test]
    fn under_a_cap_an_unknown_or_too_high_price_asks() {
        let cap = SpendCap::credits(100);
        assert!(ViralityPlan::cap_confirmation(&Cost::UNKNOWN_CREDITS, dec("10"), None).is_none());
        assert_eq!(
            ViralityPlan::cap_confirmation(&Cost::UNKNOWN_CREDITS, dec("10"), Some(&cap))
                .map(|question| question.title),
            Some("Price not known yet".to_owned())
        );
        let twelve = Cost::credits(dec("12"), false);
        assert!(ViralityPlan::cap_confirmation(&twelve, dec("10"), Some(&cap)).is_none());
        let over = ViralityPlan::cap_confirmation(&twelve, dec("95"), Some(&cap)).unwrap();
        assert_eq!(over.title, "Over today's spend cap");
        assert_eq!(over.confirm_title, "Score anyway");
    }

    /// `VariantScoringTests.theBadgeAndItsHoverNameTheReadings`.
    #[test]
    fn the_badge_and_its_hover_name_the_readings() {
        let score = score();
        assert_eq!(badge_text(&score).as_deref(), Some("46"));
        assert_eq!(
            details(&score),
            [
                "Viral potential: 45",
                "Hook (0–3 s): 29",
                "Engagement: 39",
                "Sustain: 100",
                "Peak at 12 s"
            ]
        );
        assert_eq!(
            help(&score),
            format!(
                "Virality score 46 of 100\nViral potential: 45\nHook (0–3 s): 29\nEngagement: 39\nSustain: 100\nPeak at 12 s\n{DISCLAIMER}"
            )
        );
        assert!(accessibility_label(&score).starts_with("Virality score 46 of 100, Viral potential: 45, "));
        let report = ViralityScore {
            report_url: Some("https://cdn.example/r.html".parse().unwrap()),
            ..empty_score()
        };
        assert_eq!(badge_text(&report), None);
        assert_eq!(help(&report), "Virality report on Higgsfield");
        assert_eq!(seconds(2.5), "2.5 s");
    }

    /// `VariantScoringTests.theLineSpansTheReadingsLowestToHighest`.
    #[test]
    fn the_line_spans_the_readings_lowest_to_highest() {
        let points: Vec<f64> = curve(&score())
            .iter()
            .map(|value| (value * 100.0).round())
            .collect();
        assert_eq!(points, [0.0, 100.0, 45.0]);
        let one = ViralityScore {
            per_frame: Some(vec![0.3]),
            ..empty_score()
        };
        assert_eq!(curve(&one), Vec::<f64>::new());
        let flat = ViralityScore {
            per_frame: Some(vec![0.3, 0.3]),
            ..empty_score()
        };
        assert_eq!(curve(&flat), Vec::<f64>::new());
        assert_eq!(curve(&empty_score()), Vec::<f64>::new());
    }

    /// The cell shows the score, else the failure, else why it can't be scored.
    #[test]
    fn the_view_shows_the_score_or_why_there_is_none() {
        let owner = Some(OWNER);
        let scored = scorable(
            Some("job-1"),
            ProviderKind::Account,
            owner,
            JobState::Done,
            Some(score()),
        );
        let view = ViralityView::new(&scored, owner, false, Some("Too long".to_owned()));
        assert_eq!(view.badge.as_deref(), Some("46"));
        assert_eq!(view.failure, None);
        let api = scorable(Some("job-3"), ProviderKind::ApiKey, owner, JobState::Done, None);
        let view = ViralityView::new(&api, owner, false, None);
        assert_eq!(
            view.blocker.as_deref(),
            Some("Can't be scored: made with an API key")
        );
        let failed = ViralityView::new(&account("job-2"), owner, false, Some("Too long".to_owned()));
        assert_eq!(failed.failure.as_deref(), Some("Too long"));
    }
}
