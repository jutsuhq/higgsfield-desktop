//! `quote`, `create_video`, `jobs` and `job_action` as the helper turns them into commands, and `create_video`'s
//! files handed to the app before the command.

use super::{FakeApp, Files, app_refusal, call, command, object};
use crate::{Tool, server::tool_result};
use jutsu_agent::{
    AgentActed, AgentAmount, AgentBudgetUnit, AgentCommand, AgentCreated, AgentDelivery, AgentDeliveryAction,
    AgentError, AgentExported, AgentJob, AgentJobAction, AgentJobs, AgentLibraryDetailsPage,
    AgentLibraryPage, AgentMarked, AgentQuote, AgentResponse, AgentSource, AgentVideoRequest,
    CreateVideoCommand, ErrorCode, JobActionCommand, JobsCommand, QuoteCommand, Timestamp, Uuid,
};
use serde_json::{Value, json};
use std::{fs, sync::Arc};

fn app() -> Arc<FakeApp> {
    FakeApp::new(AgentResponse {
        created: Some(AgentCreated::new("r1", false, Vec::new())),
        ..AgentResponse::default()
    })
}

fn create(extra: &Value, source: Option<&str>) -> Value {
    let mut arguments = json!({
        "request_id": "r1", "mode": "motion_transfer", "max_cost": { "amount": 2.509, "unit": "usd" },
    });
    if let Some(source) = source {
        arguments["source"] = json!({ "path": source, "start": 1, "end": 6 });
    }
    for (key, value) in object(extra) {
        arguments[key] = value;
    }
    arguments
}

fn create_video(arguments: &Value) -> CreateVideoCommand {
    match command(Tool::CreateVideo, arguments) {
        Ok(AgentCommand::CreateVideo(command)) => command,
        other => panic!("no create_video: {other:?}"),
    }
}

fn sent_video(app: &FakeApp) -> CreateVideoCommand {
    match app.commands().first() {
        Some(AgentCommand::CreateVideo(command)) => command.clone(),
        other => panic!("create_video wasn't sent: {other:?}"),
    }
}

#[tokio::test]
async fn create_video_stages_each_file_for_its_request_then_sends_their_copies() {
    let (app, files) = (app(), Files::new());
    let source = files.file("clip.mov", 3);
    let image = files.file("face.png", 3);

    let result = call(
        &app,
        "0.2.0",
        "create_video",
        create(&json!({ "images": [image] }), Some(&source)),
    )
    .await;

    assert_eq!(result["isError"], false);
    let staged = app.staged();
    assert_eq!(app.staged_paths(), [source, image]);
    assert!(staged.iter().all(|(_, request)| *request == staged[0].1));
    assert_eq!(staged[0].1.request_id, "r1");
    assert_eq!(staged[0].1.command, "create_video");
    let command = sent_video(&app);
    assert_eq!(
        command.video.source.and_then(|source| source.path).as_deref(),
        Some("/tmp/inbox/1/clip.mov")
    );
    assert_eq!(command.video.images, ["/tmp/inbox/2/face.png"]);
    assert_eq!(command.fingerprint, staged[0].1.fingerprint);
    assert!(command.parameters.is_some() && command.parameters == staged[0].1.parameters);
    // Dollars become whole cents, rounded down.
    assert_eq!(command.max_cost, AgentAmount::new(250, AgentBudgetUnit::UsdCents));
}

/// The speech is staged after the images, and its size is part of the request.
#[tokio::test]
async fn create_video_stages_the_speech_too() {
    let (app, files) = (app(), Files::new());
    let image = files.file("face.png", 3);
    let speech = files.file("lines.mp3", 3);

    let arguments = create(&json!({ "images": [image], "speech": speech }), None);
    let result = call(&app, "0.2.0", "create_video", arguments).await;

    assert_eq!(result["isError"], false);
    assert_eq!(app.staged_paths(), [image.clone(), speech.clone()]);
    let command = sent_video(&app);
    assert_eq!(command.video.speech.as_deref(), Some("/tmp/inbox/2/lines.mp3"));
    fs::write(&speech, [2; 10]).expect("speech");
    let changed = create_video(&json!({
        "request_id": "r1", "mode": "motion_transfer", "max_cost": { "amount": 1, "unit": "usd" },
        "images": [image], "speech": speech,
    }));
    assert_ne!(changed.fingerprint, command.fingerprint);
}

/// A quote reads no file: it takes the recording's seconds, never the recording.
#[test]
fn a_quote_takes_the_speech_length_not_the_file() {
    let arguments = json!({ "mode": "motion_transfer", "speech_length": 6.5 });
    let Ok(AgentCommand::Quote(quote)) = command(Tool::Quote, &arguments) else {
        panic!("no quote")
    };
    assert_eq!(quote.video.speech_length, Some(6.5));
    let Err(error) = command(
        Tool::Quote,
        &json!({ "mode": "motion_transfer", "speech": "/tmp/a.mp3" }),
    ) else {
        panic!("a quote took the speech file");
    };
    assert!(
        error
            .message
            .contains("quote takes speech_length, the recording's seconds, not the speech file")
    );
    let speech_length = json!({
        "request_id": "r1", "mode": "motion_transfer", "max_cost": { "amount": 1, "unit": "usd" },
        "speech_length": 6.5,
    });
    assert!(
        command(Tool::CreateVideo, &speech_length).is_err(),
        "create_video took speech_length"
    );
}

/// The same request gives the same fingerprint, and any change another one; the parameters' digest leaves out
/// `max_cost` only: a higher `max_cost` is the same parameters, anything else, a changed file too, isn't.
#[test]
fn the_fingerprint_follows_everything_and_the_parameters_all_but_max_cost() {
    let files = Files::new();
    let source = files.file("clip.mov", 3);
    let digests = |extra: Value| {
        let command = create_video(&create(&extra, Some(&source)));
        (command.fingerprint, command.parameters)
    };
    let first = digests(json!({}));
    assert_eq!(digests(json!({})), first);
    assert_ne!(first.1.as_deref(), Some(first.0.as_str()));
    let raised = digests(json!({ "max_cost": { "amount": 3, "unit": "usd" } }));
    assert!(raised.0 != first.0 && raised.1 == first.1);
    let prompted = digests(json!({ "prompt": "other" }));
    assert!(prompted.0 != first.0 && prompted.1 != first.1);
    fs::write(&source, [2; 10]).expect("source");
    let changed = digests(json!({}));
    assert!(changed.0 != first.0 && changed.1 != first.1);
}

/// `wait_seconds` is how the call answers, not what it asks for: a retry with another wait is the same request.
#[tokio::test]
async fn a_wait_is_sent_but_is_not_part_of_the_request() {
    let (app, files) = (app(), Files::new());
    let source = files.file("clip.mov", 3);
    let waiting = create_video(&create(&json!({ "wait_seconds": 30 }), Some(&source)));
    let plain = create_video(&create(&json!({}), Some(&source)));
    assert!(waiting.wait_seconds == 30 && plain.wait_seconds == 0);
    assert_eq!(waiting.fingerprint, plain.fingerprint);
    let long = call(
        &app,
        "0.2.0",
        "create_video",
        create(&json!({ "wait_seconds": 51 }), None),
    )
    .await;
    assert_eq!(long["structuredContent"]["error"]["code"], "bad_request");
}

/// A title goes to the app as the agent wrote it, and is part of the request; one past the protocol's bound is
/// refused before anything is sent.
#[tokio::test]
async fn a_title_is_sent_with_the_request() {
    let (app, files) = (app(), Files::new());
    let source = files.file("clip.mov", 3);
    let titled = create_video(&create(&json!({ "title": "Knight, take 3" }), Some(&source)));
    assert_eq!(titled.video.title.as_deref(), Some("Knight, take 3"));
    let plain = create_video(&create(&json!({}), Some(&source)));
    assert!(plain.video.title.is_none() && plain.fingerprint != titled.fingerprint);
    let long = call(
        &app,
        "0.2.0",
        "create_video",
        create(&json!({ "title": "a".repeat(1_001) }), None),
    )
    .await;
    assert_eq!(long["structuredContent"]["error"]["code"], "bad_request");
    let image = json!({
        "request_id": "i1", "prompt": "a fox", "title": "a".repeat(1_001), "max_cost": { "amount": 1, "unit": "usd" },
    });
    let image = call(&app, "0.2.0", "create_image", image).await;
    assert_eq!(image["structuredContent"]["error"]["code"], "bad_request");
    assert_eq!(app.calls().len(), 0);
}

/// A request the app already queued: nothing more is uploaded, and the command still goes for its job.
#[tokio::test]
async fn a_known_request_skips_the_uploads_and_still_gets_its_job() {
    let (app, files) = (app(), Files::new());
    let source = files.file("clip.mov", 3);
    app.refuse(AgentError::new(ErrorCode::REQUEST_KNOWN, "known", false, None));

    let arguments = create(&json!({ "images": [files.file("a.png", 3)] }), Some(&source));
    let result = call(&app, "0.2.0", "create_video", arguments).await;

    assert_eq!(result["isError"], false);
    let command = sent_video(&app);
    assert!(command.video.source.and_then(|source| source.path).is_none());
    assert_eq!(command.video.images.len(), 0);
}

/// A conflicting request ID is refused at the first upload; no command follows.
#[tokio::test]
async fn a_conflict_stops_before_the_command() {
    let (app, files) = (app(), Files::new());
    app.refuse(AgentError::request_conflict());

    let result = call(
        &app,
        "0.2.0",
        "create_video",
        create(&json!({}), Some(&files.file("clip.mov", 3))),
    )
    .await;

    assert_eq!(result["structuredContent"]["error"]["code"], "request_conflict");
    assert_eq!(app.calls().len(), 0);
}

#[tokio::test]
async fn bad_video_arguments_never_reach_the_app() {
    let app = app();
    let job = Uuid::new_v4().to_string();
    let dollars = |amount: Value, unit: &str| json!({ "amount": amount, "unit": unit });
    let cases = [
        (
            "create_video",
            json!({ "mode": "motion_transfer", "max_cost": dollars(json!(1), "usd") }),
        ),
        (
            "create_video",
            json!({ "request_id": "r1", "mode": "motion_transfer" }),
        ),
        (
            "create_video",
            json!({ "request_id": "../x", "mode": "motion_transfer", "max_cost": dollars(json!(1), "usd") }),
        ),
        (
            "create_video",
            json!({ "request_id": "r1", "mode": "dance", "max_cost": dollars(json!(1), "usd") }),
        ),
        (
            "create_video",
            json!({ "request_id": "r1", "mode": "restyle", "max_cost": dollars(json!(-1), "usd") }),
        ),
        (
            "create_video",
            json!({ "request_id": "r1", "mode": "restyle", "max_cost": dollars(json!(1e20), "usd") }),
        ),
        (
            "create_video",
            json!({ "request_id": "r1", "mode": "restyle", "max_cost": dollars(json!(1), "eur") }),
        ),
        (
            "quote",
            json!({ "mode": "edit", "source": { "start": 5, "end": 2 } }),
        ),
        ("quote", json!({ "mode": "edit", "source": { "end": 1e30 } })),
        ("quote", json!({ "mode": "edit", "length": 0 })),
        ("quote", json!({ "mode": "edit", "variants": 0 })),
        ("jobs", json!({ "ids": ["nope"] })),
        ("job_action", json!({ "action": "delete", "job": job })),
    ];
    for (name, arguments) in cases {
        let result = call(&app, "0.2.0", name, arguments.clone()).await;
        assert_eq!(
            result["structuredContent"]["error"]["code"], "bad_request",
            "{name} {arguments}"
        );
    }
    assert_eq!(app.calls().len(), 0);
    assert_eq!(app.staged().len(), 0);
}

/// The helper and the app refuse a call with the same words: each command's own check (`problem`) runs on both
/// sides, and the helper only names the tool before it.
#[tokio::test]
async fn the_helper_and_the_app_refuse_with_the_same_words() {
    let app = app();
    let job = Uuid::new_v4().to_string();
    let cases = [
        ("jobs", json!({ "limit": 0 })),
        ("jobs", json!({ "wait_seconds": 51 })),
        ("jobs", json!({ "ids": [job], "cursor": "x" })),
        ("jobs", json!({ "cursor": "no pe" })),
        ("job_action", json!({ "action": "cancel" })),
        ("job_action", json!({ "action": "pause_queue", "job": job })),
        ("capabilities", json!({ "model": "../x" })),
        ("show", json!({})),
    ];
    for (tool, arguments) in cases {
        let result = call(&app, "0.2.0", tool, arguments.clone()).await;
        let helper = result["structuredContent"]["error"]["message"].clone();
        // The same call as the app's command line: camelCase keys beside version and command.
        let fields = object(&arguments)
            .into_iter()
            .map(|(key, value)| {
                (
                    if key == "wait_seconds" {
                        "waitSeconds".to_owned()
                    } else {
                        key
                    },
                    value,
                )
            })
            .collect();
        let refused = app_refusal(tool, &fields).unwrap_or_else(|| panic!("{tool} {arguments} decoded"));
        // show's rule names the tool itself.
        let named = if tool == "show" {
            "Malformed request: ".to_owned()
        } else {
            format!("Malformed request: {tool}: ")
        };
        let expected = refused.message.replace("Malformed request: ", &named);
        assert_eq!(helper, json!(expected), "{tool} {arguments}");
    }
}

#[tokio::test]
async fn a_quote_sends_no_files_only_how_many_there_are() {
    let app = app();
    let arguments =
        json!({ "mode": "motion_transfer", "images": ["/tmp/a.png", "/tmp/b.png"], "source": { "end": 8 } });
    call(&app, "0.2.0", "quote", arguments).await;

    assert_eq!(app.staged().len(), 0);
    let video = AgentVideoRequest {
        source: Some(AgentSource {
            path: None,
            start: None,
            end: Some(8.0),
        }),
        image_count: Some(2),
        ..AgentVideoRequest::new("motion_transfer")
    };
    assert_eq!(
        app.commands().first(),
        Some(&AgentCommand::Quote(QuoteCommand { video }))
    );
}

#[tokio::test]
async fn jobs_and_actions_become_their_commands() {
    let app = app();
    let id = Uuid::new_v4();
    call(
        &app,
        "0.2.0",
        "jobs",
        json!({ "ids": [id.to_string()], "wait_seconds": 50 }),
    )
    .await;
    call(
        &app,
        "0.2.0",
        "job_action",
        json!({ "action": "retry", "job": id.to_string() }),
    )
    .await;
    call(&app, "0.2.0", "job_action", json!({ "action": "resume_queue" })).await;

    assert_eq!(
        app.commands(),
        [
            AgentCommand::Jobs(JobsCommand {
                ids: vec![id],
                wait_seconds: 50,
                ..JobsCommand::default()
            }),
            AgentCommand::JobAction(JobActionCommand {
                action: AgentJobAction::Retry,
                job: Some(id)
            }),
            AgentCommand::JobAction(JobActionCommand {
                action: AgentJobAction::ResumeQueue,
                job: None
            }),
        ]
    );
}

/// Every kind of answer reaches the agent as structured content, never as a lost answer.
#[test]
fn each_answer_is_a_tool_result() {
    let mut job = AgentJob::new(Uuid::new_v4(), "paused", Some(Timestamp::from_unix(0)));
    job.pause_reason = Some("agent_limit".to_owned());
    job.hold = Some("over_max_cost".to_owned());
    job.needs_person = true;
    job.mode = Some("restyle".to_owned());
    job.model = Some("genjutsu".to_owned());
    let quote: AgentQuote = serde_json::from_value(json!({
        "price": { "amount": 130, "unit": "usd_cents" }, "isExact": true, "mode": "restyle", "model": "genjutsu",
        "resolution": "720p", "duration": 5, "remainingToday": 870, "paysUnlimited": false, "maxVariants": 1, "images": 0, "minImages": 0, "maxImages": 0,
    }))
    .expect("quote");
    let answers = vec![
        AgentResponse {
            quote: Some(quote),
            ..AgentResponse::default()
        },
        AgentResponse {
            created: Some(AgentCreated::new("r1", true, vec![job.clone()])),
            ..AgentResponse::default()
        },
        AgentResponse {
            jobs: Some(AgentJobs {
                jobs: vec![job.clone()],
                timed_out: true,
                next_cursor: None,
            }),
            ..AgentResponse::default()
        },
        AgentResponse {
            acted: Some(AgentActed {
                action: AgentJobAction::Cancel,
                job: Some(job),
            }),
            ..AgentResponse::default()
        },
        AgentResponse {
            library: Some(AgentLibraryPage {
                items: Vec::new(),
                next_cursor: Some("c".to_owned()),
            }),
            ..AgentResponse::default()
        },
        AgentResponse {
            details: Some(AgentLibraryDetailsPage::new(Vec::new(), Vec::new())),
            ..AgentResponse::default()
        },
        AgentResponse {
            marked: Some(AgentMarked {
                items: Vec::new(),
                changed: 0,
            }),
            ..AgentResponse::default()
        },
        AgentResponse {
            delivery: Some(AgentDelivery {
                action: AgentDeliveryAction::List,
                set: None,
                sets: Some(Vec::new()),
                folders: Some(Vec::new()),
                job: None,
                timed_out: None,
            }),
            ..AgentResponse::default()
        },
        AgentResponse {
            exported: Some(AgentExported {
                job: AgentJob {
                    kind: "export".to_owned(),
                    ..AgentJob::new(Uuid::new_v4(), "processing", Some(Timestamp::now()))
                },
                timed_out: false,
            }),
            ..AgentResponse::default()
        },
    ];
    for answer in &answers {
        let result = tool_result(answer, true);
        assert_eq!(result["isError"], false);
        assert!(result["structuredContent"].get("error").is_none());
    }
    let jobs = tool_result(&answers[2], true)["structuredContent"].clone();
    assert_eq!(jobs["timedOut"], true);
    assert_eq!(jobs["jobs"][0]["hold"], "over_max_cost");
    assert_eq!(
        tool_result(&answers[4], true)["structuredContent"]["nextCursor"],
        "c"
    );
}
