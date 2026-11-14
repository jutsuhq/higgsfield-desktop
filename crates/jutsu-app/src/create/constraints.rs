//! The model limits Create reads, from the account's live catalog (`ModelConstraintStore.swift`): the active account's
//! cached table, read again from the server when the account connects and at most once a day after that. Never on a
//! form edit. Without a cache, or for a connection that can't report constraints, the table is the constants.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use futures::future::BoxFuture;
use jutsu_core::{ModelConstraintTable, PortError};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

/// Read again after a day.
const MAX_AGE: Duration = Duration::DAY;

/// The server's constraints of one account, as last read, with when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedConstraints {
    #[serde(with = "time::serde::rfc3339")]
    pub fetched_at: OffsetDateTime,
    pub table: ModelConstraintTable,
}

/// Read less than a day ago. A read dated after `now` (the clock was turned back) is stale too, or it would stay
/// fresh for as long as the clock is off.
pub fn is_fresh(read_at: OffsetDateTime, now: OffsetDateTime) -> bool {
    let age = now - read_at;
    age >= Duration::ZERO && age < MAX_AGE
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    accounts: BTreeMap<String, CachedConstraints>,
}

/// The file the constraints of every account the app connected are cached in, by `GenerationProvider::owner`. One
/// that is missing, unreadable or of another version reads as empty, so the constants are used.
#[derive(Debug, Clone)]
pub struct ModelConstraintCache {
    file: PathBuf,
}

impl ModelConstraintCache {
    pub const FILE_NAME: &str = "model-constraints.json";
    const VERSION: u32 = 1;

    pub fn new(folder: &Path) -> Self {
        Self {
            file: folder.join(Self::FILE_NAME),
        }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The constraints cached for `account`; `None` when there are none or the file can't be read.
    pub fn read(&self, account: &str) -> Option<CachedConstraints> {
        self.read_all().remove(account)
    }

    /// Replaces `account`'s constraints, keeping the other accounts'. Written to a side file, then moved into place.
    pub fn write(&self, constraints: CachedConstraints, account: &str) -> std::io::Result<()> {
        let mut accounts = self.read_all();
        accounts.insert(account.to_owned(), constraints);
        let data = serde_json::to_vec_pretty(&CacheFile {
            version: Self::VERSION,
            accounts,
        })
        .map_err(std::io::Error::other)?;
        if let Some(folder) = self.file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let partial = self.file.with_extension("json.partial");
        std::fs::write(&partial, data)?;
        std::fs::rename(&partial, &self.file)
    }

    fn read_all(&self) -> BTreeMap<String, CachedConstraints> {
        std::fs::read(&self.file)
            .ok()
            .and_then(|data| serde_json::from_slice::<CacheFile>(&data).ok())
            .filter(|file| file.version == Self::VERSION)
            .map(|file| file.accounts)
            .unwrap_or_default()
    }
}

#[derive(Debug, Default)]
struct StoreState {
    table: Option<ModelConstraintTable>,
    account: Option<String>,
    /// When the active account's table was read from the server, or a read found nothing to read.
    checked_at: Option<OffsetDateTime>,
    is_refreshing: bool,
}

/// What Create's limits are read through (`CreateForm::constraints`).
#[derive(Debug)]
pub struct ModelConstraintStore {
    cache: ModelConstraintCache,
    now: fn() -> OffsetDateTime,
    state: Mutex<StoreState>,
}

/// Reads the server's table; `None` when the connection can't tell.
pub type FetchConstraints = BoxFuture<'static, Result<Option<ModelConstraintTable>, PortError>>;

impl ModelConstraintStore {
    pub fn new(cache: ModelConstraintCache, now: fn() -> OffsetDateTime) -> Self {
        Self {
            cache,
            now,
            state: Mutex::new(StoreState::default()),
        }
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, StoreState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The table to use; `None` keeps the constants.
    pub fn table(&self) -> Option<ModelConstraintTable> {
        self.locked().table.clone()
    }

    /// Shows `account`'s cached table, or the constants for `None` (no connection, or one without constraints).
    /// The file is read on the blocking pool; a switch to another account meanwhile wins. Answers whether the table
    /// changed.
    pub async fn switch_account(&self, account: Option<&str>) -> bool {
        {
            let mut state = self.locked();
            state.account = account.map(str::to_owned);
            state.checked_at = None;
        }
        let cached = match account {
            Some(account) => {
                let (cache, account) = (self.cache.clone(), account.to_owned());
                tokio::task::spawn_blocking(move || cache.read(&account))
                    .await
                    .ok()
                    .flatten()
            }
            None => None,
        };
        let mut state = self.locked();
        if state.account.as_deref() != account {
            return false;
        }
        state.checked_at = cached.as_ref().map(|cached| cached.fetched_at);
        let table = cached.map(|cached| cached.table);
        let changed = state.table != table;
        state.table = table;
        changed
    }

    /// Whether `refresh_if_stale` would read the server now.
    pub fn is_stale(&self) -> bool {
        let state = self.locked();
        if state.account.is_none() {
            return false;
        }
        state
            .checked_at
            .is_none_or(|checked_at| !is_fresh(checked_at, (self.now)()))
    }

    /// Reads the table with `fetch` when the account's is a day old or missing, and caches it. `fetch` answering
    /// `None` keeps the table there is until the next day; a failed read keeps it too, and the next call tries
    /// again. Answers whether the table changed.
    pub async fn refresh_if_stale(&self, fetch: impl FnOnce() -> FetchConstraints) -> bool {
        let account = {
            if !self.is_stale() {
                return false;
            }
            let mut state = self.locked();
            let Some(account) = state.account.clone() else {
                return false;
            };
            if state.is_refreshing {
                return false;
            }
            state.is_refreshing = true;
            account
        };
        let fetched = fetch().await;
        let mut state = self.locked();
        state.is_refreshing = false;
        let Ok(fetched) = fetched else {
            return false;
        };
        let read_at = (self.now)();
        if let Some(table) = &fetched {
            let cached = CachedConstraints {
                fetched_at: read_at,
                table: table.clone(),
            };
            if let Err(error) = self.cache.write(cached, &account) {
                tracing::warn!(%error, "can't cache the model constraints");
            }
        }
        if state.account.as_deref() != Some(account.as_str()) {
            return false;
        }
        state.checked_at = Some(read_at);
        match fetched {
            Some(table) if state.table.as_ref() != Some(&table) => {
                state.table = Some(table);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::FutureExt as _;
    use jutsu_core::ProviderError;

    use super::*;

    static CLOCK: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1_800_000_000);

    fn now() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(CLOCK.load(Ordering::SeqCst)).unwrap()
    }

    fn table(max_duration: u32) -> ModelConstraintTable {
        serde_json::from_str(&format!(
            r#"{{"entries": [{{"model": "seedance25", "mode": "motionTransfer", "constraints": {{"maxDuration": {max_duration}}}}}]}}"#
        ))
        .unwrap()
    }

    fn fetch(
        reads: &Arc<AtomicUsize>,
        answer: Result<Option<ModelConstraintTable>, PortError>,
    ) -> impl FnOnce() -> FetchConstraints {
        let reads = reads.clone();
        move || {
            reads.fetch_add(1, Ordering::SeqCst);
            async move { answer }.boxed()
        }
    }

    #[test]
    fn a_read_is_fresh_for_a_day() {
        let read = now();
        assert!(is_fresh(read, read + Duration::hours(23)));
        assert!(!is_fresh(read, read + Duration::DAY));
        assert!(!is_fresh(read, read - Duration::minutes(1)));
    }

    #[tokio::test]
    async fn each_account_has_its_own_table_and_a_bad_file_reads_as_empty() {
        let folder = tempfile::tempdir().unwrap();
        let cache = ModelConstraintCache::new(folder.path());
        let read = |account| CachedConstraints {
            fetched_at: now(),
            table: table(account),
        };
        cache.write(read(45), "a").unwrap();
        cache.write(read(20), "b").unwrap();
        let store = ModelConstraintStore::new(cache.clone(), now);
        assert!(store.switch_account(Some("a")).await);
        assert_eq!(store.table(), Some(table(45)));
        store.switch_account(Some("b")).await;
        assert_eq!(store.table(), Some(table(20)));
        store.switch_account(None).await;
        assert_eq!(store.table(), None);
        std::fs::write(cache.file(), br#"{"version": 2, "accounts": {}}"#).unwrap();
        assert!(!store.switch_account(Some("a")).await);
        assert_eq!(store.table(), None);
    }

    #[tokio::test]
    async fn the_server_is_read_once_a_day_and_a_failure_tries_again() {
        let folder = tempfile::tempdir().unwrap();
        let store = ModelConstraintStore::new(ModelConstraintCache::new(folder.path()), now);
        let reads = Arc::new(AtomicUsize::new(0));
        assert!(!store.refresh_if_stale(fetch(&reads, Ok(Some(table(45))))).await);
        assert_eq!(reads.load(Ordering::SeqCst), 0, "no account, no read");

        store.switch_account(Some("a")).await;
        let offline = Err(PortError::Provider(ProviderError::Network));
        assert!(!store.refresh_if_stale(fetch(&reads, offline)).await);
        assert!(store.is_stale());
        assert!(store.refresh_if_stale(fetch(&reads, Ok(Some(table(45))))).await);
        assert!(!store.refresh_if_stale(fetch(&reads, Ok(Some(table(20))))).await);
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        assert_eq!(store.table(), Some(table(45)));

        // Cached: the next connection shows it without reading.
        let again = ModelConstraintStore::new(ModelConstraintCache::new(folder.path()), now);
        again.switch_account(Some("a")).await;
        assert_eq!(again.table(), Some(table(45)));
        assert!(!again.is_stale());
    }

    #[tokio::test]
    async fn no_answer_waits_for_the_next_day() {
        let folder = tempfile::tempdir().unwrap();
        let store = ModelConstraintStore::new(ModelConstraintCache::new(folder.path()), now);
        let reads = Arc::new(AtomicUsize::new(0));
        store.switch_account(Some("a")).await;
        assert!(!store.refresh_if_stale(fetch(&reads, Ok(None))).await);
        assert!(!store.is_stale());
        assert_eq!(store.table(), None);
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
#[path = "constraint_form_tests.rs"]
mod form_tests;
