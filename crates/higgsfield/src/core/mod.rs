//! The errors and generation states both connections report, and the HTTP transport they share.

mod error;
mod transport;

pub use error::{GenerationStatus, HiggsfieldError, ImageStatus, Moderation};
pub use transport::{HttpResponse, is_https, send, send_or, sorted_json, sorted_pretty_json};
