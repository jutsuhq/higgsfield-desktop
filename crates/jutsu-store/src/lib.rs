//! SQLite storage: jobs, materials, assets, templates and delivery sets, the file layout on disk and the
//! JSON ledgers. Each record keeps its value as JSON plus the columns queries filter and sort on.
//!
//! The port of the macOS app's `Persistence` module. SwiftData's `ModelActor` becomes [`Store`]: one pool, writes
//! serialized and each in one transaction, and a change broadcast after every committed write in place of
//! `@Query` refreshing itself.
#![warn(missing_docs)]
#![allow(
    clippy::doc_markdown,
    reason = "the docs keep the Swift original's wording and product names"
)]
#![allow(
    clippy::must_use_candidate,
    reason = "most of the crate is queries; marking each adds noise"
)]

mod assets;
mod catalog;
mod delivery;
mod error;
mod file_layout;
mod files;
mod history;
mod jobs;
mod pending_image_saves;
mod prompts;
mod store;
mod templates;

pub use catalog::{MaterialQuery, MaterialSort};
pub use error::StoreError;
pub use file_layout::FileLayout;
pub use jobs::{JobRecord, JobStateKind};
pub use pending_image_saves::PendingImageSaves;
pub use prompts::{PromptUse, StarredPrompt};
pub use store::{Store, StoreChange};
