//! Why a store call failed.

use jutsu_core::{DeliverySetError, MaterialError, PortError};

/// Why a store call failed. A damaged record is reported, never replaced or deleted.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite failed.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    /// The schema couldn't be brought up to date.
    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),
    /// A stored record can't be read, e.g. one written by a newer build. It stays as it is.
    #[error("damaged {table} record {id}: {reason}")]
    Damaged {
        /// The table.
        table: &'static str,
        /// The record's key.
        id: String,
        /// What didn't read.
        reason: String,
    },
    /// A value couldn't be encoded.
    #[error("can't encode {what}: {source}")]
    Encode {
        /// What was being written.
        what: &'static str,
        /// Why.
        source: serde_json::Error,
    },
    /// A catalog change was refused.
    #[error(transparent)]
    Material(#[from] MaterialError),
    /// A delivery set change was refused.
    #[error(transparent)]
    DeliverySet(#[from] DeliverySetError),
    /// `JobRecord::update` was given another job (`PersistenceError.jobIDMismatch`).
    #[error("jobIDMismatch")]
    JobIdMismatch,
    /// A file couldn't be read or written.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A JSON file couldn't be read.
    #[error("can't read {file}: {source}")]
    File {
        /// The file's name.
        file: String,
        /// Why.
        source: serde_json::Error,
    },
}

impl StoreError {
    pub(crate) fn damaged(
        table: &'static str,
        id: impl std::fmt::Display,
        reason: impl std::fmt::Display,
    ) -> Self {
        Self::Damaged {
            table,
            id: id.to_string(),
            reason: reason.to_string(),
        }
    }

    pub(crate) fn encode(what: &'static str) -> impl FnOnce(serde_json::Error) -> Self {
        move |source| Self::Encode { what, source }
    }
}

impl From<StoreError> for PortError {
    fn from(error: StoreError) -> Self {
        Self::Other(error.to_string())
    }
}
