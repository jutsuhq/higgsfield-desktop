//! `CharacterSheetsTests` and `LiveCharacterSheetsTests`: the AI Influencer character sheet tools, against the tool
//! schemas of the catalog snapshot, with answers in the shape of their `outputSchema`s and the live answers.

use std::collections::BTreeMap;

use higgsfield::core::HiggsfieldError;
use higgsfield::mcp::account::{
    HiggsfieldAccount, InfluencerDesign, InfluencerImage, InfluencerImageKind, InfluencerPresetDesign,
    InfluencerQuote, InfluencerSubmission, InfluencerTraitGroup, InfluencerTraitOption,
};
use serde_json::{Value, json};
use url::Url;

use crate::support::account::{account, calls, full_set, schema_violations};
use crate::support::catalog;
use crate::support::mcp::McpServer;
use crate::support::stub::Stub;

const NAMES: [&str; 5] = [
    "ai_influencer_get_options",
    "ai_influencer_list_presets",
    "ai_influencer_prepare",
    "ai_influencer_generate",
    "assets_picker_list",
];
const FACE: &str = "9b2e4c1a-7d3f-4a8b-8c6d-1e2f3a4b5c6d";
const STYLE: &str = "5a6b7c8d-1e2f-4a3b-9c4d-5e6f7a8b9c0d";
const SHEET: &str = "1a2b3c4d-5e6f-4a1b-8c2d-3e4f5a6b7c8d";
const JOBS: [&str; 2] = [
    "0f1e2d3c-4b5a-4968-8776-655443322110",
    "2b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e",
];

/// Exact params as `ai_influencer_prepare`'s `outputSchema` describes them.
fn params() -> Value {
    json!({
        "tier": "normal", "selection": {"hair": ["curly"], "eyes": ["green"]}, "brief": "a street magician",
        "seed": 4242, "batch_size": 2,
        "medias": [{"role": "image", "data": {"id": FACE, "type": "media_input"}}],
        "item_medias": [{"role": "image", "data": {"id": SHEET, "type": "image_job"}}],
    })
}

async fn sheets(
    names: &[&str],
    reply: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> (HiggsfieldAccount, Stub) {
    let mut tools = full_set();
    tools.extend(names.iter().map(|name| catalog::tool(name)));
    account(McpServer::new(tools).call(reply)).await
}

fn quote(credits: f64) -> InfluencerQuote {
    InfluencerQuote {
        params: params(),
        credits,
        free_gens_remaining: None,
    }
}

#[tokio::test]
async fn options_are_read_per_tier_with_their_limits() {
    let (account, stub) = sheets(&NAMES, |_, _| {
        json!({
            "tiers": ["normal", "freak", "total", "cats"],
            "groups": [
                {
                    "id": "hair", "label": "Hair", "max": 2,
                    "options": [
                        {"id": "curly", "label": "Curly", "slot": null, "tiers": ["normal", "freak"]},
                        {"id": "fur", "label": "Fur", "slot": "body", "tiers": ["cats"]},
                    ],
                },
                {"label": "No ID", "max": 1, "options": []},
            ],
        })
    })
    .await;

    let groups = account.influencer_options(Some("normal")).await.unwrap();

    let curly = InfluencerTraitOption {
        id: "curly".into(),
        label: "Curly".into(),
        tiers: vec!["normal".into(), "freak".into()],
    };
    assert_eq!(
        groups,
        [InfluencerTraitGroup {
            id: "hair".into(),
            label: "Hair".into(),
            max: 2,
            options: vec![curly]
        }]
    );
    assert_eq!(
        calls(&stub).await,
        [json!({"name": "ai_influencer_get_options", "arguments": {"tier": "normal"}})]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn presets_carry_the_design_that_recreates_them() {
    let (account, stub) = sheets(&NAMES, |_, _| {
        json!({
            "total": 41, "cursor": 2,
            "items": [
                {
                    "id": SHEET, "name": "Neon Rider", "priority": 3,
                    "preview": {"url": "https://cdn.higgsfield.ai/p.webp"},
                    "sheet": {"url": "https://cdn.higgsfield.ai/s.png"},
                    "recreate": {
                        "job_set_type": "ai_influencer_sheet",
                        "params": {
                            "tier": "total", "selection": {"hair": ["neon"]}, "brief": "rides at night",
                            "seed": 77, "medias": [],
                        },
                    },
                    "recreate_unavailable_reason": null,
                },
                {
                    "id": STYLE, "name": "Locked", "priority": 1,
                    "preview": {"url": "https://cdn.higgsfield.ai/l.webp"},
                    "sheet": {"url": "https://cdn.higgsfield.ai/l.png"}, "recreate": null,
                    "recreate_unavailable_reason": "Made with a reference",
                },
            ],
        })
    })
    .await;

    let page = account.influencer_presets(Some(1), 2).await.unwrap();

    assert_eq!(page.next_cursor, Some(2));
    let design = InfluencerPresetDesign {
        tier: "total".into(),
        selection: BTreeMap::from([("hair".to_owned(), vec!["neon".to_owned()])]),
        brief: "rides at night".into(),
        seed: Some(77),
    };
    assert_eq!(page.items[0].design, Some(design));
    assert_eq!(
        page.items[0].preview_url,
        Some(Url::parse("https://cdn.higgsfield.ai/p.webp").unwrap())
    );
    assert_eq!(page.items[1].design, None);
    assert_eq!(
        page.items[1].unavailable_reason.as_deref(),
        Some("Made with a reference")
    );
    assert_eq!(
        calls(&stub).await,
        [json!({"name": "ai_influencer_list_presets", "arguments": {"size": 2, "cursor": 1}})]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn prepare_sends_the_design_and_reads_the_exact_params_and_price() {
    let (account, stub) = sheets(
        &NAMES,
        |_, _| json!({"params": params(), "credits": 12.5, "free_gens_remaining": 0, "submitted": false}),
    )
    .await;
    let mut design = InfluencerDesign::new(Some("normal".into()));
    design.selection = BTreeMap::from([
        (
            "hair".to_owned(),
            ["curly", "a", "b", "c", "d"].map(str::to_owned).to_vec(),
        ),
        ("eyes".to_owned(), Vec::new()),
    ]);
    design.brief = "a street magician".into();
    design.seed = Some(4242);
    design.batch_size = 2;
    design.identity = Some(InfluencerImage {
        id: FACE.into(),
        kind: InfluencerImageKind::MediaInput,
    });
    design.styles = vec![InfluencerImage {
        id: SHEET.into(),
        kind: InfluencerImageKind::ImageJob,
    }];
    design.randomize = false;

    let quote = account.prepare_influencer(&design).await.unwrap();

    assert_eq!(
        quote,
        InfluencerQuote {
            params: params(),
            credits: 12.5,
            free_gens_remaining: Some(0)
        }
    );
    let arguments = calls(&stub).await[0]["arguments"].clone();
    assert_eq!(arguments["selection"], json!({"hair": ["curly", "a", "b", "c"]}));
    assert_eq!(arguments["randomize"], false);
    assert_eq!(arguments["batch_size"], 2);
    assert_eq!(
        arguments["medias"],
        json!([{"role": "image", "data": {"id": FACE, "type": "media_input"}}])
    );
    assert_eq!(
        arguments["item_medias"],
        json!([{"role": "image", "data": {"id": SHEET, "type": "image_job"}}])
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_random_design_leaves_the_tier_and_seed_to_the_server() {
    let (account, stub) = sheets(
        &NAMES,
        |_, _| json!({"params": params(), "credits": 6, "submitted": false}),
    )
    .await;

    let mut design = InfluencerDesign::new(None);
    design.batch_size = 9;
    account.prepare_influencer(&design).await.unwrap();

    assert_eq!(
        calls(&stub).await[0]["arguments"],
        json!({
            "selection": {}, "brief": "", "batch_size": 4, "medias": [], "item_medias": [], "randomize": true,
        })
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_quote_without_params_or_price_gives_the_reason() {
    let (account, _stub) = sheets(&NAMES, |_, _| json!({"error": "Unknown trait"})).await;
    assert_eq!(
        account
            .prepare_influencer(&InfluencerDesign::new(Some("normal".into())))
            .await,
        Err(HiggsfieldError::validation("Unknown trait"))
    );
}

#[tokio::test]
async fn generate_sends_the_quote_unchanged_with_its_price_as_the_cap() {
    let (account, stub) = sheets(&NAMES, |_, _| {
        json!({
            "submitted": true, "job_id": JOBS[0], "job_ids": JOBS, "credits": 12.5, "use_free_gens": false,
            "results": [],
        })
    })
    .await;

    let submission = account.generate_influencer(&quote(12.5), 12.5).await.unwrap();

    let expected = InfluencerSubmission {
        job_ids: JOBS.map(str::to_owned).to_vec(),
        credits: Some(12.5),
    };
    assert_eq!(submission, expected);
    assert_eq!(
        calls(&stub).await,
        [json!({"name": "ai_influencer_generate", "arguments": {"params": params(), "max_credits": 12.5}})]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_refused_submit_gives_the_reason() {
    let (account, _stub) = sheets(
        &NAMES,
        |_, _| json!({"submitted": false, "credits": 14, "error": "Price is above max_credits"}),
    )
    .await;
    assert_eq!(
        account.generate_influencer(&quote(12.0), 12.0).await,
        Err(HiggsfieldError::validation("Price is above max_credits"))
    );
}

#[tokio::test]
async fn a_submit_answer_without_jobs_may_have_started_some() {
    let (account, stub) = sheets(&NAMES, |_, _| json!({"submitted": true})).await;
    assert_eq!(
        account.generate_influencer(&quote(12.0), 12.0).await,
        Err(HiggsfieldError::Server { status: 200 })
    );
    // Sent once, never again.
    assert_eq!(calls(&stub).await.len(), 1);
}

#[tokio::test]
async fn sheets_are_listed_from_the_influencer_source() {
    let (account, stub) = sheets(&NAMES, |_, _| {
        json!({
            "items": [{
                "id": JOBS[0], "kind": "image", "url": "https://cdn.higgsfield.ai/sheet.png",
                "thumbnailUrl": "https://cdn.higgsfield.ai/sheet-small.png", "createdAt": 1_760_000_000,
            }],
            "next_cursor": null,
        })
    })
    .await;

    let page = account.influencer_sheets(None, 30).await.unwrap();

    let ids: Vec<_> = page.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, [JOBS[0]]);
    assert_eq!(page.next_cursor, None);
    assert_eq!(
        calls(&stub).await[0]["arguments"],
        json!({"source": "influencer", "kind": "image", "size": 30})
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_missing_tool_is_reported_without_a_call() {
    let (account, stub) = sheets(&[], |_, _| json!({})).await;
    assert_eq!(
        account.prepare_influencer(&InfluencerDesign::new(None)).await,
        Err(HiggsfieldError::ToolUnavailable("ai_influencer_prepare".into()))
    );
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

// The live answers of 2026-10-15: free, read-only calls, replayed by a stub that lists the snapshot's tool schemas.

/// A stub answering every call with the live answer `name`.
async fn replay(name: &str) -> (HiggsfieldAccount, Stub) {
    let live = catalog::live_answer(name);
    sheets(&NAMES, move |_, _| live.answer.clone()).await
}

#[test]
fn the_live_sheet_calls_keep_to_the_snapshot_schemas() {
    for name in [
        "ai_influencer_get_options",
        "ai_influencer_list_presets",
        "ai_influencer_prepare",
        "assets_picker_list_influencer",
    ] {
        let live = catalog::live_answer(name);
        assert_eq!(
            catalog::violations(&live.arguments, &live.tool),
            Vec::<String>::new(),
            "{name}"
        );
    }
}

#[tokio::test]
async fn options_are_read_with_their_labels_and_limits() {
    let (account, stub) = replay("ai_influencer_get_options").await;

    let groups = account.influencer_options(Some("normal")).await.unwrap();

    assert_eq!(
        calls(&stub).await[0]["arguments"],
        catalog::live_answer("ai_influencer_get_options").arguments
    );
    let ids: Vec<_> = groups.iter().map(|group| group.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "gender",
            "ethnicity_origin_base",
            "proportions",
            "freak_face",
            "accessory"
        ]
    );
    let labels: Vec<_> = groups.iter().map(|group| group.label.as_str()).collect();
    assert_eq!(
        labels,
        ["Gender", "Ethnicity", "Proportions", "Features", "Accessories"]
    );
    let limits: Vec<_> = groups.iter().map(|group| group.max).collect();
    assert_eq!(limits, [1, 1, 2, 4, 3]);
    assert_eq!(groups[0].options[0].label, "Female");
    // Every option the answer gives is offered for `normal`, so none is left out.
    let counts: Vec<_> = groups.iter().map(|group| group.options.len()).collect();
    assert_eq!(counts, [5, 6, 4, 10, 6]);
}

#[tokio::test]
async fn presets_carry_their_design_preview_and_next_page() {
    let (account, stub) = replay("ai_influencer_list_presets").await;

    let page = account.influencer_presets(None, 3).await.unwrap();

    assert_eq!(
        calls(&stub).await[0]["arguments"],
        catalog::live_answer("ai_influencer_list_presets").arguments
    );
    assert_eq!(page.items.len(), 3);
    assert_eq!(page.next_cursor, Some(2));
    let first = &page.items[0];
    assert_eq!(first.name, "AI Influencer 38");
    let preview = first.preview_url.as_ref().unwrap();
    assert!(
        preview
            .path_segments()
            .unwrap()
            .next_back()
            .unwrap()
            .starts_with("9252b193")
    );
    let design = first.design.as_ref().unwrap();
    assert_eq!(design.tier, "freak");
    assert_eq!(design.seed, Some(427_752));
    assert_eq!(design.selection["hair"], ["hair_bowl"]);
    assert_eq!(design.selection["distinctive"], Vec::<String>::new());
    assert!(design.brief.starts_with("Fashion-editorial"));
    assert!(
        page.items
            .iter()
            .all(|item| item.design.is_some() && item.unavailable_reason.is_none())
    );
}

#[tokio::test]
async fn a_last_preset_page_ends_the_list() {
    let (account, _stub) = replay("ai_influencer_list_presets").await;
    // Three items where four were asked for: the last page, whatever `cursor` says.
    assert_eq!(
        account.influencer_presets(None, 4).await.unwrap().next_cursor,
        None
    );
}

#[tokio::test]
async fn prepare_reads_the_params_and_the_fractional_price() {
    let (account, _stub) = replay("ai_influencer_prepare").await;

    let quote = account
        .prepare_influencer(&InfluencerDesign::new(None))
        .await
        .unwrap();

    let live = catalog::live_answer("ai_influencer_prepare");
    assert!((quote.credits - 1.125).abs() < f64::EPSILON);
    assert_eq!(quote.free_gens_remaining, None);
    assert_eq!(quote.params, live.answer["params"]);
    assert_eq!(quote.params["selection"]["gender"], json!(["female"]));
}

#[tokio::test]
async fn the_live_params_go_to_generate_unchanged_and_keep_to_its_schema() {
    let params = catalog::live_answer("ai_influencer_prepare").answer["params"].clone();
    let (account, stub) = sheets(
        &NAMES,
        |_, _| json!({"submitted": true, "job_id": "job-1", "job_ids": ["job-1"], "credits": 1.125}),
    )
    .await;

    let quote = InfluencerQuote {
        params: params.clone(),
        credits: 1.125,
        free_gens_remaining: None,
    };
    account.generate_influencer(&quote, 1.125).await.unwrap();

    assert_eq!(calls(&stub).await[0]["arguments"]["params"], params);
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn an_account_without_sheets_lists_none() {
    let (account, stub) = replay("assets_picker_list_influencer").await;
    let arguments = catalog::live_answer("assets_picker_list_influencer").arguments;

    let page = account.influencer_sheets(None, 30).await.unwrap();

    assert_eq!(page.items.len(), 0);
    assert_eq!(page.next_cursor, None);
    let sent = calls(&stub).await[0]["arguments"].clone();
    assert_eq!(sent["source"], arguments["source"]);
    assert_eq!(sent["kind"], arguments["kind"]);
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}
