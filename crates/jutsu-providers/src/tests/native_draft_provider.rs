//! Port of `NativeDraftProviderTests.swift`: Seedance 2.5's native draft on the account (issue #115). Draft at 480p
//! sends `draft: true`, and a finalize sends only `model`, `draft_job_id`, `resolution` and `use_unlim`, as quoted
//! live on 2026-10-15.

use std::collections::BTreeSet;

use jutsu_core::{
    AspectRatio, Cost, GenerationProvider, JobSpec, Mode, ModelConstraintTable, RemoteAsset, RemoteInputs,
    Resolution, VideoModel,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;

use super::support::account::{calls_of, full_set, schema_violations};
use super::support::agent_d::{file_url, image, provider};
use super::support::catalog::{same, violations_of_params};
use super::support::stub::Stub;
use crate::account_model::estimated_credits;

/// The paid run's draft: 15 s from reference images, 9:16.
fn draft(resolution: Resolution) -> JobSpec {
    let mut spec = JobSpec::new(Mode::MotionTransfer, VideoModel::SEEDANCE25, None, resolution);
    spec.references = vec![image(), image()];
    "a fox in the snow".clone_into(&mut spec.prompt);
    spec.is_draft = true;
    spec.length = Some(15.0);
    spec.aspect_ratio = Some(AspectRatio::Portrait);
    spec
}

fn finalize() -> JobSpec {
    let mut spec = draft(Resolution::P480);
    spec.is_draft = false;
    spec.resolution = Resolution::P1080;
    spec.draft_job_id = Some("draft-job".to_owned());
    spec
}

fn images() -> RemoteInputs {
    RemoteInputs {
        video: None,
        images: vec![
            RemoteAsset {
                url: file_url("/a.jpg"),
                id: Some("image-1".to_owned()),
            },
            RemoteAsset {
                url: file_url("/b.jpg"),
                id: Some("image-2".to_owned()),
            },
        ],
        audio: None,
    }
}

/// The `params` of every `generate_video` call.
async fn params(stub: &Stub) -> Vec<Value> {
    calls_of(stub, "generate_video")
        .await
        .into_iter()
        .filter_map(|call| call["arguments"].get("params").cloned())
        .collect()
}

fn credits(amount: i64, is_estimate: bool) -> Cost {
    Cost::credits(Decimal::from(amount), is_estimate)
}

#[tokio::test]
async fn a_draft_at480p_asks_for_the_native_draft() {
    let (provider, stub) = provider(
        full_set(),
        |_, _| json!({"results": [{"id": "job-1"}, {"id": "job-2"}]}),
    )
    .await;
    let mut spec = draft(Resolution::P480);
    spec.variants = 2;
    let job = provider.submit(&spec, &images(), Uuid::new_v4()).await.unwrap();

    let sent = params(&stub).await.first().cloned().unwrap();
    assert_eq!(sent["draft"], true);
    assert_eq!(sent["resolution"], "480p");
    assert_eq!(sent["mode"], "omni_reference");
    assert!(same(&sent["duration"], &json!(15)));
    assert!(sent.get("draft_job_id").is_none());
    assert_eq!(violations_of_params(&sent), Vec::<String>::new());
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
    assert!(job.is_native_draft);
    assert_eq!(
        job.variants
            .iter()
            .map(|variant| variant.is_native_draft)
            .collect::<Vec<_>>(),
        [true]
    );
}

/// A final, a draft at another resolution and a draft of an Edit stay plain renders.
#[tokio::test]
async fn other_jobs_send_no_draft() {
    let mut last = draft(Resolution::P480);
    last.is_draft = false;
    let mut extend = draft(Resolution::P480);
    extend.mode = Mode::Extend;
    for spec in [last, draft(Resolution::P720), extend] {
        let (provider, stub) = provider(full_set(), |_, _| json!({"results": [{"id": "job-1"}]})).await;
        let job = provider.submit(&spec, &images(), Uuid::new_v4()).await.unwrap();
        assert!(
            params(&stub)
                .await
                .first()
                .and_then(|sent| sent.get("draft"))
                .is_none(),
            "{spec:?}"
        );
        assert!(!job.is_native_draft);
    }
}

/// The draft's quote before upload asks with `draft: true`.
#[tokio::test]
async fn the_draft_quote_asks_for_a_draft() {
    let (provider, stub) = provider(full_set(), |_, _| json!({"cost": {"credits": 45}})).await;
    assert_eq!(
        provider.estimate(&draft(Resolution::P480)).await,
        Ok(credits(45, false))
    );
    let quotes = params(&stub).await;
    assert_eq!(quotes.len(), 1);
    assert!(
        quotes
            .iter()
            .all(|quote| quote["draft"] == true && quote["get_cost"] == true)
    );
}

#[tokio::test]
async fn a_finalize_sends_only_its_four_keys() {
    let (provider, stub) = provider(full_set(), |_, arguments| {
        if arguments["params"]["get_cost"] == true {
            json!({"cost": {"credits": 80}})
        } else {
            json!({"results": [{"id": "final-1"}]})
        }
    })
    .await;
    let mut spec = finalize();
    spec.uses_unlimited = true;
    assert_eq!(
        provider.server_estimate(&spec, &images()).await,
        Ok(Some(credits(80, false)))
    );
    let job = provider.submit(&spec, &images(), Uuid::new_v4()).await.unwrap();
    assert_eq!(job.request_id, "final-1");
    assert!(!job.is_native_draft);

    let sent = params(&stub).await;
    assert_eq!(sent.len(), 2);
    let expected = json!({
        "model": "seedance_2_5", "draft_job_id": "draft-job", "resolution": "1080p", "use_unlim": false,
    });
    let mut quote = sent[0].as_object().cloned().unwrap_or_default();
    assert_eq!(quote.remove("get_cost"), Some(json!(true)));
    assert_eq!(Value::Object(quote), expected);
    assert_eq!(sent[1], expected);
    assert_eq!(violations_of_params(&expected), Vec::<String>::new());
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

/// Finalize is quoted live before anything runs; a failed quote shows a third of the 1080p rate, as an estimate
/// (2026-10-15: 80 credits for a 15 s draft, against 240 for a fresh 1080p render).
#[tokio::test]
async fn a_finalize_is_quoted_and_falls_back_to_a_third_of_the_fresh_rate() {
    let (quoted, stub) = provider(full_set(), |_, _| json!({"cost": {"credits": 80}})).await;
    assert_eq!(quoted.estimate(&finalize()).await, Ok(credits(80, false)));
    let mut quote = params(&stub)
        .await
        .first()
        .and_then(|quote| quote.as_object().cloned())
        .unwrap_or_default();
    quote.remove("get_cost");
    assert_eq!(
        quote.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from(["model", "draft_job_id", "resolution", "use_unlim"])
    );

    let (failing, _failing_stub) = provider(full_set(), |_, _| json!({})).await;
    assert_eq!(failing.estimate(&finalize()).await, Ok(credits(80, true)));
    let mut fresh = finalize();
    fresh.draft_job_id = None;
    assert_eq!(
        estimated_credits(&fresh, &ModelConstraintTable::built_in()),
        Some(240)
    );
}

/// Two drafts of the same length don't share a finalize quote.
#[tokio::test]
async fn each_drafts_finalize_is_quoted_on_its_own() {
    let (provider, stub) = provider(full_set(), |_, _| json!({"cost": {"credits": 80}})).await;
    let mut other = finalize();
    other.draft_job_id = Some("other-draft".to_owned());
    provider.estimate(&finalize()).await.unwrap();
    provider.estimate(&other).await.unwrap();
    assert_eq!(
        params(&stub)
            .await
            .iter()
            .map(|sent| sent["draft_job_id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("draft-job"), json!("other-draft")]
    );
}
