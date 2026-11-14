//! The daily spend cap.

use rust_decimal::Decimal;
use time::{OffsetDateTime, UtcOffset};

use crate::models::{Cost, ImageCharge, Job};
use crate::provider::CostUnit;
use crate::support::date::same_day;

/// Today's spending couldn't be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum BudgetError {
    /// The spending data can't be read, so nothing is sent.
    #[error(
        "Today's spending couldn't be read. Nothing was sent. Repair the spending data or turn off the daily cap in Settings."
    )]
    SpendingUnavailable,
}

/// Whether a job may start under the cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BudgetDecision {
    /// It fits.
    Allowed,
    /// Starting the job would take today's spend over the cap, in the cap's unit.
    NeedsConfirmation {
        /// Today's spend with the job.
        total: Decimal,
    },
}

/// A soft daily limit: USD for an API key, credits for a Higgsfield account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpendCap {
    /// The limit.
    pub amount: Decimal,
    /// Its unit.
    pub unit: CostUnit,
}

impl SpendCap {
    /// A cap of `amount` dollars.
    pub fn usd(amount: Decimal) -> Self {
        Self {
            amount,
            unit: CostUnit::Usd,
        }
    }

    /// A cap of `amount` credits.
    pub fn credits(amount: i64) -> Self {
        Self {
            amount: Decimal::from(amount),
            unit: CostUnit::Credits,
        }
    }
}

/// Daily spend cap. A cap limits only costs in its own unit.
pub struct Budget;

impl Budget {
    /// Whether `cost` fits. `committed_today` is spend in the cap's unit of jobs finished today plus every job in
    /// flight, so queueing can't bypass the cap; `cap` is `None` when the user has no cap.
    pub fn allows(cost: &Cost, committed_today: Decimal, cap: Option<&SpendCap>) -> BudgetDecision {
        let Some(cap) = cap.filter(|cap| cost.unit() == cap.unit) else {
            return BudgetDecision::Allowed;
        };
        let total = committed_today.saturating_add(cost.amount());
        // An unknown price (`Cost::is_unknown_price`) reads as 0, which would always fit: it asks instead.
        if total > cap.amount || cost.is_unknown_price() {
            BudgetDecision::NeedsConfirmation { total }
        } else {
            BudgetDecision::Allowed
        }
    }

    /// Whether a price quoted after queueing has to go back to the user: only when it takes the job over the cap
    /// and is more than the user agreed to: `approved` when they agreed to a price over the cap, else the
    /// preliminary price as long as that one fit. `committed_today` is as in `allows`, without the job itself.
    pub fn refined_price_needs_confirmation(
        preliminary: &Cost,
        refined: &Cost,
        approved: Option<&Cost>,
        committed_today: Decimal,
        cap: Option<&SpendCap>,
    ) -> bool {
        if Self::covers(approved, refined) {
            return false;
        }
        if Self::allows(refined, committed_today, cap) == BudgetDecision::Allowed {
            return false;
        }
        if approved.is_some_and(|approved| approved.unit() == refined.unit()) {
            return true;
        }
        Self::allows(preliminary, committed_today, cap) == BudgetDecision::Allowed
    }

    /// Whether the price the person agreed to, `approved`, covers `price`: in the same unit and no lower.
    pub fn covers(approved: Option<&Cost>, price: &Cost) -> bool {
        approved
            .is_some_and(|approved| approved.unit() == price.unit() && price.amount() <= approved.amount())
    }

    /// Spend in `unit` committed on `day`: jobs charged that day plus every job still in the queue, so queueing
    /// can't bypass the cap. Failed and canceled jobs count when the API finished them, or when it may have
    /// (`Job::may_have_been_charged`), on the day they ended. Each job counts at `Job::price`, the server's quote
    /// once there is one. A job paid from the plan's unlimited allowance (`JobSpec::uses_unlimited`) doesn't
    /// count. `offset` stands in for the Mac's calendar.
    pub fn committed(jobs: &[Job], day: OffsetDateTime, unit: CostUnit, offset: UtcOffset) -> Decimal {
        jobs.iter().fold(Decimal::ZERO, |total, job| {
            if job.price().unit() != unit || job.spec.uses_unlimited {
                return total;
            }
            if job.state.is_active() {
                return total.saturating_add(job.price().amount());
            }
            let Some(charged) = job.charged_at.or(job.finished_at) else {
                return total;
            };
            if !(job.is_charged() || job.may_have_been_charged()) || !same_day(charged, day, offset) {
                return total;
            }
            total.saturating_add(job.price().amount())
        })
    }

    /// What the image requests in `charges` add to `committed` on `day`: the ones sent that day, and every one
    /// still running.
    pub fn committed_images(
        charges: &[ImageCharge],
        day: OffsetDateTime,
        unit: CostUnit,
        offset: UtcOffset,
    ) -> Decimal {
        charges.iter().fold(Decimal::ZERO, |total, charge| {
            if charge.cost.unit() != unit || !(charge.is_pending || same_day(charge.date, day, offset)) {
                return total;
            }
            total.saturating_add(charge.cost.amount())
        })
    }
}

/// What the daily cap counts at the moment a job is about to be submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpendStatus {
    /// Spend in the cap's unit of every other job the cap counts today.
    pub committed_today: Decimal,
    /// `None` when the user has no cap.
    pub cap: Option<SpendCap>,
}
