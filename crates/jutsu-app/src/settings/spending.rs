//! Settings › Spending (`Spending.swift`, `SpendingSettingsView`, `CommittedSpendKey`): today's committed spend
//! toward the daily cap, the spending summary of a period, its CSV, and the cap's own rules.

use jutsu_core::{
    Cost, CostUnit, CreditEntry, CreditEntryKind, ImageCharge, ImageChargePurpose, Job, JobState, Mode,
    Pricing, Resolution,
};
use jutsu_store::Store;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime, Time, UtcOffset};

use crate::Preferences;
use crate::connection::{UserMessage, credits_text, mode_title};
use crate::create::paged_feed::FeedView;
use crate::queue::format;

use crate::images::spend_log::ImageSpendLog;

/// Why today's spend can't be read: nothing is queued at an unknown spend.
pub const SPENDING_UNAVAILABLE_TITLE: &str = "Can't read today's spending";

/// Today's spend toward a cap in `unit` (`AppEnvironment.committedToday`): every job finished today or still in
/// the queue, plus the image charges (reference images, character sheets, virality scores). The error says why it
/// can't be read; a spend log that couldn't be read counts as unreadable, never as nothing.
pub async fn committed_today(
    store: &Store,
    images: &ImageSpendLog,
    unit: CostUnit,
    now: OffsetDateTime,
    offset: UtcOffset,
) -> Result<Decimal, String> {
    committed_today_excluding(store, images, unit, now, offset, None)
        .await
        .map_err(|error| error.to_string())
}

/// `committed_today` with `excluded` left out of the sum, e.g. the job whose own price is being checked. The one
/// place that computes today's committed spend: every reader of it goes through here.
pub async fn committed_today_excluding(
    store: &Store,
    images: &ImageSpendLog,
    unit: CostUnit,
    now: OffsetDateTime,
    offset: UtcOffset,
    excluded: Option<uuid::Uuid>,
) -> Result<Decimal, jutsu_core::BudgetError> {
    let images = images.committed(now, unit, offset)?;
    let jobs = store
        .committed_spend(now, unit, excluded, offset)
        .await
        .map_err(|error| {
            tracing::error!(%error, "can't read today's spend");
            jutsu_core::BudgetError::SpendingUnavailable
        })?;
    Ok(jobs.saturating_add(images))
}

// MARK: Summary

/// The period the summary covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SpendingPeriod {
    Today,
    Week,
    Month,
}

impl SpendingPeriod {
    pub const ALL: [Self; 3] = [Self::Today, Self::Week, Self::Month];

    pub fn title(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Week => "7 days",
            Self::Month => "30 days",
        }
    }

    fn days(self) -> i64 {
        match self {
            Self::Today => 1,
            Self::Week => 7,
            Self::Month => 30,
        }
    }

    /// Local midnight `days - 1` days before `now`'s day.
    pub fn start(self, now: OffsetDateTime, offset: UtcOffset) -> OffsetDateTime {
        now.to_offset(offset).replace_time(Time::MIDNIGHT) - Duration::days(self.days() - 1)
    }
}

/// Dollars and credits spent, kept apart.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpendTotal {
    pub usd: Decimal,
    pub credits: Decimal,
}

impl SpendTotal {
    fn add(&mut self, cost: &Cost) {
        match cost {
            Cost::Usd { amount, .. } => self.usd += *amount,
            Cost::Credits { amount, .. } => self.credits += *amount,
        }
    }

    /// "$12.40 · 96 cr"; "$0.00" when nothing was spent.
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        if !self.usd.is_zero() || self.credits.is_zero() {
            parts.push(format::dollars(self.usd));
        }
        if !self.credits.is_zero() {
            parts.push(format::credits(self.credits));
        }
        parts.join(" · ")
    }
}

/// "1 job", "3 jobs".
pub fn jobs_text(count: usize) -> String {
    if count == 1 {
        "1 job".to_owned()
    } else {
        format!("{count} jobs")
    }
}

/// One line of a breakdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SpendLine {
    pub title: String,
    pub jobs_text: String,
    pub total: String,
}

fn line(title: impl Into<String>, count: usize, total: &SpendTotal) -> SpendLine {
    SpendLine {
        title: title.into(),
        jobs_text: jobs_text(count),
        total: total.text(),
    }
}

/// What a job is counted at: nothing for one the plan's unlimited generations paid for.
fn spent_price(job: &Job) -> Cost {
    if job.spec.uses_unlimited {
        Cost::credits(Decimal::ZERO, job.price().is_estimate())
    } else {
        job.price().clone()
    }
}

fn lines<K: PartialEq + Copy>(
    jobs: &[&Job],
    keys: &[K],
    key: impl Fn(&Job) -> K,
    title: impl Fn(K) -> String,
) -> Vec<SpendLine> {
    keys.iter()
        .filter_map(|value| {
            let group: Vec<&&Job> = jobs.iter().filter(|job| key(job) == *value).collect();
            if group.is_empty() {
                return None;
            }
            let mut total = SpendTotal::default();
            for job in &group {
                total.add(&spent_price(job));
            }
            Some(line(title(*value), group.len(), &total))
        })
        .collect()
}

/// "Reference images", "Character sheets", "Virality scores".
pub fn purpose_title(purpose: ImageChargePurpose) -> &'static str {
    match purpose {
        ImageChargePurpose::ReferenceImages => "Reference images",
        ImageChargePurpose::ViralityScore => "Virality scores",
        ImageChargePurpose::CharacterSheets => "Character sheets",
    }
}

/// What a period cost (`SpendingSummary`): the jobs finished in it, split by whether they were charged, and the
/// image charges, which count as charged.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpendingSummary {
    pub charged: SpendTotal,
    pub charged_jobs: usize,
    pub not_charged: SpendTotal,
    pub not_charged_jobs: usize,
    pub by_mode: Vec<SpendLine>,
    pub by_resolution: Vec<SpendLine>,
    pub by_kind: Vec<SpendLine>,
}

impl SpendingSummary {
    pub fn new(
        jobs: &[Job],
        images: &[ImageCharge],
        period: SpendingPeriod,
        now: OffsetDateTime,
        offset: UtcOffset,
    ) -> Self {
        let start = period.start(now, offset);
        let finished: Vec<&Job> = jobs
            .iter()
            .filter(|job| job.state.is_terminal() && job.finished_at.is_some_and(|at| at >= start))
            .collect();
        let mut summary = Self::default();
        let mut charged: Vec<&Job> = Vec::new();
        for job in &finished {
            if job.is_charged() {
                charged.push(job);
                summary.charged.add(&spent_price(job));
            } else {
                summary.not_charged.add(&spent_price(job));
            }
        }
        summary.charged_jobs = charged.len();
        summary.not_charged_jobs = finished.len() - charged.len();
        summary.by_mode = lines(
            &charged,
            Mode::ALL,
            |job| job.spec.mode,
            |mode| mode_title(mode).to_owned(),
        );
        let requests: Vec<&ImageCharge> = images
            .iter()
            .filter(|charge| !charge.is_pending && charge.date >= start)
            .collect();
        for purpose in [
            ImageChargePurpose::ReferenceImages,
            ImageChargePurpose::CharacterSheets,
            ImageChargePurpose::ViralityScore,
        ] {
            let paid: Vec<&&ImageCharge> = requests
                .iter()
                .filter(|charge| charge.paid_for() == purpose)
                .collect();
            if paid.is_empty() {
                continue;
            }
            let mut total = SpendTotal::default();
            for charge in &paid {
                total.add(&charge.cost);
            }
            summary.charged.usd += total.usd;
            summary.charged.credits += total.credits;
            summary
                .by_mode
                .push(line(purpose_title(purpose), paid.len(), &total));
        }
        summary.by_resolution = lines(
            &charged,
            Resolution::ALL,
            |job| job.spec.resolution,
            |resolution| resolution.raw_value().to_owned(),
        );
        summary.by_kind = lines(
            &charged,
            &[true, false],
            |job| job.spec.is_draft,
            |draft| if draft { "Draft" } else { "Final" }.to_owned(),
        );
        summary
    }
}

// MARK: CSV

pub const CSV_HEADER: [&str; 10] = [
    "Date",
    "Mode",
    "Resolution",
    "Draft",
    "Billed seconds",
    "Estimated cost",
    "Currency",
    "State",
    "Charged",
    "Request ID",
];

/// RFC 4180: a field with a comma, quote or line break is quoted, its quotes doubled.
fn escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

fn csv_line(fields: &[String]) -> String {
    let mut line = fields
        .iter()
        .map(|field| escape(field))
        .collect::<Vec<_>>()
        .join(",");
    line.push_str("\r\n");
    line
}

/// ISO 8601 with the offset, "Z" for UTC.
fn iso(date: OffsetDateTime, offset: UtcOffset) -> String {
    date.to_offset(offset)
        .replace_nanosecond(0)
        .unwrap_or(date)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn amount_and_currency(cost: &Cost) -> (String, &'static str) {
    match cost {
        Cost::Usd { amount, .. } => (amount.normalize().to_string(), "USD"),
        Cost::Credits { amount, .. } => (amount.normalize().to_string(), "credits"),
    }
}

fn state_title(state: &JobState) -> &'static str {
    match state {
        JobState::Done => "Done",
        JobState::Failed { .. } => "Failed",
        JobState::Rejected => "Rejected",
        JobState::Canceled => "Canceled",
        _ => "In queue",
    }
}

/// Every finished job and settled image charge, oldest first, as CSV (`SpendingCSV.csv`).
pub fn spending_csv(jobs: &[Job], images: &[ImageCharge], offset: UtcOffset) -> String {
    let mut rows: Vec<(OffsetDateTime, Vec<String>)> = jobs
        .iter()
        .filter(|job| job.state.is_terminal())
        .map(|job| {
            let date = job.finished_at.unwrap_or(job.created_at);
            let (amount, currency) = amount_and_currency(job.price());
            (
                date,
                vec![
                    iso(date, offset),
                    mode_title(job.spec.mode).to_owned(),
                    job.spec.resolution.raw_value().to_owned(),
                    if job.spec.is_draft { "Yes" } else { "No" }.to_owned(),
                    Pricing::billed_seconds_on(&job.spec, Some(job.provider_kind)).to_string(),
                    amount,
                    currency.to_owned(),
                    state_title(&job.state).to_owned(),
                    if job.is_charged() { "Yes" } else { "No" }.to_owned(),
                    job.remote
                        .as_ref()
                        .map(|remote| remote.request_id.clone())
                        .unwrap_or_default(),
                ],
            )
        })
        .collect();
    rows.extend(images.iter().filter(|charge| !charge.is_pending).map(|charge| {
        let (amount, currency) = amount_and_currency(&charge.cost);
        (
            charge.date,
            vec![
                iso(charge.date, offset),
                purpose_title(charge.paid_for()).to_owned(),
                String::new(),
                "No".to_owned(),
                "0".to_owned(),
                amount,
                currency.to_owned(),
                "Done".to_owned(),
                "Yes".to_owned(),
                String::new(),
            ],
        )
    }));
    rows.sort_by_key(|(date, _)| *date);
    let header: Vec<String> = CSV_HEADER.iter().map(|field| (*field).to_owned()).collect();
    std::iter::once(header)
        .chain(rows.into_iter().map(|(_, row)| row))
        .map(|row| csv_line(&row))
        .collect()
}

/// "Jutsu Studio spending 2026-10-15".
pub fn csv_file_name(now: OffsetDateTime, offset: UtcOffset) -> String {
    let date = now.to_offset(offset).date();
    format!(
        "Jutsu Studio spending {:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

// MARK: Cap

/// Accepts "75", "$75.00", "1,000.5" and surrounding spaces (`AppSettings.parseCapAmount`); `None` for anything
/// else, a non-positive amount or one of a billion dollars or more. Rounded to cents.
pub fn parse_cap_amount(text: &str) -> Option<Decimal> {
    let cleaned: String = text.chars().filter(|c| !c.is_whitespace() && *c != ',').collect();
    let digits = cleaned.strip_prefix('$').unwrap_or(&cleaned);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let value: Decimal = digits.parse().ok()?;
    (value > Decimal::ZERO && value < Decimal::from(1_000_000_000)).then(|| value.round_dp(2))
}

/// Accepts "1000", "1,000", "1 000 cr" and "1000 credits" (`AppSettings.parseCapCredits`); `None` for anything else
/// or a non-positive amount.
pub fn parse_cap_credits(text: &str) -> Option<i32> {
    let mut cleaned: String = text
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',')
        .collect();
    for suffix in ["credits", "cr"] {
        if let Some(stripped) = cleaned.strip_suffix(suffix) {
            cleaned = stripped.to_owned();
            break;
        }
    }
    if cleaned.is_empty() || !cleaned.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    cleaned.parse::<i32>().ok().filter(|value| *value > 0)
}

/// "$12.50" or "1,250 credits".
pub fn format_spend(amount: Decimal, unit: CostUnit) -> String {
    match unit {
        CostUnit::Usd => format::dollars(amount),
        CostUnit::Credits => credits_text(amount.trunc().to_i64().unwrap_or(0)),
    }
}

// MARK: Credit history

/// "Spent", "Refunded".
pub fn credit_kind_title(kind: CreditEntryKind) -> &'static str {
    match kind {
        CreditEntryKind::Spend => "Spent",
        CreditEntryKind::Refund => "Refunded",
        CreditEntryKind::Grant => "Added",
        CreditEntryKind::Deduct => "Deducted",
        CreditEntryKind::Other => "Other",
    }
}

/// "−45", "+39", "0".
pub fn credit_amount_text(credits: Decimal) -> String {
    let magnitude = credits.abs().round_dp(2).normalize();
    match credits.cmp(&Decimal::ZERO) {
        std::cmp::Ordering::Less => format!("−{magnitude}"),
        std::cmp::Ordering::Greater => format!("+{magnitude}"),
        std::cmp::Ordering::Equal => magnitude.to_string(),
    }
}

impl crate::create::paged_feed::FeedItem for CreditEntry {
    /// The provider names no ID; the time, amount and title tell entries apart.
    fn feed_id(&self) -> String {
        format!(
            "{}|{}|{}",
            self.date.unix_timestamp_nanos(),
            self.credits,
            self.title
        )
    }
}

/// How many credit entries "Show More" adds.
pub const CREDIT_HISTORY_STEP: usize = 10;
/// What a failed page says when the error has no message of its own.
pub const CREDIT_HISTORY_FAILURE: &str = "Can't load the credit history";

/// One credit movement: its title, "Spent · Oct 22, 2026 at 14:05", and "−45" or "+39".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CreditRow {
    pub title: String,
    pub detail: String,
    pub amount: String,
    /// Refunded or added: shown in the accent color.
    pub is_added: bool,
}

/// The Credit history section: the entries shown, then a failure, "Loading…", "Older entries" or the empty row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CreditHistoryView {
    pub rows: Vec<CreditRow>,
    pub failure: Option<UserMessage>,
    pub is_loading: bool,
    /// "Older entries" with Show More.
    pub has_older: bool,
    /// "No credit movements yet".
    pub is_empty: bool,
}

impl CreditHistoryView {
    /// The first `shown` of `feed`'s entries and the row under them, as `CreditHistorySection` picks it.
    pub fn new(feed: &FeedView<CreditEntry>, shown: usize, offset: UtcOffset) -> Self {
        let rows = feed
            .items
            .iter()
            .take(shown)
            .map(|entry| CreditRow {
                title: entry.title.clone(),
                detail: format!(
                    "{} · {}",
                    credit_kind_title(entry.kind),
                    date_at_time(entry.date, offset)
                ),
                amount: credit_amount_text(entry.credits),
                is_added: entry.credits > Decimal::ZERO,
            })
            .collect();
        let is_loading = feed.failure.is_none() && feed.is_loading;
        let has_older =
            feed.failure.is_none() && !feed.is_loading && (feed.items.len() > shown || feed.has_more);
        Self {
            rows,
            failure: feed.failure.clone(),
            is_loading,
            has_older,
            is_empty: feed.failure.is_none() && !is_loading && !has_older && feed.items.is_empty(),
        }
    }
}

// MARK: Page

/// One choice of the period picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SpendingPeriodOption {
    pub value: SpendingPeriod,
    pub title: String,
}

/// A breakdown section, left out when it has no lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SpendingBreakdown {
    pub title: String,
    pub lines: Vec<SpendLine>,
}

/// The Daily cap section of the connection's unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DailyCapView {
    pub is_enabled: bool,
    /// The amount field's text: "$75.00" or "1,000 credits".
    pub amount_text: String,
    /// "$12.50", "…" before the first read, "Unavailable" when it can't be read.
    pub committed_text: String,
    /// Why today's spend can't be read.
    pub committed_error: Option<String>,
    pub footer: String,
}

/// Settings › Spending (`SpendingSettingsView`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SpendingView {
    pub periods: Vec<SpendingPeriodOption>,
    pub period: SpendingPeriod,
    pub charged: String,
    /// "3 jobs".
    pub charged_detail: String,
    pub not_charged: String,
    /// "Failed, rejected or canceled · 1 job".
    pub not_charged_detail: String,
    pub spent_footer: String,
    pub breakdowns: Vec<SpendingBreakdown>,
    pub cap: DailyCapView,
    /// The Low credits field's text; accounts only.
    pub low_credits: Option<String>,
    /// Credit history is listed; accounts only.
    pub shows_credit_history: bool,
    /// Milliseconds to local midnight, when today's spend starts again from zero.
    pub refresh_after_ms: u32,
}

/// "Oct 22, 2026 at 14:05".
pub fn date_at_time(date: OffsetDateTime, offset: UtcOffset) -> String {
    let local = date.to_offset(offset);
    let month = local.month().to_string();
    format!(
        "{} {}, {} at {:02}:{:02}",
        month.get(..3).unwrap_or(&month),
        local.day(),
        local.year(),
        local.hour(),
        local.minute()
    )
}

fn spent_footer(unit: CostUnit) -> &'static str {
    match unit {
        CostUnit::Credits => {
            "Estimates made before each job was submitted. Credit history below lists what the account was \
             actually charged and refunded. USD is the API key balance, credits are the account plan."
        }
        CostUnit::Usd => {
            "Estimates made before each job was submitted. Higgsfield doesn't report the amount an API key \
             was actually charged. USD is the API key balance, credits are the account plan."
        }
    }
}

fn cap_footer(unit: CostUnit) -> &'static str {
    match unit {
        CostUnit::Usd => {
            "A soft cap on the API balance: jobs over it ask for confirmation instead of starting. \
             It counts jobs finished today and every job in the queue."
        }
        CostUnit::Credits => {
            "A soft cap on plan credits: jobs over it ask for confirmation instead of starting. \
             It counts jobs finished today and every job in the queue."
        }
    }
}

/// Whether the cap of `unit` is on.
pub fn is_cap_enabled(preferences: &Preferences, unit: CostUnit) -> bool {
    match unit {
        CostUnit::Usd => preferences
            .spend_cap_enabled
            .unwrap_or(Preferences::DEFAULT_SPEND_CAP_ENABLED),
        CostUnit::Credits => preferences
            .spend_cap_credits_enabled
            .unwrap_or(Preferences::DEFAULT_SPEND_CAP_CREDITS_ENABLED),
    }
}

/// The cap of `unit` as its field shows it: "$75.00" (the one form `parse_cap_amount` reads back) or "1,000 credits".
pub fn cap_text(preferences: &Preferences, unit: CostUnit) -> String {
    match unit {
        CostUnit::Usd => format::dollars(
            Decimal::try_from(
                preferences
                    .spend_cap_usd
                    .unwrap_or(Preferences::DEFAULT_SPEND_CAP_USD),
            )
            .unwrap_or_default(),
        ),
        CostUnit::Credits => credits_text(
            preferences
                .spend_cap_credits
                .map_or(Preferences::DEFAULT_SPEND_CAP_CREDITS, i64::from),
        ),
    }
}

/// The settings that turn the cap of `unit` on or off.
pub fn cap_enabled_patch(unit: CostUnit, is_on: bool) -> Preferences {
    let mut patch = Preferences::default();
    match unit {
        CostUnit::Usd => patch.spend_cap_enabled = Some(is_on),
        CostUnit::Credits => patch.spend_cap_credits_enabled = Some(is_on),
    }
    patch
}

/// The settings `text` sets as the cap of `unit`; `None` when it doesn't parse, which keeps the last valid amount.
pub fn cap_patch(text: &str, unit: CostUnit) -> Option<Preferences> {
    let mut patch = Preferences::default();
    match unit {
        CostUnit::Usd => patch.spend_cap_usd = Some(parse_cap_amount(text)?.to_f64()?),
        CostUnit::Credits => patch.spend_cap_credits = Some(parse_cap_credits(text)?),
    }
    Some(patch)
}

/// The Low credits field's text: "50 credits".
pub fn low_credits_text(preferences: &Preferences) -> String {
    credits_text(preferences.low_credit_threshold())
}

/// The settings `text` sets as the low-credit threshold; `None` when it doesn't parse.
pub fn low_credits_patch(text: &str) -> Option<Preferences> {
    Some(Preferences {
        low_credit_threshold: Some(parse_cap_credits(text)?),
        ..Preferences::default()
    })
}

/// What Settings › Spending shows for `summary` on a connection paying in `unit`. `committed` is today's spend
/// toward the cap, its error, or `None` before the first read.
pub fn spending_view(
    summary: &SpendingSummary,
    period: SpendingPeriod,
    unit: CostUnit,
    preferences: &Preferences,
    committed: Option<Result<Decimal, String>>,
    now: OffsetDateTime,
    offset: UtcOffset,
) -> SpendingView {
    let breakdowns = [
        ("By mode", &summary.by_mode),
        ("By resolution", &summary.by_resolution),
        ("Draft or final", &summary.by_kind),
    ]
    .into_iter()
    .filter(|(_, lines)| !lines.is_empty())
    .map(|(title, lines)| SpendingBreakdown {
        title: title.to_owned(),
        lines: lines.clone(),
    })
    .collect();
    let (committed_text, committed_error) = match committed {
        None => ("…".to_owned(), None),
        Some(Ok(amount)) => (format_spend(amount, unit), None),
        Some(Err(error)) => ("Unavailable".to_owned(), Some(error)),
    };
    let midnight = SpendingPeriod::Today.start(now, offset) + Duration::days(1);
    let is_account = unit == CostUnit::Credits;
    SpendingView {
        periods: SpendingPeriod::ALL
            .iter()
            .map(|period| SpendingPeriodOption {
                value: *period,
                title: period.title().to_owned(),
            })
            .collect(),
        period,
        charged: summary.charged.text(),
        charged_detail: jobs_text(summary.charged_jobs),
        not_charged: summary.not_charged.text(),
        not_charged_detail: format!(
            "Failed, rejected or canceled · {}",
            jobs_text(summary.not_charged_jobs)
        ),
        spent_footer: spent_footer(unit).to_owned(),
        breakdowns,
        cap: DailyCapView {
            is_enabled: is_cap_enabled(preferences, unit),
            amount_text: cap_text(preferences, unit),
            committed_text,
            committed_error,
            footer: cap_footer(unit).to_owned(),
        },
        low_credits: is_account.then(|| low_credits_text(preferences)),
        shows_credit_history: is_account,
        refresh_after_ms: u32::try_from((midnight - now).whole_milliseconds().max(1_000)).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jutsu_core::{
        ClosedRange, FailureReason, PixelSize, ProviderKind, RemoteJob, SourceClip, VideoModel,
    };
    use url::Url;
    use uuid::Uuid;

    fn dec(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn usd(text: &str) -> Cost {
        Cost::usd(dec(text), false)
    }

    fn job(
        state: JobState,
        cost: Cost,
        finished: Option<OffsetDateTime>,
        mode: Mode,
        resolution: Resolution,
        draft: bool,
        length: f64,
    ) -> Job {
        let source = SourceClip::new(
            Url::parse("file:///tmp/source.mov").unwrap(),
            30.0,
            PixelSize::new(1280, 720),
            ClosedRange::new(0.0, length),
        );
        let mut spec = jutsu_core::JobSpec::new(mode, VideoModel::GENJUTSU, Some(source), resolution);
        spec.is_draft = draft;
        let created = finished.unwrap_or(OffsetDateTime::UNIX_EPOCH);
        let mut job = Job::new(
            Uuid::new_v4(),
            spec,
            ProviderKind::ApiKey,
            cost,
            Uuid::new_v4(),
            created,
        );
        job.state = state;
        job.finished_at = finished;
        job
    }

    fn simple(state: JobState, cost: Cost, finished: Option<OffsetDateTime>) -> Job {
        job(
            state,
            cost,
            finished,
            Mode::MotionTransfer,
            Resolution::P720,
            false,
            10.0,
        )
    }

    fn failed() -> JobState {
        JobState::Failed {
            reason: FailureReason::Message("x".to_owned()),
        }
    }

    /// UTC+10; 2025-10-13 09:00 local, 2025-10-12 23:00 UTC.
    fn plus10() -> UtcOffset {
        UtcOffset::from_hms(10, 0, 0).unwrap()
    }

    fn now() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_760_310_000).unwrap()
    }

    #[test]
    fn periods_start_at_local_midnight() {
        let midnight = now() - Duration::hours(9);
        assert_eq!(SpendingPeriod::Today.start(now(), plus10()), midnight);
        assert_eq!(
            SpendingPeriod::Week.start(now(), plus10()),
            midnight - Duration::days(6)
        );
    }

    #[test]
    fn week_excludes_the_eighth_day() {
        let start = SpendingPeriod::Week.start(now(), plus10());
        let jobs = [
            simple(JobState::Done, usd("2"), Some(start)),
            simple(JobState::Done, usd("5"), Some(start - Duration::seconds(1))),
        ];
        let week = SpendingSummary::new(&jobs, &[], SpendingPeriod::Week, now(), plus10());
        let month = SpendingSummary::new(&jobs, &[], SpendingPeriod::Month, now(), plus10());
        assert_eq!(week.charged.usd, dec("2"));
        assert_eq!(month.charged.usd, dec("7"));
    }

    #[test]
    fn failed_rejected_and_canceled_are_not_charged() {
        let mut download_failed = simple(
            JobState::Failed {
                reason: FailureReason::Message("download".to_owned()),
            },
            usd("3"),
            Some(now()),
        );
        download_failed.charged_at = Some(now());
        let jobs = [
            simple(JobState::Done, usd("10"), Some(now())),
            simple(failed(), usd("4"), Some(now())),
            simple(JobState::Rejected, Cost::credits(dec("20"), true), Some(now())),
            simple(JobState::Canceled, usd("1"), Some(now())),
            download_failed,
        ];
        let summary = SpendingSummary::new(&jobs, &[], SpendingPeriod::Today, now(), plus10());
        assert_eq!(
            summary.charged,
            SpendTotal {
                usd: dec("13"),
                credits: Decimal::ZERO
            }
        );
        assert_eq!(summary.charged_jobs, 2);
        assert_eq!(
            summary.not_charged,
            SpendTotal {
                usd: dec("5"),
                credits: dec("20")
            }
        );
        assert_eq!(summary.not_charged_jobs, 3);
    }

    #[test]
    fn usd_and_credits_are_kept_apart_and_breakdowns_follow_the_order() {
        let jobs = [
            simple(JobState::Done, usd("3.816"), Some(now())),
            simple(JobState::Done, Cost::credits(dec("96"), true), Some(now())),
            simple(JobState::Processing, usd("9"), None),
        ];
        let summary = SpendingSummary::new(&jobs, &[], SpendingPeriod::Today, now(), plus10());
        assert_eq!(summary.charged.text(), "$3.82 · 96 cr");
        assert_eq!(SpendTotal::default().text(), "$0.00");
        let jobs = [
            job(
                JobState::Done,
                usd("1"),
                Some(now()),
                Mode::Restyle,
                Resolution::P480,
                true,
                10.0,
            ),
            job(
                JobState::Done,
                usd("2"),
                Some(now()),
                Mode::Restyle,
                Resolution::P1080,
                false,
                10.0,
            ),
            job(
                JobState::Done,
                usd("4"),
                Some(now()),
                Mode::MotionTransfer,
                Resolution::P1080,
                false,
                10.0,
            ),
            job(
                failed(),
                usd("8"),
                Some(now()),
                Mode::ObjectSwap,
                Resolution::P720,
                false,
                10.0,
            ),
        ];
        let summary = SpendingSummary::new(&jobs, &[], SpendingPeriod::Today, now(), plus10());
        let titles = |lines: &[SpendLine]| {
            lines
                .iter()
                .map(|line| (line.title.clone(), line.jobs_text.clone(), line.total.clone()))
                .collect::<Vec<_>>()
        };
        let row =
            |title: &str, jobs: &str, total: &str| (title.to_owned(), jobs.to_owned(), total.to_owned());
        assert_eq!(
            titles(&summary.by_mode),
            vec![
                row("Motion transfer", "1 job", "$4.00"),
                row("Restyle", "2 jobs", "$3.00")
            ]
        );
        assert_eq!(
            titles(&summary.by_resolution),
            vec![row("480p", "1 job", "$1.00"), row("1080p", "2 jobs", "$6.00")]
        );
        assert_eq!(
            titles(&summary.by_kind),
            vec![row("Draft", "1 job", "$1.00"), row("Final", "2 jobs", "$6.00")]
        );
    }

    #[test]
    fn image_charges_count_as_charged_after_the_modes() {
        let charge = |purpose| {
            let mut charge = ImageCharge::new(
                Uuid::new_v4(),
                now(),
                Cost::credits(dec("5"), false),
                ProviderKind::Account,
                "x",
                1,
                Some(purpose),
            );
            charge.is_pending = false;
            charge
        };
        let mut pending = charge(ImageChargePurpose::ReferenceImages);
        pending.is_pending = true;
        let images = [
            charge(ImageChargePurpose::ViralityScore),
            charge(ImageChargePurpose::ReferenceImages),
            pending,
        ];
        let summary = SpendingSummary::new(&[], &images, SpendingPeriod::Today, now(), plus10());
        assert_eq!(summary.charged.credits, dec("10"));
        let titles: Vec<&str> = summary.by_mode.iter().map(|line| line.title.as_str()).collect();
        assert_eq!(titles, ["Reference images", "Virality scores"]);
        let csv = spending_csv(&[], &images, UtcOffset::UTC);
        assert_eq!(csv.lines().count(), 3);
        assert!(csv.contains(",Virality scores,,No,0,5,credits,Done,Yes,\r\n"));
    }

    fn noon() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_760_356_800).unwrap()
    }

    #[test]
    fn the_csv_has_one_row_per_finished_job_oldest_first() {
        let plus2 = UtcOffset::from_hms(2, 0, 0).unwrap();
        let mut done = job(
            JobState::Done,
            usd("3.816"),
            Some(noon()),
            Mode::Restyle,
            Resolution::P1080,
            false,
            11.2,
        );
        done.remote = Some(RemoteJob::new("req-1"));
        let draft = job(
            JobState::Rejected,
            Cost::credits(dec("40"), true),
            Some(noon() - Duration::hours(1)),
            Mode::MotionTransfer,
            Resolution::P480,
            true,
            10.0,
        );
        let active = simple(JobState::Processing, usd("1"), None);
        assert_eq!(
            spending_csv(&[done, active, draft], &[], plus2),
            "Date,Mode,Resolution,Draft,Billed seconds,Estimated cost,Currency,State,Charged,Request ID\r\n\
             2025-10-13T13:00:00+02:00,Motion transfer,480p,Yes,10,40,credits,Rejected,No,\r\n\
             2025-10-13T14:00:00+02:00,Restyle,1080p,No,12,3.816,USD,Done,Yes,req-1\r\n"
        );
        let mut failed_download = simple(
            JobState::Failed {
                reason: FailureReason::Message("download".to_owned()),
            },
            usd("2"),
            Some(noon()),
        );
        failed_download.charged_at = Some(noon());
        assert!(
            spending_csv(&[failed_download], &[], UtcOffset::UTC)
                .ends_with("2025-10-13T12:00:00Z,Motion transfer,720p,No,10,2,USD,Failed,Yes,\r\n")
        );
        assert_eq!(
            spending_csv(&[], &[], UtcOffset::UTC),
            format!("{}\r\n", CSV_HEADER.join(","))
        );
        assert_eq!(
            csv_file_name(OffsetDateTime::from_unix_timestamp(1_760_396_400).unwrap(), plus2),
            "Jutsu Studio spending 2025-10-14"
        );
    }

    #[test]
    fn fields_are_escaped_per_rfc_4180() {
        assert_eq!(escape("plain"), "plain");
        assert_eq!(escape("a,b"), "\"a,b\"");
        assert_eq!(escape("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(escape("two\nlines"), "\"two\nlines\"");
        assert_eq!(
            csv_line(&["a".to_owned(), "b,c".to_owned(), String::new()]),
            "a,\"b,c\",\r\n"
        );
    }

    #[test]
    fn cap_amounts_parse_like_swift() {
        for (text, expected) in [
            ("75", "75"),
            ("$75.00", "75"),
            (" $ 1,000.5 ", "1000.5"),
            ("12.346", "12.35"),
            (".5", "0.5"),
        ] {
            assert_eq!(parse_cap_amount(text), Some(dec(expected)), "{text}");
        }
        for text in [
            "",
            "$",
            "abc",
            "-5",
            "0",
            "$0.00",
            "1e3",
            "7.5.1",
            "inf",
            "75 USD",
            "99999999999999999999",
        ] {
            assert_eq!(parse_cap_amount(text), None, "{text}");
        }
        for (text, expected) in [
            ("1000", 1000),
            ("1,000", 1000),
            (" 1 000 cr", 1000),
            ("250 Credits", 250),
        ] {
            assert_eq!(parse_cap_credits(text), Some(expected), "{text}");
        }
        for text in ["", "cr", "abc", "-5", "0", "1.5", "1e3", "$100"] {
            assert_eq!(parse_cap_credits(text), None, "{text}");
        }
        assert_eq!(format_spend(dec("1250"), CostUnit::Credits), "1,250 credits");
        assert_eq!(format_spend(dec("12.5"), CostUnit::Usd), "$12.50");
        assert_eq!(credits_text(1), "1 credit");
        assert_eq!(credit_amount_text(dec("-45")), "−45");
        assert_eq!(credit_amount_text(dec("39")), "+39");
        assert_eq!(credit_amount_text(Decimal::ZERO), "0");
    }

    /// `AppEnvironment.committedToday`: a spend log that couldn't be read blocks, never counts as nothing.
    #[tokio::test]
    async fn an_unreadable_spend_log_is_an_error() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join(ImageSpendLog::FILE_NAME), "not json").unwrap();
        let now = OffsetDateTime::now_utc();
        let images = ImageSpendLog::open(Some(folder.path()), now, UtcOffset::UTC);
        let store = Store::open_in_memory().await.unwrap();
        assert!(
            committed_today(&store, &images, CostUnit::Credits, now, UtcOffset::UTC)
                .await
                .is_err()
        );
        let readable = ImageSpendLog::open(None, now, UtcOffset::UTC);
        assert_eq!(
            committed_today(&store, &readable, CostUnit::Credits, now, UtcOffset::UTC).await,
            Ok(Decimal::ZERO)
        );
    }

    fn entry(credits: &str, minute: i64) -> CreditEntry {
        CreditEntry {
            date: noon() + Duration::minutes(minute),
            credits: dec(credits),
            kind: if credits.starts_with('-') {
                CreditEntryKind::Spend
            } else {
                CreditEntryKind::Refund
            },
            title: "Higgsfield Genjutsu - Object Swap".to_owned(),
        }
    }

    fn feed(items: Vec<CreditEntry>, has_more: bool, failure: Option<UserMessage>) -> FeedView<CreditEntry> {
        FeedView {
            items,
            is_loading: false,
            failure,
            has_more,
            has_loaded: true,
        }
    }

    #[test]
    fn the_credit_history_shows_ten_then_older_entries() {
        let items: Vec<CreditEntry> = (0..12).map(|minute| entry("-45", minute)).collect();
        let view = CreditHistoryView::new(
            &feed(items.clone(), false, None),
            CREDIT_HISTORY_STEP,
            UtcOffset::UTC,
        );
        assert_eq!(view.rows.len(), 10);
        assert!(view.has_older && !view.is_empty);
        assert_eq!(view.rows[0].detail, "Spent · Oct 13, 2025 at 12:00");
        assert_eq!(view.rows[0].amount, "−45");
        assert!(!view.rows[0].is_added);
        let all = CreditHistoryView::new(&feed(items, false, None), 20, UtcOffset::UTC);
        assert_eq!(all.rows.len(), 12);
        assert!(!all.has_older);
        let refund = CreditHistoryView::new(&feed(vec![entry("39", 0)], false, None), 10, UtcOffset::UTC);
        assert!(refund.rows[0].is_added);
        assert_eq!(refund.rows[0].amount, "+39");
    }

    #[test]
    fn a_failure_replaces_the_row_under_the_entries() {
        let failure = UserMessage::new(CREDIT_HISTORY_FAILURE, "Try again.", None);
        let view = CreditHistoryView::new(&feed(Vec::new(), true, Some(failure.clone())), 10, UtcOffset::UTC);
        assert_eq!(view.failure, Some(failure));
        assert!(!view.has_older && !view.is_empty && !view.is_loading);
        let empty = CreditHistoryView::new(&feed(Vec::new(), false, None), 10, UtcOffset::UTC);
        assert!(empty.is_empty);
    }

    #[test]
    fn the_cap_field_keeps_the_last_valid_amount() {
        let preferences = Preferences::default();
        assert_eq!(cap_text(&preferences, CostUnit::Usd), "$75.00");
        assert_eq!(cap_text(&preferences, CostUnit::Credits), "1,000 credits");
        assert!(is_cap_enabled(&preferences, CostUnit::Usd));
        assert!(!is_cap_enabled(&preferences, CostUnit::Credits));
        assert_eq!(
            cap_patch("$120.50", CostUnit::Usd).and_then(|patch| patch.spend_cap_usd),
            Some(120.5)
        );
        assert_eq!(
            cap_patch("2,500 cr", CostUnit::Credits).and_then(|patch| patch.spend_cap_credits),
            Some(2_500)
        );
        assert_eq!(cap_patch("lots", CostUnit::Usd), None);
        assert_eq!(
            cap_enabled_patch(CostUnit::Credits, true).spend_cap_credits_enabled,
            Some(true)
        );
        assert_eq!(low_credits_text(&preferences), "50 credits");
        assert_eq!(low_credits_patch("0"), None);
    }

    #[test]
    fn the_page_follows_the_connections_unit() {
        let jobs = vec![
            simple(JobState::Done, usd("10"), Some(now())),
            simple(failed(), usd("4"), Some(now())),
        ];
        let summary = SpendingSummary::new(&jobs, &[], SpendingPeriod::Today, now(), plus10());
        let preferences = Preferences::default();
        let view = spending_view(
            &summary,
            SpendingPeriod::Today,
            CostUnit::Usd,
            &preferences,
            Some(Ok(dec("12.5"))),
            now(),
            plus10(),
        );
        assert_eq!(view.charged, "$10.00");
        assert_eq!(view.charged_detail, "1 job");
        assert_eq!(view.not_charged_detail, "Failed, rejected or canceled · 1 job");
        assert_eq!(
            view.breakdowns
                .iter()
                .map(|breakdown| breakdown.title.as_str())
                .collect::<Vec<_>>(),
            ["By mode", "By resolution", "Draft or final"]
        );
        assert_eq!(view.cap.committed_text, "$12.50");
        assert!(view.low_credits.is_none() && !view.shows_credit_history);
        // 09:00 local: fifteen hours to midnight.
        assert_eq!(view.refresh_after_ms, 15 * 3_600_000);
        let account = spending_view(
            &SpendingSummary::default(),
            SpendingPeriod::Week,
            CostUnit::Credits,
            &preferences,
            Some(Err("disk".to_owned())),
            now(),
            plus10(),
        );
        assert_eq!(account.breakdowns, Vec::<SpendingBreakdown>::new());
        assert_eq!(account.cap.committed_text, "Unavailable");
        assert_eq!(account.cap.committed_error.as_deref(), Some("disk"));
        assert_eq!(account.low_credits.as_deref(), Some("50 credits"));
        assert!(account.shows_credit_history);
    }
}
