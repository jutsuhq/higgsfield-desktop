//! The database: one pool, serialized writes, and the change broadcast.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, Sqlite, Transaction};
use time::OffsetDateTime;
use tokio::sync::{Mutex, MutexGuard, broadcast};
use uuid::Uuid;

use crate::error::StoreError;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// What a committed write changed, so the UI can read those queries again the way `@Query` refreshes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoreChange {
    /// Job records.
    Jobs,
    /// The Library catalog.
    Materials,
    /// Templates.
    Templates,
    /// Delivery sets.
    DeliverySets,
    /// Starred prompts.
    StarredPrompts,
}

/// Jobs, the Library catalog, uploads, templates, starred prompts and delivery sets in SQLite (Swift's
/// `SwiftDataStore` and `StoreContainer`). Writes run one at a time, as on the Swift actor, each in one transaction
/// that a failure rolls back whole; after each commit the store broadcasts what changed.
pub struct Store {
    pool: SqlitePool,
    writes: Mutex<()>,
    changes: broadcast::Sender<StoreChange>,
}

impl Store {
    /// The store in the SQLite file at `path` (`FileLayout::store_path`), created when missing, in WAL mode with
    /// foreign keys on and the schema brought up to date.
    pub async fn open(path: &Path) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        Self::with_pool(pool).await
    }

    /// A store in memory only, for tests and previews. One connection holds the database for the store's life.
    pub async fn open_in_memory() -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(options)
            .await?;
        Self::with_pool(pool).await
    }

    async fn with_pool(pool: SqlitePool) -> Result<Self, StoreError> {
        MIGRATOR.run(&pool).await?;
        let (changes, _) = broadcast::channel(64);
        Ok(Self {
            pool,
            writes: Mutex::new(()),
            changes,
        })
    }

    /// The pool, for reads the store doesn't offer.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// What each committed write changes from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<StoreChange> {
        self.changes.subscribe()
    }

    /// Waits for the other writes, then begins a transaction. Dropping it without `commit` rolls it back.
    pub(crate) async fn begin(&self) -> Result<Write<'_>, StoreError> {
        let guard = self.writes.lock().await;
        let transaction = self.pool.begin().await?;
        Ok(Write {
            transaction,
            _guard: guard,
            store: self,
        })
    }
}

/// One write: its transaction, and the turn it holds among the store's writes.
pub(crate) struct Write<'a> {
    pub(crate) transaction: Transaction<'static, Sqlite>,
    _guard: MutexGuard<'a, ()>,
    store: &'a Store,
}

impl Write<'_> {
    /// Commits, then broadcasts `changes`. Nobody listening is not an error.
    pub(crate) async fn commit(self, changes: &[StoreChange]) -> Result<(), StoreError> {
        self.transaction.commit().await?;
        for change in changes {
            let _ = self.store.changes.send(*change);
        }
        Ok(())
    }
}

/// A date as stored: nanoseconds since 1970, clamped to the column's range.
pub(crate) fn nanos(date: OffsetDateTime) -> i64 {
    let value = date.unix_timestamp_nanos();
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

/// A stored date.
pub(crate) fn date(table: &'static str, id: &str, value: i64) -> Result<OffsetDateTime, StoreError> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value))
        .map_err(|error| StoreError::damaged(table, id, error))
}

/// A stored UUID.
pub(crate) fn uuid(table: &'static str, text: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(text).map_err(|error| StoreError::damaged(table, text, error))
}

/// The text column `name` of `row`.
pub(crate) fn text(row: &SqliteRow, name: &str) -> Result<String, StoreError> {
    Ok(row.try_get(name)?)
}
