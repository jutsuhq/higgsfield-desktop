//! `create_image`, `create_element`, the Library tools and their paging, as the helper turns them into commands.

use super::{FakeApp, Files, call, command_in, object};
use crate::Tool;
use jutsu_agent::{
    AgentAmount, AgentBudgetUnit, AgentCommand, AgentCreated, AgentError, AgentExportPreset,
    AgentLibraryKind, AgentRequest, AgentResponse, AgentVerdict, AgentVideoRequest, CreateElementCommand,
    CreateImageCommand, ErrorCode, QuoteCommand, Timestamp, Uuid, names, wire,
};
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Arc};

fn app() -> Arc<FakeApp> {
    FakeApp::new(AgentResponse {
        created: Some(AgentCreated::new("img-1", false, Vec::new())),
        ..AgentResponse::default()
    })
}

fn with(mut arguments: Value, extra: &Value) -> Value {
    for (key, value) in object(extra) {
        arguments[key] = value;
    }
    arguments
}

fn image(extra: &Value) -> Value {
    with(
        json!({ "request_id": "img-1", "prompt": "a red fox", "max_cost": { "amount": 40, "unit": "credits" } }),
        extra,
    )
}

fn element(extra: &Value) -> Value {
    with(json!({ "request_id": "el-1", "name": "Mara" }), extra)
}

fn create_image(arguments: &Value, folder: &Path) -> CreateImageCommand {
    match command_in(Tool::CreateImage, arguments, folder) {
        Ok(AgentCommand::CreateImage(command)) => command,
        other => panic!("not a create_image: {other:?}"),
    }
}

fn refused(tool: Tool, arguments: &Value, folder: &Path) -> AgentError {
    match command_in(tool, arguments, folder) {
        Err(error) => error,
        Ok(command) => panic!("taken: {arguments}: {command:?}"),
    }
}

fn round_trips(command: &AgentCommand) {
    let line = wire::line(&AgentRequest::new(command.clone())).expect("line");
    assert_eq!(&wire::request(&line).1.expect("request").command, command);
}

// create_image

#[tokio::test]
async fn create_image_stages_its_input_then_sends_the_copy() {
    let (app, files) = (app(), Files::new());
    let input = files.file("fox.png", 3);
    let result = call(
        &app,
        "0.3.0",
        "create_image",
        image(&json!({ "input": input, "count": 4 })),
    )
    .await;

    assert_eq!(result["isError"], false);
    let staged = app.staged();
    assert_eq!(app.staged_paths(), [input]);
    assert_eq!(staged[0].1.command, "create_image");
    let Some(AgentCommand::CreateImage(command)) = app.commands().first().cloned() else {
        panic!("create_image wasn't sent");
    };
    assert_eq!(command.image.inputs, ["/tmp/inbox/1/fox.png"]);
    assert_eq!(command.image.count, Some(4));
    assert_eq!(command.fingerprint, staged[0].1.fingerprint);
    assert_eq!(command.max_cost, AgentAmount::new(40, AgentBudgetUnit::Credits));
}

/// inputs are staged and sent in order; input is the same request with one image, so it keeps its fingerprint.
#[tokio::test]
async fn several_inputs_are_staged_in_order_and_input_is_their_one_item_form() {
    let (app, files) = (app(), Files::new());
    let paths: Vec<String> = ["a.png", "b.png", "c.png"]
        .iter()
        .map(|name| files.file(name, 3))
        .collect();
    let result = call(&app, "0.3.0", "create_image", image(&json!({ "inputs": paths }))).await;
    assert_eq!(result["isError"], false);
    assert_eq!(app.staged_paths(), paths);
    let Some(AgentCommand::CreateImage(staged)) = app.commands().first().cloned() else {
        panic!("create_image wasn't sent");
    };
    assert_eq!(staged.image.inputs.len(), 3);
    assert!(
        staged
            .image
            .inputs
            .iter()
            .all(|input| input.starts_with("/tmp/inbox/"))
    );

    let one = create_image(&image(&json!({ "input": paths[0] })), files.path());
    assert_eq!(
        create_image(&image(&json!({ "inputs": [paths[0]] })), files.path()).fingerprint,
        one.fingerprint
    );
    // One input goes on the wire as `input`, as the 0.2.0 helper sends it.
    let line = String::from_utf8(
        wire::line(&AgentRequest::new(AgentCommand::CreateImage(one.clone()))).expect("line"),
    )
    .expect("text");
    assert!(line.contains("\"input\":") && !line.contains("\"inputs\""));
    round_trips(&AgentCommand::CreateImage(one));

    let many: Vec<String> = (0..=16).map(|index| format!("/tmp/{index}.png")).collect();
    for bad in [
        image(&json!({ "input": paths[0], "inputs": [paths[1]] })),
        image(&json!({ "inputs": many })),
        image(&json!({ "inputs": "a.png" })),
    ] {
        assert_eq!(
            refused(Tool::CreateImage, &bad, files.path()).code,
            ErrorCode::BAD_REQUEST
        );
    }
}

// create_element

#[tokio::test]
async fn create_element_stages_its_images_in_order_then_sends_the_copies() {
    let (app, files) = (app(), Files::new());
    let paths: Vec<String> = ["face.png", "side.png"]
        .iter()
        .map(|name| files.file(name, 3))
        .collect();
    let result = call(
        &app,
        "0.3.0",
        "create_element",
        element(&json!({ "images": paths, "category": "prop" })),
    )
    .await;

    assert_eq!(result["isError"], false);
    let staged = app.staged();
    assert_eq!(app.staged_paths(), paths);
    assert!(
        staged
            .iter()
            .all(|(_, request)| request.command == "create_element" && request.request_id == "el-1")
    );
    let Some(AgentCommand::CreateElement(command)) = app.commands().first().cloned() else {
        panic!("create_element wasn't sent");
    };
    assert_eq!(command.images, ["/tmp/inbox/1/face.png", "/tmp/inbox/2/side.png"]);
    assert_eq!(command.name, "Mara");
    assert_eq!(command.category.as_deref(), Some("prop"));
    assert_eq!(command.fingerprint, staged[0].1.fingerprint);
    round_trips(&AgentCommand::CreateElement(command));
}

/// The same images and fields are the same request, whatever the category left out stands for; anything else, or an
/// image that changed, is another.
#[test]
fn the_element_fingerprint_follows_the_request() {
    let files = Files::new();
    let face = files.file("face.png", 3);
    let fingerprint = |arguments: Value| -> String {
        match command_in(Tool::CreateElement, &arguments, files.path()) {
            Ok(AgentCommand::CreateElement(CreateElementCommand { fingerprint, .. })) => fingerprint,
            other => panic!("not a create_element: {other:?}"),
        }
    };
    let first = fingerprint(element(&json!({ "images": [face] })));
    assert_eq!(
        fingerprint(element(&json!({ "images": [face], "category": "character" }))),
        first
    );
    assert_ne!(
        fingerprint(element(&json!({ "images": [face], "category": "prop" }))),
        first
    );
    assert_ne!(
        fingerprint(element(&json!({ "images": [face], "name": "Mira" }))),
        first
    );
    fs::write(&face, [2; 9]).expect("face");
    assert_ne!(fingerprint(element(&json!({ "images": [face] }))), first);
}

#[tokio::test]
async fn an_element_request_is_checked_before_anything_is_sent() {
    let (app, files) = (app(), Files::new());
    let face = json!([files.file("face.png", 3)]);
    let many: Vec<String> = (0..=10).map(|index| format!("/tmp/{index}.png")).collect();
    for bad in [
        element(&json!({})),
        element(&json!({ "images": [] })),
        json!({ "request_id": "el-1", "images": face }),
        element(&json!({ "images": face, "name": "   " })),
        element(&json!({ "images": face, "name": "a".repeat(33) })),
        element(&json!({ "images": face, "category": "auto" })),
        element(&json!({ "images": many })),
        element(&json!({ "images": face, "max_cost": { "amount": 1, "unit": "credits" } })),
    ] {
        assert_eq!(
            refused(Tool::CreateElement, &bad, files.path()).code,
            ErrorCode::BAD_REQUEST
        );
    }
    assert_eq!(app.staged().len(), 0);
}

#[tokio::test]
async fn a_conflicting_image_request_stops_before_any_byte_or_command() {
    let (app, files) = (app(), Files::new());
    app.refuse(AgentError::request_conflict());
    let result = call(
        &app,
        "0.3.0",
        "create_image",
        image(&json!({ "input": files.file("fox.png", 3) })),
    )
    .await;
    assert_eq!(result["structuredContent"]["error"]["code"], "request_conflict");
    assert_eq!(app.calls().len(), 0);
}

#[test]
fn the_image_fingerprint_follows_the_request_and_is_never_a_videos() {
    let files = Files::new();
    let input = files.file("fox.png", 3);
    let fingerprint = |arguments: Value| create_image(&arguments, files.path()).fingerprint;
    let first = fingerprint(image(&json!({ "input": input })));
    assert_eq!(fingerprint(image(&json!({ "input": input }))), first);
    assert_ne!(fingerprint(image(&json!({ "input": input, "count": 4 }))), first);
    assert_ne!(
        fingerprint(image(
            &json!({ "max_cost": { "amount": 41, "unit": "credits" }, "input": input })
        )),
        first
    );
    fs::write(&input, [2; 9]).expect("input");
    assert_ne!(fingerprint(image(&json!({ "input": input }))), first);
    let video = json!({
        "request_id": "img-1", "mode": "restyle", "prompt": "a red fox", "max_cost": { "amount": 40, "unit": "credits" },
    });
    let Ok(AgentCommand::CreateVideo(video)) = command_in(Tool::CreateVideo, &video, files.path()) else {
        panic!("no create_video");
    };
    assert_ne!(video.fingerprint, first);
}

#[test]
fn bad_image_arguments_never_reach_the_app() {
    let files = Files::new();
    let bad = [
        image(&json!({ "request_id": null })),
        image(&json!({ "max_cost": null })),
        image(&json!({ "prompt": "  " })),
        image(&json!({ "count": 0 })),
        image(&json!({ "count": 17 })),
        image(&json!({ "model": "../x" })),
        image(&json!({ "size": "big" })),
    ];
    for arguments in bad {
        assert_eq!(
            refused(Tool::CreateImage, &arguments, files.path()).code,
            ErrorCode::BAD_REQUEST
        );
    }
}

// The Library

#[test]
fn library_search_takes_its_filters() {
    let files = Files::new();
    let arguments = json!({
        "query": "fox", "kind": "video", "starred": false, "verdict": "kept", "mode": "restyle",
        "from": "2026-10-01T00:00:00Z", "to": "2026-10-21T00:00:00.500Z", "limit": 5, "cursor": "abc_1",
    });
    let Ok(AgentCommand::LibrarySearch(search)) = command_in(Tool::LibrarySearch, &arguments, files.path())
    else {
        panic!("no library_search");
    };
    assert_eq!(search.kind, Some(AgentLibraryKind::Video));
    assert_eq!(search.starred, Some(false));
    assert_eq!(search.verdict, Some(AgentVerdict::Kept));
    assert_eq!(search.from, Timestamp::parse_wire("2026-10-01T00:00:00Z"));
    assert!(search.to.is_some());
    assert!(search.include_higgsfield.is_none());
    let history = json!({ "include_higgsfield": true, "refresh_higgsfield": true });
    let Ok(AgentCommand::LibrarySearch(history)) = command_in(Tool::LibrarySearch, &history, files.path())
    else {
        panic!("no library_search of the history");
    };
    assert!(history.include_higgsfield == Some(true) && history.refresh_higgsfield == Some(true));
    for bad in [
        json!({ "kind": "audio" }),
        json!({ "from": "yesterday" }),
        json!({ "limit": 51 }),
        json!({ "mode": "dance" }),
        json!({ "cursor": "a b" }),
        json!({ "refresh_higgsfield": true }),
        json!({ "from": "2026-10-02T00:00:00Z", "to": "2026-10-01T00:00:00Z" }),
    ] {
        refused(Tool::LibrarySearch, &bad, files.path());
    }
}

/// Elements by name, in `create_video`, `quote` and `create_image`; a request without them keeps its wire shape.
#[test]
fn elements_are_names_checked_before_the_app() {
    let files = Files::new();
    let listed = json!({ "query": "mara", "category": "character" });
    let Ok(AgentCommand::Elements(list)) = command_in(Tool::Elements, &listed, files.path()) else {
        panic!("no elements");
    };
    assert!(list.query.as_deref() == Some("mara") && list.category.as_deref() == Some("character"));
    let made = create_image(&image(&json!({ "elements": ["Mara", "Roof"] })), files.path());
    assert_eq!(made.image.elements, ["Mara", "Roof"]);
    let plain = AgentRequest::new(AgentCommand::Quote(QuoteCommand {
        video: AgentVideoRequest::new("restyle"),
    }));
    assert!(!String::from_utf8_lossy(&wire::line(&plain).expect("line")).contains("elements"));
    let quote = json!({ "mode": "motion_transfer", "elements": ["Mara"] });
    let Ok(AgentCommand::Quote(quoted)) = command_in(Tool::Quote, &quote, files.path()) else {
        panic!("no quote with elements");
    };
    assert_eq!(quoted.video.elements, ["Mara"]);
    let cases = [
        (Tool::Elements, json!({ "category": "pet" })),
        (Tool::Elements, json!({ "limit": 51 })),
        (
            Tool::Quote,
            json!({ "mode": "restyle", "elements": ["a", "b", "c", "d", "e"] }),
        ),
        (
            Tool::Quote,
            json!({ "mode": "restyle", "elements": ["Mara", "mara"] }),
        ),
        // The lookup trims names, so these are one element twice.
        (
            Tool::Quote,
            json!({ "mode": "restyle", "elements": ["Mara", "Mara "] }),
        ),
        (Tool::Quote, json!({ "mode": "restyle", "elements": [" "] })),
        (Tool::CreateImage, image(&json!({ "elements": "Mara" }))),
    ];
    for (tool, bad) in cases {
        assert_eq!(refused(tool, &bad, files.path()).code, ErrorCode::BAD_REQUEST);
    }
}

/// `library_get` and `library_download` take the same IDs: one to twenty items, each once.
#[test]
fn library_get_and_download_take_up_to_twenty_item_ids() {
    let files = Files::new();
    let id = Uuid::new_v4().to_string();
    let many: Vec<String> = (0..=20).map(|_| Uuid::new_v4().to_string()).collect();
    for tool in [Tool::LibraryGet, Tool::LibraryDownload] {
        let ids = match (tool, command_in(tool, &json!({ "ids": [id] }), files.path())) {
            (Tool::LibraryGet, Ok(AgentCommand::LibraryGet(get))) => get.ids,
            (Tool::LibraryDownload, Ok(AgentCommand::LibraryDownload(download))) => download.ids,
            (tool, other) => panic!("{tool:?}: {other:?}"),
        };
        assert_eq!(
            ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
            std::slice::from_ref(&id)
        );
        for bad in [
            json!({}),
            json!({ "ids": [] }),
            json!({ "ids": [id, id] }),
            json!({ "ids": ["not-an-id"] }),
            json!({ "ids": [id], "full": true }),
            json!({ "ids": many }),
        ] {
            refused(tool, &bad, files.path());
        }
    }
}

/// frames' own files are staged under a request named by its fingerprint, then the command goes with inbox paths.
#[tokio::test]
async fn frames_stages_its_files_and_sends_the_command() {
    let (app, files) = (app(), Files::new());
    let (main, other) = (files.file("take.mov", 3), files.file("source.mov", 3));
    let arguments = json!({ "file": main, "times": [1, 2.5], "compare_file": other });
    let result = call(&app, "0.3.0", "frames", arguments).await;
    assert_eq!(result["isError"], false);
    assert_eq!(app.staged_paths(), [main, other]);
    assert!(
        app.staged()
            .iter()
            .all(|(_, request)| request.command == "frames")
    );
    let Some(AgentCommand::Frames(staged)) = app.commands().first().cloned() else {
        panic!("frames wasn't sent")
    };
    assert_eq!(staged.times, [1.0, 2.5]);
    assert!(staged.files().iter().all(|file| file.starts_with("/tmp/inbox/")));
}

#[test]
fn frames_takes_one_clip_and_at_most_one_comparison() {
    let files = Files::new();
    let id = Uuid::new_v4().to_string();
    let asked = json!({ "item": id, "times": [0], "compare_source": true });
    let Ok(AgentCommand::Frames(item)) = command_in(Tool::Frames, &asked, files.path()) else {
        panic!("no frames")
    };
    assert!(item.compare_source && item.item.map(|item| item.to_string()) == Some(id.clone()));
    let many: Vec<f64> = (0..=12).map(f64::from).collect();
    for bad in [
        json!({ "times": [1] }),
        json!({ "item": id, "file": "/tmp/a.mov", "times": [1] }),
        json!({ "item": id }),
        json!({ "item": id, "times": [] }),
        json!({ "item": id, "times": many }),
        json!({ "item": id, "times": [-1] }),
        json!({ "file": "/tmp/a.mov", "times": [1], "compare_source": true }),
        json!({ "item": id, "times": [1], "compare_item": id, "compare_file": "/tmp/b.mov" }),
        json!({ "item": "nope", "times": [1] }),
    ] {
        assert_eq!(
            refused(Tool::Frames, &bad, files.path()).code,
            ErrorCode::BAD_REQUEST
        );
    }
}

#[test]
fn library_mark_needs_something_to_mark() {
    let files = Files::new();
    let id = Uuid::new_v4().to_string();
    let Ok(AgentCommand::LibraryMark(mark)) = command_in(
        Tool::LibraryMark,
        &json!({ "ids": [id], "star": true }),
        files.path(),
    ) else {
        panic!("no library_mark");
    };
    assert!(mark.star == Some(true) && mark.verdict.is_none());
    for bad in [
        json!({ "ids": [id] }),
        json!({ "ids": [], "star": true }),
        json!({ "ids": [id], "verdict": "delete" }),
    ] {
        refused(Tool::LibraryMark, &bad, files.path());
    }
}

#[test]
fn enhance_is_fingerprinted_and_checks_its_options() {
    let files = Files::new();
    let item = Uuid::new_v4().to_string();
    let base = json!({
        "request_id": "up-1", "item": item, "operation": "upscale", "target": "4k",
        "max_cost": { "amount": 20, "unit": "credits" },
    });
    let enhance = |arguments: &Value| match command_in(Tool::Enhance, arguments, files.path()) {
        Ok(AgentCommand::Enhance(command)) => command,
        other => panic!("no enhance: {other:?}"),
    };
    let first = enhance(&base);
    assert_eq!(first.target.as_deref(), Some("4k"));
    let second = enhance(&with(base.clone(), &json!({ "target": "2k" })));
    assert_ne!(first.fingerprint, second.fingerprint);
    assert!(names::is_valid_fingerprint(&first.fingerprint));
    refused(
        Tool::Enhance,
        &with(base, &json!({ "language": "fra" })),
        files.path(),
    );
}

#[test]
fn delivery_set_and_export_commands() {
    let files = Files::new();
    let id = Uuid::new_v4().to_string();
    let Ok(AgentCommand::DeliverySet(create)) = command_in(
        Tool::DeliverySet,
        &json!({ "action": "create", "items": [id] }),
        files.path(),
    ) else {
        panic!("no delivery_set");
    };
    assert_eq!(create.items.len(), 1);
    refused(Tool::DeliverySet, &json!({ "action": "export" }), files.path());
    let Ok(AgentCommand::Export(export)) = command_in(
        Tool::Export,
        &json!({ "items": [id], "preset": "premiere" }),
        files.path(),
    ) else {
        panic!("no export");
    };
    assert_eq!(export.preset, AgentExportPreset::Premiere);
    assert_eq!(export.wait_seconds, None);
    refused(
        Tool::Export,
        &json!({ "items": [id, id], "preset": "premiere" }),
        files.path(),
    );
}

/// An export answers with its handle; a wait for it is as bounded as a jobs wait, and only an export waits.
#[test]
fn exports_take_a_bounded_wait() {
    let files = Files::new();
    let id = Uuid::new_v4().to_string();
    let waiting = json!({ "items": [id], "preset": "premiere", "wait_seconds": 50 });
    let Ok(AgentCommand::Export(export)) = command_in(Tool::Export, &waiting, files.path()) else {
        panic!("no export");
    };
    assert_eq!(export.wait_seconds, Some(50));
    let set = Uuid::new_v4().to_string();
    let delivery = json!({ "action": "export", "set": set, "wait_seconds": 10 });
    let Ok(AgentCommand::DeliverySet(delivery)) = command_in(Tool::DeliverySet, &delivery, files.path())
    else {
        panic!("no delivery export");
    };
    assert_eq!(delivery.wait_seconds, Some(10));
    let cases = [
        (
            Tool::Export,
            json!({ "items": [id], "preset": "premiere", "wait_seconds": 51 }),
        ),
        (
            Tool::Export,
            json!({ "items": [id], "preset": "premiere", "wait_seconds": -1 }),
        ),
        (Tool::DeliverySet, json!({ "action": "list", "wait_seconds": 5 })),
    ];
    for (tool, arguments) in cases {
        refused(tool, &arguments, files.path());
    }
}

#[test]
fn jobs_pages_with_a_cursor_and_capabilities_lists_more() {
    let files = Files::new();
    let Ok(AgentCommand::Jobs(jobs)) = command_in(
        Tool::Jobs,
        &json!({ "cursor": "1729500000123_ABC" }),
        files.path(),
    ) else {
        panic!("no jobs");
    };
    assert_eq!(jobs.cursor.as_deref(), Some("1729500000123_ABC"));
    refused(
        Tool::Jobs,
        &json!({ "cursor": "x", "ids": [Uuid::new_v4().to_string()] }),
        files.path(),
    );
    let more = json!({ "image_models": true, "styles": true, "voices": true });
    let Ok(AgentCommand::Capabilities(capabilities)) = command_in(Tool::Capabilities, &more, files.path())
    else {
        panic!("no capabilities");
    };
    assert!(capabilities.image_models && capabilities.styles && capabilities.voices);
}
