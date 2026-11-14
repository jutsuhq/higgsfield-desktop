//! `ImageGenerationTests` and the network half of `JobsWaitTests`.

use higgsfield::core::{HiggsfieldError, ImageStatus, Moderation};
use higgsfield::mcp::account::{AccountImageModel, AccountImageRequest, HiggsfieldAccount};
use serde_json::{Value, json};
use url::Url;

use crate::support::account::{account, answering, calls, full_set, schema_violations};
use crate::support::catalog;
use crate::support::mcp::{McpServer, tool};
use crate::support::stub::Stub;

/// `generate_image` and `models_explore` as the catalog snapshot lists them.
fn image_tools() -> Vec<Value> {
    let mut tools: Vec<Value> = full_set()
        .into_iter()
        .filter(|tool| tool["name"] != "generate_image")
        .collect();
    tools.extend([catalog::tool("generate_image"), catalog::tool("models_explore")]);
    tools
}

async fn images(call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static) -> (HiggsfieldAccount, Stub) {
    account(McpServer::new(image_tools()).call(call)).await
}

fn request() -> AccountImageRequest {
    let mut request = AccountImageRequest::new("soul_2", "a knight in silver armor", Some("9:16".into()), 4);
    request.image_ids = vec!["media-1".into()];
    request.image_role = "image".into();
    request
}

#[tokio::test]
async fn generate_sends_the_params_the_schema_takes_and_reads_every_job() {
    let (account, stub) = images(|_, arguments| {
        if arguments["params"]["get_cost"] == true {
            json!({"cost": {"credits": 8, "credits_exact": 7.5}})
        } else {
            json!({"results": [{"id": "job-1"}, {"id": "job-2"}], "request_id": "req-1", "cost": {"credits": 8}})
        }
    })
    .await;

    assert_eq!(account.image_cost(&request()).await, Ok(Some(7.5)));
    let jobs = account.generate_images(&request()).await.unwrap();

    let ids: Vec<_> = jobs.iter().map(|job| job.id.as_str()).collect();
    assert_eq!(ids, ["job-1", "job-2"]);
    let credits: Vec<_> = jobs.iter().map(|job| job.credits).collect();
    assert_eq!(credits, [Some(8.0), None]);
    let params = json!({
        "model": "soul_2", "prompt": "a knight in silver armor", "aspect_ratio": "9:16", "count": 4,
        "medias": [{"role": "image", "value": "media-1"}], "use_unlim": false,
    });
    let mut quoted = params.clone();
    quoted["get_cost"] = json!(true);
    let calls = calls(&stub).await;
    let arguments: Vec<_> = calls.iter().map(|call| call["arguments"].clone()).collect();
    assert_eq!(arguments, [json!({"params": quoted}), json!({"params": params})]);
    assert!(calls.iter().all(|call| call["name"] == "generate_image"));
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn an_answer_without_a_job_is_not_a_refusal() {
    let (account, _stub) = images(|_, _| json!({"ok": true})).await;
    assert_eq!(
        account.generate_images(&request()).await,
        Err(HiggsfieldError::Server { status: 200 })
    );
}

#[tokio::test]
async fn a_missing_tool_is_reported_before_any_call() {
    let tools = full_set()
        .into_iter()
        .filter(|tool| tool["name"] != "generate_image")
        .collect();
    let (account, stub) = account(McpServer::new(tools)).await;
    assert_eq!(
        account.check_image_tool().await,
        Err(HiggsfieldError::ToolUnavailable("generate_image".into()))
    );
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

#[tokio::test]
async fn image_models_are_read_from_the_snapshot_list() {
    let (account, stub) = images(|_, arguments| {
        if arguments.get("after").is_none() {
            catalog::value(catalog::IMAGE_LIST, &[])
        } else {
            json!({"items": [{"id": "flux_2", "name": "FLUX.2"}, {"id": "soul_2"}], "has_more": false})
        }
    })
    .await;

    let models = account.image_models().await.unwrap().unwrap();

    let ids: Vec<_> = models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(
        ids,
        ["soul_2", "gpt_image_2", "soul_cast", "nano_banana_pro", "flux_2"]
    );
    let soul = AccountImageModel {
        id: "soul_2".into(),
        name: "Higgsfield Soul 2.0".into(),
        aspect_ratios: ["1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"]
            .map(str::to_owned)
            .to_vec(),
        image_role: Some("image".into()),
        max_images: Some(1),
    };
    assert_eq!(models[0], soul);
    let roles: Vec<_> = models.iter().map(|model| model.image_role.as_deref()).collect();
    assert_eq!(
        roles,
        [Some("image"), Some("image"), None, Some("image_references"), None]
    );
    assert_eq!(models[3].max_images, None);
    let arguments: Vec<_> = calls(&stub)
        .await
        .iter()
        .map(|call| call["arguments"].clone())
        .collect();
    assert_eq!(
        arguments,
        [
            json!({"action": "list", "type": "image", "limit": 100}),
            json!({"action": "list", "type": "image", "limit": 100, "after": "20"}),
        ]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn one_model_is_described_by_get() {
    let (account, stub) = images(|_, _| {
        let list = catalog::value(catalog::IMAGE_LIST, &[]);
        list["items"]
            .as_array()
            .and_then(|items| items.last())
            .cloned()
            .unwrap_or_default()
    })
    .await;

    let model = account.image_model("nano_banana_pro").await.unwrap();

    assert_eq!(
        model.and_then(|model| model.image_role).as_deref(),
        Some("image_references")
    );
    assert_eq!(
        calls(&stub).await[0]["arguments"],
        json!({"action": "get", "model_id": "nano_banana_pro"})
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn without_models_explore_there_is_no_list() {
    let (account, _stub) = account(McpServer::new(full_set())).await;
    assert_eq!(account.image_models().await, Ok(None));
    assert_eq!(account.image_model("soul_2").await, Ok(None));
}

#[tokio::test]
async fn image_status_reads_the_result() {
    let cases = [
        (json!({"status": "queued"}), ImageStatus::Queued),
        (json!({"status": "in_progress"}), ImageStatus::InProgress),
        (
            json!({"status": "nsfw"}),
            ImageStatus::Rejected(Moderation::ExplicitContent),
        ),
        (
            json!({"status": "failed", "error": "bad prompt"}),
            ImageStatus::Failed {
                message: Some("bad prompt".into()),
            },
        ),
        (
            json!({"status": "completed", "results": {"rawUrl": "https://cdn.example.com/a.png"}}),
            ImageStatus::Completed {
                image_urls: vec![Url::parse("https://cdn.example.com/a.png").unwrap()],
            },
        ),
        (
            json!({"status": "completed", "results": {"rawUrl": "http://cdn.example.com/a.png"}}),
            ImageStatus::Failed {
                message: Some("Completed without an image URL.".into()),
            },
        ),
    ];
    for (generation, expected) in cases {
        let answer = json!({ "generation": generation });
        let (account, _stub) = answering(move |_, _| answer.clone()).await;
        assert_eq!(account.image_status("job-1").await, Ok(expected), "{generation}");
    }
}

// `jobs_wait` calls.

/// The tool's arguments as the catalog snapshot of 2026-10-15 lists them.
fn jobs_wait() -> Value {
    tool(
        "jobs_wait",
        "Long-poll 1-12 generation jobs together without opening a widget",
        &["jobs", "timeout_seconds"],
    )
}

fn with(extra: Value) -> Vec<Value> {
    let mut tools = full_set();
    tools.push(extra);
    tools
}

#[tokio::test]
async fn sends_each_id_with_its_index_and_reads_the_answer() {
    let answer = json!({
        "jobs": [
            {
                "index": 0, "job_id": "job-1", "status": "completed", "type": "video", "model": "seedance_2_5",
                "result_url": "https://cdn.example/1.mp4", "thumbnail_url": "https://cdn.example/1.jpg",
            },
            {"index": 1, "job_id": "job-2", "status": "in_progress"},
            {"index": 2, "job_id": "job-3", "status": "failed", "error": "Bad input"},
        ],
        "summary": {"total": 3, "completed": 1, "failed": 1, "active": 1, "errors": 0},
        "all_terminal": false, "poll_after_seconds": 5, "timed_out": true,
    });
    let (account, stub) = account(McpServer::new(with(jobs_wait())).call(move |_, _| answer.clone())).await;

    let ids = ["job-1", "job-2", "job-3"].map(str::to_owned);
    let wait = account.wait_for_jobs(&ids).await.unwrap().unwrap();

    assert_eq!(wait.statuses.len(), 3);
    assert_eq!(wait.poll_after_seconds, Some(5.0));
    let calls = calls(&stub).await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["name"], "jobs_wait");
    assert_eq!(
        calls[0]["arguments"],
        json!({"jobs": [
            {"job_id": "job-1", "index": 0}, {"job_id": "job-2", "index": 1}, {"job_id": "job-3", "index": 2},
        ]})
    );
}

#[tokio::test]
async fn without_jobs_wait_there_is_no_answer() {
    let (account, stub) = account(McpServer::new(full_set())).await;
    assert_eq!(account.wait_for_jobs(&["job-1".to_owned()]).await, Ok(None));
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

/// The first guess took plain IDs under `job_ids`; a schema without `jobs` counts as no tool.
#[tokio::test]
async fn a_schema_without_jobs_is_not_called() {
    let (account, stub) = account(McpServer::new(with(tool("jobs_wait", "Wait", &["job_ids"])))).await;
    assert_eq!(account.wait_for_jobs(&["job-1".to_owned()]).await, Ok(None));
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

#[tokio::test]
async fn none_or_more_than_twelve_jobs_are_not_called() {
    let (account, stub) = account(McpServer::new(with(jobs_wait()))).await;
    assert_eq!(account.wait_for_jobs(&[]).await, Ok(None));
    let thirteen: Vec<String> = (0..=12).map(|n| format!("job-{n}")).collect();
    assert_eq!(account.wait_for_jobs(&thirteen).await, Ok(None));
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}
