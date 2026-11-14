//! The agents' `create_image` requests (`AgentImageQueue.swift`), sent through Generate's own runs
//! (`ReferenceGenerator::submit_for_agent`) with the same guarantees as an agent's video job. Each request is recorded
//! before anything happens. Right before the submit, once the input is uploaded, the guard reads the provider's quote
//! (else the model's list price, an estimate), the person's own daily cap strictly, the agent's `max_cost` and today's
//! agent budget (`AgentSpendDecision`), then reserves the price in the agent spend ledger and saves the record as
//! possibly sent. Once the person approved it, a price above the one they approved holds it again. Over any limit,
//! with an unknown price or a ledger that can't be written, nothing is sent and the request is held in the Queue for
//! the person. A reservation is released only when the submit certainly never went out (stopped before it, or
//! refused by the provider); a submit cut off or unanswered keeps it, as does a cancel the provider refused. Once
//! every job of an accepted request has ended, the jobs Higgsfield doesn't bill (failed, refused by moderation,
//! cancelled) stop counting their share, so the request counts what its images that were made cost.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use jutsu_core::{
    AgentMoney, AgentOrigin, AgentSpend, AgentSpendContext, AgentSpendDecision, AgentSpendLedger, Budget,
    BudgetDecision, Cost, ImageProvider, ImageRequest, PortError, ProviderError, ProviderKind,
};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::image_record::{
    AgentImageHold, AgentImageOrder, AgentImageRecord, AgentImageRefusal, AgentImageState, LOST_ANSWER,
    NOT_RECORDED, hold_text,
};
use super::job::newer_first;
use crate::images::generator::{
    AgentImageSubmit, AgentSubmitOutcome, ImageRun, ReferenceGenerator, RunState, UploadFn, message,
};
use crate::images::job_log::ImageJobState;
use crate::images::log_file::{LogFile, LogWriteError};

/// The person's daily agent budget for a connection, in its unit.
pub type DailyLimit = Arc<dyn Fn(ProviderKind) -> i64 + Send + Sync>;
/// The clock.
pub type Now = Arc<dyn Fn() -> OffsetDateTime + Send + Sync>;

const KEPT_DAYS: i64 = 30;

/// The agents' image requests and their records.
pub struct AgentImageQueue {
    me: Weak<AgentImageQueue>,
    generator: Arc<ReferenceGenerator>,
    ledger: Arc<dyn AgentSpendLedger>,
    daily_limit: DailyLimit,
    now: Now,
    file: LogFile<AgentImageRecord>,
    loaded: Mutex<Option<Vec<AgentImageRecord>>>,
}

impl std::fmt::Debug for AgentImageQueue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AgentImageQueue").finish_non_exhaustive()
    }
}

/// A job Higgsfield doesn't bill: it failed, was refused by moderation or was cancelled.
fn is_unbilled(state: &ImageJobState) -> bool {
    matches!(
        state,
        ImageJobState::Failed { .. } | ImageJobState::Rejected {} | ImageJobState::Canceled {}
    )
}

impl AgentImageQueue {
    /// `folder`: `None` keeps it in memory, for tests. The queue hears how each of its runs ends from `generator`.
    pub fn new(
        folder: Option<&Path>,
        generator: Arc<ReferenceGenerator>,
        ledger: Arc<dyn AgentSpendLedger>,
        daily_limit: DailyLimit,
        now: Now,
    ) -> Arc<Self> {
        let queue = Arc::new_cyclic(|me| Self {
            me: me.clone(),
            generator,
            ledger,
            daily_limit,
            now,
            file: LogFile::new(folder, "agent-images.json"),
            loaded: Mutex::new(None),
        });
        let weak = Arc::downgrade(&queue);
        queue.generator.set_on_run_ended(Arc::new(move |id, state| {
            if let Some(queue) = weak.upgrade() {
                queue.ended(id, state);
            }
        }));
        queue
    }

    fn locked(&self) -> MutexGuard<'_, Option<Vec<AgentImageRecord>>> {
        self.loaded.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // MARK: Records

    /// Every request kept, newest first. Fails when the file can't be read: nothing is taken then.
    pub fn records(&self) -> Result<Vec<AgentImageRecord>, LogWriteError> {
        let mut records = self.load()?;
        records.sort_by(|a, b| newer_first(&(a.created_at, a.id), &(b.created_at, b.id)));
        Ok(records)
    }

    pub fn record(&self, id: Uuid) -> Option<AgentImageRecord> {
        self.load().ok()?.into_iter().find(|record| record.id == id)
    }

    fn load(&self) -> Result<Vec<AgentImageRecord>, LogWriteError> {
        let mut loaded = self.locked();
        if let Some(records) = loaded.as_ref() {
            return Ok(records.clone());
        }
        let records = self.file.read_strict()?;
        *loaded = Some(records.clone());
        Ok(records)
    }

    fn save(&self, record: AgentImageRecord) -> Result<(), LogWriteError> {
        let mut records = self.load()?;
        records.retain(|kept| kept.id != record.id);
        records.push(record);
        let cutoff = (self.now)() - Duration::days(KEPT_DAYS);
        records.retain(|kept| !(kept.is_over() && kept.finished_at.unwrap_or(kept.created_at) < cutoff));
        self.file.write(&records)?;
        *self.locked() = Some(records);
        Ok(())
    }

    /// Saves a change that isn't money: a write that fails keeps it for this launch.
    fn update(&self, id: Uuid, change: impl FnOnce(&mut AgentImageRecord)) {
        let Some(mut record) = self.record(id) else {
            return;
        };
        change(&mut record);
        if let Err(error) = self.save(record.clone()) {
            tracing::error!(%id, %error, "an agent image request's change can't be saved");
            if let Some(records) = self.locked().as_mut()
                && let Some(kept) = records.iter_mut().find(|kept| kept.id == id)
            {
                *kept = record;
            }
        }
    }

    // MARK: Requests

    /// Takes a request: checked as the form would check it, recorded, then sent through the guard. Returns its
    /// record. `id` is the run's ID, chosen by the caller and recorded with the agent's `request_id` first.
    pub fn create(
        &self,
        order: &AgentImageOrder,
        id: Uuid,
        origin: AgentOrigin,
        provider: Arc<dyn ImageProvider>,
        upload: UploadFn,
    ) -> Result<AgentImageRecord, AgentImageRefusal> {
        let form = self.generator.form();
        let draft = order.draft(form.models(), form.models_are_saved())?;
        let record =
            AgentImageRecord::taken(id, origin, provider.kind(), provider.owner(), draft, (self.now)());
        self.save(record.clone()).map_err(|error| AgentImageRefusal {
            message: error.to_string(),
            is_not_saved: true,
        })?;
        self.show_run(&record, RunState::Running);
        self.send(&record, provider, upload);
        Ok(record)
    }

    /// The person's Send on a held request: it goes out at the price they saw, past the agent's limits but still
    /// under their own daily cap unless that price was over it too. Nothing happens to a request that isn't held.
    pub fn approve(&self, id: Uuid, provider: Arc<dyn ImageProvider>, upload: UploadFn) {
        let Some(mut record) = self.record(id) else {
            return;
        };
        if record.state != AgentImageState::Held
            || record.may_have_been_sent()
            || record.provider_kind != provider.kind()
            || record.owner != provider.owner()
        {
            return;
        }
        record.origin.approved_by_person = true;
        record.approved_price.clone_from(&record.price);
        record.hold = None;
        record.state = AgentImageState::Pricing;
        if let Err(error) = self.save(record.clone()) {
            return self.generator.finish(id, RunState::Failed(error.to_string()));
        }
        self.show_run(&record, RunState::Running);
        self.send(&record, provider, upload);
    }

    /// Stops a request. One held or still being priced was never sent; one sent is cancelled as Generate cancels its
    /// own, and its reservation stays. Returns `false` for a request that is already over.
    pub fn cancel(&self, id: Uuid) -> bool {
        let Some(record) = self.record(id) else {
            return false;
        };
        if record.is_over() {
            return false;
        }
        if record.state == AgentImageState::Held
            && !self.generator.is_submitting(id)
            && !self.generator.has_run(id)
        {
            // Held at an earlier launch and not shown again: nothing runs it.
            self.generator.report_ended(id, &RunState::Canceled);
        } else {
            self.generator.cancel(id);
        }
        true
    }

    /// At launch, before anything is followed again: gives back reservations a crash orphaned (saved to the ledger,
    /// never saved with their request, so never sent), fails a request that was being sent and whose answer was lost
    /// (its reservation stays), and shows the held ones again.
    pub async fn restore(&self) {
        let records = match self.load() {
            Ok(records) => records,
            Err(error) => {
                tracing::error!(%error, "the agent image requests can't be read to restore them");
                return;
            }
        };
        match self.ledger.unsettled().await {
            Ok(spends) => {
                for spend in spends {
                    let orphaned = records.iter().any(|record| {
                        record.id == spend.job_id && record.origin.reservation != Some(spend.id)
                    });
                    if orphaned {
                        self.release(spend.id, spend.job_id).await;
                    }
                }
            }
            // Reservations kept, which only counts too much.
            Err(error) => {
                tracing::error!(%error, "the agent spend ledger can't be read to reconcile image requests");
            }
        }
        let followed = self.generator.job_log().records();
        for record in records {
            let is_followed = followed.iter().any(|job| job.run_id == record.id);
            match record.state {
                AgentImageState::Held => {
                    let hold = record
                        .hold
                        .unwrap_or(AgentImageHold::Agent(jutsu_core::AgentHold::UnknownPrice));
                    self.show_run(&record, RunState::Held(hold_text(hold).to_owned()));
                }
                AgentImageState::Sending if is_followed => {
                    self.update(record.id, |record| record.state = AgentImageState::Following);
                }
                AgentImageState::Sending => {
                    let now = (self.now)();
                    self.update(record.id, |record| {
                        record.state = AgentImageState::Failed;
                        record.error = Some(LOST_ANSWER.to_owned());
                        record.finished_at = Some(now);
                    });
                }
                AgentImageState::Following if !is_followed => {
                    // Its jobs ended and were forgotten before the record heard: its images are in Library › Images.
                    let now = (self.now)();
                    self.update(record.id, |record| {
                        record.state = AgentImageState::Finished;
                        record.finished_at = Some(now);
                    });
                }
                _ => {}
            }
        }
    }

    /// A connection: requests of its account that were taken but never priced are sent now, and the requests it
    /// follows again show which agent asked for them. Call after `ReferenceGenerator::resume`.
    pub fn resume(&self, provider: &Arc<dyn ImageProvider>, upload: &UploadFn) {
        let records = match self.load() {
            Ok(records) => records,
            Err(error) => {
                tracing::error!(%error, "the agent image requests can't be read to resume them");
                return;
            }
        };
        for record in records
            .iter()
            .filter(|record| record.provider_kind == provider.kind() && record.owner == provider.owner())
        {
            match record.state {
                AgentImageState::Pricing if !self.generator.is_submitting(record.id) => {
                    self.show_run(record, RunState::Running);
                    self.send(record, provider.clone(), upload.clone());
                }
                AgentImageState::Following | AgentImageState::Held => {
                    self.generator.update_run(record.id, |run| {
                        run.agent = Some(record.origin.clone());
                        run.price.clone_from(&record.price);
                    });
                }
                _ => {}
            }
        }
    }

    /// The images a request made that are saved in Library › Images, as far as this session's results know.
    pub fn files(&self, id: Uuid) -> Vec<PathBuf> {
        self.generator
            .images()
            .into_iter()
            .filter(|image| image.variant_group_id == Some(id))
            .filter_map(|image| image.file)
            .collect()
    }

    // MARK: The guard around each submit

    fn show_run(&self, record: &AgentImageRecord, state: RunState) {
        let mut run = ImageRun::new(record.id, record.model.title.clone(), record.count, state);
        run.agent = Some(record.origin.clone());
        run.price.clone_from(&record.price);
        run.name.clone_from(&record.title);
        self.generator.show_run(run);
    }

    fn send(&self, record: &AgentImageRecord, provider: Arc<dyn ImageProvider>, upload: UploadFn) {
        let id = record.id;
        let (clearing, answering) = (self.me.clone(), self.me.clone());
        let guard_provider = provider.clone();
        let hook = AgentImageSubmit {
            clear: Arc::new(move |request| {
                let (queue, provider) = (clearing.upgrade(), guard_provider.clone());
                Box::pin(async move {
                    match queue {
                        Some(queue) => queue.clear(id, request, provider.as_ref()).await,
                        None => None,
                    }
                })
            }),
            answered: Arc::new(move |outcome| {
                let queue = answering.upgrade();
                Box::pin(async move {
                    if let Some(queue) = queue {
                        queue.answered(id, outcome).await;
                    }
                })
            }),
        };
        self.generator
            .submit_for_agent(id, record.draft(), provider, upload, hook);
    }

    /// The guard, right before the submit: the price to send at, or `None` once the request is held or failed.
    async fn clear(&self, id: Uuid, request: ImageRequest, provider: &dyn ImageProvider) -> Option<Cost> {
        let Some(record) = self.record(id) else {
            self.generator
                .finish(id, RunState::Failed(NOT_RECORDED.to_owned()));
            return None;
        };
        let (price, is_known) = self.quote_or_fail(&request, &record, provider).await?;
        let status = match self.generator.spend_status(provider.kind()).await {
            Ok(status) => status,
            Err(error) => {
                self.generator.finish(id, RunState::Failed(error.to_string()));
                return None;
            }
        };
        if let Some(status) = status
            && let Some(cap) = status.cap.as_ref()
            && matches!(
                Budget::allows(&price, status.committed_today, Some(cap)),
                BudgetDecision::NeedsConfirmation { .. }
            )
            && !Budget::covers(record.approved_price.as_ref(), &price)
        {
            return self.hold(id, AgentImageHold::PriceAboveCap, price, is_known);
        }
        // Image requests are always sent without the plan's unlimited allowance (`use_unlim` false), so each one
        // counts against the budget like any paid request and Let agents use unlimited generations never applies.
        let context = AgentSpendContext {
            pays_unlimited: false,
            allows_unlimited: false,
            approved: record.approved_price.clone(),
        };
        let decision = AgentSpendDecision::decide(
            &price,
            is_known,
            &record.origin,
            (self.daily_limit)(provider.kind()),
            &context,
        );
        match decision {
            AgentSpendDecision::Hold(reason) => self.hold(id, AgentImageHold::Agent(reason), price, is_known),
            AgentSpendDecision::Send { money, limit } => {
                self.reserve(record, money, limit, price, is_known).await
            }
        }
    }

    /// The provider's quote for the request, else the model's list price as an estimate; and whether it is exact. A
    /// quote that turns the request down, such as more input images than a model whose catalog states no limit
    /// takes, fails the request (`None`): it comes before anything is reserved or sent, so the refusal costs
    /// nothing.
    async fn quote_or_fail(
        &self,
        request: &ImageRequest,
        record: &AgentImageRecord,
        provider: &dyn ImageProvider,
    ) -> Option<(Cost, bool)> {
        match provider.image_estimate(request).await {
            Ok(price) => {
                let known = !price.is_estimate() && !price.is_unknown_price();
                Some((price, known))
            }
            Err(error @ PortError::Provider(ProviderError::Validation { .. })) => {
                self.generator
                    .finish(record.id, RunState::Failed(message(&error, provider.kind())));
                None
            }
            Err(_) => Some((record.list_price().unwrap_or(Cost::UNKNOWN_CREDITS), false)),
        }
    }

    /// Reserves the price in the ledger, then saves the request as possibly sent; held or failed (`None`) otherwise.
    async fn reserve(
        &self,
        record: AgentImageRecord,
        money: AgentMoney,
        limit: Option<i64>,
        price: Cost,
        is_known: bool,
    ) -> Option<Cost> {
        let id = record.id;
        let spend = AgentSpend::new(Uuid::new_v4(), id, money, (self.now)(), false);
        let reservation = spend.id;
        match self.ledger.reserve(spend, limit).await {
            Ok(true) => {}
            Ok(false) => {
                return self.hold(
                    id,
                    AgentImageHold::Agent(jutsu_core::AgentHold::OverDailyBudget),
                    price,
                    is_known,
                );
            }
            Err(error) => {
                tracing::error!(%id, %error, "the agent spend ledger can't be written for an image request");
                return self.hold(
                    id,
                    AgentImageHold::Agent(jutsu_core::AgentHold::LedgerUnavailable),
                    price,
                    is_known,
                );
            }
        }
        let mut sending = record;
        sending.origin.reservation = Some(reservation);
        sending.sent_at = Some((self.now)());
        sending.state = AgentImageState::Sending;
        sending.price = Some(price.clone());
        sending.price_is_known = is_known;
        if let Err(error) = self.save(sending) {
            // Not recorded as sent, so it isn't sent.
            tracing::error!(%id, %error, "an agent image request can't be recorded as sent");
            self.release(reservation, id).await;
            self.generator.finish(id, RunState::Failed(error.to_string()));
            return None;
        }
        let shown = price.clone();
        self.generator.update_run(id, |run| run.price = Some(shown));
        Some(price)
    }

    fn hold(&self, id: Uuid, reason: AgentImageHold, price: Cost, is_known: bool) -> Option<Cost> {
        let held = price.clone();
        self.update(id, |record| {
            record.state = AgentImageState::Held;
            record.hold = Some(reason);
            record.price = Some(held);
            record.price_is_known = is_known;
        });
        self.generator.update_run(id, |run| {
            run.state = RunState::Held(hold_text(reason).to_owned());
            run.price = Some(price);
        });
        None
    }

    async fn answered(&self, id: Uuid, outcome: AgentSubmitOutcome) {
        let Some(reservation) = self.record(id).and_then(|record| record.origin.reservation) else {
            return;
        };
        match outcome {
            AgentSubmitOutcome::Accepted => {
                self.update(id, |record| record.state = AgentImageState::Following);
                if let Err(error) = self.ledger.settle(reservation, None).await {
                    // Still reserved, which counts the same.
                    tracing::error!(%id, %error, "an agent image request's charge can't be recorded");
                }
            }
            AgentSubmitOutcome::Refused | AgentSubmitOutcome::NotSent => {
                // Certainly never sent: its reservation is given back.
                if !self.release(reservation, id).await {
                    return;
                }
                self.update(id, |record| {
                    record.origin.reservation = None;
                    record.sent_at = None;
                });
            }
            AgentSubmitOutcome::Unknown => {}
        }
    }

    /// A run ended or stopped being followed.
    fn ended(&self, id: Uuid, state: &RunState) {
        let Some(record) = self.record(id) else {
            return;
        };
        if record.is_over() {
            return;
        }
        let now = (self.now)();
        let (new, error) = match state {
            RunState::Finished => (AgentImageState::Finished, None),
            RunState::Failed(message) => (AgentImageState::Failed, Some(message.clone())),
            RunState::Canceled => (AgentImageState::Canceled, None),
            RunState::Running | RunState::Held(_) | RunState::Interrupted(_) => return,
        };
        self.settle(&record);
        self.update(id, |record| {
            record.state = new;
            if error.is_some() {
                record.error = error;
            }
            record.finished_at = Some(now);
        });
    }

    /// Settles an accepted request's reservation to what its jobs were really charged once they all ended: Higgsfield
    /// doesn't bill a job that failed, was refused by moderation or was cancelled, so each of those stops counting
    /// its share. The full reservation stays while any job's outcome is unknown: still pending when following
    /// stopped, or a cancel the provider refused. Read from the job log before the run forgets it.
    fn settle(&self, record: &AgentImageRecord) {
        let Some(reservation) = record.origin.reservation else {
            return;
        };
        let jobs: Vec<_> = self
            .generator
            .job_log()
            .records()
            .into_iter()
            .filter(|job| job.run_id == record.id)
            .collect();
        if jobs.is_empty() || jobs.iter().any(|job| job.state == ImageJobState::Pending {}) {
            return;
        }
        let unbilled: Vec<String> = jobs
            .iter()
            .filter(|job| is_unbilled(&job.state))
            .map(|job| job.id().to_owned())
            .collect();
        if unbilled.is_empty() {
            return;
        }
        let parts = i64::try_from(jobs.len()).unwrap_or(i64::MAX);
        let (ledger, id) = (self.ledger.clone(), record.id);
        tokio::spawn(async move {
            // A ledger that can't be written keeps the reservation, which only counts too much.
            for part in unbilled {
                if let Err(error) = ledger.unbill(reservation, part, parts).await {
                    tracing::error!(%id, %error, "an unbilled image job can't be recorded");
                }
            }
        });
    }

    /// Gives back a reservation whose submit certainly never went out; `false` when the ledger can't be written,
    /// which keeps it and only counts too much.
    async fn release(&self, reservation: Uuid, id: Uuid) -> bool {
        match self.ledger.release(reservation).await {
            Ok(()) => true,
            Err(error) => {
                tracing::error!(%id, %error, "an agent image request's reservation can't be released");
                false
            }
        }
    }
}

#[cfg(test)]
#[path = "image_queue_tests.rs"]
mod tests;
