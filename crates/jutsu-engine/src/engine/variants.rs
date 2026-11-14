//! One submit of several variants (`JobSpec::variants`) becomes one job per variant the answer started.

use jutsu_core::{Cost, Job, JobState, RemoteInputs, RemoteJob};
use uuid::Uuid;

use super::JobEngine;

/// What the job that sent a submit of variants keeps for itself: its share of the request's price, the batch its
/// variants joined, and how many variants the answer didn't start.
#[derive(Debug, Clone)]
pub(crate) struct VariantShare {
    cost: Cost,
    server_price: Option<Cost>,
    approved_price: Option<Cost>,
    batch_id: Option<Uuid>,
    variant_group_id: Option<Uuid>,
    missing: Option<i64>,
    /// The jobs the submit started, when more than one: an agent's job is one share of its reservation.
    parts: Option<i64>,
}

impl VariantShare {
    /// From here on the job is one variant: a retry of it makes one.
    pub(crate) fn apply(&self, job: &mut Job) {
        job.cost = self.cost.clone();
        job.server_price.clone_from(&self.server_price);
        job.approved_price.clone_from(&self.approved_price);
        job.batch_id = self.batch_id;
        job.variant_group_id = self.variant_group_id;
        job.missing_variants = self.missing;
        if let Some(parts) = self.parts
            && let Some(agent) = job.agent.as_mut()
        {
            agent.paid_parts = Some(parts);
        }
        job.spec.variants = 1;
    }
}

/// A variant past the ones asked for is still a paid job; it takes a share like the last one.
fn share(parts: &[Cost], index: usize, fallback: &Cost) -> Cost {
    parts
        .get(index.min(parts.len().saturating_sub(1)))
        .cloned()
        .unwrap_or_else(|| fallback.clone())
}

impl JobEngine {
    /// Makes a job of each further variant the submit of `sender` started (`others`) and returns what `sender`
    /// keeps, for its `Submitted`. Every variant was paid for, so each is saved and polled before `sender` takes its
    /// own answer: a crash in between leaves them tracked, and `sender`'s unanswered submit fails as unknown without
    /// being sent again. A variant that can't be saved still runs and is saved with its next change.
    ///
    /// The request's price is for every variant it asked for; each job takes an even share of it (`Cost::split`), so
    /// the jobs together count what the variants cost. Variants the answer didn't start count nothing and are noted
    /// on `sender` (`Job::missing_variants`).
    pub(crate) async fn start_variants(
        &self,
        others: &[RemoteJob],
        sender: &Job,
        inputs: &RemoteInputs,
        paid_with_unlimited: bool,
    ) -> VariantShare {
        let asked = sender.spec.variants.max(1);
        let started = 1 + i64::try_from(others.len()).unwrap_or(i64::MAX - 1);
        let costs = sender.cost.split(asked);
        let quotes = sender.server_price.as_ref().map(|price| price.split(asked));
        let approvals = sender.approved_price.as_ref().map(|price| price.split(asked));
        let batch_id = sender
            .batch_id
            .or_else(|| (!others.is_empty()).then(Uuid::new_v4));
        let group = sender.variant_group_id.unwrap_or(sender.idempotency_key);
        let now = self.now();
        for (offset, remote) in others.iter().enumerate() {
            let index = offset + 1;
            let mut spec = sender.spec.clone();
            spec.variants = 1;
            if !paid_with_unlimited {
                spec.uses_unlimited = false;
            }
            let mut job = Job::new(
                Uuid::new_v4(),
                spec,
                sender.provider_kind,
                share(&costs, index, &sender.cost),
                Uuid::new_v4(),
                sender.created_at,
            );
            job.owner.clone_from(&sender.owner);
            job.parent_id = sender.parent_id;
            job.batch_id = batch_id;
            job.state = JobState::Queued;
            job.variant_group_id = Some(group);
            job.server_price = quotes.as_ref().map(|quotes| share(quotes, index, &sender.cost));
            job.approved_price = approvals
                .as_ref()
                .map(|approvals| share(approvals, index, &sender.cost));
            job.inputs = Some(inputs.clone());
            job.remote = Some(remote.clone());
            job.submitted_at = Some(now);
            // The sender's reservation pays for the whole submit; this variant is one share of it.
            job.agent = sender.agent.clone().map(|mut origin| {
                origin.paid_by = origin.reservation;
                origin.paid_parts = Some(started);
                origin.reservation = None;
                origin.hold = None;
                origin
            });
            if let Err(error) = self.0.repository.save(&job).await {
                tracing::error!(job = %job.id, sender = %sender.id, %error, "a variant: saving failed");
            }
            self.st().jobs.insert(job.id, job.clone());
            self.publish(&job);
            self.start_runner(job.id);
        }
        if started < asked {
            tracing::error!(job = %sender.id, asked, started, "variants asked for, the answer started fewer");
        }
        VariantShare {
            cost: share(&costs, 0, &sender.cost),
            server_price: quotes.as_ref().map(|quotes| share(quotes, 0, &sender.cost)),
            approved_price: approvals
                .as_ref()
                .map(|approvals| share(approvals, 0, &sender.cost)),
            batch_id,
            variant_group_id: if asked > 1 {
                Some(group)
            } else {
                sender.variant_group_id
            },
            missing: (started < asked).then_some(asked - started),
            parts: (started > 1).then_some(started),
        }
    }
}
