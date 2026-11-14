//! Maps HTTP answers of the API to `HiggsfieldError`.

use serde::Deserialize;
use serde_json::Value;

use crate::core::HiggsfieldError;

/// `403 {code, detail, expires_at}` of a key that expired or was disabled in the Console.
pub(crate) const REVOKED_KEY_CODES: [&str; 2] = ["api_key_expired", "api_key_disabled"];

/// `None` for a 2xx status. `retry_after` is the answer's `Retry-After` header, when it had one.
pub(crate) fn error(status: u16, body: &[u8], retry_after: Option<&str>) -> Option<HiggsfieldError> {
    let code_of_body = || code(body);
    match status {
        200..=299 => None,
        401 => Some(HiggsfieldError::Unauthorized),
        // An expired or disabled key is refused with a code of its own; any other 403 is the balance.
        403 if code_of_body().is_some_and(|code| REVOKED_KEY_CODES.contains(&code.as_str())) => {
            Some(HiggsfieldError::Unauthorized)
        }
        403 => Some(HiggsfieldError::InsufficientFunds),
        // The account's generation queue is full, or an endpoint's own limit was reached: nothing was created or
        // charged, and the same request may be sent again once the wait is over.
        429 => Some(HiggsfieldError::ConcurrencyLimit {
            retry_after: seconds(retry_after).or_else(|| {
                error_body(body).and_then(|parsed| parsed.detail.and_then(|d| d.info()?.retry_after()))
            }),
        }),
        // Queue admission was briefly unavailable: a passing server fault, unlike a model that is disabled.
        503 if code_of_body().as_deref() == Some("admission_control_unavailable") => {
            Some(HiggsfieldError::Server { status })
        }
        423 | 503 => Some(HiggsfieldError::ModelUnavailable),
        // The request or model doesn't exist for this account; retrying can't change that.
        404 => Some(HiggsfieldError::validation(detail(body).1)),
        400 | 422 => {
            let (field, message) = detail(body);
            Some(HiggsfieldError::Validation { field, message })
        }
        _ => Some(HiggsfieldError::Server { status }),
    }
}

/// A `Retry-After` in seconds. The API sends a number; the HTTP-date form isn't read.
pub(crate) fn seconds(header: Option<&str>) -> Option<f64> {
    let seconds: f64 = header?.trim_matches([' ', '\t']).parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then_some(seconds)
}

/// Reads a FastAPI-style `detail`: a string, a list of `{loc, msg}`, or an object with a `message`.
/// Falls back to the raw body.
pub(crate) fn detail(body: &[u8]) -> (Option<String>, String) {
    if let Some(parsed) = error_body(body) {
        match parsed.detail {
            Some(Detail::Text(text)) => return (None, text),
            Some(Detail::Items(items)) if !items.is_empty() => {
                let field = items.first().and_then(Item::field);
                let message = items
                    .iter()
                    .map(|item| item.msg.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                return (field, message);
            }
            detail => {
                let info_message = detail
                    .as_ref()
                    .and_then(Detail::info)
                    .and_then(|info| info.message.clone());
                if let Some(message) = info_message.or(parsed.message).or(parsed.error) {
                    return (None, message);
                }
            }
        }
    }
    let text = String::from_utf8_lossy(body).trim().to_owned();
    (
        None,
        if text.is_empty() {
            "The request was rejected.".to_owned()
        } else {
            text
        },
    )
}

/// The machine-readable `code` of an error, in the `detail` object or next to `detail`.
pub(crate) fn code(body: &[u8]) -> Option<String> {
    let parsed = error_body(body)?;
    parsed
        .detail
        .as_ref()
        .and_then(Detail::info)
        .and_then(|info| info.code.clone())
        .or(parsed.code)
}

fn error_body(body: &[u8]) -> Option<ErrorBody> {
    serde_json::from_slice(body).ok()
}

#[derive(Deserialize)]
struct ErrorBody {
    code: Option<String>,
    detail: Option<Detail>,
    message: Option<String>,
    error: Option<String>,
}

enum Detail {
    Text(String),
    Items(Vec<Item>),
    Object(DetailObject),
    Other,
}

impl Detail {
    fn info(&self) -> Option<&DetailObject> {
        if let Self::Object(info) = self {
            Some(info)
        } else {
            None
        }
    }
}

impl<'de> Deserialize<'de> for Detail {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if let Value::String(text) = value {
            return Ok(Self::Text(text));
        }
        if let Ok(items) = Vec::<Item>::deserialize(&value) {
            return Ok(Self::Items(items));
        }
        if let Ok(info) = DetailObject::deserialize(&value) {
            return Ok(Self::Object(info));
        }
        Ok(Self::Other)
    }
}

#[derive(Deserialize)]
struct Item {
    loc: Option<Vec<LocationPart>>,
    msg: String,
}

impl Item {
    /// `["body", "image_urls", 0]` → `image_urls`: the request parameter the UI can point at.
    fn field(&self) -> Option<String> {
        let mut keys = self.loc.iter().flatten().filter_map(LocationPart::key).peekable();
        if keys
            .peek()
            .is_some_and(|first| ["body", "query", "path", "header"].contains(first))
        {
            keys.next();
        }
        keys.next().map(str::to_owned)
    }
}

/// `{code, message, retryable, limit, retry_after_seconds}`, as a full queue (429) or a queue admission outage
/// (503) answers. Every field may be missing.
#[derive(Deserialize)]
struct DetailObject {
    code: Option<String>,
    message: Option<String>,
    retry_after_seconds: Option<f64>,
}

impl DetailObject {
    fn retry_after(&self) -> Option<f64> {
        self.retry_after_seconds
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LocationPart {
    Index(#[allow(dead_code)] i64),
    Key(String),
}

impl LocationPart {
    fn key(&self) -> Option<&str> {
        if let Self::Key(key) = self {
            Some(key)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALIDATION_DETAIL: &str = r#"{"detail": [
      {"type": "too_short", "loc": ["body", "image_urls"], "msg": "List should have at least 1 item"},
      {"type": "string_type", "loc": ["body", "prompt"], "msg": "Input should be a valid string"}
    ]}"#;
    const EXPIRED_KEY: &str = r#"{"code": "api_key_expired", "detail": "This API key has expired.", "expires_at": "2026-09-21T12:00:00Z"}"#;
    const QUEUE_FULL: &str = r#"{"detail": {"code": "account_queue_full", "message": "Too many queued generations for this account",
      "retryable": true, "limit": 40, "retry_after_seconds": 30}}"#;
    const ADMISSION_UNAVAILABLE: &str = r#"{"detail": {"code": "admission_control_unavailable", "message": "Queue admission is unavailable", "retryable": true}}"#;
    /// Seedance `reference-to-video` without any reference array. The schema's rule ("Provide at least one nonempty
    /// reference array") is documented; the exact wording and `loc` of the 422 are assumed.
    const SEEDANCE_WITHOUT_REFERENCES: &str = r#"{"detail": [{"type": "value_error", "loc": ["body"],
      "msg": "Provide at least one nonempty reference array: image_urls, video_urls or audio_urls"}]}"#;

    fn validation(field: Option<&str>, message: &str) -> HiggsfieldError {
        HiggsfieldError::Validation {
            field: field.map(str::to_owned),
            message: message.to_owned(),
        }
    }

    #[test]
    fn http_errors_map_to_higgsfield_errors() {
        use HiggsfieldError::{ConcurrencyLimit, InsufficientFunds, ModelUnavailable, Server, Unauthorized};
        let cases: Vec<(u16, &str, HiggsfieldError)> = vec![
            (401, r#"{"detail": "Invalid credentials"}"#, Unauthorized),
            (403, r#"{"detail": "Not enough credits"}"#, InsufficientFunds),
            (403, "", InsufficientFunds),
            (
                403,
                r#"{"code": "insufficient_balance", "detail": "Not enough credits"}"#,
                InsufficientFunds,
            ),
            (403, EXPIRED_KEY, Unauthorized),
            (
                403,
                r#"{"code": "api_key_disabled", "detail": "This API key is disabled."}"#,
                Unauthorized,
            ),
            (
                403,
                r#"{"detail": {"code": "api_key_expired", "message": "This API key has expired."}}"#,
                Unauthorized,
            ),
            (
                400,
                r#"{"detail": "Maximum number of concurrent requests (2) has been reached"}"#,
                validation(None, "Maximum number of concurrent requests (2) has been reached"),
            ),
            (
                400,
                r#"{"detail": "Request is already in progress"}"#,
                validation(None, "Request is already in progress"),
            ),
            (
                422,
                VALIDATION_DETAIL,
                validation(
                    Some("image_urls"),
                    "List should have at least 1 item; Input should be a valid string",
                ),
            ),
            (
                422,
                r#"{"detail": [{"loc": ["image_urls", 0], "msg": "Invalid URL"}]}"#,
                validation(Some("image_urls"), "Invalid URL"),
            ),
            (
                422,
                r#"{"detail": "Idempotency key reused with a different body"}"#,
                validation(None, "Idempotency key reused with a different body"),
            ),
            (
                422,
                SEEDANCE_WITHOUT_REFERENCES,
                validation(
                    None,
                    "Provide at least one nonempty reference array: image_urls, video_urls or audio_urls",
                ),
            ),
            (400, "", validation(None, "The request was rejected.")),
            (423, "", ModelUnavailable),
            (503, "Service Unavailable", ModelUnavailable),
            (503, r#"{"detail": "Model is disabled"}"#, ModelUnavailable),
            (503, ADMISSION_UNAVAILABLE, Server { status: 503 }),
            (
                503,
                r#"{"code": "admission_control_unavailable", "detail": "Queue admission is unavailable"}"#,
                Server { status: 503 },
            ),
            (
                429,
                QUEUE_FULL,
                ConcurrencyLimit {
                    retry_after: Some(30.0),
                },
            ),
            (
                429,
                r#"{"detail": "Maximum number of concurrent requests (2) has been reached"}"#,
                ConcurrencyLimit { retry_after: None },
            ),
            (429, "", ConcurrencyLimit { retry_after: None }),
            (500, "", Server { status: 500 }),
            (502, "", Server { status: 502 }),
            (404, r#"{"detail": "Not found"}"#, validation(None, "Not found")),
        ];
        for (status, body, expected) in cases {
            assert_eq!(
                error(status, body.as_bytes(), None),
                Some(expected),
                "{status} {body}"
            );
        }
    }

    #[test]
    fn success_is_not_an_error() {
        for status in [200, 202, 204] {
            assert_eq!(error(status, b"", None), None);
        }
    }

    #[test]
    fn an_object_detail_gives_its_message() {
        assert_eq!(
            detail(QUEUE_FULL.as_bytes()).1,
            "Too many queued generations for this account"
        );
        assert_eq!(
            detail(br#"{"detail": {"code": "x"}}"#).1,
            r#"{"detail": {"code": "x"}}"#
        );
    }

    /// `Retry-After` is the wait; the body's `retry_after_seconds` (30) stands in when the header is missing or
    /// unreadable.
    #[test]
    fn a_full_queue_waits_as_the_server_asks() {
        let cases = [
            (Some("45"), 45.0),
            (Some(" 7 "), 7.0),
            (None, 30.0),
            (Some("soon"), 30.0),
            (Some("-1"), 30.0),
            (Some("Wed, 21 Oct 2026 07:28:00 GMT"), 30.0),
        ];
        for (header, wait) in cases {
            assert_eq!(
                error(429, QUEUE_FULL.as_bytes(), header),
                Some(HiggsfieldError::ConcurrencyLimit {
                    retry_after: Some(wait)
                }),
                "{header:?}"
            );
        }
        // Without a body, the header alone is the wait.
        assert_eq!(
            error(429, b"", Some("12")),
            Some(HiggsfieldError::ConcurrencyLimit {
                retry_after: Some(12.0)
            })
        );
    }
}
