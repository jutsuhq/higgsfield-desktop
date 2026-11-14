//! What runs an agent's paid request once for its request ID (`AgentRequestGateTests.swift`).

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use time::OffsetDateTime;
use tokio::sync::watch;
use uuid::Uuid;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Refused(&'static str);

type Gate = AgentRequestGate<String, Refused>;
type Raise = std::future::Ready<Result<AgentRaiseOutcome<String>, Refused>>;

fn refusals() -> Refusals<Refused> {
    Refusals {
        conflict: Refused("conflict"),
        unreadable: Refused("unreadable"),
        never_queued: Refused("never queued"),
    }
}

fn gate() -> Arc<Gate> {
    Gate::open(None)
}

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

fn count(counter: &Arc<AtomicUsize>) -> usize {
    counter.load(Ordering::SeqCst)
}

/// Waits until `condition` holds, for at most five seconds.
async fn eventually(condition: impl Fn() -> bool) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the condition never held");
}

/// Runs `queue` through `gate` for `r1`, counting the discards; `known` answers for the recorded IDs.
async fn run<QF>(
    gate: &Arc<Gate>,
    fingerprint: &str,
    discards: &Arc<AtomicUsize>,
    known: impl FnOnce(Vec<Uuid>) -> Option<String>,
    queue: impl FnOnce() -> QF,
) -> Result<String, Refused>
where
    QF: Future<Output = Result<String, Refused>> + Send + 'static,
{
    let discards = Arc::clone(discards);
    gate.run(
        "r1",
        fingerprint,
        None,
        refusals(),
        move || {
            discards.fetch_add(1, Ordering::SeqCst);
        },
        |ids| async move { Ok(known(ids)) },
        None::<fn(Vec<Uuid>) -> Raise>,
        queue,
    )
    .await
}

fn nothing_known(_: Vec<Uuid>) -> Option<String> {
    None
}

fn ran() -> std::future::Ready<Result<String, Refused>> {
    std::future::ready(Ok("ran".to_owned()))
}

#[tokio::test]
async fn a_recorded_request_answers_with_what_it_made_and_runs_nothing() {
    let gate = gate();
    let job = Uuid::new_v4();
    gate.record("r1", "f1", None, vec![job], OffsetDateTime::now_utc(), None)
        .unwrap();
    let discards = counter();
    let answer = run(
        &gate,
        "f1",
        &discards,
        |ids| (ids == vec![job]).then(|| "known".to_owned()),
        || async { panic!("a known request ran again") },
    )
    .await;
    assert_eq!(answer, Ok("known".to_owned()));
    assert_eq!(count(&discards), 1);
    assert_eq!(
        gate.state("r1", "f1", None).unwrap(),
        AgentRequestState::Known(vec![job])
    );
}

/// Recorded, but what it made was never saved: it is forgotten, so the same request runs when sent again.
#[tokio::test]
async fn a_recorded_request_that_made_nothing_is_forgotten() {
    let gate = gate();
    gate.record(
        "r1",
        "f1",
        None,
        vec![Uuid::new_v4()],
        OffsetDateTime::now_utc(),
        None,
    )
    .unwrap();
    let discards = counter();
    assert_eq!(
        run(&gate, "f1", &discards, nothing_known, ran).await,
        Err(Refused("never queued"))
    );
    assert_eq!(gate.state("r1", "f1", None).unwrap(), AgentRequestState::New);
    assert_eq!(
        run(&gate, "f1", &discards, nothing_known, ran).await,
        Ok("ran".to_owned())
    );
}

#[tokio::test]
async fn the_id_with_other_parameters_is_a_conflict() {
    let gate = gate();
    gate.record(
        "r1",
        "f1",
        None,
        vec![Uuid::new_v4()],
        OffsetDateTime::now_utc(),
        None,
    )
    .unwrap();
    let discards = counter();
    assert_eq!(
        run(&gate, "f2", &discards, nothing_known, ran).await,
        Err(Refused("conflict"))
    );
    assert_eq!(count(&discards), 1);
}

/// Sends `r1` with `p1` again under `fingerprint`; `outcome` is what the raise makes of the recorded IDs.
async fn raise(
    gate: &Arc<Gate>,
    fingerprint: &str,
    discards: &Arc<AtomicUsize>,
    outcome: impl FnOnce(Vec<Uuid>) -> Raise,
) -> Result<String, Refused> {
    let discards = Arc::clone(discards);
    gate.run(
        "r1",
        fingerprint,
        Some("p1"),
        refusals(),
        move || {
            discards.fetch_add(1, Ordering::SeqCst);
        },
        |_| async { Ok(Some("known".to_owned())) },
        Some(outcome),
        || async { panic!("a request sent again ran again") },
    )
    .await
}

/// The same parameters with another `max_cost` go to `other_max_cost`, which may answer or refuse; a request without
/// parameters, or recorded without them, stays a conflict.
#[tokio::test]
async fn the_same_parameters_with_another_max_cost_ask_whether_the_hold_lifts() {
    let gate = gate();
    let job = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();
    gate.record("r1", "f1", Some("p1"), vec![job], now, None).unwrap();
    assert_eq!(
        gate.state("r1", "f2", Some("p1")).unwrap(),
        AgentRequestState::OtherMaxCost(vec![job])
    );
    assert_eq!(
        gate.state("r1", "f2", Some("p2")).unwrap(),
        AgentRequestState::Conflict
    );
    assert_eq!(gate.state("r1", "f2", None).unwrap(), AgentRequestState::Conflict);
    let discards = counter();
    let lifted = raise(&gate, "f2", &discards, |ids| {
        std::future::ready(Ok(if ids == vec![job] {
            AgentRaiseOutcome::Answer("lifted".to_owned())
        } else {
            AgentRaiseOutcome::NotLiftable
        }))
    })
    .await;
    assert_eq!(lifted, Ok("lifted".to_owned()));
    let refused = raise(&gate, "f3", &discards, |_| {
        std::future::ready(Ok(AgentRaiseOutcome::NotLiftable))
    })
    .await;
    assert_eq!(refused, Err(Refused("conflict")));
    assert_eq!(count(&discards), 2);

    gate.update("r1", "f2").unwrap();
    assert_eq!(
        gate.state("r1", "f2", Some("p1")).unwrap(),
        AgentRequestState::Known(vec![job])
    );
    assert_eq!(
        gate.state("r1", "f1", Some("p1")).unwrap(),
        AgentRequestState::OtherMaxCost(vec![job])
    );

    gate.record("r2", "f1", None, vec![job], now, None).unwrap();
    assert_eq!(
        gate.state("r2", "f2", Some("p1")).unwrap(),
        AgentRequestState::Conflict
    );
}

/// The raise sent again while the first one runs joins it, so the engine checks the job once; the raise to another
/// `max_cost` meanwhile runs on its own.
#[tokio::test]
async fn the_same_raise_sent_while_it_runs_joins_it() {
    let gate = gate();
    gate.record(
        "r1",
        "f1",
        Some("p1"),
        vec![Uuid::new_v4()],
        OffsetDateTime::now_utc(),
        None,
    )
    .unwrap();
    let (release, released) = watch::channel(false);
    let raises = counter();
    let discards = counter();
    let raising = |gate: Arc<Gate>, fingerprint: &'static str| {
        let raises = Arc::clone(&raises);
        let discards = Arc::clone(&discards);
        let mut released = released.clone();
        async move {
            let counted = Arc::clone(&discards);
            gate.run(
                "r1",
                fingerprint,
                Some("p1"),
                refusals(),
                move || {
                    counted.fetch_add(1, Ordering::SeqCst);
                },
                |_| async { Ok(Some("known".to_owned())) },
                Some(move |_| async move {
                    raises.fetch_add(1, Ordering::SeqCst);
                    if fingerprint == "f2" {
                        let _ = released.wait_for(|open| *open).await;
                    }
                    Ok(AgentRaiseOutcome::Answer(format!("raised to {fingerprint}")))
                }),
                || async { panic!("a request sent again ran again") },
            )
            .await
        }
    };
    let first = tokio::spawn(raising(Arc::clone(&gate), "f2"));
    eventually(|| count(&raises) == 1).await;
    let second = tokio::spawn(raising(Arc::clone(&gate), "f2"));
    eventually(|| count(&discards) == 2).await;
    assert_eq!(
        raising(Arc::clone(&gate), "f3").await,
        Ok("raised to f3".to_owned())
    );
    release.send(true).unwrap();
    assert_eq!(first.await.unwrap(), Ok("raised to f2".to_owned()));
    assert_eq!(second.await.unwrap(), Ok("raised to f2".to_owned()));
    assert_eq!(count(&raises), 2);
    assert_eq!(count(&discards), 3);
}

/// A raise whose recorded job is gone is answered as never queued and forgotten, so the request runs anew.
#[tokio::test]
async fn a_raise_of_a_job_that_is_gone_is_forgotten() {
    let gate = gate();
    gate.record(
        "r1",
        "f1",
        Some("p1"),
        vec![Uuid::new_v4()],
        OffsetDateTime::now_utc(),
        None,
    )
    .unwrap();
    let gone = raise(&gate, "f2", &counter(), |_| {
        std::future::ready(Ok(AgentRaiseOutcome::Gone))
    })
    .await;
    assert_eq!(gone, Err(Refused("never queued")));
    assert_eq!(
        gate.state("r1", "f2", Some("p1")).unwrap(),
        AgentRequestState::New
    );
}

/// The same request sent while it runs waits for that run, and its own copies of the files are discarded; the ID with
/// other parameters is refused meanwhile.
#[tokio::test]
async fn the_same_request_sent_while_it_runs_joins_it() {
    let gate = gate();
    let (release, mut released) = watch::channel(false);
    let runs = counter();
    let discards = counter();
    let first = {
        let gate = Arc::clone(&gate);
        let runs = Arc::clone(&runs);
        let discards = Arc::clone(&discards);
        tokio::spawn(async move {
            let recording = Arc::clone(&gate);
            run(&gate, "f1", &discards, nothing_known, move || async move {
                runs.fetch_add(1, Ordering::SeqCst);
                let _ = recording.record(
                    "r1",
                    "f1",
                    None,
                    vec![Uuid::new_v4()],
                    OffsetDateTime::now_utc(),
                    None,
                );
                let _ = released.wait_for(|open| *open).await;
                Ok("made".to_owned())
            })
            .await
        })
    };
    eventually(|| count(&runs) == 1).await;
    assert_eq!(gate.state("r1", "f1", None).unwrap(), AgentRequestState::Running);
    assert_eq!(
        run(&gate, "f2", &discards, nothing_known, || async {
            Ok("other".to_owned())
        })
        .await,
        Err(Refused("conflict"))
    );
    let second = {
        let gate = Arc::clone(&gate);
        let discards = Arc::clone(&discards);
        tokio::spawn(async move {
            run(&gate, "f1", &discards, nothing_known, || async {
                Ok("ran twice".to_owned())
            })
            .await
        })
    };
    eventually(|| count(&discards) == 2).await;
    release.send(true).unwrap();
    assert_eq!(first.await.unwrap(), Ok("made".to_owned()));
    assert_eq!(second.await.unwrap(), Ok("made".to_owned()));
    assert_eq!(count(&runs), 1);
    // The conflict's files and the second request's; the first run's files are its job's.
    assert_eq!(count(&discards), 2);
}

/// A run that fails is forgotten and its files discarded, so the agent can fix it and send it again.
#[tokio::test]
async fn a_failed_run_is_forgotten() {
    let gate = gate();
    let discards = counter();
    let recording = Arc::clone(&gate);
    let failed = run(&gate, "f1", &discards, nothing_known, move || async move {
        let _ = recording.record(
            "r1",
            "f1",
            None,
            vec![Uuid::new_v4()],
            OffsetDateTime::now_utc(),
            None,
        );
        Err(Refused("bad prompt"))
    })
    .await;
    assert_eq!(failed, Err(Refused("bad prompt")));
    assert_eq!(count(&discards), 1);
    assert_eq!(gate.state("r1", "f1", None).unwrap(), AgentRequestState::New);
    assert_eq!(
        run(&gate, "f1", &discards, nothing_known, ran).await,
        Ok("ran".to_owned())
    );
    assert_eq!(count(&discards), 1);
}
