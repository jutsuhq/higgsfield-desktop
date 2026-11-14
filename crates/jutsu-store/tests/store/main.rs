//! The ported `PersistenceTests`: one module per Swift test file, against in-memory or temp-file SQLite.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail the test the way `#require` does"
)]
#![allow(clippy::assert_is_empty, reason = "reads as the Swift `#expect(x.isEmpty)`")]
#![allow(clippy::doc_markdown, reason = "the doc comments are the Swift ones")]

mod common;

mod change_notifications;
mod delivery_set_store;
mod file_layout;
mod material_catalog;
mod material_versions_store;
mod pending_image_saves;
mod starred_prompt_record;
mod store_jobs;
mod template_record;
