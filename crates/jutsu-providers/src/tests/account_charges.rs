//! Port of `AccountChargesTests.swift`: a Topaz upscale's charge, matched in the account's credit history (issue
//! #113). Every transaction here is synthetic: no Topaz charge has been read from the live server, so the matching
//! relies on nothing but "Topaz" in the name, the action and the time.

use higgsfield::mcp::account::CreditTransaction;
use jutsu_core::{
    ClosedRange, Cost, GenerationProvider, Job, Mode, PostAction, PostOperation, ProviderKind, UpscaleEngine,
    UpscaleOptions, UpscaleTarget,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::support::account::{full_set, schema_violations};
use super::support::agent_d::{names, provider, spec};
use super::support::mcp::tool;
use crate::account_charges::{credits, window};

fn submitted() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_791_980_000).unwrap()
}

fn finished() -> OffsetDateTime {
    submitted() + Duration::seconds(300)
}

fn the_window() -> ClosedRange<OffsetDateTime> {
    window(submitted(), Some(finished()))
}

fn at(offset: f64) -> OffsetDateTime {
    submitted() + Duration::seconds_f64(offset)
}

fn spend(title: &str, credits: f64, offset: f64) -> CreditTransaction {
    CreditTransaction {
        date: at(offset),
        credits,
        action: "spend".to_owned(),
        title: title.to_owned(),
    }
}

fn refund(offset: f64) -> CreditTransaction {
    CreditTransaction {
        date: at(offset),
        credits: 20.0,
        action: "refund".to_owned(),
        title: "Topaz Video Upscale".to_owned(),
    }
}

#[test]
fn the_window_runs_from_before_the_submit_to_after_the_end() {
    assert_eq!(the_window().lower(), submitted() - Duration::seconds(120));
    assert_eq!(the_window().upper(), finished() + Duration::seconds(120));
    let unfinished = window(submitted(), None);
    assert_eq!(unfinished.upper(), submitted() + Duration::seconds(120));
}

/// One Topaz spend in the window, whatever the case or wording of its title, is the charge: one booked when the job
/// finishes too, and the same entry read on two pages once.
#[test]
fn a_topaz_spend_in_the_window_is_the_charge() {
    for title in [
        "Topaz Video Upscale",
        "Video upscale - TOPAZ",
        "topaz_video",
        "Upscale (Topaz)",
    ] {
        let history = [
            spend("Higgsfield Genjutsu - Object Swap", -39.0, 10.0),
            spend(title, -42.5, 3.0),
            spend("Seedance 2.5", -35.0, -30.0),
        ];
        assert_eq!(credits(&history, the_window()), Some(42.5), "{title}");
    }
    let finished = [spend("Topaz Video Upscale", -20.0, 310.0)];
    assert_eq!(credits(&finished, the_window()), Some(20.0));
    let entry = spend("Topaz Video Upscale", -12.0, 4.0);
    assert_eq!(credits(&[entry.clone(), entry], the_window()), Some(12.0));
}

/// The price stays unknown when nothing is named for Topaz, when a Topaz spend is outside the window (someone
/// else's), when two upscales sent together can't be told apart, when a refund means the spend isn't the final
/// price, and for a refund or a zero alone.
#[test]
fn an_unmatched_or_ambiguous_history_leaves_the_price_unknown() {
    let cases: [Vec<CreditTransaction>; 7] = [
        vec![
            spend("Video Upscale", -2.0, 3.0),
            spend("ByteDance Upscale", -4.0, 5.0),
            spend("Topaz Image Upscale", -8.0, 6.0),
        ],
        vec![
            spend("Topaz Video Upscale", -20.0, -121.0),
            spend("Topaz Video Upscale", -20.0, 300.0 + 121.0),
        ],
        vec![
            spend("Topaz Video Upscale", -20.0, 2.0),
            spend("Topaz Video Upscale", -30.0, 8.0),
        ],
        vec![spend("Topaz Video Upscale", -20.0, 2.0), refund(60.0)],
        vec![refund(0.0)],
        vec![spend("Topaz", 0.0, 1.0)],
        vec![spend("Topaz", f64::NAN, 1.0)],
    ];
    for history in cases {
        assert_eq!(credits(&history, the_window()), None, "{history:?}");
    }
}

// `charge(of:)` against a stub `transactions` tool (the Swift `AccountChargeReading` suite).

fn tools() -> Vec<Value> {
    let mut tools = full_set();
    tools.push(tool(
        "transactions",
        "List the user's credit transactions",
        &["cursor", "size"],
    ));
    tools
}

/// The time `offset` seconds after the submit, as Swift's `ISO8601DateFormatter` with fractional seconds writes it.
fn iso(offset: f64) -> Value {
    let text = at(offset).format(&Rfc3339).unwrap();
    let text = if text.contains('.') {
        text
    } else {
        text.replace('Z', ".000Z")
    };
    Value::String(text)
}

fn topaz_job(submitted_at: Option<OffsetDateTime>) -> Job {
    let mut spec = spec(Mode::MotionTransfer);
    spec.operation = Some(PostOperation {
        action: PostAction::Upscale(UpscaleOptions {
            target: UpscaleTarget::K4,
            engine: UpscaleEngine::Topaz,
            ..UpscaleOptions::default()
        }),
        source_job_id: Some("job-1".to_owned()),
    });
    let mut job = Job::new(
        Uuid::new_v4(),
        spec,
        ProviderKind::Account,
        Cost::UNKNOWN_CREDITS,
        Uuid::new_v4(),
        OffsetDateTime::now_utc(),
    );
    job.owner = Some("user_1".to_owned());
    job.submitted_at = submitted_at;
    job.charged_at = submitted_at.map(|date| date + Duration::seconds(200));
    job
}

/// Newest first: the second page reaches back past the window, so no third is asked for.
#[tokio::test]
async fn reads_pages_until_the_window_and_records_the_exact_charge() {
    let (provider, stub) = provider(tools(), |_, arguments| {
        if arguments.get("cursor").is_none() {
            return json!({
                "next_cursor": 50,
                "items": [
                    {"created_at": iso(900.0), "credits": -7, "action": "spend", "display_name": "Seedance 2.5"},
                ],
            });
        }
        json!({
            "next_cursor": 100,
            "items": [
                {"created_at": iso(5.0), "credits": -40.5, "action": "spend", "display_name": "Topaz Video Upscale"},
                {"created_at": iso(-600.0), "credits": -3, "action": "spend", "display_name": "Soul V2"},
            ],
        })
    })
    .await;

    assert_eq!(
        provider.charge(&topaz_job(Some(submitted()))).await.unwrap(),
        Some(Cost::credits(Decimal::new(405, 1), false))
    );
    assert_eq!(
        names(&stub).await,
        vec![json!("transactions"), json!("transactions")]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn no_match_leaves_the_charge_unknown() {
    let (provider, _stub) = provider(tools(), |_, _| {
        json!({
            "next_cursor": null,
            "items": [{"created_at": iso(5.0), "credits": -2, "action": "spend", "display_name": "Upscale"}],
        })
    })
    .await;
    assert_eq!(
        provider.charge(&topaz_job(Some(submitted()))).await.unwrap(),
        None
    );
}

/// Only a Topaz upscale with a submit time is looked up; nothing is asked for any other job.
#[tokio::test]
async fn other_jobs_are_never_looked_up() {
    let (provider, stub) = provider(tools(), |_, _| json!({})).await;
    let mut standard = topaz_job(Some(submitted()));
    standard.spec.operation = Some(PostOperation {
        action: PostAction::Upscale(UpscaleOptions {
            target: UpscaleTarget::K4,
            ..UpscaleOptions::default()
        }),
        source_job_id: Some("job-1".to_owned()),
    });

    assert_eq!(provider.charge(&standard).await.unwrap(), None);
    assert_eq!(provider.charge(&topaz_job(None)).await.unwrap(), None);
    assert_eq!(names(&stub).await, Vec::<Value>::new());
}
