//! Untyped JSON for tool schemas, arguments and results whose shape is only known at runtime, as
//! `serde_json::Value`, with the readers the Swift `JSONValue` offered.

use serde_json::{Map, Number, Value};
use url::Url;

use crate::core::is_https;

pub type Object = Map<String, Value>;

/// A number as JSON: a whole one as an integer, so it is written `5` and not `5.0`, as `JSONEncoder` writes a
/// whole `Double`. `None` for NaN or an infinity, which JSON can't hold.
pub fn number(value: f64) -> Option<Value> {
    // 2^53: every whole number below it is exact as an `f64`.
    const EXACT: f64 = 9_007_199_254_740_992.0;
    if !value.is_finite() {
        return None;
    }
    if value.fract() == 0.0 && value.abs() < EXACT {
        #[allow(clippy::cast_possible_truncation)]
        return Some(Value::from(value as i64));
    }
    Number::from_f64(value).map(Value::Number)
}

/// A whole count as JSON.
#[must_use]
pub fn int(value: usize) -> Value {
    Value::from(value)
}

/// `value[key]` of an object; `None` for anything else.
#[must_use]
pub fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object()?.get(key)
}

/// `value[key]` as a string.
#[must_use]
pub fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    get(value, key)?.as_str()
}

/// `value[key]` as a number, integers included.
#[must_use]
pub fn num(value: &Value, key: &str) -> Option<f64> {
    get(value, key)?.as_f64()
}

/// `value[key]` as an array.
#[must_use]
pub fn array<'a>(value: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    get(value, key)?.as_array()
}

/// Whether `value[key]` is the boolean `flag`.
pub fn is_bool(value: &Value, key: &str, flag: bool) -> bool {
    get(value, key).and_then(Value::as_bool) == Some(flag)
}

/// A media URL the server sent, only when it is https: the app downloads these, and another scheme such as
/// `file:` would reach local files. Characters a URL can't hold are percent-encoded.
pub fn server_media(text: &str) -> Option<Url> {
    // `Url` reads `https:///a.mp4` or `https:/a.mp4` with host `a.mp4`; Foundation finds no host there.
    let (_, rest) = text.split_once("://")?;
    if rest.starts_with('/') {
        return None;
    }
    Url::parse(text).ok().filter(is_https)
}

/// The value of `key` as a server media URL.
pub fn media_url(value: &Value, key: &str) -> Option<Url> {
    string(value, key).and_then(server_media)
}

/// A whole count of 0 or more.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[must_use]
pub fn count(value: f64) -> Option<i64> {
    // Every whole `f64` from 0 below 2^63 is an exact `i64`.
    (value >= 0.0 && value.fract() == 0.0 && value < 9_223_372_036_854_775_808.0).then_some(value as i64)
}

/// The next-page cursor of a page: `None` when missing or `null`, else the value as it came.
#[must_use]
pub fn cursor(value: &Value, key: &str) -> Option<Value> {
    get(value, key).filter(|cursor| !cursor.is_null()).cloned()
}

/// A copy of `value` with every occurrence of `secrets` in strings and object keys replaced by `<redacted>`.
#[must_use]
pub fn redacting(value: &Value, secrets: &[String]) -> Value {
    if secrets.is_empty() {
        return value.clone();
    }
    let scrub = |text: &str| {
        secrets
            .iter()
            .fold(text.to_owned(), |text, secret| text.replace(secret, "<redacted>"))
    };
    match value {
        Value::String(text) => Value::String(scrub(text)),
        Value::Array(items) => Value::Array(items.iter().map(|item| redacting(item, secrets)).collect()),
        Value::Object(object) => {
            let mut scrubbed = Object::new();
            for (key, item) in object {
                scrubbed
                    .entry(scrub(key))
                    .or_insert_with(|| redacting(item, secrets));
            }
            Value::Object(scrubbed)
        }
        other => other.clone(),
    }
}
