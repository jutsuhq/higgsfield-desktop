//! The MCP catalog snapshots: recorded `models_explore` answers, the tool schemas of the newest snapshot, the
//! live answers of 2026-10-15, and a small JSON Schema checker for the calls the tests' stubs get.

use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

use serde_json::{Map, Value};

const SNAPSHOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mcp-snapshots");

/// `models_explore {action: "get", model_id: "seedance_2_5"}` of the 2026-10-28 snapshot.
pub const SEEDANCE: &str = include_str!("../../fixtures/mcp-snapshots/answers-2026-10-28/seedance.json");
/// `models_explore {action: "get", model_id: "kling3_0_motion_control"}`.
pub const KLING: &str = include_str!("../../fixtures/mcp-snapshots/answers-2026-10-28/kling.json");
/// `models_explore {action: "get", model_id: "hf_mult_motion_control"}`.
pub const MOTION_CONTROL: &str =
    include_str!("../../fixtures/mcp-snapshots/answers-2026-10-28/motion-control.json");
/// The first `models_explore {action: "list", type: "video"}` page, cut to two of its 20 items.
pub const VIDEO_LIST: &str = include_str!("../../fixtures/mcp-snapshots/answers-2026-10-28/video-list.json");
/// Four items of the first `models_explore {action: "list", type: "image"}` page (of 20; `has_more: true`).
pub const IMAGE_LIST: &str = include_str!("../../fixtures/mcp-snapshots/answers-2026-10-28/image-list.json");

/// One of the answers above as JSON, with the top-level keys in `changes` replaced.
pub fn value(text: &str, changes: &[(&str, Value)]) -> Value {
    let mut value: Value = serde_json::from_str(text).unwrap();
    if let Value::Object(object) = &mut value {
        for (key, change) in changes {
            object.insert((*key).to_owned(), change.clone());
        }
    }
    value
}

/// The newest snapshot; `tool` and `model` read it.
static SNAPSHOT: LazyLock<Value> = LazyLock::new(|| {
    let text = std::fs::read_to_string(format!("{SNAPSHOT_DIR}/2026-11-10.json")).unwrap();
    serde_json::from_str(&text).unwrap()
});

/// A call the server answered live, and its answer.
#[derive(Debug, Clone)]
pub struct LiveAnswer {
    pub tool: String,
    pub arguments: Value,
    pub answer: Value,
    /// The answer was `isError`, with `text` as its first line.
    pub is_error: bool,
    pub text: Option<String>,
}

/// A live answer from `live-2026-10-15/<name>.json`: `{tool, arguments, answer}`.
pub fn live_answer(name: &str) -> LiveAnswer {
    let text = std::fs::read_to_string(format!("{SNAPSHOT_DIR}/live-2026-10-15/{name}.json")).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    LiveAnswer {
        tool: value["tool"].as_str().unwrap().to_owned(),
        arguments: value["arguments"].clone(),
        answer: value["answer"].clone(),
        is_error: value["isError"] == true,
        text: value["text"].as_str().map(str::to_owned),
    }
}

/// A `tools/list` entry of the snapshot as the server sent it, for a stub to list.
pub fn tool(name: &str) -> Value {
    SNAPSHOT["tools"]
        .as_array()
        .and_then(|tools| tools.iter().find(|tool| tool["name"] == name))
        .cloned()
        .unwrap_or_else(|| panic!("no tool {name} in the snapshot"))
}

/// The snapshot's `models_explore {action: "get", model_id: id}` answer.
pub fn model(id: &str) -> Value {
    let model = &SNAPSHOT["models"]["get"][id]["structuredContent"];
    assert!(model.is_object(), "no model {id} in the snapshot");
    model.clone()
}

/// `generate_video`'s own `params`, which no model schema lists.
const TOOL_PARAMS: [&str; 6] = ["model", "prompt", "medias", "use_unlim", "get_cost", "count"];

/// JSON equality with numbers compared by value, as Swift's `JSONValue` holds `5` and `5.0` alike.
pub fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b)),
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).is_some_and(|other| same(value, other)))
        }
        _ => a == b,
    }
}

fn contains(options: &[Value], value: &Value) -> bool {
    options.iter().any(|option| same(option, value))
}

/// Where the `params` of a `generate_video` call break the schema `models_explore get` gives for their `model`.
pub fn violations_of_params(params: &Value) -> Vec<String> {
    let (Some(object), Some(id)) = (params.as_object(), params["model"].as_str()) else {
        return vec!["no model".into()];
    };
    let model = model(id);
    let mut found = Vec::new();
    let roles: BTreeSet<&str> = model["medias"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|media| media["roles"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    for media in params["medias"].as_array().into_iter().flatten() {
        let role = media["role"].as_str().unwrap_or("");
        if !roles.contains(role) {
            found.push(format!("{id}: role {role} isn't one of {roles:?}"));
        }
    }
    let mut parameters: HashMap<&str, &Value> = HashMap::new();
    for parameter in model["parameters"].as_array().into_iter().flatten() {
        if let Some(name) = parameter["name"].as_str() {
            parameters.entry(name).or_insert(parameter);
        }
    }
    let mut keys: Vec<_> = object
        .iter()
        .filter(|(key, _)| !TOOL_PARAMS.contains(&key.as_str()))
        .collect();
    keys.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in keys {
        if key == "aspect_ratio" {
            if !model["aspect_ratios"]
                .as_array()
                .is_some_and(|ratios| contains(ratios, value))
            {
                found.push(format!("{id}: aspect_ratio {value} isn't listed"));
            }
            continue;
        }
        let Some(parameter) = parameters.get(key.as_str()) else {
            found.push(format!("{id}: {key} isn't a parameter"));
            continue;
        };
        found.extend(parameter_violations(value, parameter, &format!("{id}.{key}")));
    }
    found
}

/// One value against one entry of a model's `parameters`.
fn parameter_violations(value: &Value, parameter: &Value, path: &str) -> Vec<String> {
    let typed = match (parameter["type"].as_str(), value) {
        (Some("number"), Value::Number(_))
        | (Some("string"), Value::String(_))
        | (Some("bool"), Value::Bool(_)) => true,
        (Some("string_array"), Value::Array(items)) => items.iter().all(Value::is_string),
        _ => false,
    };
    if !typed {
        return vec![format!("{path}: {value} isn't a {}", parameter["type"])];
    }
    let mut found = Vec::new();
    if let Some(options) = parameter["options"].as_array()
        && !contains(options, value)
    {
        found.push(format!("{path}: {value} is none of {options:?}"));
    }
    if let Some(number) = value.as_f64() {
        if parameter["min"].as_f64().is_some_and(|minimum| number < minimum) {
            found.push(format!("{path}: below min"));
        }
        if parameter["max"].as_f64().is_some_and(|maximum| number > maximum) {
            found.push(format!("{path}: above max"));
        }
    }
    found
}

/// Where `arguments` break the `inputSchema` of the snapshot's tool `name`; empty when they keep to it.
pub fn violations(arguments: &Value, name: &str) -> Vec<String> {
    let tool = tool(name);
    schema_violations(arguments, &tool["inputSchema"], name)
}

/// The parts of JSON Schema the catalog uses: `type`, `properties`, `required`, `additionalProperties: false`,
/// `enum`, `const`, `anyOf`, `oneOf`, `items`, `minItems`/`maxItems`, `minimum`/`maximum`/`exclusiveMinimum`,
/// `minLength`/`maxLength` and `pattern`.
fn schema_violations(value: &Value, schema: &Value, path: &str) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(kind) = schema["type"].as_str()
        && !matches_type(value, kind)
    {
        return vec![format!("{path}: not of type {kind}")];
    }
    if let Some(options) = schema["enum"].as_array()
        && !contains(options, value)
    {
        found.push(format!("{path}: {value} is none of {options:?}"));
    }
    if let Some(constant) = schema.get("const")
        && !same(constant, value)
    {
        found.push(format!("{path}: {value} isn't {constant}"));
    }
    found.extend(variants(value, schema, path));
    found.extend(bounds(value, schema, path));
    if let Value::Object(object) = value {
        found.extend(properties(object, schema, path));
    }
    if let (Value::Array(items), Some(item_schema)) = (value, schema.get("items")) {
        for (index, item) in items.iter().enumerate() {
            found.extend(schema_violations(item, item_schema, &format!("{path}[{index}]")));
        }
    }
    found
}

fn matches_type(value: &Value, kind: &str) -> bool {
    match (kind, value) {
        ("object", Value::Object(_))
        | ("array", Value::Array(_))
        | ("string", Value::String(_))
        | ("boolean", Value::Bool(_))
        | ("null", Value::Null)
        | ("number", Value::Number(_)) => true,
        ("integer", Value::Number(number)) => {
            number.is_i64() || number.is_u64() || number.as_f64().is_some_and(|number| number.fract() == 0.0)
        }
        _ => false,
    }
}

/// `anyOf` needs one variant to hold, `oneOf` exactly one.
fn variants(value: &Value, schema: &Value, path: &str) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(any_of) = schema["anyOf"].as_array()
        && !any_of
            .iter()
            .any(|variant| schema_violations(value, variant, path).is_empty())
    {
        found.push(format!("{path}: matches no anyOf variant"));
    }
    if let Some(one_of) = schema["oneOf"].as_array() {
        let holding = one_of
            .iter()
            .filter(|variant| schema_violations(value, variant, path).is_empty())
            .count();
        if holding != 1 {
            found.push(format!("{path}: matches {holding} oneOf variants, not 1"));
        }
    }
    found
}

#[allow(clippy::cast_precision_loss)]
fn bounds(value: &Value, schema: &Value, path: &str) -> Vec<String> {
    let mut found = Vec::new();
    let below = |key: &str, number: f64| schema[key].as_f64().is_some_and(|limit| number < limit);
    let above = |key: &str, number: f64| schema[key].as_f64().is_some_and(|limit| number > limit);
    match value {
        Value::Number(number) => {
            let number = number.as_f64().unwrap_or(0.0);
            if below("minimum", number) {
                found.push(format!("{path}: below minimum"));
            }
            if above("maximum", number) {
                found.push(format!("{path}: above maximum"));
            }
            if schema["exclusiveMinimum"]
                .as_f64()
                .is_some_and(|floor| number <= floor)
            {
                found.push(format!("{path}: not above exclusiveMinimum"));
            }
        }
        Value::String(text) => {
            let length = text.chars().count() as f64;
            if below("minLength", length) {
                found.push(format!("{path}: shorter than minLength"));
            }
            if above("maxLength", length) {
                found.push(format!("{path}: longer than maxLength"));
            }
            if let Some(pattern) = schema["pattern"].as_str()
                && !regex::Regex::new(pattern).unwrap().is_match(text)
            {
                found.push(format!("{path}: \"{text}\" doesn't match the pattern"));
            }
        }
        Value::Array(items) => {
            if below("minItems", items.len() as f64) {
                found.push(format!("{path}: fewer than minItems"));
            }
            if above("maxItems", items.len() as f64) {
                found.push(format!("{path}: more than maxItems"));
            }
        }
        _ => {}
    }
    found
}

fn properties(object: &Map<String, Value>, schema: &Value, path: &str) -> Vec<String> {
    let mut found = Vec::new();
    for key in schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !object.contains_key(key) {
            found.push(format!("{path}: missing {key}"));
        }
    }
    let mut entries: Vec<_> = object.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in entries {
        if let Some(property) = schema["properties"].get(key) {
            found.extend(schema_violations(value, property, &format!("{path}.{key}")));
        } else if schema["additionalProperties"] == false {
            found.push(format!("{path}: {key} isn't allowed"));
        }
    }
    found
}
