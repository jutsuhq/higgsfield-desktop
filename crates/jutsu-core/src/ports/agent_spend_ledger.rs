//! The durable record of what agents spend.

use async_trait::async_trait;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::models::{AgentMoneyUnit, AgentSpend, AgentSpendTotal};
use crate::ports::PortError;

/// The durable record of what agents spend, apart from the jobs: a job deleted from the library takes nothing
/// back. Every call fails when the record can't be read or written, and the engine then sends nothing for an agent.
#[async_trait]
pub trait AgentSpendLedger: Send + Sync {
    /// Saves `spend` as reserved if it fits under `daily_limit` with everything else taken on its day in its unit;
    /// `None` saves it whatever is taken. Returns `false`, saving nothing, when it doesn't fit. Checked and saved in
    /// one step, so two submits can't both take the last of the budget.
    async fn reserve(&self, spend: AgentSpend, daily_limit: Option<i64>) -> Result<bool, PortError>;
    /// Removes a reservation whose submit certainly never reached the provider.
    async fn release(&self, id: Uuid) -> Result<(), PortError>;
    /// Marks a reservation as charged: what its submit charged, or `None` for the amount reserved when the answer
    /// didn't say.
    async fn settle(&self, id: Uuid, amount: Option<i64>) -> Result<(), PortError>;
    /// One of the `parts` jobs the submit `id` paid for ended without being billed: Higgsfield doesn't charge a
    /// generation that failed, was refused by moderation or was cancelled. From then on the entry counts only its
    /// billed parts' share (`AgentSpend::counted`), 0 when none is left; `part` names the job, and counts once. An
    /// entry the ledger no longer keeps is left alone.
    async fn unbill(&self, id: Uuid, part: String, parts: i64) -> Result<(), PortError>;
    /// What is taken on `day` in `unit`.
    async fn total(&self, day: OffsetDateTime, unit: AgentMoneyUnit) -> Result<AgentSpendTotal, PortError>;
    /// The reservations not settled yet, of every day the ledger keeps: what a launch reconciles with the jobs.
    async fn unsettled(&self) -> Result<Vec<AgentSpend>, PortError>;
}
