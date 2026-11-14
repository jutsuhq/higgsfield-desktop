//! A job an AI agent queued, what it may spend, and the guard that decides before it is sent.

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::models::{Cost, Job};
use crate::provider::{CostUnit, ProviderKind};

/// A job an AI agent queued through the `jutsu` helper, and what it may spend. Set when the job is queued and kept
/// by its variants; `None` for every job the person queued, and in jobs stored before agents existed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AgentOrigin {
    /// The MCP client's own name ("claude-code"), as the helper reported it; `None` when it gave none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// The agent's `request_id`: the same request sent again returns this job instead of queueing another.
    #[serde(rename = "requestID")]
    pub request_id: String,
    /// The most the agent agreed to pay for this job, in whole units of its connection.
    pub max_cost: AgentMoney,
    /// The agent spend ledger's entry held for the submit in flight (`AgentSpendLedger::reserve`); `None` before the
    /// first submit, and on variants, whose submit their sender's entry pays for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation: Option<Uuid>,
    /// Why the spending guard paused the job (`PauseReason::AgentLimit`); `None` while it isn't paused for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<AgentHold>,
    /// The person resumed the job past an agent limit: it goes on at the price they saw (the job's `approved_price`),
    /// still under their own daily cap; a higher price waits for them again. Cleared by a retry, which is a new
    /// attempt.
    pub approved_by_person: bool,
    /// A variant's: the ledger entry of the submit that started it, its sender's `reservation`. Cleared by a retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_by: Option<Uuid>,
    /// How many jobs the submit that started this one started, itself included, when it was more than one: each is
    /// that share of the entry (`AgentSpendLedger::unbill`). Cleared by a retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_parts: Option<i64>,
    /// An agent's `pause_queue` paused the job, so its `resume_queue` may lift that pause; `None` once the person's
    /// own Pause all took it over, or the job was resumed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_by_agent: Option<bool>,
}

impl AgentOrigin {
    /// A new agent's job, before its first submit.
    pub fn new(client: Option<String>, request_id: impl Into<String>, max_cost: AgentMoney) -> Self {
        Self {
            client,
            request_id: request_id.into(),
            max_cost,
            reservation: None,
            hold: None,
            approved_by_person: false,
            paid_by: None,
            paid_parts: None,
            paused_by_agent: None,
        }
    }

    /// A retry, a new attempt: it forgets the person's approval, the hold and what paid for the attempt before.
    pub fn start_over(&mut self) {
        self.approved_by_person = false;
        self.hold = None;
        self.reservation = None;
        self.paid_by = None;
        self.paid_parts = None;
        self.paused_by_agent = None;
    }

    /// The ledger entry this job's charge is part of: its own submit's, else its sender's.
    pub fn ledger_entry(&self) -> Option<Uuid> {
        self.reservation.or(self.paid_by)
    }
}

/// Why the spending guard stopped an agent's job before its submit. Only a person resumes it; the agent may lift
/// `OverMaxCost` alone, by raising `max_cost` to a price that fits every limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum AgentHold {
    /// Neither the provider nor the price list could say what it costs.
    UnknownPrice,
    /// Its price is above the `max_cost` the agent sent.
    OverMaxCost,
    /// Its price is more than is left of today's agent budget, reservations of submits in flight included.
    OverDailyBudget,
    /// The agent spend ledger couldn't be read or written, so nothing an agent asks for is sent.
    LedgerUnavailable,
    /// It would be paid from the plan's unlimited allowance, which Settings › Agents › Let agents use unlimited
    /// generations doesn't allow.
    UnlimitedNotAllowed,
    /// The person approved it at a price, and the price is higher now.
    AboveApprovedPrice,
}

/// What agent budgets and `max_cost` count in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub enum AgentMoneyUnit {
    /// Whole cents of a dollar.
    #[serde(rename = "usd_cents")]
    UsdCents,
    /// Whole credits.
    #[serde(rename = "credits")]
    Credits,
}

/// Whole cents or whole credits: what agent budgets and `max_cost` count in. Integers only, so a sum can't drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AgentMoney {
    /// The unit.
    pub unit: AgentMoneyUnit,
    /// Never negative.
    pub amount: i64,
}

impl AgentMoney {
    /// The most any amount can be: far above any real price, and far enough below `i64::MAX` that sums of many of
    /// them can't overflow.
    pub const CEILING: i64 = 1 << 50;

    /// `amount` of `unit`, never below 0.
    pub fn new(amount: i64, unit: AgentMoneyUnit) -> Self {
        Self {
            unit,
            amount: amount.max(0),
        }
    }

    /// What a connection's agent budget counts in: cents with an API key, credits on an account.
    pub fn unit_for(kind: ProviderKind) -> AgentMoneyUnit {
        if kind == ProviderKind::ApiKey {
            AgentMoneyUnit::UsdCents
        } else {
            AgentMoneyUnit::Credits
        }
    }

    /// `cost` in whole units, rounded up so a fraction of a cent or credit is never read as free; `None` for an
    /// unknown price (`Cost::is_unknown_price`) or a negative amount. Huge amounts are clamped to `CEILING`.
    pub fn from_cost(cost: &Cost) -> Option<Self> {
        if cost.is_unknown_price() {
            return None;
        }
        let scaled = match cost.unit() {
            CostUnit::Usd => cost.amount().saturating_mul(Decimal::ONE_HUNDRED),
            CostUnit::Credits => cost.amount(),
        };
        if scaled.is_sign_negative() && !scaled.is_zero() {
            return None;
        }
        let rounded = scaled.round_dp_with_strategy(0, RoundingStrategy::ToPositiveInfinity);
        let units = if rounded >= Decimal::from(Self::CEILING) {
            Self::CEILING
        } else {
            rounded.to_i64().unwrap_or(Self::CEILING)
        };
        let unit = match cost.unit() {
            CostUnit::Usd => AgentMoneyUnit::UsdCents,
            CostUnit::Credits => AgentMoneyUnit::Credits,
        };
        Some(Self::new(units, unit))
    }

    /// `a + b`, stopping at `CEILING`.
    pub fn sum(a: i64, b: i64) -> i64 {
        a.max(0)
            .checked_add(b.max(0))
            .map_or(Self::CEILING, |total| total.min(Self::CEILING))
    }
}

/// What agents took on one day, in one unit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AgentSpendTotal {
    /// Charged by submits that were answered.
    pub spent: i64,
    /// Held for submits sent and not answered yet, or never known to be answered.
    pub reserved: i64,
    /// A submit sent for the plan's unlimited allowance was charged anyway (`AgentSpend::pays_unlimited`): agents'
    /// jobs for the allowance keep every limit for the rest of the day.
    pub unlimited_charged: bool,
}

impl AgentSpendTotal {
    /// Spent and reserved together.
    pub fn taken(&self) -> i64 {
        AgentMoney::sum(self.spent, self.reserved)
    }

    /// What is left of `daily_limit`, never below 0.
    pub fn remaining(&self, daily_limit: i64) -> i64 {
        (daily_limit - self.taken()).max(0)
    }
}

/// One submit of an agent's job, as the ledger holds it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AgentSpend {
    /// The entry's identity.
    pub id: Uuid,
    /// The job whose submit it is.
    #[serde(rename = "jobID")]
    pub job_id: Uuid,
    /// What it reserved, or what it charged once settled.
    pub money: AgentMoney,
    /// When it was reserved: the day it counts on.
    #[serde(with = "crate::support::date::rfc3339")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub at: OffsetDateTime,
    /// The submit was answered and `money` is what it charged; before that it is reserved.
    pub is_settled: bool,
    /// How many equal parts `money` pays for (the jobs of a submit of variants, the jobs of an image request), once
    /// one of them ended unbilled; `None` before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<i64>,
    /// The parts that ended without being billed (`AgentSpendLedger::unbill`), each once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unbilled: Option<Vec<String>>,
    /// Sent for the plan's unlimited allowance, reserving 0; settled above 0, the allowance didn't pay for it
    /// (`AgentSpendTotal::unlimited_charged`). `None` for any other submit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pays_unlimited: Option<bool>,
}

impl AgentSpend {
    /// The most parts an entry is split into: far above any request, and small enough that `counted` can't
    /// overflow.
    pub const MAX_PARTS: i64 = 1_000;

    /// A new, unsettled entry.
    pub fn new(id: Uuid, job_id: Uuid, money: AgentMoney, at: OffsetDateTime, pays_unlimited: bool) -> Self {
        Self {
            id,
            job_id,
            money,
            at,
            is_settled: false,
            parts: None,
            unbilled: None,
            pays_unlimited: pays_unlimited.then_some(true),
        }
    }

    /// Sent for the allowance, and charged anyway.
    pub fn is_unlimited_charged(&self) -> bool {
        self.pays_unlimited == Some(true) && self.is_settled && self.money.amount > 0
    }

    /// What it counts against the budget: `money`, less the share of the parts that ended unbilled, rounded up so a
    /// fraction is never read as free; 0 once every part did.
    pub fn counted(&self) -> i64 {
        let (Some(parts), Some(unbilled)) = (self.parts, self.unbilled.as_ref()) else {
            return self.money.amount;
        };
        if parts <= 0 || unbilled.is_empty() {
            return self.money.amount;
        }
        let billed = (parts - unbilled.len() as i64).max(0);
        let (whole, rest) = (self.money.amount / parts, self.money.amount % parts);
        whole * billed + (rest * billed + parts - 1) / parts
    }
}

/// What the spending guard decides before an agent's job is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentSpendDecision {
    /// Send it, reserving this much against the day's agent budget; `limit` is that budget, or `None` when the
    /// person approved the job past it.
    Send {
        /// What to reserve.
        money: AgentMoney,
        /// The day's agent budget, or `None`.
        limit: Option<i64>,
    },
    /// Hold it for the person.
    Hold(AgentHold),
}

/// The inputs of `AgentSpendDecision::decide` beyond the price and the origin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentSpendContext {
    /// The request is to be paid from the plan's unlimited allowance (`JobSpec::uses_unlimited`).
    pub pays_unlimited: bool,
    /// Settings › Agents › Let agents use unlimited generations.
    pub allows_unlimited: bool,
    /// The price the person approved the job at (`origin.approved_by_person`); `None`, or an unknown price, agreed
    /// to whatever it costs.
    pub approved: Option<Cost>,
}

impl AgentSpendDecision {
    /// The guard's rule, without the reservation itself (`AgentSpendLedger::reserve` checks the budget as it saves).
    /// A request the plan's unlimited allowance pays costs the budgets nothing, so it goes out reserving 0 with no
    /// limit, but only when the person lets agents use the allowance (or sent it themselves); otherwise it is held.
    /// One the person approved goes past the agent's limits at the price they approved, and is held again when its
    /// price is now higher (or in another unit).
    ///
    /// `price` is the job's best known price (`Job::price`); `is_known` says it is the provider's quote or an exact
    /// list price; `daily_limit` is the person's daily agent budget for the connection, in `origin.max_cost`'s unit.
    pub fn decide(
        price: &Cost,
        is_known: bool,
        origin: &AgentOrigin,
        daily_limit: i64,
        context: &AgentSpendContext,
    ) -> Self {
        if context.pays_unlimited {
            if !(context.allows_unlimited || origin.approved_by_person) {
                return Self::Hold(AgentHold::UnlimitedNotAllowed);
            }
            return Self::Send {
                money: AgentMoney::new(0, origin.max_cost.unit),
                limit: None,
            };
        }
        let money = AgentMoney::from_cost(price);
        if origin.approved_by_person {
            if let Some(money) = money
                && let Some(agreed) = context.approved.as_ref().and_then(AgentMoney::from_cost)
                && (money.unit != agreed.unit || money.amount > agreed.amount)
            {
                return Self::Hold(AgentHold::AboveApprovedPrice);
            }
            return Self::Send {
                money: money.unwrap_or(AgentMoney::new(0, origin.max_cost.unit)),
                limit: None,
            };
        }
        let Some(money) = money.filter(|_| is_known) else {
            return Self::Hold(AgentHold::UnknownPrice);
        };
        if money.unit != origin.max_cost.unit || money.amount > origin.max_cost.amount {
            return Self::Hold(AgentHold::OverMaxCost);
        }
        if money.amount > daily_limit {
            return Self::Hold(AgentHold::OverDailyBudget);
        }
        Self::Send {
            money,
            limit: Some(daily_limit),
        }
    }

    /// Whether the plan's unlimited allowance pays for an agent's request that asks for it
    /// (`JobSpec::uses_unlimited`): it does, unless the person lets agents use the allowance and a request of theirs
    /// for it was charged anyway today (`AgentSpendTotal::unlimited_charged`). Then the allowance isn't trusted to
    /// pay until the day ends, and the request keeps the cap, `max_cost` and the budget like any other.
    pub fn pays_unlimited(uses_unlimited: bool, allows_unlimited: bool, charged_today: bool) -> bool {
        uses_unlimited && !(allows_unlimited && charged_today)
    }
}

/// What raising an agent's `max_cost` on its held job did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentMaxCostRaise {
    /// It fits every limit now: the job goes on with the new `max_cost`, and the guard checks it again at submit.
    Lifted,
    /// It still doesn't fit; nothing changed.
    Held(AgentHold),
    /// It would take today's spending over the person's daily cap; nothing changed.
    OverCap,
    /// Not a job the agent can lift: not held over its `max_cost` (or the person decided on it), already sent,
    /// for the plan's unlimited allowance, or the new `max_cost` isn't higher in the same unit.
    NotLiftable,
}

impl Job {
    /// The price the guard may trust: the provider's quote, or an exact preliminary price.
    pub fn agent_price_is_known(&self) -> bool {
        !self.price_is_unknown() && (self.server_price.is_some() || !self.cost.is_estimate())
    }
}
