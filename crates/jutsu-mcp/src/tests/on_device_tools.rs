//! `enhance` on this computer and the sheet `show` opens, as the helper turns them into commands and lists them.

use super::{app_refusal, command, object};
use crate::Tool;
use jutsu_agent::{
    AgentCommand, AgentError, AgentSheet, EnhanceCommand, ErrorCode, ShowCommand, Uuid, names,
};
use serde_json::{Value, json};

fn enhance(item: &str, arguments: &Value) -> Result<AgentCommand, AgentError> {
    let mut all = json!({ "request_id": "run-1", "item": item });
    for (key, value) in object(arguments) {
        all[key] = value;
    }
    command(Tool::Enhance, &all)
}

fn enhance_command(item: &str, arguments: &Value) -> EnhanceCommand {
    match enhance(item, arguments) {
        Ok(AgentCommand::Enhance(command)) => command,
        other => panic!("no enhance for {arguments}: {other:?}"),
    }
}

#[test]
fn a_run_on_this_computer_needs_no_max_cost_and_ignores_one() {
    let item = Uuid::new_v4().to_string();
    let smooth = enhance_command(&item, &json!({ "operation": "smooth_motion", "mode": "slow_4x" }));
    assert!(smooth.max_cost.is_none() && smooth.mode.as_deref() == Some("slow_4x") && smooth.is_on_device());
    let paid_for = enhance_command(
        &item,
        &json!({ "operation": "smooth_motion", "mode": "slow_4x", "max_cost": { "amount": 5, "unit": "credits" } }),
    );
    assert!(paid_for.max_cost.is_none() && paid_for.fingerprint == smooth.fingerprint);
    let upscale = enhance_command(&item, &json!({ "operation": "upscale", "engine": "mac" }));
    assert!(upscale.max_cost.is_none() && upscale.is_on_device());
    // A malformed max_cost on a free run is no reason to refuse it.
    let free = json!({ "operation": "upscale", "engine": "mac", "max_cost": "free" });
    assert!(enhance_command(&item, &free).is_on_device());
}

#[test]
fn the_fingerprint_follows_what_the_run_is() {
    let item = Uuid::new_v4().to_string();
    let first = enhance_command(&item, &json!({ "operation": "smooth_motion", "mode": "60fps" }));
    let fingerprint = |arguments: Value| enhance_command(&item, &arguments).fingerprint;
    assert_eq!(
        fingerprint(json!({ "operation": "smooth_motion", "mode": "60fps" })),
        first.fingerprint
    );
    assert_ne!(
        fingerprint(json!({ "operation": "smooth_motion", "mode": "slow_2x" })),
        first.fingerprint
    );
    let on_mac = json!({ "operation": "smooth_motion", "mode": "60fps", "engine": "mac" });
    assert_ne!(fingerprint(on_mac), first.fingerprint);
    assert!(names::is_valid_fingerprint(&first.fingerprint));
}

#[test]
fn bad_on_device_arguments_are_refused_before_the_app() {
    let item = Uuid::new_v4().to_string();
    let cases = [
        json!({ "operation": "smooth_motion" }),
        json!({ "operation": "smooth_motion", "mode": "120fps" }),
        json!({ "operation": "smooth_motion", "mode": 60 }),
        json!({ "operation": "smooth_motion", "mode": "60fps", "engine": "standard" }),
        json!({ "operation": "upscale", "engine": "mac", "frame_rate": 60 }),
        json!({ "operation": "upscale", "engine": "mac", "preset": "aigc" }),
        json!({ "operation": "reframe", "aspect_ratio": "9:16", "preset": "aigc" }),
        json!({ "operation": "upscale", "engine": "mac", "target": "4k", "aspect_ratio": "9:16" }),
        json!({ "operation": "upscale", "engine": "standard" }),
        json!({ "operation": "upscale" }),
        json!({ "operation": "reframe", "aspect_ratio": "9:16", "mode": "60fps" }),
    ];
    for arguments in cases {
        let Err(error) = enhance(&item, &arguments) else {
            panic!("taken: {arguments}")
        };
        assert_eq!(error.code, ErrorCode::BAD_REQUEST, "{arguments}");
    }
}

/// The helper and the app refuse the same enhance and show with the same words, as for the other tools.
#[test]
fn the_helper_and_the_app_refuse_with_the_same_words() {
    let item = Uuid::new_v4().to_string();
    let cases = [
        ("enhance", json!({ "operation": "smooth_motion" })),
        (
            "enhance",
            json!({ "operation": "smooth_motion", "mode": "90fps" }),
        ),
        (
            "enhance",
            json!({ "operation": "smooth_motion", "mode": "60fps", "engine": "topaz" }),
        ),
        (
            "enhance",
            json!({ "operation": "upscale", "engine": "mac", "target": "2k", "mode": "60fps" }),
        ),
        ("show", json!({ "item": item })),
        ("show", json!({ "sheet": "upscale_on_mac" })),
        (
            "show",
            json!({ "page": "library", "item": item, "sheet": "upscale_on_mac" }),
        ),
    ];
    for (tool, arguments) in cases {
        let mut given = arguments.clone();
        if tool == "enhance" {
            given["request_id"] = json!("run-1");
            given["item"] = json!(item);
        }
        let Err(helper) = command(Tool::parse(tool).expect("tool"), &given) else {
            panic!("{tool} {arguments} taken")
        };
        let mut fields: serde_json::Map<String, Value> = object(&given)
            .into_iter()
            .map(|(key, value)| {
                (
                    if key == "request_id" {
                        "requestId".to_owned()
                    } else {
                        key
                    },
                    value,
                )
            })
            .collect();
        if tool == "enhance" {
            fields.insert("fingerprint".to_owned(), json!("0".repeat(64)));
        }
        let app = app_refusal(tool, &fields).unwrap_or_else(|| panic!("{tool} {arguments} decoded"));
        let named = if tool == "show" {
            "Malformed request: ".to_owned()
        } else {
            format!("Malformed request: {tool}: ")
        };
        assert_eq!(
            helper.message,
            app.message.replace("Malformed request: ", &named),
            "{tool} {arguments}"
        );
    }
}

#[test]
fn show_opens_an_items_sheet() {
    let id = Uuid::new_v4();
    let shown = command(
        Tool::Show,
        &json!({ "item": id.to_string(), "sheet": "upscale_on_mac" }),
    );
    assert_eq!(
        shown.ok(),
        Some(AgentCommand::Show(ShowCommand::sheet(
            AgentSheet::UpscaleOnMac,
            id
        )))
    );
    let Err(error) = command(Tool::Show, &json!({ "item": id.to_string(), "sheet": "smooth" })) else {
        panic!("an unknown sheet was taken");
    };
    assert_eq!(
        error.message,
        "Malformed request: show: sheet is one of upscale_on_mac, subtitles."
    );
}

#[test]
fn the_schemas_state_the_on_device_options() {
    let enhance = Tool::Enhance.definition(true);
    let properties = &enhance["inputSchema"]["properties"];
    assert_eq!(
        enhance["inputSchema"]["required"],
        json!(["request_id", "item", "operation"])
    );
    assert_eq!(properties["engine"]["enum"], json!(["standard", "mac"]));
    assert_eq!(properties["mode"]["enum"], json!(["60fps", "slow_2x", "slow_4x"]));
    assert!(
        properties["operation"]["enum"]
            .as_array()
            .expect("operations")
            .contains(&json!("smooth_motion"))
    );
    let show = Tool::Show.definition(true)["inputSchema"]["properties"].clone();
    assert_eq!(show["sheet"]["enum"], json!(["upscale_on_mac", "subtitles"]));
    assert_eq!(show["item"]["format"], "uuid");
    let job = Tool::Jobs.definition(true)["outputSchema"]["properties"]["jobs"]["items"].clone();
    assert!(job["properties"].get("estimatedSeconds").is_some() && job["properties"].get("item").is_some());
    let capabilities = Tool::Capabilities.definition(true)["outputSchema"]["properties"].clone();
    assert!(
        capabilities["onDevice"]["properties"]
            .get("upscaleImage")
            .is_some()
    );
    assert!(capabilities["window"]["properties"].get("open").is_some());
    assert!(capabilities["window"]["properties"].get("menuBarMode").is_some());
}
