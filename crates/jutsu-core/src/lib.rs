//! Pure domain of Jutsu Studio: models, pricing, validation, budget, the job state machine and the
//! ports (I/O traits) the other crates implement. No I/O happens here.
//!
//! `resources/models.json` and `resources/pricing.json` are copied unchanged from the macOS app. Every module is
//! the port of the Swift file of the same name in `JutsuCore`; the stored forms (JSON keys, raw values) are the
//! same, so data written by one reads in the other.
#![warn(missing_docs)]
#![allow(
    clippy::doc_markdown,
    reason = "the docs keep the Swift original's wording, product names such as VideoToolbox included"
)]
#![allow(
    clippy::struct_excessive_bools,
    reason = "the stored shapes mirror the Swift structs field for field"
)]
#![allow(clippy::return_self_not_must_use, reason = "as `must_use_candidate`")]
#![allow(
    clippy::must_use_candidate,
    reason = "most of the crate is pure queries; marking each adds noise"
)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "the rules are ported from Swift, which mixes Int and Double the same way; values stay far inside both ranges"
)]

pub mod files;
pub mod job;
pub mod models;
mod models_strings;
pub mod ports;
pub mod provider;
pub mod resources;
pub mod rules;
pub mod support;

pub use files::*;
pub use job::*;
pub use models::*;
pub use ports::*;
pub use provider::*;
pub use rules::*;
pub use support::range::ClosedRange;
