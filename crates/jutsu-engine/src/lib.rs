//! The job engine: queue, slots, polling, downloads, resume and the agent spending guard.
//!
//! The port of the macOS app's `JobEngine` module. [`JobEngine`] is a cheap handle over one state that every
//! clone shares; every change of a job is saved before it is acted on, and each running job has one task that
//! drives it: prepare → upload → slot → submit → poll → download.
#![allow(
    clippy::doc_markdown,
    reason = "the docs keep the Swift original's wording, product names included"
)]
#![allow(
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    reason = "most accessors are plain queries; marking each adds noise"
)]
#![allow(
    clippy::too_many_lines,
    reason = "the pipeline steps keep the Swift original's shape, one function per step"
)]

mod agent_spend_file;
mod bookkeeping;
mod cancel;
mod clock;
mod downloader;
mod engine;
mod error;
mod file_replacement;
mod media_link_downloader;
mod server_image_reader;
mod settings;
mod slot_limiter;

pub use agent_spend_file::{AgentSpendFile, AgentSpendPolicy};
pub use clock::{EngineClock, Random, SystemClock, system_random};
pub use downloader::{
    DownloadError, DownloadRefusal, Downloader, FreeSpace, HttpDownloader, HttpFetch, HttpHead, HttpReply,
    ReqwestFetch, VolumeFreeSpace,
};
pub use engine::{
    AgentClearance, EngineDeps, EngineEvent, EnqueueOptions, JobEngine, PresetName, ScratchFile,
    SpendStatusFn,
};
pub use error::{EngineError, OverCap};
pub use file_replacement::FileReplacement;
pub use media_link_downloader::{DownloadedMedia, MediaDownloadError, MediaLinkDownloader};
pub use server_image_reader::ServerImageReader;
pub use settings::EngineSettings;
pub use slot_limiter::{SlotLimiter, SlotOrder};

#[cfg(test)]
mod tests;
