//! Port of `ImageProviderTests.swift`.

use std::str::FromStr;

use jutsu_core::{
    Cost, ImageJobStatus, ImageModel, ImageModelInput, ImageProvider, ImageRequest, PortError, ProviderError,
    RemoteAsset,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;

use super::support::account::{calls_of, full_set, schema_violations};
use super::support::agent_a::{image_listing, image_model, mcp_provider, routes, url};
use super::support::catalog;
use super::support::mcp::Reply;
use super::support::provider::make_api_provider;
use super::support::stub::{body_text, header};
use crate::account_images::{models, version};
use crate::mcp_provider::not_cancellable;

const SUBMITTED: &str = r#"{"status": "queued", "request_id": "req-7",
 "status_url": "https://api.higgsfield.ai/requests/req-7/status",
 "cancel_url": "https://api.higgsfield.ai/requests/req-7/cancel"}"#;

const SOUL: &str = "higgsfield-ai/soul/v2/standard";
const GROK: &str = "xai/grok-imagine-image-2.0";
const QWEN: &str = "alibaba/qwen-image-3/edit";

fn input() -> RemoteAsset {
    RemoteAsset {
        url: url("https://cdn.example.com/in.png"),
        id: Some("media-1".to_owned()),
    }
}

fn request(
    model: &str,
    prompt: &str,
    aspect_ratio: &str,
    count: i64,
    input_images: Vec<RemoteAsset>,
) -> ImageRequest {
    ImageRequest {
        model: model.to_owned(),
        prompt: prompt.to_owned(),
        aspect_ratio: aspect_ratio.to_owned(),
        count,
        input_images,
    }
}

fn validation(field: Option<&str>, message: &str) -> PortError {
    PortError::Provider(ProviderError::Validation {
        field: field.map(str::to_owned),
        message: message.to_owned(),
    })
}

// Reference images with an API key (issue #42): the curated models' REST bodies, `/estimate` and the list price when
// it can't answer.

#[tokio::test]
async fn estimate_quotes_the_exact_body() {
    let stub = routes(vec![(
        "POST",
        format!("/estimate/{SOUL}"),
        Reply::json_text(r#"{"usd": "0.0128"}"#),
    )])
    .await;
    let image = request(SOUL, "a knight", "9:16", 4, Vec::new());

    let cost = make_api_provider(&stub, "key-id")
        .image_estimate(&image)
        .await
        .unwrap();

    assert_eq!(cost, Cost::usd(Decimal::from_str("0.0128").unwrap(), false));
    assert_eq!(
        body_text(stub.requests().await.first().unwrap()),
        r#"{"aspect_ratio":"9:16","batch_size":4,"prompt":"a knight"}"#
    );
}

#[tokio::test]
async fn an_unreachable_estimate_falls_back_to_the_list_price() {
    let stub = routes(vec![(
        "POST",
        format!("/estimate/{SOUL}"),
        Reply::json_text("{}").with_status(500),
    )])
    .await;
    let image = request(SOUL, "a knight", "1:1", 4, Vec::new());

    let cost = make_api_provider(&stub, "key-id")
        .image_estimate(&image)
        .await
        .unwrap();

    assert_eq!(cost, Cost::usd(Decimal::from_str("0.0128").unwrap(), true));
}

#[tokio::test]
async fn a_refused_estimate_is_not_hidden() {
    let stub = routes(vec![(
        "POST",
        format!("/estimate/{SOUL}"),
        Reply::json_text(r#"{"detail": "Invalid credentials"}"#).with_status(401),
    )])
    .await;
    let image = request(SOUL, "a knight", "1:1", 1, Vec::new());
    assert_eq!(
        make_api_provider(&stub, "key-id").image_estimate(&image).await,
        Err(PortError::Provider(ProviderError::Unauthorized))
    );
}

#[tokio::test]
async fn edit_sends_the_input_and_polls_the_images() {
    let stub = routes(vec![
        ("POST", format!("/{GROK}"), Reply::json_text(SUBMITTED)),
        (
            "GET",
            "/requests/req-7/status".to_owned(),
            Reply::json_text(
                r#"{"status": "completed", "images": [{"url": "https://cdn.example.com/1.png"}]}"#,
            ),
        ),
    ])
    .await;
    let provider = make_api_provider(&stub, "key-id");
    let image = request(GROK, "add a cape", "3:4", 1, vec![input()]);

    let jobs = provider.submit_images(&image, Uuid::new_v4()).await.unwrap();
    let status = provider.image_status(jobs.first().unwrap()).await.unwrap();

    assert_eq!(
        jobs.iter().map(|job| job.request_id.as_str()).collect::<Vec<_>>(),
        ["req-7"]
    );
    assert_eq!(
        status,
        ImageJobStatus::Completed(vec![url("https://cdn.example.com/1.png")])
    );
    let requests = stub.requests().await;
    let first = requests.first().unwrap();
    assert_eq!(
        body_text(first),
        r#"{"aspect_ratio":"3:4","image_urls":["https://cdn.example.com/in.png"],"prompt":"add a cape"}"#
    );
    assert!(header(first, "Idempotency-Key").is_some());
}

#[tokio::test]
async fn an_invalid_request_is_refused_before_sending() {
    let cases = [
        (
            request(QWEN, "add a cape", "1:1", 1, Vec::new()),
            validation(Some("image_urls"), "Qwen Image 3 Edit needs an image to edit."),
        ),
        (
            request(SOUL, "a knight", "1:1", 2, Vec::new()),
            validation(Some("batch_size"), "Soul 2 can't make that many images."),
        ),
        (
            request(SOUL, "  ", "1:1", 1, Vec::new()),
            validation(Some("prompt"), "Describe the image to make."),
        ),
        (
            request("bytedance/seedance-2.5/text-to-video", "x", "1:1", 1, Vec::new()),
            validation(Some("model"), "This image model can't run with an API key."),
        ),
    ];
    for (image, error) in cases {
        let stub = routes(Vec::new()).await;
        assert_eq!(
            make_api_provider(&stub, "key-id")
                .submit_images(&image, Uuid::new_v4())
                .await,
            Err(error)
        );
        assert!(stub.requests().await.is_empty());
    }
}

// Reference images on the account, with the tool definitions and the image list of the catalog snapshot of
// 2026-10-15.

fn tools() -> Vec<Value> {
    let mut tools: Vec<Value> = full_set()
        .into_iter()
        .filter(|tool| tool["name"] != "generate_image")
        .collect();
    tools.push(catalog::tool("generate_image"));
    tools.push(catalog::tool("models_explore"));
    tools
}

fn account_request() -> ImageRequest {
    request("nano_banana_pro", "a knight", "9:16", 2, vec![input()])
}

const JOB_IDS: [&str; 2] = [
    "3f1c8a52-6b0e-4d7a-9a51-2f4b8c9d0e17",
    "7d2e9b41-0c3a-4f6e-8b12-5a9c0d1e2f34",
];

/// The snapshot's image list, and each of its items for `get`.
fn reply(name: &str, arguments: &Value) -> Value {
    let list = catalog::value(catalog::IMAGE_LIST, &[]);
    match name {
        "models_explore" if arguments["action"] == "get" => list["items"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["id"] == arguments["model_id"]))
            .cloned()
            .unwrap_or_else(|| json!({})),
        "models_explore" if arguments.get("after").is_none() => list,
        "models_explore" => json!({"items": [], "has_more": false}),
        "generate_image" if arguments["params"]["get_cost"] == true => {
            json!({"cost": {"credits": 3, "credits_exact": 2.5}})
        }
        "generate_image" => {
            json!({"results": [{"id": JOB_IDS[0]}, {"id": JOB_IDS[1]}], "request_id": "req-1"})
        }
        "job_status" => {
            json!({"generation": {"status": "completed", "results": {"rawUrl": "https://cdn.example.com/1.png"}}})
        }
        _ => json!({}),
    }
}

/// The whole image list of the catalog snapshot of 2026-11-10, in its order, and each model's `get` answer.
fn snapshot_reply(name: &str, arguments: &Value) -> Value {
    match name {
        "models_explore" if arguments["action"] == "get" => {
            catalog::model(arguments["model_id"].as_str().unwrap_or(""))
        }
        "models_explore" if arguments.get("after").is_none() => image_listing(),
        _ => reply(name, arguments),
    }
}

/// The `generate_image` calls the stub got.
async fn generates(stub: &super::support::stub::Stub) -> Vec<Value> {
    calls_of(stub, "generate_image").await
}

/// The newest model of each family the snapshot lists, and Soul Cinema; each request, with an image where the model
/// takes one, keeps to that model's `models_explore get` schema. MiniMax H3 Image lists no frame, so it is offered as
/// "auto" and sends none.
#[tokio::test]
async fn the_snapshots_newest_model_of_each_family_is_offered() {
    let (provider, stub) = mcp_provider(tools(), snapshot_reply).await;

    let models = provider.image_models().await.unwrap();
    for model in &models {
        let ratio = if model.aspect_ratios.iter().any(|ratio| ratio == "1:1") {
            "1:1".to_owned()
        } else {
            model.aspect_ratios.first().unwrap().clone()
        };
        let inputs = if model.input == ImageModelInput::None {
            Vec::new()
        } else {
            vec![input()]
        };
        let image = request(&model.id, "a knight", &ratio, 4, inputs);
        provider.image_estimate(&image).await.unwrap();
        provider.submit_images(&image, Uuid::new_v4()).await.unwrap();
    }

    assert_eq!(
        models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(),
        [
            "soul_2",
            "soul_cinematic",
            "nano_banana_2_1",
            "gpt_image_2_6",
            "seedream_5_5",
            "grok_image_2_0",
            "minimax_h3_image",
        ]
    );
    assert_eq!(
        models
            .iter()
            .find(|model| model.id == "minimax_h3_image")
            .map(|model| model.aspect_ratios.clone()),
        Some(vec!["auto".to_owned()])
    );
    let mut limits = vec![1, 1];
    limits.extend([ImageModel::INPUT_BOUND; 5]);
    assert_eq!(
        models.iter().map(ImageModel::input_limit).collect::<Vec<_>>(),
        limits
    );
    let generates = generates(&stub).await;
    assert_eq!(generates.len(), models.len() * 2);
    for call in &generates {
        let params = &call["arguments"]["params"];
        assert!(catalog::violations_of_params(params).is_empty(), "{params}");
    }
    let minimax = generates
        .iter()
        .rev()
        .find(|call| call["arguments"]["params"]["model"] == "minimax_h3_image")
        .unwrap();
    assert_eq!(minimax["arguments"]["params"].get("aspect_ratio"), None);
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn images_run_with_each_models_own_frames_and_image_role() {
    let (provider, stub) = mcp_provider(tools(), reply).await;

    let models = provider.image_models().await.unwrap();
    let cost = provider.image_estimate(&account_request()).await.unwrap();
    let jobs = provider
        .submit_images(&account_request(), Uuid::new_v4())
        .await
        .unwrap();
    let status = provider.image_status(jobs.first().unwrap()).await.unwrap();

    assert_eq!(
        models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(),
        ["soul_2", "gpt_image_2", "soul_cast", "nano_banana_pro"]
    );
    assert_eq!(
        models.first().map(|model| model.variant_counts.clone()),
        Some(vec![1, 2, 3, 4])
    );
    assert_eq!(
        models.first().map(|model| model.aspect_ratios.clone()),
        Some(
            ["1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"]
                .map(str::to_owned)
                .to_vec()
        )
    );
    assert_eq!(
        models.iter().map(|model| model.input).collect::<Vec<_>>(),
        [
            ImageModelInput::Optional,
            ImageModelInput::Optional,
            ImageModelInput::None,
            ImageModelInput::Optional
        ]
    );
    // get_cost prices one image: 2.5 credits times the request's 2.
    assert_eq!(cost, Cost::credits(Decimal::from(5), false));
    assert_eq!(
        jobs.iter().map(|job| job.request_id.as_str()).collect::<Vec<_>>(),
        JOB_IDS
    );
    assert_eq!(
        status,
        ImageJobStatus::Completed(vec![url("https://cdn.example.com/1.png")])
    );
    let generates = generates(&stub).await;
    let generate = generates.last().unwrap();
    assert_eq!(
        generate["arguments"]["params"]["medias"],
        json!([{"role": "image_references", "value": "media-1"}])
    );
    assert_eq!(generate["arguments"]["params"]["use_unlim"], json!(false));
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
    assert_eq!(provider.cancel_images(&jobs[0]).await, Err(not_cancellable()));
}

#[tokio::test]
async fn an_image_goes_in_under_the_models_role_and_within_its_count() {
    let (provider, stub) = mcp_provider(tools(), reply).await;
    let mut soul = account_request();
    "soul_2".clone_into(&mut soul.model);

    provider.submit_images(&soul, Uuid::new_v4()).await.unwrap();
    let generate = generates(&stub).await.last().cloned().unwrap();
    assert_eq!(
        generate["arguments"]["params"]["medias"],
        json!([{"role": "image", "value": "media-1"}])
    );

    soul.input_images = vec![input(), input()];
    assert_eq!(
        provider.submit_images(&soul, Uuid::new_v4()).await,
        Err(validation(None, "This model takes at most 1 image."))
    );
    let mut cast = account_request();
    "soul_cast".clone_into(&mut cast.model);
    assert_eq!(
        provider.submit_images(&cast, Uuid::new_v4()).await,
        Err(validation(None, "This model doesn't take an image."))
    );
    assert_eq!(generates(&stub).await.len(), 1);
}

#[test]
fn each_family_gets_a_turn_newest_first_and_a_twice_listed_model_shows_once() {
    let listed: Vec<_> = [
        "soul_2",
        "soul_cinematic",
        "gpt_image_2",
        "soul_cast",
        "soul_location",
        "soul_v2",
        "nano_banana",
        "grok_image",
        "grok_image_2_0",
        "nano_banana_pro",
        "gpt_image_2_6",
    ]
    .iter()
    .map(|id| {
        image_model(
            id,
            if *id == "soul_v2" { "soul_2" } else { id },
            &["1:1"],
            None,
            None,
        )
    })
    .collect();
    assert_eq!(
        models(&listed)
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        [
            "soul_2",
            "soul_cinematic",
            "nano_banana",
            "grok_image",
            "grok_image_2_0",
            "nano_banana_pro",
            "gpt_image_2_6",
        ]
    );
}

#[test]
fn a_familys_version_is_the_first_numbers_after_its_fragment() {
    let cases: [(&str, &str, &[i64]); 6] = [
        ("seedream_5_5", "seedream", &[5, 5]),
        ("seedream_v5_pro", "seedream", &[5]),
        ("seedream_5_0_flash", "seedream", &[5, 0]),
        ("grok_image_2_0", "grok", &[2, 0]),
        ("nano_banana_pro", "nano_banana", &[]),
        ("soul_cinematic", "soul", &[]),
    ];
    for (id, fragment, expected) in cases {
        assert_eq!(version(id, fragment), expected, "{id}");
    }
}

/// How many images a model edits comes from its `medias` entry's `max`. Where the catalog names none, as
/// `models_explore list` does for GPT Image 2 and Nano Banana Pro, the bound: `get_cost` refuses the rest.
#[test]
fn the_catalogs_maximum_is_how_many_inputs_a_model_takes() {
    let listed = [
        image_model(
            "nano_banana",
            "Nano Banana",
            &["1:1"],
            Some("image_references"),
            Some(8),
        ),
        image_model("soul_2", "Soul 2", &["1:1"], Some("image"), Some(1)),
        image_model("gpt_image_2", "GPT Image 2", &["1:1"], Some("image"), None),
        image_model("soul_cast", "Soul Cast", &["1:1"], None, None),
    ];
    let models = models(&listed);
    assert_eq!(
        models.iter().map(ImageModel::input_limit).collect::<Vec<_>>(),
        [8, 1, ImageModel::INPUT_BOUND, 0]
    );
    assert_eq!(
        models
            .iter()
            .map(ImageModel::input_limit_is_bound)
            .collect::<Vec<_>>(),
        [false, false, true, false]
    );
}

#[test]
fn an_account_listing_none_of_the_curated_models_offers_its_first() {
    let mut listed = vec![image_model("soul_sprite", "Sprite", &[], None, None)];
    listed.extend((1..=8).map(|index| {
        image_model(
            &format!("model_{index}"),
            &format!("Model {index}"),
            &["1:1"],
            None,
            None,
        )
    }));
    assert_eq!(
        models(&listed)
            .iter()
            .map(|model| model.id.clone())
            .collect::<Vec<_>>(),
        (1..=7).map(|index| format!("model_{index}")).collect::<Vec<_>>()
    );
}
