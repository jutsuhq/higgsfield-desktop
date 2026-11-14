//! `subtitles` as the helper turns its arguments into commands: make idempotent by its fingerprint, get and export
//! without one, and arguments refused before anything reaches the app.

use super::{command, object};
use crate::Tool;
use jutsu_agent::{
    AgentCommand, AgentError, AgentSheet, AgentSubtitleEngine, AgentSubtitleFormat, ErrorCode,
    SubtitlesCommand, Uuid, names,
};
use serde_json::{Value, json};

fn subtitles(item: &str, arguments: &Value) -> Result<AgentCommand, AgentError> {
    let mut all = json!({ "item": item });
    for (key, value) in object(arguments) {
        all[key] = value;
    }
    command(Tool::Subtitles, &all)
}

fn subtitles_command(item: &str, arguments: &Value) -> SubtitlesCommand {
    match subtitles(item, arguments) {
        Ok(AgentCommand::Subtitles(command)) => command,
        other => panic!("no subtitles for {arguments}: {other:?}"),
    }
}

#[test]
fn make_is_fingerprinted_by_its_parameters() {
    let item = Uuid::new_v4().to_string();
    let make = |arguments: Value| subtitles_command(&item, &arguments);
    let first = make(json!({ "action": "make", "request_id": "subs-1", "language": "en-US" }));
    assert!(first.engine.is_none() && first.language.as_deref() == Some("en-US"));
    let fingerprint = first.fingerprint.clone().expect("fingerprint");
    assert!(names::is_valid_fingerprint(&fingerprint));
    let same = make(json!({ "action": "make", "request_id": "subs-1", "language": "en-US" }));
    assert_eq!(same.fingerprint.as_deref(), Some(fingerprint.as_str()));
    let other =
        make(json!({ "action": "make", "request_id": "subs-1", "language": "en-US", "engine": "parakeet" }));
    assert_ne!(other.fingerprint.as_deref(), Some(fingerprint.as_str()));
    assert_eq!(other.engine, Some(AgentSubtitleEngine::Parakeet));
    let unnamed = make(json!({ "action": "make", "request_id": "subs-1" }));
    assert_ne!(unnamed.fingerprint.as_deref(), Some(fingerprint.as_str()));
}

#[test]
fn get_and_export_take_no_request() {
    let item = Uuid::new_v4().to_string();
    let get = subtitles_command(&item, &json!({ "action": "get", "format": "vtt" }));
    assert!(
        get.fingerprint.is_none() && get.request_id.is_none() && get.format == Some(AgentSubtitleFormat::Vtt)
    );
    let export = subtitles_command(&item, &json!({ "action": "export", "format": "srt" }));
    assert_eq!(export.format, Some(AgentSubtitleFormat::Srt));
}

#[test]
fn bad_arguments_are_refused_before_the_app() {
    let item = Uuid::new_v4().to_string();
    let cases = [
        json!({}),
        json!({ "action": "transcribe" }),
        json!({ "action": "make" }),
        json!({ "action": "make", "request_id": "subs 1" }),
        json!({ "action": "make", "request_id": "subs-1", "engine": "whisper" }),
        json!({ "action": "make", "request_id": "subs-1", "format": "srt" }),
        json!({ "action": "get", "request_id": "subs-1" }),
        json!({ "action": "export" }),
        json!({ "action": "export", "format": "ass" }),
        json!({ "action": "get", "item": "not-an-id" }),
        json!({ "action": "get", "max_cost": { "amount": 1, "unit": "usd" } }),
    ];
    for arguments in cases {
        let Err(error) = subtitles(&item, &arguments) else {
            panic!("{arguments} was taken")
        };
        assert_eq!(error.code, ErrorCode::BAD_REQUEST, "{arguments}");
    }
}

#[test]
fn the_tool_is_listed_with_its_schema_and_the_sheet() {
    let definition = Tool::Subtitles.definition(true);
    assert_eq!(definition["name"], "subtitles");
    assert_eq!(definition["inputSchema"]["required"], json!(["action", "item"]));
    let output = &definition["outputSchema"]["properties"];
    assert!(output.get("cues").is_some() && output.get("jobs").is_some() && output.get("file").is_some());
    assert!(Tool::Subtitles.about().contains("needs_person"));
    let item = Uuid::new_v4().to_string();
    let Ok(AgentCommand::Show(show)) = command(Tool::Show, &json!({ "item": item, "sheet": "subtitles" }))
    else {
        panic!("no show");
    };
    assert_eq!(show.sheet, Some(AgentSheet::Subtitles));
}
