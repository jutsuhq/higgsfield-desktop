//! The Create screen's video side: the form and its rules, the prices and the submit, the draft kept between
//! launches, the source's media, the style gallery, the Motion Library and the account's elements.

pub mod account_references;
pub mod constraints;
pub mod context;
pub mod draft;
pub mod edit;
pub mod elements;
pub mod follow_up;
pub mod form;
pub mod links;
pub mod make_video;
pub mod media;
pub mod motion;
pub mod paged_feed;
pub mod presets;
pub mod service;
pub mod view;

#[cfg(test)]
mod edit_extend_tests;
#[cfg(test)]
mod form_tests;
#[cfg(test)]
mod groups_tests;
#[cfg(test)]
mod model_options_tests;
#[cfg(test)]
mod service_tests;

pub use context::{CostLine, CreateContext, EstimateCache, ModelMenuRow, SubmitDecision, price_key};
pub use edit::CreateEdit;
pub use form::{CreateForm, DraftLength, FrameSlot, Notice, SourceVideo, TrimPart};
pub use media::{FfmpegSourceMedia, SourceMedia};
pub use service::{
    AUDIO_EXTENSIONS, AddOutcome, CreateDeps, CreateService, IMAGE_EXTENSIONS, LoadOutcome,
    RoleChangeOutcome, SubmitOutcome, VIDEO_EXTENSIONS,
};
pub use view::CreateSnapshot;

/// Why a Create command failed; the message is for the user.
#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    /// A file couldn't be read or prepared.
    #[error("{0}")]
    Media(String),
    /// What the command names doesn't exist.
    #[error("{0}")]
    NotFound(String),
    /// A value the snapshot never lists.
    #[error("{0}")]
    Invalid(String),
    #[error("Jutsu Studio can't write the file: {0}")]
    Io(#[from] std::io::Error),
}
