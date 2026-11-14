//! Port of `AccountCharactersTests.swift`: character sheets on the account, with the tool definitions of the
//! catalog snapshot and answers in the shape of their `outputSchema`s. Every call the stub gets is checked against
//! the snapshot's schema.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use jutsu_core::{
    CharacterDesign, CharacterPresetSettings, CharacterQuote, CharacterSheet, CharacterSheetMaker,
    CharacterTier, CharacterTrait, CharacterTraitGroup, Cost, GenerationProvider, ImageJobStatus, Mode,
    PortError, ProviderError, RemoteAsset, RemoteInputs, RemoteJob, ServerMedia,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;

use super::support::account::{calls, calls_of, full_set, schema_violations};
use super::support::agent_d::{answering, inputs, names, provider, spec};
use super::support::catalog::{self, live_answer, same};
use super::support::provider::make_provider;

const FACE: &str = "9b2e4c1a-7d3f-4a8b-8c6d-1e2f3a4b5c6d";
const JOBS: [&str; 2] = [
    "0f1e2d3c-4b5a-4968-8776-655443322110",
    "2b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e",
];

fn params() -> Value {
    json!({
        "tier": "cats", "selection": {"fur": ["ginger"]}, "brief": "a sleepy chef", "seed": 12, "batch_size": 2,
        "medias": [{"role": "image", "data": {"id": FACE, "type": "media_input"}}], "item_medias": [],
    })
}

fn tools() -> Vec<Value> {
    let mut tools = full_set();
    tools.extend(
        [
            "ai_influencer_get_options",
            "ai_influencer_list_presets",
            "ai_influencer_prepare",
            "ai_influencer_generate",
            "assets_picker_list",
            "jobs_wait",
        ]
        .map(catalog::tool),
    );
    tools
}

fn reply(name: &str, _arguments: &Value) -> Value {
    match name {
        "ai_influencer_get_options" => json!({
            "tiers": ["cats"],
            "groups": [
                {
                    "id": "fur", "label": "Fur", "max": 1,
                    "options": [{"id": "ginger", "label": "Ginger", "slot": null, "tiers": ["cats"]}],
                },
                {"id": "hair", "label": "Hair", "max": 2, "options": []},
            ],
        }),
        "ai_influencer_list_presets" => json!({
            "total": 1, "cursor": null,
            "items": [
                {
                    "id": FACE, "name": "Chef", "priority": 1,
                    "preview": {"url": "https://cdn.example.com/p.webp"},
                    "sheet": {"url": "https://cdn.example.com/s.png"},
                    "recreate": {
                        "job_set_type": "ai_influencer_sheet",
                        "params": {
                            "tier": "cats", "selection": {"fur": ["ginger"]}, "brief": "a chef", "seed": 12,
                            "medias": [],
                        },
                    },
                    "recreate_unavailable_reason": null,
                },
            ],
        }),
        "ai_influencer_prepare" => {
            json!({"params": params(), "credits": 23.2, "free_gens_remaining": 1, "submitted": false})
        }
        "ai_influencer_generate" => json!({
            "submitted": true, "job_id": JOBS[0], "job_ids": JOBS,
            "credits": 23.2, "use_free_gens": false, "results": [],
        }),
        "jobs_wait" => json!({
            "jobs": [
                {
                    "index": 0, "job_id": JOBS[0], "status": "completed", "type": "image",
                    "result_url": "https://cdn.example.com/sheet.png",
                },
            ],
            "poll_after_seconds": 3,
        }),
        "generate_video" => json!({"results": [{"id": "job-1"}]}),
        "job_status" => json!({"generation": {"status": "in_progress"}}),
        "assets_picker_list" => json!({
            "items": [
                {"id": JOBS[1], "kind": "image", "url": "https://cdn.example.com/other.png", "createdAt": 1_760_000_000},
            ],
            "next_cursor": null,
        }),
        _ => json!({}),
    }
}

fn quote_of_ten() -> CharacterQuote {
    CharacterQuote {
        credits: Decimal::from(10),
        count: 1,
        request: serde_json::to_vec(&params()).unwrap(),
    }
}

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

#[tokio::test]
async fn traits_and_presets_are_read_in_the_apps_terms() {
    let (provider, stub) = provider(tools(), reply).await;

    let groups = provider.character_traits(CharacterTier::Cats).await.unwrap();
    let presets = provider.character_presets(None).await.unwrap();

    // A group the tier has no option in isn't shown.
    assert_eq!(
        groups,
        vec![CharacterTraitGroup::new(
            "fur",
            "Fur",
            1,
            vec![CharacterTrait {
                id: "ginger".to_owned(),
                title: "Ginger".to_owned()
            }]
        )]
    );
    assert_eq!(
        presets.items.first().and_then(|preset| preset.settings.clone()),
        Some(CharacterPresetSettings {
            tier: CharacterTier::Cats,
            traits: BTreeMap::from([("fur".to_owned(), vec!["ginger".to_owned()])]),
            brief: "a chef".to_owned(),
            seed: Some(12),
        })
    );
    assert_eq!(presets.next_cursor, None);
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn the_live_answers_read_in_the_apps_terms() {
    let answers = [
        "ai_influencer_get_options",
        "ai_influencer_list_presets",
        "ai_influencer_prepare",
        "assets_picker_list_influencer",
    ]
    .map(live_answer);
    let mut by_tool: HashMap<String, Value> = HashMap::new();
    for answer in answers {
        by_tool.entry(answer.tool).or_insert(answer.answer);
    }
    let (provider, stub) = provider(tools(), move |name, _| {
        by_tool.get(name).cloned().unwrap_or_else(|| json!({}))
    })
    .await;

    let groups = provider.character_traits(CharacterTier::Normal).await.unwrap();
    let presets = provider.character_presets(None).await.unwrap();
    let mut design = CharacterDesign::new(None);
    design.count = 1;
    let quote = provider.quote_characters(design).await.unwrap();
    let sheets = provider.character_sheets(None).await.unwrap();

    assert_eq!(
        groups
            .iter()
            .find(|group| group.id == "freak_face")
            .map(|group| group.max_picks),
        Some(4)
    );
    assert_eq!(
        groups.first().and_then(|group| group.traits.first()).cloned(),
        Some(CharacterTrait {
            id: "female".to_owned(),
            title: "Female".to_owned()
        })
    );
    let first = presets.items.first().unwrap();
    assert_eq!(
        first.settings.as_ref().map(|settings| settings.tier),
        Some(CharacterTier::Freak)
    );
    assert_eq!(
        first.settings.as_ref().and_then(|settings| settings.seed),
        Some(427_752)
    );
    assert!(first.preview_url.is_some());
    // Three presets where thirty were asked for: the last page.
    assert_eq!(presets.next_cursor, None);
    assert_eq!(quote.credits, Decimal::from_str("1.125").unwrap());
    assert_eq!(
        quote.cost(),
        Cost::credits(Decimal::from_str("1.125").unwrap(), false)
    );
    assert_eq!(sheets.items, Vec::<CharacterSheet>::new());
    assert_eq!(sheets.next_cursor, None);
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn the_quote_is_generated_unchanged_with_its_price_as_the_cap() {
    let (provider, stub) = provider(tools(), reply).await;
    let mut design = CharacterDesign::new(Some(CharacterTier::Cats));
    design.traits = BTreeMap::from([("fur".to_owned(), vec!["ginger".to_owned()])]);
    "a sleepy chef".clone_into(&mut design.brief);
    design.count = 2;
    design.face = Some(RemoteAsset {
        url: url("https://upload.example.com/f.jpg"),
        id: Some(FACE.to_owned()),
    });
    design.randomize = false;

    let quote = provider.quote_characters(design).await.unwrap();
    let jobs = provider.generate_characters(quote.clone()).await.unwrap();

    assert_eq!(quote.credits, Decimal::from_str("23.2").unwrap());
    assert_eq!(
        quote.cost(),
        Cost::credits(Decimal::from_str("23.2").unwrap(), false)
    );
    assert_eq!(
        jobs.iter().map(|job| job.request_id.as_str()).collect::<Vec<_>>(),
        JOBS
    );
    let calls = calls(&stub).await;
    assert_eq!(
        names(&stub).await,
        vec![json!("ai_influencer_prepare"), json!("ai_influencer_generate")]
    );
    assert_eq!(calls[0]["arguments"]["tier"], "cats");
    assert!(same(&calls[0]["arguments"]["batch_size"], &json!(2)));
    assert!(same(
        &calls[1]["arguments"],
        &json!({"params": params(), "max_credits": 23.2})
    ));
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_submit_that_may_have_reached_the_server_is_never_retried() {
    let (provider, stub) = provider(tools(), |name, arguments| {
        if name == "ai_influencer_generate" {
            json!({"submitted": true})
        } else {
            reply(name, arguments)
        }
    })
    .await;

    assert_eq!(
        provider.generate_characters(quote_of_ten()).await,
        Err(PortError::Provider(ProviderError::OutcomeUnknown))
    );
    assert_eq!(calls_of(&stub, "ai_influencer_generate").await.len(), 1);
}

/// `ai_influencer_generate` charges: a failure it reports may come after the charge, so it isn't retried.
#[tokio::test]
async fn a_failed_submit_may_have_been_charged() {
    for payload in [
        json!({"result": {"isError": true, "content": [{"type": "text", "text": "Internal failure"}]}}),
        json!({"error": {"code": -32603, "message": "Internal failure"}}),
    ] {
        let stub = answering("ai_influencer_generate", payload.clone(), tools(), reply).await;
        let provider = make_provider(&stub, false);

        assert_eq!(
            provider.generate_characters(quote_of_ten()).await,
            Err(PortError::Provider(ProviderError::OutcomeUnknown)),
            "{payload}"
        );
    }
}

#[tokio::test]
async fn a_refused_submit_gives_its_reason() {
    let (provider, _stub) = provider(tools(), |name, arguments| {
        if name == "ai_influencer_generate" {
            json!({"submitted": false, "credits": 30, "error": "Price is above max_credits"})
        } else {
            reply(name, arguments)
        }
    })
    .await;

    assert_eq!(
        provider.generate_characters(quote_of_ten()).await,
        Err(PortError::Provider(ProviderError::Validation {
            field: None,
            message: "Price is above max_credits".to_owned()
        }))
    );
}

#[tokio::test]
async fn statuses_come_from_jobs_wait_then_job_status_for_the_rest() {
    let (provider, stub) = provider(tools(), reply).await;

    let statuses = provider
        .character_sheet_statuses(JOBS.map(RemoteJob::new).to_vec())
        .await
        .unwrap();

    assert_eq!(
        statuses.get(JOBS[0]),
        Some(&ImageJobStatus::Completed(vec![url(
            "https://cdn.example.com/sheet.png"
        )]))
    );
    assert_eq!(statuses.get(JOBS[1]), Some(&ImageJobStatus::Pending));
    assert_eq!(names(&stub).await, vec![json!("jobs_wait"), json!("job_status")]);
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn the_account_list_is_read_from_the_influencer_source() {
    let (provider, stub) = provider(tools(), reply).await;

    let page = provider.character_sheets(None).await.unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|sheet| sheet.id.as_str())
            .collect::<Vec<_>>(),
        [JOBS[1]]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_sheet_is_sent_to_a_video_job_by_its_job_id() {
    let (provider, stub) = provider(tools(), reply).await;
    let sheet = ServerMedia {
        id: JOBS[0].to_owned(),
        url: url("https://cdn.example.com/sheet.png"),
        owner: Some("user_1".to_owned()),
    };

    let asset = provider.server_asset(&sheet).await.unwrap().unwrap();
    let inputs = RemoteInputs {
        video: inputs().video,
        images: vec![asset],
        audio: None,
    };
    GenerationProvider::submit(&provider, &spec(Mode::MotionTransfer), &inputs, Uuid::new_v4())
        .await
        .unwrap();

    let generate = calls_of(&stub, "generate_video").await;
    assert_eq!(
        generate
            .first()
            .map(|call| call["arguments"]["params"]["medias"].clone()),
        Some(json!([
            {"role": "video_references", "value": "video-1"},
            {"role": "image_references", "value": JOBS[0]},
        ]))
    );
}
