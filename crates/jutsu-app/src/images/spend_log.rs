//! The prices of reference image requests (`ImageSpendLog.swift`), in `image-spend.json` next to the store: they
//! never enter the job store, so the daily cap reads them beside the jobs.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use jutsu_core::{Budget, BudgetError, Cost, CostUnit, ImageCharge};
use rust_decimal::Decimal;
use time::{Duration, OffsetDateTime, Time, UtcOffset};
use uuid::Uuid;

use super::log_file::{LogFile, LogWriteError};

/// A price is added before its request is sent, and `add` fails when it can't be written: a paid request never goes
/// out with a price only this launch knows. A settle or removal that can't be written stays in memory, is said in
/// `save_error`, and is written with the next change or `retry_save`. An unreadable file stays intact and blocks new
/// charges until repaired: a partial history must not pass the daily cap.
#[derive(Debug)]
pub struct ImageSpendLog {
    file: LogFile<ImageCharge>,
    state: Mutex<SpendState>,
}

#[derive(Debug, Default)]
struct SpendState {
    charges: Vec<ImageCharge>,
    save_error: Option<LogWriteError>,
    read_error: Option<LogWriteError>,
}

impl ImageSpendLog {
    pub const FILE_NAME: &'static str = "image-spend.json";
    /// Spending looks back 30 days; one more keeps a request sent just before midnight.
    pub const KEPT_DAYS: i64 = 31;

    /// Reads the file in `folder`, dropping charges older than `KEPT_DAYS` before `now`'s day; `None` keeps
    /// everything in memory.
    pub fn open(folder: Option<&Path>, now: OffsetDateTime, offset: UtcOffset) -> Self {
        let file = LogFile::new(folder, Self::FILE_NAME);
        let start = now.to_offset(offset).replace_time(Time::MIDNIGHT) - Duration::days(Self::KEPT_DAYS);
        let read: Result<Vec<ImageCharge>, LogWriteError> = file.read_strict();
        let state = match read {
            Ok(charges) => SpendState {
                charges: charges
                    .into_iter()
                    .filter(|charge| charge.date >= start)
                    .collect(),
                ..SpendState::default()
            },
            Err(error) => SpendState {
                charges: Vec::new(),
                save_error: Some(error.clone()),
                read_error: Some(error),
            },
        };
        Self {
            file,
            state: Mutex::new(state),
        }
    }

    fn locked(&self) -> MutexGuard<'_, SpendState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn charges(&self) -> Vec<ImageCharge> {
        self.locked().charges.clone()
    }

    /// Why the last write failed; `None` once one goes through.
    pub fn save_error(&self) -> Option<LogWriteError> {
        self.locked().save_error.clone()
    }

    pub fn read_error(&self) -> Option<LogWriteError> {
        self.locked().read_error.clone()
    }

    /// Counts from now: a request is added before it is submitted. A price that can't be written isn't counted
    /// either, and its request must not be sent.
    pub fn add(&self, charge: ImageCharge) -> Result<(), LogWriteError> {
        let mut state = self.locked();
        let id = charge.id;
        state.charges.push(charge);
        let saved = self.save(&mut state);
        if saved.is_err() {
            state.charges.retain(|charge| charge.id != id);
        }
        saved
    }

    /// The request finished with images, so it was charged at its price, or at `cost` when the answer named it.
    pub fn settle(&self, id: Uuid, cost: Option<Cost>) {
        let mut state = self.locked();
        let Some(charge) = state.charges.iter_mut().find(|charge| charge.id == id) else {
            return;
        };
        charge.is_pending = false;
        if let Some(cost) = cost {
            charge.cost = cost;
        }
        // The change stands in memory either way; a failed write is kept in `save_error` for the user.
        let _ = self.save(&mut state);
    }

    /// The request wasn't charged: refused, failed, or cancelled while queued.
    pub fn remove(&self, id: Uuid) {
        let mut state = self.locked();
        state.charges.retain(|charge| charge.id != id);
        let _ = self.save(&mut state);
    }

    /// Writes the charges again after `save_error`; reads the file again first when it couldn't be read.
    pub fn retry_save(&self) {
        let mut state = self.locked();
        if state.read_error.is_some() {
            match self.file.read_strict() {
                Ok(charges) => {
                    state.charges = charges;
                    state.read_error = None;
                }
                Err(error) => {
                    state.read_error = Some(error.clone());
                    state.save_error = Some(error);
                    return;
                }
            }
        }
        let _ = self.save(&mut state);
    }

    fn save(&self, state: &mut SpendState) -> Result<(), LogWriteError> {
        if let Some(error) = &state.read_error {
            return Err(error.clone());
        }
        match self.file.write(&state.charges) {
            Ok(()) => {
                state.save_error = None;
                Ok(())
            }
            Err(error) => {
                state.save_error = Some(error.clone());
                Err(error)
            }
        }
    }

    /// What the image requests add to today's spend in `unit`: the ones sent on `day`, and every one still running.
    /// A file that can't be read makes today's spend unknown.
    pub fn committed(
        &self,
        day: OffsetDateTime,
        unit: CostUnit,
        offset: UtcOffset,
    ) -> Result<Decimal, BudgetError> {
        let state = self.locked();
        if state.read_error.is_some() {
            return Err(BudgetError::SpendingUnavailable);
        }
        Ok(Budget::committed_images(&state.charges, day, unit, offset))
    }

    /// Why a request whose price couldn't be written wasn't sent.
    pub fn not_sent(error: &LogWriteError) -> String {
        format!(
            "Nothing was sent: Jutsu Studio couldn't save the price first. {}",
            error.reason
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jutsu_core::ProviderKind;

    fn day() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_760_310_000).unwrap()
    }

    fn usd(text: &str, is_estimate: bool) -> Cost {
        Cost::usd(text.parse().unwrap(), is_estimate)
    }

    fn charge(date: OffsetDateTime, cost: Cost, model: &str, images: i64) -> ImageCharge {
        ImageCharge::new(
            Uuid::new_v4(),
            date,
            cost,
            ProviderKind::ApiKey,
            model,
            images,
            None,
        )
    }

    fn settled(mut charge: ImageCharge) -> ImageCharge {
        charge.is_pending = false;
        charge
    }

    #[test]
    fn an_unreadable_spending_file_cannot_be_overwritten_or_forgotten_on_relaunch() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join(ImageSpendLog::FILE_NAME);
        let damaged = br#"[{"cost":80}]"#;
        std::fs::write(&file, damaged).unwrap();
        let mut charge = charge(day(), Cost::credits(Decimal::from(30), false), "Soul", 1);
        charge.provider_kind = ProviderKind::Account;
        for _ in 0..2 {
            let log = ImageSpendLog::open(Some(folder.path()), day(), UtcOffset::UTC);
            assert!(log.add(charge.clone()).is_err());
            assert_eq!(log.charges().len(), 0);
            assert_eq!(std::fs::read(&file).unwrap(), damaged);
            assert_eq!(
                log.committed(day(), CostUnit::Credits, UtcOffset::UTC),
                Err(BudgetError::SpendingUnavailable)
            );
        }
        let log = ImageSpendLog::open(Some(folder.path()), day(), UtcOffset::UTC);
        std::fs::write(&file, "[]").unwrap();
        log.retry_save();
        log.add(charge.clone()).unwrap();
        let reopened = ImageSpendLog::open(Some(folder.path()), day(), UtcOffset::UTC);
        assert_eq!(
            reopened
                .charges()
                .iter()
                .map(|charge| charge.id)
                .collect::<Vec<_>>(),
            vec![charge.id]
        );
    }

    #[test]
    fn charges_survive_a_relaunch_and_old_ones_are_dropped() {
        let folder = tempfile::tempdir().unwrap();
        let log = ImageSpendLog::open(Some(folder.path()), day(), UtcOffset::UTC);
        let recent = charge(day(), usd("0.08", false), "Grok Image 2", 1);
        let old = settled(charge(day() - Duration::days(40), usd("1", false), "Soul 2", 4));
        let dropped = charge(day(), usd("1", false), "Soul 2", 1);
        log.add(old).unwrap();
        log.add(recent.clone()).unwrap();
        log.add(dropped.clone()).unwrap();
        log.settle(recent.id, None);
        log.remove(dropped.id);

        let reopened = ImageSpendLog::open(Some(folder.path()), day(), UtcOffset::UTC);
        let charges = reopened.charges();
        assert_eq!(
            charges.iter().map(|charge| charge.id).collect::<Vec<_>>(),
            vec![recent.id]
        );
        assert!(!charges[0].is_pending);
        assert_eq!(charges[0].cost, recent.cost);
    }

    #[test]
    fn a_write_that_fails_is_told_to_the_caller() {
        let folder = tempfile::tempdir().unwrap();
        let blocker = folder.path().join("file");
        std::fs::write(&blocker, "").unwrap();
        let spend = ImageSpendLog::open(Some(&blocker), day(), UtcOffset::UTC);
        assert!(spend.add(charge(day(), usd("1", false), "Soul 2", 1)).is_err());
        assert_eq!(spend.charges().len(), 0);
        assert_eq!(
            spend.save_error().map(|error| error.file),
            Some(ImageSpendLog::FILE_NAME.to_owned())
        );
    }

    #[test]
    fn today_counts_running_requests_and_todays_finished_ones() {
        let log = ImageSpendLog::open(None, day(), UtcOffset::UTC);
        let mut credits = charge(day(), Cost::credits(Decimal::from(9), false), "d", 1);
        credits.provider_kind = ProviderKind::Account;
        for charge in [
            settled(charge(day(), usd("0.08", false), "a", 1)),
            charge(day() - Duration::days(2), usd("0.5", false), "b", 1),
            settled(charge(day() - Duration::days(2), usd("3", false), "c", 1)),
            credits,
        ] {
            log.add(charge).unwrap();
        }
        assert_eq!(
            log.committed(day(), CostUnit::Usd, UtcOffset::UTC),
            Ok("0.58".parse().unwrap())
        );
        assert_eq!(
            log.committed(day(), CostUnit::Credits, UtcOffset::UTC),
            Ok(Decimal::from(9))
        );
    }

    #[test]
    fn the_file_reads_as_swift_writes_it() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join(ImageSpendLog::FILE_NAME),
            r#"[{"cost":{"usd":{"_0":0.0128,"isEstimate":false}},"date":"2025-10-12T23:00:00Z","id":"0F5E2D51-6F2B-4E57-9F2A-0D7E5C9A1B11","images":4,"isPending":false,"model":"Soul 2","providerKind":"apiKey"}]"#,
        )
        .unwrap();
        let log = ImageSpendLog::open(Some(folder.path()), day(), UtcOffset::UTC);
        assert!(log.read_error().is_none());
        assert_eq!(log.charges().len(), 1);
        assert_eq!(
            log.committed(day(), CostUnit::Usd, UtcOffset::UTC),
            Ok("0.0128".parse().unwrap())
        );
    }
}
