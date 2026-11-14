//! The agent spend ledger as one JSON file, and what the engine needs to let agents spend.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use jutsu_core::support::date::{same_day, start_of_day};
use jutsu_core::{
    AgentMoney, AgentMoneyUnit, AgentSpend, AgentSpendLedger, AgentSpendTotal, PortError, ProviderKind,
};
use time::{Duration, OffsetDateTime, UtcOffset};
use tokio::sync::Mutex;
use uuid::Uuid;

/// What the engine needs to let agents spend: the ledger, and the person's daily agent budget per connection.
#[derive(Clone)]
pub struct AgentSpendPolicy {
    /// The ledger.
    pub ledger: Arc<dyn AgentSpendLedger>,
    /// Settings › Agents › Daily agent budget for the connection, in its `AgentMoney::unit_for`; 0 lets agents spend
    /// nothing.
    pub daily_limit: Arc<dyn Fn(ProviderKind) -> i64 + Send + Sync>,
    /// Settings › Agents › Let agents use unlimited generations: whether an agent's job the plan's unlimited
    /// allowance would pay for may go out without the person. Off holds it.
    pub allows_unlimited: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl AgentSpendPolicy {
    /// `ledger` with `daily_limit`, agents not allowed the unlimited allowance.
    pub fn new(
        ledger: Arc<dyn AgentSpendLedger>,
        daily_limit: impl Fn(ProviderKind) -> i64 + Send + Sync + 'static,
    ) -> Self {
        Self {
            ledger,
            daily_limit: Arc::new(daily_limit),
            allows_unlimited: Arc::new(|| false),
        }
    }

    /// The same policy with `allows_unlimited`.
    pub fn allowing_unlimited(mut self, allows_unlimited: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        self.allows_unlimited = Arc::new(allows_unlimited);
        self
    }
}

/// The agent spend ledger as one JSON file, apart from the job store, so deleting jobs or results gives nothing
/// back. Read strictly: a file that exists and can't be read fails every call rather than counting from zero, and the
/// engine then sends nothing an agent asked for. Written atomically on every change; entries older than `KEPT_DAYS`
/// days are dropped as it writes.
pub struct AgentSpendFile {
    file: PathBuf,
    offset: UtcOffset,
    now: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    entries: Mutex<Option<Vec<AgentSpend>>>,
}

impl AgentSpendFile {
    /// The file's name in the app's data folder.
    pub const FILE_NAME: &'static str = "agent-spend.json";
    /// Today and yesterday: what a budget can count, with room for a day that just ended.
    pub const KEPT_DAYS: i64 = 2;

    /// The ledger in `file`, counting days at `offset` (the computer's time zone), with the system's clock.
    pub fn new(file: PathBuf, offset: UtcOffset) -> Self {
        Self::with_clock(file, offset, Arc::new(OffsetDateTime::now_utc))
    }

    /// The ledger in `file` with `now` as its clock.
    pub fn with_clock(
        file: PathBuf,
        offset: UtcOffset,
        now: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    ) -> Self {
        Self {
            file,
            offset,
            now,
            entries: Mutex::new(None),
        }
    }

    /// What is taken on `day` in `unit` by `entries`.
    pub fn total_of(
        entries: &[AgentSpend],
        day: OffsetDateTime,
        unit: AgentMoneyUnit,
        offset: UtcOffset,
    ) -> AgentSpendTotal {
        entries
            .iter()
            .fold(AgentSpendTotal::default(), |mut total, entry| {
                if entry.money.unit != unit || !same_day(entry.at, day, offset) {
                    return total;
                }
                if entry.is_unlimited_charged() {
                    total.unlimited_charged = true;
                }
                if entry.is_settled {
                    total.spent = AgentMoney::sum(total.spent, entry.counted());
                } else {
                    total.reserved = AgentMoney::sum(total.reserved, entry.counted());
                }
                total
            })
    }

    async fn load(&self, cached: &mut Option<Vec<AgentSpend>>) -> Result<Vec<AgentSpend>, PortError> {
        if let Some(entries) = cached {
            return Ok(entries.clone());
        }
        let loaded = match tokio::fs::read(&self.file).await {
            Ok(data) => Self::decode(&data)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(PortError::Other(error.to_string())),
        };
        *cached = Some(loaded.clone());
        Ok(loaded)
    }

    /// Strict: anything but a list of entries fails. The numbers stay as written; only `settle` and `unbill` clamp
    /// what they write.
    fn decode(data: &[u8]) -> Result<Vec<AgentSpend>, PortError> {
        serde_json::from_slice(data).map_err(|error| PortError::Other(error.to_string()))
    }

    async fn write(
        &self,
        cached: &mut Option<Vec<AgentSpend>>,
        updated: Vec<AgentSpend>,
    ) -> Result<(), PortError> {
        let start = start_of_day((self.now)(), self.offset);
        let cutoff = start - Duration::days(Self::KEPT_DAYS - 1);
        let kept: Vec<AgentSpend> = updated.into_iter().filter(|entry| entry.at >= cutoff).collect();
        let value = serde_json::to_value(&kept).map_err(|error| PortError::Other(error.to_string()))?;
        write_atomically(&self.file, &higgsfield::core::sorted_json(&value))
            .await
            .map_err(|error| PortError::Other(error.to_string()))?;
        *cached = Some(kept);
        Ok(())
    }
}

/// Writes `data` to a hidden file next to `file`, then renames it over `file`.
async fn write_atomically(file: &Path, data: &[u8]) -> std::io::Result<()> {
    let folder = file.parent().unwrap_or(Path::new("."));
    tokio::fs::create_dir_all(folder).await?;
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("ledger");
    let staged = folder.join(format!(".{name}.{}", Uuid::new_v4().simple()));
    if let Err(error) = tokio::fs::write(&staged, data).await {
        let _ = tokio::fs::remove_file(&staged).await;
        return Err(error);
    }
    if let Err(error) = tokio::fs::rename(&staged, file).await {
        let _ = tokio::fs::remove_file(&staged).await;
        return Err(error);
    }
    Ok(())
}

#[async_trait]
impl AgentSpendLedger for AgentSpendFile {
    async fn reserve(&self, spend: AgentSpend, daily_limit: Option<i64>) -> Result<bool, PortError> {
        let mut cached = self.entries.lock().await;
        let mut entries = self.load(&mut cached).await?;
        if let Some(limit) = daily_limit {
            let taken = Self::total_of(&entries, spend.at, spend.money.unit, self.offset).taken();
            if AgentMoney::sum(taken, spend.money.amount) > limit {
                return Ok(false);
            }
        }
        entries.push(spend);
        self.write(&mut cached, entries).await?;
        Ok(true)
    }

    async fn release(&self, id: Uuid) -> Result<(), PortError> {
        let mut cached = self.entries.lock().await;
        let mut entries = self.load(&mut cached).await?;
        let Some(index) = entries
            .iter()
            .position(|entry| entry.id == id && !entry.is_settled)
        else {
            return Ok(());
        };
        entries.remove(index);
        self.write(&mut cached, entries).await
    }

    async fn settle(&self, id: Uuid, amount: Option<i64>) -> Result<(), PortError> {
        let mut cached = self.entries.lock().await;
        let mut entries = self.load(&mut cached).await?;
        let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) else {
            return Ok(());
        };
        if let Some(amount) = amount {
            entry.money.amount = amount.clamp(0, AgentMoney::CEILING);
        }
        entry.is_settled = true;
        self.write(&mut cached, entries).await
    }

    async fn unbill(&self, id: Uuid, part: String, parts: i64) -> Result<(), PortError> {
        let mut cached = self.entries.lock().await;
        let mut entries = self.load(&mut cached).await?;
        let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) else {
            return Ok(());
        };
        if entry
            .unbilled
            .as_ref()
            .is_some_and(|unbilled| unbilled.contains(&part))
        {
            return Ok(());
        }
        entry.parts = Some(
            entry
                .parts
                .unwrap_or(1)
                .max(parts)
                .clamp(1, AgentSpend::MAX_PARTS),
        );
        entry.unbilled.get_or_insert_with(Vec::new).push(part);
        self.write(&mut cached, entries).await
    }

    async fn total(&self, day: OffsetDateTime, unit: AgentMoneyUnit) -> Result<AgentSpendTotal, PortError> {
        let mut cached = self.entries.lock().await;
        let entries = self.load(&mut cached).await?;
        Ok(Self::total_of(&entries, day, unit, self.offset))
    }

    async fn unsettled(&self) -> Result<Vec<AgentSpend>, PortError> {
        let mut cached = self.entries.lock().await;
        Ok(self
            .load(&mut cached)
            .await?
            .into_iter()
            .filter(|entry| !entry.is_settled)
            .collect())
    }
}
