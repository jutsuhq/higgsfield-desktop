//! The digest that tells a retry of a request from another request under the same `request_id`: SHA-256 of the value
//! as JSON with sorted keys, so the same parameters always give the same digest, on every platform.

use crate::json;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// Lowercase hex SHA-256 of `value` as canonical JSON.
pub fn of<T: Serialize + ?Sized>(value: &T) -> Result<String, serde_json::Error> {
    let text = json::write(&json::to_value(value)?);
    let digest = Sha256::digest(text.as_bytes());
    Ok(digest.iter().fold(String::with_capacity(64), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    }))
}
