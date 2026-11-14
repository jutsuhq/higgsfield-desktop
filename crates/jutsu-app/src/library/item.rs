//! One finished video as the Library shows it (`LibraryItemModel.swift`): the card's texts, the details page's
//! facts, billing and timing lines.

use std::path::PathBuf;

use jutsu_core::{
    Cost, CostUnit, HistoryItem, Job, JobSpec, Mode, PostAction, Pricing, ProviderKind, VideoModel,
    decimal_text,
};
use rust_decimal::{Decimal, RoundingStrategy};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::connection::mode_title;
use crate::queue::format;
use crate::queue::row::price_source;

/// One finished job as the library shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryItem {
    pub job: Job,
    /// Name of the restyle preset, when the catalog knows it.
    pub preset_name: Option<String>,
    /// Kept on the material, not on the job; for an item only in the account's history, its like.
    pub is_starred: bool,
    /// The account's history item while it isn't downloaded yet.
    pub history_item: Option<HistoryItem>,
}

impl LibraryItem {
    pub fn new(job: Job, is_starred: bool) -> Self {
        Self {
            job,
            preset_name: None,
            is_starred,
            history_item: None,
        }
    }

    /// A history item not yet in the library, shown as the job it becomes once imported.
    pub fn from_history(item: HistoryItem) -> Self {
        let job = Job::importing(&item, None, item.created_at);
        Self {
            job,
            preset_name: None,
            is_starred: item.is_liked,
            history_item: Some(item),
        }
    }

    pub fn id(&self) -> Uuid {
        self.job.id
    }

    pub fn spec(&self) -> &JobSpec {
        &self.job.spec
    }

    /// The generation whose like on the account follows this item's star; `None` for an API key result.
    pub fn like_id(&self) -> Option<String> {
        if self.job.provider_kind != ProviderKind::Account {
            return None;
        }
        self.history_item
            .as_ref()
            .map(|item| item.id.clone())
            .or_else(|| self.job.remote.as_ref().map(|remote| remote.request_id.clone()))
    }

    /// Only on Higgsfield so far: opening or playing it downloads the result first.
    pub fn needs_import(&self) -> bool {
        self.history_item.is_some()
    }

    /// Made outside this app: imported, or still only in the account's history.
    pub fn is_from_higgsfield(&self) -> bool {
        self.job.imported_at.is_some()
    }

    /// The downloaded result; `None` unless it is a local file.
    pub fn output_file(&self) -> Option<PathBuf> {
        self.job
            .output_file
            .as_ref()
            .filter(|url| url.scheme() == "file")
            .and_then(|url| url.to_file_path().ok())
    }

    /// Where the result plays from without a download: the account's link of an item only on Higgsfield.
    pub fn remote_url(&self) -> Option<&Url> {
        self.history_item.as_ref().map(|item| &item.output_url)
    }

    /// The job's `title` when an agent gave one, else the source file name without its extension; the model for a
    /// source only the account has. A clip made without a source goes by its prompt (`JobSpec::name`).
    pub fn source_name(&self) -> String {
        let spec = self.spec();
        if let Some(title) = &spec.title {
            return title.clone();
        }
        let Some(source) = &spec.source else {
            return spec.name();
        };
        if source.file_url.scheme() == "file" {
            spec.name()
        } else {
            spec.model.title()
        }
    }

    /// "rooftop · Claymation": the source and the preset, or the mode when there is no preset name.
    pub fn title(&self) -> String {
        let what = self
            .spec()
            .operation
            .as_ref()
            .map(|operation| format::post_action_title(&operation.action))
            .or_else(|| self.preset_name.clone())
            .unwrap_or_else(|| mode_title(self.spec().mode).to_owned());
        format!("{} · {what}", self.source_name())
    }

    pub fn finished_at(&self) -> OffsetDateTime {
        self.job.finished_at.unwrap_or(self.job.created_at)
    }

    pub fn resolution_title(&self) -> String {
        format::resolution_title(self.spec().resolution, &self.spec().model)
    }

    /// "480p · $1.91 · 36 min ago"; without the price, which isn't known, for a video made on Higgsfield.
    pub fn subtitle(&self, now: OffsetDateTime) -> String {
        let mut parts = vec![self.resolution_title()];
        if !self.is_from_higgsfield() {
            parts.push(format::cost_text(self.job.price()));
        }
        parts.push(format::relative_time(self.finished_at(), now));
        parts.join(" · ")
    }

    /// "36 min ago · Draft", above the details title.
    pub fn headline(&self, now: OffsetDateTime) -> String {
        let kind = if self.spec().is_draft { "Draft" } else { "Final" };
        format!("{} · {kind}", format::relative_time(self.finished_at(), now))
    }

    /// The short mode tag on the card.
    pub fn mode_tag(&self) -> &'static str {
        if let Some(operation) = &self.spec().operation {
            return post_action_tag(&operation.action);
        }
        match self.spec().mode {
            Mode::MotionTransfer => "Motion",
            Mode::Restyle => "Restyle",
            Mode::ObjectSwap => "Swap",
            Mode::Edit => "Edit",
            Mode::Extend => "Extend",
        }
    }

    /// The result's length, "6 s".
    pub fn duration_text(&self) -> String {
        format!("{} s", whole(self.job.output_duration()))
    }

    /// What a screen reader reads for the card: every tag and badge it shows.
    pub fn accessibility_label(&self, now: OffsetDateTime) -> String {
        let mut parts = vec![self.title(), self.mode_tag().to_owned()];
        if self.spec().is_draft {
            parts.push("Draft".to_owned());
        }
        parts.push(self.duration_text());
        parts.push(self.subtitle(now));
        if self.is_starred {
            parts.push("Starred".to_owned());
        }
        if self.needs_import() {
            parts.push("On Higgsfield, not downloaded yet".to_owned());
        }
        parts.join(", ")
    }

    /// The settings the result was made with, as short chips: "Restyle", "Film Noir", "480p", "6 s", "2 images".
    pub fn facts(&self) -> Vec<String> {
        let spec = self.spec();
        let mut facts = vec![mode_title(spec.mode).to_owned()];
        if spec.model != VideoModel::GENJUTSU {
            facts.push(spec.model.title());
        }
        facts.extend(self.preset_name.clone());
        facts.push(self.resolution_title());
        facts.push(self.duration_text());
        match spec.references.len() {
            0 => {}
            1 => facts.push("1 image".to_owned()),
            count => facts.push(format!("{count} images")),
        }
        facts
    }

    /// The source range sent, "00:04.0 – 00:10.0"; `None` for a clip made without a source.
    pub fn range_text(&self) -> Option<String> {
        let range = self.spec().source.as_ref()?.range;
        Some(format!(
            "{} – {}",
            format::timecode(range.lower()),
            format::timecode(range.upper())
        ))
    }

    /// "$1.91 · Estimate · 6 s × $0.318 · API key": the price, where it came from, how it was billed and paid.
    pub fn billing_summary(&self) -> String {
        let payment = payment_title(self.job.provider_kind);
        if self.is_from_higgsfield() {
            return format!("Made on Higgsfield · {payment}");
        }
        let mut parts = vec![format::cost_text(self.job.price()), price_source(&self.job)];
        parts.extend(self.billed_text());
        parts.push(payment.to_owned());
        parts.join(" · ")
    }

    /// "6 s × $0.318": billed seconds times the rate the stored cost works out to.
    pub fn billed_text(&self) -> Option<String> {
        let seconds = Pricing::billed_seconds_on(self.spec(), Some(self.job.provider_kind));
        if seconds <= 0 {
            return None;
        }
        let price = self.job.price();
        let rate = price.amount() / Decimal::from(seconds);
        let rate = match price.unit() {
            CostUnit::Usd => format!(
                "${:.3}",
                rate.round_dp_with_strategy(3, RoundingStrategy::MidpointAwayFromZero)
            ),
            CostUnit::Credits => format!(
                "{} cr",
                decimal_text(rate.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero))
            ),
        };
        Some(format!("{seconds} s × {rate}"))
    }

    /// "Queue wait 2:05 · Processing 1:12": the stage times that were recorded; `None` when none was.
    pub fn timing_summary(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(wait) = self.job.queue_wait() {
            parts.push(format!("Queue wait {}", format::duration(wait.as_seconds_f64())));
        }
        if let Some(processing) = self.job.processing_time() {
            parts.push(format!(
                "Processing {}",
                format::duration(processing.as_seconds_f64())
            ));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// Width over height of the downloaded result, else of the source, else the frame a clip made without one asked
    /// for; `None` when unknown.
    pub fn aspect_ratio(&self) -> Option<f64> {
        let size = self
            .job
            .output_media
            .as_ref()
            .map(|media| media.pixel_size)
            .or_else(|| self.spec().source.as_ref().map(|source| source.pixel_size));
        if let Some(size) = size {
            return size.is_valid().then(|| ratio(size.width, size.height));
        }
        self.spec()
            .aspect_ratio
            .and_then(jutsu_core::AspectRatio::width_over_height)
    }

    /// The price the "Highest price" order compares: dollars before credits, a video made on Higgsfield last.
    pub(crate) fn price_key(&self) -> (u8, Decimal) {
        if self.is_from_higgsfield() {
            return (2, Decimal::ZERO);
        }
        let price: &Cost = self.job.price();
        (u8::from(price.unit() != CostUnit::Usd), price.amount())
    }
}

/// How a job was paid, for the billing line.
pub fn payment_title(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::ApiKey => "API key",
        ProviderKind::Account => "Higgsfield account",
    }
}

/// The card's tag of an operation's result.
pub fn post_action_tag(action: &PostAction) -> &'static str {
    match action {
        PostAction::Upscale(_) => "Upscale",
        PostAction::RemoveBackground => "Cutout",
        PostAction::Reframe(_) => "Reframe",
        PostAction::Dub(_) => "Dub",
        PostAction::ChangeVoice(_) => "Voice",
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "a clip's length in whole seconds is far inside i64"
)]
fn whole(seconds: f64) -> i64 {
    seconds.max(0.0).round() as i64
}

#[allow(clippy::cast_precision_loss, reason = "frame sizes are far below 2^52")]
pub(crate) fn ratio(width: i64, height: i64) -> f64 {
    width as f64 / height as f64
}

#[cfg(test)]
#[path = "item_tests.rs"]
mod tests;
