//! The I/O traits the other crates implement (Swift's `Ports`), and the error they share.
//!
//! Every trait is async and object safe, so the engine holds them as `Arc<dyn Trait>`.

mod agent_spend_ledger;
mod connectivity;
mod job_repository;
mod media_preparing;
mod on_device_enhancing;
mod speech_model_downloading;
mod speech_transcribing;
mod upload_cache;

use std::sync::Arc;

pub use agent_spend_ledger::*;
pub use connectivity::*;
pub use job_repository::*;
pub use media_preparing::*;
pub use on_device_enhancing::*;
pub use speech_model_downloading::*;
pub use speech_transcribing::*;
pub use upload_cache::*;

use crate::job::FailureReason;
use crate::models::TranscriptionError;
use crate::provider::{ProviderError, SignInError};

/// A progress callback, called from any thread.
pub type Progress<T> = Arc<dyn Fn(T) + Send + Sync>;

/// Why a port's call failed. Swift's ports throw any `Error`; these are the kinds the engine tells apart.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PortError {
    /// A provider failure.
    #[error(transparent)]
    Provider(#[from] ProviderError),
    /// A step failed its job with this reason.
    #[error(transparent)]
    Failure(#[from] FailureReason),
    /// Speech couldn't be transcribed.
    #[error(transparent)]
    Transcription(#[from] TranscriptionError),
    /// Sign-in failed.
    #[error(transparent)]
    SignIn(#[from] SignInError),
    /// The work was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// Anything else, as its description.
    #[error("{0}")]
    Other(String),
}
