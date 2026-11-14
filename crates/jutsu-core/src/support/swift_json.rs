//! JSON as Swift's `JSONEncoder` writes it with `[.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]`.

use std::fmt::Write;

use serde::Serialize;
use serde_json::Value;

/// `value` pretty-printed with two-space indents, keys sorted and written `"key" : value`, and whole doubles
/// without a fraction ("0", not "0.0"), as Swift writes them.
pub fn pretty_sorted<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    let mut out = String::new();
    write(&value, 0, &mut out)?;
    Ok(out)
}

fn write(value: &Value, depth: usize, out: &mut String) -> Result<(), serde_json::Error> {
    let indent = |depth: usize| "  ".repeat(depth);
    match value {
        Value::Object(map) if !map.is_empty() => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push_str("{\n");
            for (index, key) in keys.iter().enumerate() {
                out.push_str(&indent(depth + 1));
                out.push_str(&serde_json::to_string(key)?);
                out.push_str(" : ");
                write(&map[key.as_str()], depth + 1, out)?;
                out.push_str(if index + 1 < keys.len() { ",\n" } else { "\n" });
            }
            out.push_str(&indent(depth));
            out.push('}');
        }
        Value::Array(items) if !items.is_empty() => {
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                out.push_str(&indent(depth + 1));
                write(item, depth + 1, out)?;
                out.push_str(if index + 1 < items.len() { ",\n" } else { "\n" });
            }
            out.push_str(&indent(depth));
            out.push(']');
        }
        Value::Number(number) => match number.as_f64() {
            Some(double) if number.is_f64() && double.fract() == 0.0 && double.abs() < 1e15 => {
                let _ = write!(out, "{double:.0}");
            }
            _ => out.push_str(&number.to_string()),
        },
        other => out.push_str(&serde_json::to_string(other)?),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_as_swift_does() {
        let value = serde_json::json!({"b": 0.0, "a": {"c": [1.5, "x/y"]}, "e": []});
        assert_eq!(
            super::pretty_sorted(&value).unwrap(),
            "{\n  \"a\" : {\n    \"c\" : [\n      1.5,\n      \"x/y\"\n    ]\n  },\n  \"b\" : 0,\n  \"e\" : []\n}"
        );
    }
}
