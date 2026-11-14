//! What the Create screen knows besides the form, and the rules that need it (`CreateForm+Submit.swift`,
//! `CreateForm+Variants.swift`, `ModeOption.swift`, `ModelMenu.swift`, `EstimateCache.swift`): availability, prices,
//! the cost line, blockers and what pressing Draft or Generate does.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use jutsu_core::{
    Budget, BudgetDecision, Capabilities, Cost, CostUnit, JobSpec, Mode, ModelParameter, Pricing,
    ProviderKind, Resolution, SpendCap, Validation, ValidationIssue, VideoModel,
};
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use serde::Serialize;

use super::form::{CreateForm, MOTION_TASKS};
use crate::queue::QueueConfirmation;
use crate::queue::format;
use crate::queue::service::budgeted;

/// The spec as far as its price goes, as a key: the same without the prompt, which no provider prices by.
pub fn price_key(spec: &JobSpec) -> String {
    let mut key = spec.clone();
    key.prompt = String::new();
    serde_json::to_string(&key).unwrap_or_default()
}

/// The last complete totals of Draft and Generate, shown while a changed form is quoted again; never used to decide.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LastTotals {
    pub draft: Option<Cost>,
    pub generate: Option<Cost>,
}

impl LastTotals {
    pub fn get(&self, is_draft: bool) -> Option<&Cost> {
        if is_draft {
            self.draft.as_ref()
        } else {
            self.generate.as_ref()
        }
    }
}

/// The provider's quotes by price key, for one connection (`EstimateCache`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EstimateCache {
    pub costs: HashMap<String, Cost>,
    pub last_totals: LastTotals,
}

impl EstimateCache {
    /// The specs whose price key has no quote yet, each key once, in order.
    pub fn missing(&self, specs: &[JobSpec]) -> Vec<(String, JobSpec)> {
        let mut seen = BTreeSet::new();
        specs
            .iter()
            .filter_map(|spec| {
                let key = price_key(spec);
                (!self.costs.contains_key(&key) && seen.insert(key.clone())).then(|| (key, spec.clone()))
            })
            .collect()
    }

    /// Adds `fresh` quotes, then remembers the totals of `generate` and `draft` that are now fully known.
    pub fn record(&mut self, fresh: Vec<(String, Cost)>, generate: &[JobSpec], draft: &[JobSpec]) {
        self.costs.extend(fresh);
        if let Some(total) = self.total(generate) {
            self.last_totals.generate = Some(total);
        }
        if let Some(total) = self.total(draft) {
            self.last_totals.draft = Some(total);
        }
    }

    /// The sum over `specs`; `None` when any is unknown, or when there are none.
    pub fn total(&self, specs: &[JobSpec]) -> Option<Cost> {
        let known: Vec<Cost> = specs
            .iter()
            .filter_map(|spec| self.costs.get(&price_key(spec)).cloned())
            .collect();
        if known.len() == specs.len() {
            format::total(&known)
        } else {
            None
        }
    }
}

/// What the Create screen knows besides the form: the connection, prices and today's spend.
#[derive(Debug, Clone, Default)]
pub struct CreateContext {
    /// `None` while disconnected.
    pub connection: Option<ProviderKind>,
    /// Plan credits left, when a provider call has reported them.
    pub credits_left: Option<i64>,
    /// `None` while loading or when they couldn't be read; modes are then not restricted.
    pub capabilities: Option<Capabilities>,
    /// The bundled list prices.
    pub pricing: Option<Arc<Pricing>>,
    /// Quotes by price key.
    pub estimates: EstimateCache,
    /// Spend in the cap's unit of jobs finished today plus every job in flight.
    pub committed_today: Decimal,
    /// `None` without a daily cap.
    pub cap: Option<SpendCap>,
    /// The user opted in to paying from the plan's unlimited allowance.
    pub unlimited_opt_in: bool,
}

impl CreateContext {
    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    pub fn pays_unlimited(&self, spec: &JobSpec) -> bool {
        self.capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.pays_unlimited(spec, self.unlimited_opt_in))
    }

    /// Every one of `specs`, and there is at least one.
    pub fn pays_unlimited_all(&self, specs: &[JobSpec]) -> bool {
        !specs.is_empty() && specs.iter().all(|spec| self.pays_unlimited(spec))
    }

    pub fn cost_unit(&self) -> CostUnit {
        self.capabilities.as_ref().map_or(
            if self.connection == Some(ProviderKind::Account) {
                CostUnit::Credits
            } else {
                CostUnit::Usd
            },
            |capabilities| capabilities.cost_unit,
        )
    }

    /// Genjutsu alone until the capabilities are known.
    pub fn offered_models(&self) -> BTreeSet<VideoModel> {
        self.capabilities.as_ref().map_or_else(
            || BTreeSet::from([VideoModel::GENJUTSU]),
            |capabilities| capabilities.models.clone(),
        )
    }

    /// Whether a job on `model` in `mode` runs on this connection.
    pub fn runs(&self, model: &VideoModel, mode: Mode) -> bool {
        self.capabilities.as_ref().map_or_else(
            || self.offered_models().contains(model) && model.serves(mode),
            |capabilities| capabilities.runs(model, mode),
        )
    }

    /// The quote, else the list price for USD; `None` for credits until quoted. Nothing for a job the unlimited
    /// allowance pays.
    pub fn cost(&self, spec: &JobSpec) -> Option<Cost> {
        if self.pays_unlimited(spec) {
            return Some(Cost::credits(Decimal::ZERO, false));
        }
        if let Some(quote) = self.estimates.costs.get(&price_key(spec)) {
            return Some(quote.clone());
        }
        if self.cost_unit() != CostUnit::Usd {
            return None;
        }
        self.pricing.as_ref()?.cost(spec).ok()
    }

    /// The sum over `specs`; `None` when any isn't priced yet, or there are none. An unknown price makes the sum one.
    pub fn cost_of(&self, specs: &[JobSpec]) -> Option<Cost> {
        let mut costs = Vec::with_capacity(specs.len());
        for spec in specs {
            costs.push(self.cost(spec)?);
        }
        if let Some(unknown) = costs.iter().find(|cost| cost.is_unknown_price()) {
            return Some(unknown.clone());
        }
        format::total(&costs)
    }

    /// Every mode with why the connection can't run it; `None` when it can.
    pub fn mode_reason(&self, mode: Mode) -> Option<String> {
        if !self.is_connected() {
            return Some("Connect Higgsfield first".to_owned());
        }
        let Some(capabilities) = &self.capabilities else {
            return Some("Checking what this connection can run…".to_owned());
        };
        if capabilities.modes.contains(&mode) {
            return None;
        }
        Some(self.not_available().to_owned())
    }

    fn not_available(&self) -> &'static str {
        if self.connection == Some(ProviderKind::Account) {
            "Not available for this account"
        } else {
            "Not available with this API key"
        }
    }

    /// Why `mode` can't run on this connection; `None` while capabilities are unknown.
    pub fn unavailable_reason(&self, mode: Mode) -> Option<String> {
        self.capabilities.as_ref()?;
        self.mode_reason(mode)
    }

    /// Why this connection can't send `parameter`; `None` when it can, or while the capabilities are unknown.
    pub fn parameter_reason(&self, parameter: ModelParameter) -> Option<String> {
        let capabilities = self.capabilities.as_ref()?;
        if capabilities.parameters.contains(&parameter) {
            return None;
        }
        if self.connection != Some(ProviderKind::Account) {
            return Some(if parameter == ModelParameter::KlingVideoBackground {
                "Kling keeps the video's background only on a Higgsfield account, not with an API key"
                    .to_owned()
            } else {
                "Not available with this API key".to_owned()
            });
        }
        Some(
            match parameter {
                ModelParameter::SeedanceBitrate => {
                    "Seedance's bitrate is set with an API key; the account doesn't offer it yet"
                }
                ModelParameter::SeedanceFrames => {
                    "Seedance's start and end frames run with an API key; the account doesn't offer them yet"
                }
                ModelParameter::KlingOriginalSound => {
                    "Kling keeps the source's sound on the server only with an API key, not on the account"
                }
                ModelParameter::KlingOrientation => {
                    "Kling's character orientation is set only with an API key, not on the account"
                }
                ModelParameter::CinemaControls => {
                    "Cinema Studio's creative controls are set only with an API key, not on the account"
                }
                ModelParameter::KlingVideoBackground => "Not available for this account",
            }
            .to_owned(),
        )
    }

    /// `parameter_reason` short enough for a tooltip.
    pub fn parameter_hint(&self, parameter: ModelParameter) -> Option<String> {
        self.parameter_reason(parameter)?;
        let is_account = self.connection == Some(ProviderKind::Account);
        Some(
            match (parameter == ModelParameter::KlingVideoBackground, is_account) {
                (true, true) => "Not available for this account",
                (true, false) => "Only on a Higgsfield account",
                (false, true) => "Only with an API key",
                (false, false) => "Not available with this API key",
            }
            .to_owned(),
        )
    }
}

/// The line above the buttons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CostLine {
    /// "13 s × $0.681 = $8.85".
    pub text: String,
    /// "List price · failed runs are free".
    pub detail: String,
    /// The detail is a warning (over the cap).
    pub is_warning: bool,
}

/// What pressing Draft or Generate does.
#[derive(Debug, Clone, PartialEq)]
pub enum SubmitDecision {
    /// Not submittable; the reason is the blocker text.
    Blocked(String),
    /// Queue these jobs, in order.
    Start(Vec<JobSpec>),
    /// Over today's cap, or the price isn't known under one: ask once, then start them all.
    Confirm(Vec<JobSpec>, QueueConfirmation),
}

impl SubmitDecision {
    /// Start, or ask first when `cost` may take today past `cap`.
    pub fn budgeted(
        specs: Vec<JobSpec>,
        cost: Option<&Cost>,
        committed_today: Decimal,
        cap: Option<&SpendCap>,
    ) -> Self {
        let Some(cap) = cap else {
            return Self::Start(specs);
        };
        match budgeted(JobSpec::job_count(&specs), cost, committed_today, cap) {
            Some(confirmation) => Self::Confirm(specs, confirmation),
            None => Self::Start(specs),
        }
    }
}

/// One model of the picker: "Kling O3", "$0.126 / s · up to 10 s".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelMenuRow {
    pub id: String,
    pub title: String,
    pub detail: Option<String>,
    /// Shown but not pickable, with why (Intone, whose speech input isn't ported).
    pub unavailable_reason: Option<String>,
}

fn dollars_up(amount: Decimal, digits: u32) -> String {
    let rounded = amount.round_dp_with_strategy(digits, RoundingStrategy::AwayFromZero);
    format!("${rounded:.prec$}", prec = digits as usize)
}

/// "$0.681": three decimals.
pub fn rate_text(amount: Decimal) -> String {
    let rounded = amount.round_dp_with_strategy(3, RoundingStrategy::MidpointAwayFromZero);
    format!("${rounded:.3}")
}

impl CreateForm {
    /// Why the jobs make one variant whatever is picked; `None` when they make `variants`.
    pub fn variants_unavailable_reason(&self, context: &CreateContext) -> Option<String> {
        let capabilities = context.capabilities.as_ref()?;
        if capabilities.max_variants <= 1 {
            return Some("This connection makes one video per job".to_owned());
        }
        if self.mode() == Mode::Restyle {
            return Some("A style makes one video per run".to_owned());
        }
        if self.is_batch() && self.offers_batch() {
            return Some("A batch makes one video per style or character".to_owned());
        }
        if context.pays_unlimited_all(&self.make_specs(false)) {
            return Some("Unlimited generations make one video at a time".to_owned());
        }
        None
    }

    pub fn shows_variants(&self, context: &CreateContext) -> bool {
        context
            .capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.max_variants > 1)
    }

    /// The variants each job makes on this connection.
    pub fn variants_in(&self, context: &CreateContext) -> i64 {
        match &context.capabilities {
            Some(capabilities) if self.variants_unavailable_reason(context).is_none() => {
                self.variants().min(capabilities.max_variants)
            }
            _ => 1,
        }
    }

    /// The jobs Draft or Generate queues on this connection, each making `variants_in` variants.
    pub fn specs_in(&self, is_draft: bool, context: &CreateContext) -> Vec<JobSpec> {
        let count = self.variants_in(context);
        self.make_specs(is_draft)
            .into_iter()
            .map(|mut spec| {
                spec.variants = count;
                spec
            })
            .collect()
    }

    /// What the Motion tab can do with the picked model on this connection; shown when there is more than one.
    pub fn task_choices(&self, context: &CreateContext) -> Vec<Mode> {
        if self.tab() != Mode::MotionTransfer {
            return Vec::new();
        }
        let model = self.selected_model();
        MOTION_TASKS
            .into_iter()
            .filter(|task| {
                model.serves(*task) && (*task == self.mode() || context.unavailable_reason(*task).is_none())
            })
            .collect()
    }

    /// The models the picker offers for the tab, in catalog order.
    pub fn model_options(&self, context: &CreateContext) -> Vec<VideoModel> {
        let modes: Vec<Mode> = if self.tab() == Mode::MotionTransfer {
            MOTION_TASKS
                .into_iter()
                .filter(|task| *task == self.mode() || context.unavailable_reason(*task).is_none())
                .collect()
        } else {
            vec![self.mode()]
        };
        VideoModel::all()
            .into_iter()
            .filter(|model| modes.iter().any(|mode| context.runs(model, *mode)))
            .collect()
    }

    /// The picker's rows, each with its rate and, when the models' longest lengths differ, the longest.
    pub fn model_menu(&self, context: &CreateContext) -> Vec<ModelMenuRow> {
        let rows: Vec<(VideoModel, Option<String>, f64)> = self
            .model_options(context)
            .into_iter()
            .map(|model| {
                let mut form = self.clone();
                form.set_model(model.clone());
                let longest = form.limits().duration.upper();
                (model, form.rate_line(context), longest)
            })
            .collect();
        let shows_length = rows
            .iter()
            .any(|row| (row.2 - rows.first().map_or(row.2, |first| first.2)).abs() > f64::EPSILON);
        rows.into_iter()
            .map(|(model, rate, longest)| {
                let mut parts: Vec<String> = rate.into_iter().collect();
                if shows_length {
                    parts.push(format!("up to {}", format::limit(longest)));
                }
                let unavailable_reason = model
                    .declares(jutsu_core::VideoControl::Speech)
                    .then(|| "Speech recordings aren't supported here yet".to_owned());
                ModelMenuRow {
                    id: model.raw_value().to_owned(),
                    title: model.title(),
                    detail: (!parts.is_empty()).then(|| parts.join(" · ")),
                    unavailable_reason,
                }
            })
            .collect()
    }

    /// Why Draft or Generate can't be submitted; `None` when it can. Credits are checked against that action's cost.
    pub fn blocker(&self, is_draft: bool, context: &CreateContext) -> Option<String> {
        let generate = self.specs_in(false, context);
        let cost = context.cost_of(&self.specs_in(is_draft, context));
        self.blocker_with(is_draft, &generate, cost.as_ref(), context)
    }

    fn blocker_with(
        &self,
        is_draft: bool,
        specs: &[JobSpec],
        cost: Option<&Cost>,
        context: &CreateContext,
    ) -> Option<String> {
        if !context.is_connected() {
            return Some("Connect Higgsfield in Settings".to_owned());
        }
        if let Some(reason) = context.unavailable_reason(self.mode()) {
            return Some(reason);
        }
        let model = self.selected_model();
        if context.capabilities.is_some() && !context.runs(&model, self.mode()) {
            return Some(format!("{} isn't available on this connection", model.title()));
        }
        if self.source().is_none() && !self.is_without_source() {
            return Some("Add a source video".to_owned());
        }
        if self.uses_frames()
            && let Some(reason) = context.parameter_reason(ModelParameter::SeedanceFrames)
        {
            return Some(reason);
        }
        if let Some(reason) = self.frames_blocker() {
            return Some(reason);
        }
        if let Some(reason) = self.batch_blocker() {
            return Some(reason);
        }
        if specs.is_empty() {
            return Some("Add a source video".to_owned());
        }
        if let Some(reason) = self.validation_blocker(specs) {
            return Some(reason);
        }
        if let (Some(left), Some(Cost::Credits { amount, .. })) = (context.credits_left, cost)
            && *amount > Decimal::from(left)
        {
            return Some(if is_draft || !self.offers_draft() {
                "Not enough credits on your plan".to_owned()
            } else {
                "Not enough credits for Generate, try a draft".to_owned()
            });
        }
        None
    }

    /// Every validation issue of `specs` as an action, in order, each once.
    pub fn validation_issues(&self, specs: &[JobSpec]) -> Vec<String> {
        let constraints = self.constraints();
        let mut texts: Vec<String> = Vec::new();
        for spec in specs {
            for issue in Validation::check(spec, &constraints) {
                let text = self.issue_text(&issue);
                if !texts.contains(&text) {
                    texts.push(text);
                }
            }
        }
        texts
    }

    /// The first job's validation problem as the cost line shows it; a character set's problem names the set.
    pub fn validation_blocker(&self, specs: &[JobSpec]) -> Option<String> {
        let constraints = self.constraints();
        specs.iter().enumerate().find_map(|(index, spec)| {
            let issue = Validation::check(spec, &constraints).into_iter().next()?;
            let text = self.issue_text(&issue);
            Some(if self.is_batch() && self.mode() != Mode::Restyle {
                format!("Character {}: {text}", index + 1)
            } else {
                text
            })
        })
    }

    /// The issue as an action, with the selected model's limits.
    pub fn issue_text(&self, issue: &ValidationIssue) -> String {
        let model = self.selected_model();
        let limits = self.limits();
        match issue {
            ValidationIssue::SelectionOutsideSource => "Pick a range inside the video".to_owned(),
            ValidationIssue::SourceRequired => "Add a source video".to_owned(),
            ValidationIssue::PromptRequired => match self.mode() {
                Mode::Edit => "Say what should change".to_owned(),
                Mode::Extend => "Say what happens next".to_owned(),
                _ if model.descriptor().requires_prompt => "Write a prompt".to_owned(),
                _ => "Write a prompt or add an image".to_owned(),
            },
            ValidationIssue::InvalidFrameSize(_) => "Can't read the video's frame size".to_owned(),
            ValidationIssue::DurationTooShort(_) => {
                format!(
                    "Pick at least {} of video",
                    format::limit(limits.duration.lower())
                )
            }
            ValidationIssue::DurationTooLong(_) => {
                format!("Pick at most {} of video", format::limit(limits.duration.upper()))
            }
            ValidationIssue::TooFewReferences { minimum, .. } if *minimum == 1 => {
                "Add at least one image".to_owned()
            }
            ValidationIssue::TooFewReferences { minimum, .. } => format!("Add at least {minimum} images"),
            ValidationIssue::TooManyReferences { maximum, .. } => {
                match (model == VideoModel::GENJUTSU, *maximum == 1) {
                    (true, true) => "Remove images, this mode takes one".to_owned(),
                    (true, false) => format!("Remove images, this mode takes up to {maximum}"),
                    (false, true) => format!("Remove images, {} takes one", model.title()),
                    (false, false) => format!("Remove images, {} takes up to {maximum}", model.title()),
                }
            }
            ValidationIssue::PresetRequired => "Pick a style".to_owned(),
            ValidationIssue::FrameTooSmall(_) => "Object swap needs 640×640 px, allow the upscale".to_owned(),
            ValidationIssue::PromptTooLong { .. } => "Shorten the prompt to 10 000 characters".to_owned(),
            ValidationIssue::SpeechRequired => "Add a speech recording".to_owned(),
            ValidationIssue::SpeechTooShort(_) => {
                format!(
                    "Use a speech of at least {}",
                    format::limit(self.speech_seconds().lower())
                )
            }
            ValidationIssue::SpeechTooLong(_) => {
                format!(
                    "Use a speech of at most {}",
                    format::limit(self.speech_seconds().upper())
                )
            }
            ValidationIssue::SourceNotTaken => {
                format!("Remove the source video, {} takes none", model.title())
            }
        }
    }

    fn rate_at(&self, resolution: Resolution, context: &CreateContext) -> Option<Decimal> {
        let uses_frames = self.uses_frames();
        context
            .pricing
            .as_ref()?
            .rate(
                &self.selected_model(),
                self.mode(),
                resolution,
                self.source().is_some(),
                !self.references().is_empty() || uses_frames,
                uses_frames,
            )
            .ok()
    }

    fn is_upper_bound_at(&self, resolution: Resolution, context: &CreateContext) -> bool {
        let uses_frames = self.uses_frames();
        context.pricing.as_ref().is_some_and(|pricing| {
            pricing.is_upper_bound(
                &self.selected_model(),
                self.mode(),
                resolution,
                self.source().is_some(),
                !self.references().is_empty() || uses_frames,
                uses_frames,
            )
        })
    }

    /// The price on a button: "$8.85", "96 cr"; the last known one while a changed form is quoted again.
    pub fn price_text(&self, is_draft: bool, context: &CreateContext) -> Option<String> {
        let specs = self.specs_in(is_draft, context);
        if context.pays_unlimited_all(&specs) {
            return Some(format::UNLIMITED.to_owned());
        }
        let cost = context
            .cost_of(&specs)
            .or_else(|| context.estimates.last_totals.get(is_draft).cloned())?;
        if let Cost::Usd {
            amount,
            is_estimate: true,
        } = &cost
            && self.is_upper_bound_at(self.job_resolution(is_draft), context)
        {
            return Some(dollars_up(*amount, 2));
        }
        Some(format::cost_text(&cost))
    }

    /// "$0.681 / s" next to the resolution picker, "up to $0.463 / s" for the top of a published range; `None` for
    /// credits.
    pub fn rate_line(&self, context: &CreateContext) -> Option<String> {
        let resolution = self.job_resolution(false);
        if context.cost_unit() != CostUnit::Usd {
            return None;
        }
        let rate = self.rate_at(resolution, context)?;
        if self.is_upper_bound_at(resolution, context) {
            return Some(format!("up to {} / s", dollars_up(rate, 3)));
        }
        Some(format!("{} / s", rate_text(rate)))
    }

    /// "5.0 s · $1.59" next to the draft length picker.
    pub fn draft_length_detail(&self, context: &CreateContext) -> String {
        let length = super::form::seconds_text(self.draft_duration());
        match self.price_text(true, context) {
            Some(price) => format!("{length} · {price}"),
            None => length,
        }
    }

    /// The cost of Generate over all ranges, or of the drafts of a batch, which runs as drafts by default
    /// (`runs_as_draft`); `None` without anything to queue.
    pub fn cost_line(&self, context: &CreateContext) -> Option<CostLine> {
        let is_draft = self.runs_as_draft();
        let specs = self.specs_in(is_draft, context);
        if specs.is_empty() {
            return None;
        }
        let exact = context.cost_of(&specs);
        let billed: i64 = specs
            .iter()
            .map(|spec| Pricing::billed_seconds_on(spec, context.connection) * spec.variants)
            .sum();
        let jobs = JobSpec::job_count(&specs);
        let mut free: Vec<String> = Vec::new();
        if jobs > 1 {
            free.push(format!("{jobs} jobs"));
        }
        free.push("failed runs are free".to_owned());
        let detail = |first: Vec<String>| {
            first
                .into_iter()
                .chain(free.iter().cloned())
                .collect::<Vec<_>>()
                .join(" · ")
        };

        if context.pays_unlimited_all(&specs) {
            return Some(CostLine {
                text: format!("{billed} s · {}", format::UNLIMITED),
                detail: "Plan's unlimited generations · not counted in the daily cap".to_owned(),
                is_warning: false,
            });
        }
        if exact.as_ref().is_some_and(Cost::is_unknown_price) {
            return Some(CostLine {
                text: format!("{billed} s · {}", format::PRICE_UNKNOWN),
                detail: detail(vec![
                    "Higgsfield quotes it once the files are uploaded".to_owned(),
                ]),
                is_warning: false,
            });
        }
        let is_updating = exact.is_none();
        let cost = exact.or_else(|| context.estimates.last_totals.get(is_draft).cloned());
        let is_over_cap = cost.as_ref().is_some_and(|cost| {
            !matches!(
                Budget::allows(cost, context.committed_today, context.cap.as_ref()),
                BudgetDecision::Allowed
            )
        });
        let over_cap = "Over today's cap, you'll be asked to confirm".to_owned();
        match cost {
            Some(Cost::Usd { amount, is_estimate }) => {
                let (text, status) = self.usd_line(billed, amount, is_estimate, context);
                if is_over_cap {
                    return Some(CostLine {
                        text,
                        detail: over_cap,
                        is_warning: true,
                    });
                }
                Some(CostLine {
                    text,
                    detail: detail(vec![status]),
                    is_warning: false,
                })
            }
            Some(Cost::Credits { amount, is_estimate }) => {
                let text = format!("{billed} s = {} credits", format::exact_credits(amount));
                if is_over_cap {
                    return Some(CostLine {
                        text,
                        detail: over_cap,
                        is_warning: true,
                    });
                }
                let status = if is_updating {
                    "Updating the price"
                } else if is_estimate {
                    "Estimate, Higgsfield quotes it after upload"
                } else {
                    "Price from Higgsfield"
                };
                let mut first = vec![status.to_owned()];
                if let Some(left) = context.credits_left {
                    first.push(format!("{left} cr left on your plan"));
                }
                Some(CostLine {
                    text,
                    detail: detail(first),
                    is_warning: false,
                })
            }
            None => Some(CostLine {
                text: format!("{billed} s · estimating…"),
                detail: detail(vec!["Estimate".to_owned()]),
                is_warning: false,
            }),
        }
    }

    fn usd_line(
        &self,
        billed: i64,
        amount: Decimal,
        is_estimate: bool,
        context: &CreateContext,
    ) -> (String, String) {
        let resolution = self.job_resolution(self.runs_as_draft());
        if is_estimate && self.is_upper_bound_at(resolution, context) {
            return (
                format!("{billed} s = up to {}", dollars_up(amount, 2)),
                "Top of Higgsfield's published range, quoted after upload".to_owned(),
            );
        }
        let total = format::dollars(amount);
        if is_estimate {
            return (format!("{billed} s = {total}"), "Estimate".to_owned());
        }
        if !self.selected_model().descriptor().list_price_is_exact {
            return (
                format!("{billed} s = {total}"),
                "Price from Higgsfield".to_owned(),
            );
        }
        let text = self.rate_at(resolution, context).map_or_else(
            || format!("{billed} s = {total}"),
            |rate| format!("{billed} s × {} = {total}", rate_text(rate)),
        );
        (text, "List price".to_owned())
    }

    /// What pressing Draft or Generate does.
    pub fn submit_decision(&self, is_draft: bool, context: &CreateContext) -> SubmitDecision {
        if let Some(reason) = self.blocker(is_draft, context) {
            return SubmitDecision::Blocked(reason);
        }
        let specs = self.specs_in(is_draft, context);
        if specs.is_empty() {
            return SubmitDecision::Blocked("Add a source video".to_owned());
        }
        let cost = context.cost_of(&specs);
        SubmitDecision::budgeted(
            specs,
            cost.as_ref(),
            context.committed_today,
            context.cap.as_ref(),
        )
    }
}
