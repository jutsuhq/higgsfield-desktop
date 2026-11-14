//! JSON as the macOS app writes it (`JSONEncoder` with sorted keys and unescaped slashes): compact, object keys
//! sorted, a whole number written without a fraction. The wire, the tool results and the fingerprints all use it, so
//! the same value gives the same bytes on both platforms.

use serde::Serialize;
use serde_json::{Number, Value};
use std::fmt::Write as _;

/// `value` as one compact JSON text with sorted keys.
pub fn write(value: &Value) -> String {
    let mut out = String::new();
    write_into(&mut out, value);
    out
}

/// A typed value as JSON, with the object members that are `null` left out, as Swift's synthesized `Codable`
/// leaves out a `nil` optional. Arrays keep their `null`s.
pub fn to_value<T: Serialize + ?Sized>(value: &T) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(value)?;
    strip_nulls(&mut value);
    Ok(value)
}

fn strip_nulls(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|_, member| !member.is_null());
            map.values_mut().for_each(strip_nulls);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_nulls),
        _ => {}
    }
}

fn write_into(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => write_number(out, number),
        Value::String(text) => write_string(out, text),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_into(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(out, key);
                out.push(':');
                if let Some(member) = map.get(key) {
                    write_into(out, member);
                }
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            control if u32::from(control) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(control));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

fn write_number(out: &mut String, number: &Number) {
    if let Some(float) = number.as_f64().filter(|_| number.is_f64()) {
        out.push_str(&format_double(float));
    } else {
        out.push_str(&number.to_string());
    }
}

/// 2^53: past it, Swift writes a double with an exponent.
const EXACT_LIMIT: f64 = 9_007_199_254_740_992.0;

/// A double as Swift's `JSONEncoder` writes it: a whole number without `.0`, the shortest digits that read back the
/// same, and an exponent (`1e-05`, `1e+16`) for the very small and the very large.
pub fn format_double(value: f64) -> String {
    let magnitude = value.abs();
    if value.fract() == 0.0 && magnitude < EXACT_LIMIT {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a whole number below 2^53 fits an i64 exactly"
        )]
        let whole = value as i64;
        return whole.to_string();
    }
    if value != 0.0 && (magnitude < 1e-4 || magnitude >= EXACT_LIMIT) {
        let text = format!("{value:e}");
        if let Some((mantissa, exponent)) = text.split_once('e') {
            let exponent: i32 = exponent.parse().unwrap_or(0);
            let sign = if exponent < 0 { '-' } else { '+' };
            return format!("{mantissa}e{sign}{:02}", exponent.unsigned_abs());
        }
        return text;
    }
    format!("{value}")
}

#[cfg(test)]
mod tests {
    use super::{format_double, write};
    use serde_json::json;

    #[test]
    fn keys_are_sorted_and_slashes_kept() {
        let value = json!({"b": 1, "a": {"z": "/tmp/a", "y": [true, null]}});
        assert_eq!(write(&value), r#"{"a":{"y":[true,null],"z":"/tmp/a"},"b":1}"#);
    }

    #[test]
    fn doubles_read_as_swift_writes_them() {
        assert_eq!(format_double(1.0), "1");
        assert_eq!(format_double(2.5), "2.5");
        assert_eq!(format_double(86_400.0), "86400");
        assert_eq!(format_double(0.1), "0.1");
        assert_eq!(format_double(1e-5), "1e-05");
        assert_eq!(format_double(1e16), "1e+16");
        assert_eq!(write(&json!(6.5)), "6.5");
    }

    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(write(&json!("a\nb\u{1}\"")), r#""a\nb\u0001\"""#);
    }
}
