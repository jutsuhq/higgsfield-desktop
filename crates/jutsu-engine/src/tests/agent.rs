//! The spending guard of agents' jobs and the agent ledger file: a port of `AgentGuardTests` and
//! `AgentSpendFileTests`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use jutsu_core::{
    AgentHold, AgentMoney, AgentMoneyUnit, AgentOrigin, AgentSpend, AgentSpendLedger, AgentSpendTotal, Cost,
    FailureReason, Job, JobState, PauseReason, PortError, ProviderError, ProviderKind, Rejection,
    RemoteStatus,
};
use rust_decimal::Decimal;
use time::{Duration, OffsetDateTime, UtcOffset};
use uuid::Uuid;

use super::{Harness, SubmitStep, eventually, failed, make_spec, paused, quiesce, read_spend, spend, usd};
use crate::clock::EngineClock;
use crate::{AgentSpendFile, AgentSpendPolicy, EnqueueOptions, JobEngine};

pub(super) fn cents(amount: i64) -> AgentMoney {
    AgentMoney::new(amount, AgentMoneyUnit::UsdCents)
}

pub(super) fn origin(max_cents: i64, request: &str) -> AgentOrigin {
    AgentOrigin::new(Some("claude-code".into()), request, cents(max_cents))
}

/// Queued by an agent.
pub(super) fn by_agent(origin: AgentOrigin) -> EnqueueOptions {
    EnqueueOptions {
        agent: Some(origin),
        ..EnqueueOptions::default()
    }
}

pub(super) fn total(spent: i64, reserved: i64) -> AgentSpendTotal {
    AgentSpendTotal {
        spent,
        reserved,
        unlimited_charged: false,
    }
}

/// A ledger that refuses every write, as a full disk or a broken file would.
struct BrokenLedger;

fn broken() -> PortError {
    PortError::Other("broken".into())
}

#[async_trait]
impl AgentSpendLedger for BrokenLedger {
    async fn reserve(&self, _spend: AgentSpend, _daily_limit: Option<i64>) -> Result<bool, PortError> {
        Err(broken())
    }
    async fn release(&self, _id: Uuid) -> Result<(), PortError> {
        Err(broken())
    }
    async fn settle(&self, _id: Uuid, _amount: Option<i64>) -> Result<(), PortError> {
        Err(broken())
    }
    async fn unbill(&self, _id: Uuid, _part: String, _parts: i64) -> Result<(), PortError> {
        Err(broken())
    }
    async fn total(&self, _day: OffsetDateTime, _unit: AgentMoneyUnit) -> Result<AgentSpendTotal, PortError> {
        Err(broken())
    }
    async fn unsettled(&self) -> Result<Vec<AgentSpend>, PortError> {
        Err(broken())
    }
}

/// A harness with an agent ledger file on its clock.
pub(super) struct Agents {
    pub(super) harness: Harness,
    pub(super) ledger: Arc<AgentSpendFile>,
    pub(super) file: PathBuf,
    _folder: tempfile::TempDir,
}

impl Agents {
    pub(super) fn new() -> Self {
        Self::over(Harness::new())
    }

    pub(super) fn over(harness: Harness) -> Self {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join(AgentSpendFile::FILE_NAME);
        let clock = harness.clock.clone();
        let ledger = Arc::new(AgentSpendFile::with_clock(
            file.clone(),
            UtcOffset::UTC,
            Arc::new(move || clock.now()),
        ));
        Self {
            harness,
            ledger,
            file,
            _folder: folder,
        }
    }

    pub(super) fn policy(&self, daily_limit: i64, unlimited: bool) -> AgentSpendPolicy {
        AgentSpendPolicy::new(self.ledger.clone(), move |_| daily_limit).allowing_unlimited(move || unlimited)
    }

    pub(super) async fn engine(&self) -> JobEngine {
        self.engine_with(1_000, false).await
    }

    pub(super) async fn engine_with(&self, daily_limit: i64, unlimited: bool) -> JobEngine {
        self.harness
            .engine_with(self.harness.deps(Some(self.policy(daily_limit, unlimited))))
            .await
    }

    pub(super) async fn total(&self) -> AgentSpendTotal {
        self.ledger
            .total(self.harness.clock.now(), AgentMoneyUnit::UsdCents)
            .await
            .unwrap()
    }

    /// Waits until the ledger's total is `expected`; fails the test after 10 s.
    pub(super) async fn settles(&self, expected: AgentSpendTotal) {
        let waited = tokio::time::timeout(StdDuration::from_secs(10), async {
            while self.total().await != expected {
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(
            waited.is_ok(),
            "the ledger shows {:?}, not {expected:?}",
            self.total().await
        );
    }

    pub(super) fn submits(&self) -> usize {
        self.harness.log.count("submit")
    }

    pub(super) fn stored(&self, id: Uuid) -> Job {
        self.harness.repository.stored(id).unwrap()
    }

    pub(super) async fn held(&self, id: Uuid) -> Job {
        self.harness
            .repository
            .wait_state(id, &paused(PauseReason::AgentLimit))
            .await
    }

    pub(super) async fn done(&self, id: Uuid) -> Job {
        self.harness.repository.wait_state(id, &JobState::Done).await
    }
}

/// The fake's list price is an exact $1.30, 130 cents.
#[tokio::test]
async fn a_job_under_every_limit_is_sent_and_its_charge_recorded() {
    let agents = Agents::new();
    let engine = agents.engine().await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    let done = agents.done(job.id).await;

    assert_eq!(agents.submits(), 1);
    assert!(done.agent.unwrap().reservation.is_some());
    assert_eq!(agents.total().await, total(130, 0));
}

/// A crash between the ledger's write and the job's: the job never held the reservation and has no submit marker, so
/// nothing was sent and the launch gives it back. One the job holds stays, as does an image request's, whose job
/// isn't in the store.
#[tokio::test]
async fn a_launch_releases_reservations_no_submit_took() {
    let agents = Agents::new();
    let now = agents.harness.clock.now();
    let new_job = || {
        Job::new(
            Uuid::new_v4(),
            make_spec(1),
            ProviderKind::ApiKey,
            usd("1.30"),
            Uuid::new_v4(),
            now,
        )
    };
    let mut orphaned = new_job();
    orphaned.state = paused(PauseReason::ByUser);
    orphaned.agent = Some(origin(500, "r-1"));
    let mut holding = new_job();
    holding.state = paused(PauseReason::ByUser);
    holding.agent = Some(origin(500, "r-2"));
    let held = AgentSpend::new(Uuid::new_v4(), holding.id, cents(50), now, false);
    holding.agent.as_mut().unwrap().reservation = Some(held.id);
    let lost = AgentSpend::new(Uuid::new_v4(), orphaned.id, cents(130), now, false);
    let image = AgentSpend::new(Uuid::new_v4(), Uuid::new_v4(), cents(7), now, false);
    agents.harness.repository.put(orphaned);
    agents.harness.repository.put(holding);
    let (held_id, image_id) = (held.id, image.id);
    for spend in [lost, held, image] {
        assert!(agents.ledger.reserve(spend, None).await.unwrap());
    }

    let _engine = agents.engine().await;

    let mut unsettled: Vec<Uuid> = agents
        .ledger
        .unsettled()
        .await
        .unwrap()
        .into_iter()
        .map(|spend| spend.id)
        .collect();
    unsettled.sort();
    let mut expected = vec![held_id, image_id];
    expected.sort();
    assert_eq!(unsettled, expected);
    assert_eq!(agents.submits(), 0);
}

#[tokio::test]
async fn a_price_over_max_cost_pauses_without_sending() {
    let agents = Agents::new();
    let engine = agents.engine().await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(129, "r-1")))
        .await
        .unwrap();
    let held = agents.held(job.id).await;

    assert_eq!(held.agent.unwrap().hold, Some(AgentHold::OverMaxCost));
    assert_eq!(agents.submits(), 0);
    assert_eq!(agents.total().await, AgentSpendTotal::default());
}

/// A reservation in flight counts against the budget as much as a charge.
#[tokio::test]
async fn reservations_in_flight_count_against_the_daily_budget() {
    let agents = Agents::new();
    let now = agents.harness.clock.now();
    let held = AgentSpend::new(Uuid::new_v4(), Uuid::new_v4(), cents(100), now, false);
    assert!(agents.ledger.reserve(held, None).await.unwrap());
    let engine = agents.engine_with(200, false).await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    let held = agents.held(job.id).await;

    assert_eq!(held.agent.unwrap().hold, Some(AgentHold::OverDailyBudget));
    assert_eq!(agents.submits(), 0);
    assert_eq!(agents.total().await.reserved, 100);
}

#[tokio::test]
async fn no_budget_set_sends_nothing() {
    let agents = Agents::new();
    let engine = agents.engine_with(0, false).await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    let held = agents.held(job.id).await;

    assert_eq!(held.agent.unwrap().hold, Some(AgentHold::OverDailyBudget));
    assert_eq!(agents.submits(), 0);
}

#[tokio::test]
async fn a_price_not_known_pauses() {
    let preliminaries = [
        Cost::UNKNOWN_USD,
        Cost::usd(Decimal::ONE, true),
        Cost::credits(Decimal::from(24), true),
    ];
    for preliminary in preliminaries {
        let agents = Agents::new();
        agents
            .harness
            .provider
            .with(|state| state.preliminary = Some(preliminary.clone()));
        let engine = agents.engine().await;

        let job = engine
            .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
            .await
            .unwrap();
        let held = agents.held(job.id).await;

        assert_eq!(
            held.agent.unwrap().hold,
            Some(AgentHold::UnknownPrice),
            "{preliminary:?}"
        );
        assert_eq!(agents.submits(), 0);
    }
}

/// The server's quote right before submit is the price the guard checks.
#[tokio::test]
async fn the_server_quote_is_what_is_checked() {
    let agents = Agents::new();
    agents.harness.provider.with(|state| {
        state.preliminary = Some(Cost::usd(Decimal::ONE, true));
        state.quote = Some(usd("6"));
    });
    let engine = agents.engine().await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    let held = agents.held(job.id).await;

    assert_eq!(held.agent.unwrap().hold, Some(AgentHold::OverMaxCost));
    assert_eq!(held.server_price, Some(usd("6")));
}

/// Only the person resumes a held job; it then goes past the agent's limits at the price they saw.
#[tokio::test]
async fn the_persons_resume_sends_a_held_job_and_resume_all_does_not() {
    let agents = Agents::new();
    let engine = agents.engine().await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(10, "r-1")))
        .await
        .unwrap();
    agents.held(job.id).await;
    engine.resume_all(false).await;
    quiesce().await;
    assert_eq!(agents.stored(job.id).state, paused(PauseReason::AgentLimit));

    engine.resume(job.id).await.unwrap();
    let done = agents.done(job.id).await;

    assert!(done.agent.unwrap().approved_by_person);
    assert_eq!(agents.submits(), 1);
    assert_eq!(agents.total().await.spent, 130);
}

/// The person approved the price they saw; a quote that came only after that and is higher waits for them again.
#[tokio::test]
async fn a_quote_above_the_approved_price_is_held_again() {
    let agents = Agents::new();
    agents.harness.provider.with(|state| {
        state.preliminary = Some(Cost::usd(Decimal::ONE, true));
        state.quote_error = Some(PortError::Provider(ProviderError::Network));
    });
    let engine = agents.engine().await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    agents.held(job.id).await;
    agents.harness.provider.with(|state| {
        state.quote_error = None;
        state.quote = Some(usd("6"));
    });
    engine.resume(job.id).await.unwrap();
    agents
        .harness
        .repository
        .wait(job.id, |job| {
            job.agent.as_ref().and_then(|agent| agent.hold) == Some(AgentHold::AboveApprovedPrice)
        })
        .await;
    assert_eq!(agents.submits(), 0);

    engine.resume(job.id).await.unwrap();
    agents.done(job.id).await;
    assert_eq!(agents.submits(), 1);
    assert_eq!(agents.total().await.spent, 600);
}

#[tokio::test]
async fn the_ledger_failing_pauses_without_sending() {
    let agents = Agents::new();
    let policy = AgentSpendPolicy::new(Arc::new(BrokenLedger), |_| 1_000);
    let engine = agents
        .harness
        .engine_with(agents.harness.deps(Some(policy)))
        .await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    let held = agents.held(job.id).await;

    assert_eq!(held.agent.unwrap().hold, Some(AgentHold::LedgerUnavailable));
    assert_eq!(agents.submits(), 0);
}

#[tokio::test]
async fn without_a_ledger_nothing_an_agent_asks_for_is_sent() {
    let agents = Agents::new();
    let engine = agents.harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    let held = agents.held(job.id).await;

    assert_eq!(held.agent.unwrap().hold, Some(AgentHold::LedgerUnavailable));
    assert_eq!(agents.submits(), 0);
}

/// The person's own cap holds for agents too, and is checked strictly at the price about to be sent.
#[tokio::test]
async fn the_persons_daily_cap_still_applies() {
    let mut harness = Harness::new();
    harness.spend_status = Some(spend("0", "1"));
    let agents = Agents::over(harness);
    let engine = agents.engine().await;

    let options = EnqueueOptions {
        over_cap_approved: true,
        ..by_agent(origin(500, "r-1"))
    };
    assert!(engine.enqueue(make_spec(1), options).await.is_err());
    assert_eq!(agents.submits(), 0);
}

/// Spend that grew after the job was queued still stops it at submit, before the agent's own limits.
#[tokio::test]
async fn the_persons_cap_is_checked_again_right_before_the_submit() {
    let agents = Agents::new();
    let reads = Arc::new(Mutex::new(0));
    let mut deps = agents.harness.deps(Some(agents.policy(1_000, false)));
    deps.spend_status = Some(read_spend(move || {
        let mut count = reads.lock().unwrap();
        *count += 1;
        Ok(Some(spend(if *count > 1 { "1.5" } else { "0" }, "2")))
    }));
    let engine = agents.harness.engine_with(deps).await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    agents
        .harness
        .repository
        .wait_state(job.id, &paused(PauseReason::PriceAboveCap))
        .await;

    assert_eq!(agents.submits(), 0);
}

#[tokio::test]
async fn a_refused_submit_gives_its_reservation_back() {
    let agents = Agents::new();
    agents.harness.provider.with(|state| {
        state.submit_steps = [SubmitStep::Fail(PortError::Provider(ProviderError::Validation {
            field: None,
            message: "bad prompt".into(),
        }))]
        .into();
    });
    let engine = agents.engine().await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    let failed = agents
        .harness
        .repository
        .wait_state(job.id, &failed(FailureReason::Message("bad prompt".into())))
        .await;

    assert_eq!(failed.agent.unwrap().reservation, None);
    assert_eq!(agents.total().await, AgentSpendTotal::default());
}

/// A lost answer may have been charged: the reservation stays, and the resend with the same key reserves nothing
/// more.
#[tokio::test]
async fn a_lost_answer_keeps_its_reservation_and_the_resend_reserves_no_more() {
    let agents = Agents::new();
    agents.harness.provider.with(|state| {
        state.submit_steps = [SubmitStep::Fail(PortError::Provider(ProviderError::Network))].into();
    });
    let engine = agents.engine().await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    agents.done(job.id).await;

    assert_eq!(agents.submits(), 2);
    assert_eq!(agents.total().await, total(130, 0));
}

/// Cut off while the submit waits for its answer (quit, cancel): it may have reached Higgsfield, so the reservation
/// stays.
#[tokio::test]
async fn a_submit_cut_off_keeps_its_reservation() {
    let agents = Agents::new();
    agents
        .harness
        .provider
        .with(|state| state.submit_steps = [SubmitStep::Hang].into());
    let engine = agents.engine().await;

    engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    eventually(|| agents.submits() == 1).await;
    engine.stop(Duration::ZERO).await;

    assert_eq!(agents.total().await, total(0, 130));
}

/// A submit cut off without an idempotency key may have been charged: the retry, a new attempt, reserves anew, and
/// the old reservation keeps counting, through the next launch too.
#[tokio::test]
async fn a_retry_keeps_the_charge_of_a_submit_that_may_have_been_sent() {
    let agents = Agents::new();
    agents.harness.provider.with(|state| {
        state.not_idempotent = true;
        state.submit_steps = [SubmitStep::Hang].into();
    });
    let first = agents.engine().await;
    let job = first
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    eventually(|| agents.submits() == 1).await;
    first.stop(Duration::ZERO).await;

    let second = agents.engine().await;
    agents
        .harness
        .repository
        .wait_state(job.id, &failed(FailureReason::SubmitCutOff))
        .await;
    second.retry(job.id).await.unwrap();
    agents.done(job.id).await;
    second.stop(Duration::ZERO).await;
    let _third = agents.engine().await;

    assert_eq!(agents.submits(), 2);
    assert_eq!(agents.total().await, total(260, 0));
}

/// A submit without an idempotency key that may have been taken is sent again as a new submit: its reservation keeps
/// counting next to the new one's, through the next launch too.
#[tokio::test]
async fn a_new_submit_keeps_the_charge_of_the_one_before() {
    let agents = Agents::new();
    agents.harness.provider.with(|state| {
        state.not_idempotent = true;
        state.submit_steps = [SubmitStep::Fail(PortError::Provider(ProviderError::Network))].into();
    });
    let first = agents.engine().await;
    let job = first
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    agents.done(job.id).await;
    first.stop(Duration::ZERO).await;
    let _second = agents.engine().await;

    assert_eq!(agents.submits(), 2);
    assert_eq!(agents.total().await, total(260, 0));
}

#[tokio::test]
async fn variants_keep_the_agent_but_not_the_reservation() {
    let agents = Agents::new();
    let engine = agents.engine().await;
    let mut spec = make_spec(1);
    spec.variants = 2;

    let job = engine.enqueue(spec, by_agent(origin(300, "r-1"))).await.unwrap();
    let batch = job.batch_id;
    let in_batch = move |jobs: Vec<Job>| -> Vec<Job> {
        jobs.into_iter().filter(|other| other.batch_id == batch).collect()
    };
    agents
        .harness
        .repository
        .wait_until(|repository| {
            in_batch(repository.all())
                .iter()
                .filter(|job| job.state == JobState::Done)
                .count()
                == 2
        })
        .await;

    let jobs = in_batch(agents.harness.repository.all());
    assert!(
        jobs.iter()
            .all(|job| job.agent.as_ref().is_some_and(|agent| agent.request_id == "r-1"))
    );
    assert_eq!(
        jobs.iter()
            .filter(|job| job
                .agent
                .as_ref()
                .is_some_and(|agent| agent.reservation.is_some()))
            .count(),
        1
    );
    assert_eq!(agents.total().await.spent, 260);
}

/// A job the plan's unlimited allowance would pay for waits for the person while agents may not use it, even with
/// room in the budget; their Confirm sends it, and it counts nothing.
#[tokio::test]
async fn an_unlimited_job_waits_for_the_person_while_agents_may_not_use_unlimited() {
    let agents = Agents::new();
    agents.harness.provider.with(|state| {
        state.pays_unlimited = true;
        state.unlimited_answer = (true, None);
    });
    let engine = agents.engine().await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    let held = agents.held(job.id).await;
    assert_eq!(held.agent.unwrap().hold, Some(AgentHold::UnlimitedNotAllowed));
    assert_eq!(agents.submits(), 0);

    engine.resume(job.id).await.unwrap();
    agents.done(job.id).await;
    assert_eq!(agents.submits(), 1);
    assert_eq!(agents.total().await, AgentSpendTotal::default());
}

/// Allowed, it runs with no budget at all and counts nothing against it.
#[tokio::test]
async fn an_unlimited_job_runs_without_the_budget_when_agents_may_use_unlimited() {
    let agents = Agents::new();
    agents.harness.provider.with(|state| {
        state.pays_unlimited = true;
        state.unlimited_answer = (true, None);
    });
    let engine = agents.engine_with(0, true).await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(0, "r-1")))
        .await
        .unwrap();
    agents.done(job.id).await;

    assert_eq!(agents.submits(), 1);
    assert_eq!(agents.total().await, AgentSpendTotal::default());
}

/// The allowance didn't pay for a job sent for it: what it was charged counts, and for the rest of the day agents'
/// jobs for the allowance keep every limit, so with no budget the next one waits for the person.
#[tokio::test]
async fn an_unlimited_job_charged_anyway_counts_and_ends_unlimited_for_the_day() {
    let agents = Agents::new();
    agents.harness.provider.with(|state| {
        state.pays_unlimited = true;
        state.unlimited_answer = (false, Some(usd("2")));
    });
    let engine = agents.engine_with(0, true).await;

    let first = engine
        .enqueue(make_spec(1), by_agent(origin(0, "r-1")))
        .await
        .unwrap();
    agents.done(first.id).await;
    assert_eq!(
        agents.total().await,
        AgentSpendTotal {
            spent: 200,
            reserved: 0,
            unlimited_charged: true
        }
    );

    let second = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-2")))
        .await
        .unwrap();
    let held = agents.held(second.id).await;
    assert_eq!(held.agent.unwrap().hold, Some(AgentHold::OverDailyBudget));
    assert_eq!(agents.submits(), 1);

    let tomorrow_at = agents.harness.clock.now() + Duration::days(1);
    let tomorrow =
        AgentSpendFile::with_clock(agents.file.clone(), UtcOffset::UTC, Arc::new(move || tomorrow_at));
    assert!(
        !tomorrow
            .total(tomorrow_at, AgentMoneyUnit::UsdCents)
            .await
            .unwrap()
            .unlimited_charged
    );
}

/// Once the allowance is no longer trusted, a job for it is quoted like any other before it is sent: an estimate alone
/// would hold every such job as unknown_price for the rest of the day, however far within max_cost.
#[tokio::test]
async fn a_job_the_allowance_no_longer_pays_is_quoted_not_held_for_an_unknown_price() {
    let agents = Agents::new();
    agents.harness.provider.with(|state| {
        state.preliminary = Some(Cost::usd(Decimal::ONE, true));
        state.quote = Some(usd("1.2"));
        state.pays_unlimited = true;
        state.unlimited_answer = (false, Some(usd("2")));
    });
    let engine = agents.engine_with(1_000, true).await;

    let first = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    agents.done(first.id).await;
    assert_eq!(agents.harness.log.count("serverEstimate"), 0);

    let second = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-2")))
        .await
        .unwrap();
    agents.done(second.id).await;
    assert_eq!(agents.harness.log.count("serverEstimate"), 1);
    assert_eq!(agents.submits(), 2);
    // It reserved the quote, then what it was charged replaced that.
    assert_eq!(
        agents.total().await,
        AgentSpendTotal {
            spent: 400,
            reserved: 0,
            unlimited_charged: true
        }
    );
}

/// The setting only covers the allowance: a job it doesn't pay for keeps every limit.
#[tokio::test]
async fn letting_agents_use_unlimited_leaves_other_jobs_within_the_budget() {
    let agents = Agents::new();
    let engine = agents.engine_with(0, true).await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    let held = agents.held(job.id).await;

    assert_eq!(held.agent.unwrap().hold, Some(AgentHold::OverDailyBudget));
    assert_eq!(agents.submits(), 0);
}

/// Higgsfield doesn't bill a generation that failed, was refused by moderation or was cancelled on its side: the
/// job's reservation stops counting once the provider says so.
#[tokio::test]
async fn a_job_that_ended_unbilled_counts_nothing() {
    let statuses = [
        RemoteStatus::Failed { message: None },
        RemoteStatus::Failed {
            message: Some("Boom".into()),
        },
        RemoteStatus::Rejected(Rejection::ExplicitContent),
        RemoteStatus::Canceled,
    ];
    for status in statuses {
        let agents = Agents::new();
        agents
            .harness
            .provider
            .with(|state| state.held_status = Some(status.clone()));
        let engine = agents.engine().await;

        let job = engine
            .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
            .await
            .unwrap();
        agents
            .harness
            .repository
            .wait(job.id, |job| job.state.is_terminal())
            .await;
        agents.settles(AgentSpendTotal::default()).await;

        assert_eq!(agents.submits(), 1, "{status:?}");
    }
}

/// One variant of two failed: the submit counts the share of the one that was made.
#[tokio::test]
async fn a_failed_variant_gives_its_share_back() {
    let agents = Agents::new();
    agents.harness.provider.with(|state| {
        state.status_batch_limit = 4;
        state
            .batch_statuses
            .insert("r1-v1".into(), RemoteStatus::Failed { message: None });
    });
    let engine = agents.engine().await;
    let mut spec = make_spec(1);
    spec.variants = 2;

    let job = engine.enqueue(spec, by_agent(origin(300, "r-1"))).await.unwrap();
    agents
        .harness
        .repository
        .wait_until(|repository| {
            repository
                .all()
                .iter()
                .filter(|other| other.batch_id == job.batch_id && other.state.is_terminal())
                .count()
                == 2
        })
        .await;

    agents.settles(total(130, 0)).await;
}

/// The person's cancel of a job still queued at Higgsfield: cancelled there, so never billed.
#[tokio::test]
async fn a_job_canceled_while_queued_counts_nothing() {
    let agents = Agents::new();
    agents
        .harness
        .provider
        .with(|state| state.held_status = Some(RemoteStatus::Queued));
    let engine = agents.engine().await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    agents
        .harness
        .repository
        .wait_state(job.id, &JobState::Queued)
        .await;
    engine.cancel(job.id).await.unwrap();

    assert_eq!(agents.total().await, AgentSpendTotal::default());
}

/// Past the deadline the generation may still finish and be charged: the reservation keeps counting.
#[tokio::test]
async fn a_job_past_the_deadline_keeps_its_charge() {
    let mut harness = Harness::new();
    harness.generation_deadline = Duration::hours(1);
    harness.provider.with(|state| {
        state.status_steps = std::iter::repeat_with(|| Err(PortError::Provider(ProviderError::Network)))
            .take(500)
            .collect();
    });
    let agents = Agents::over(harness);
    let engine = agents.engine().await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(500, "r-1")))
        .await
        .unwrap();
    let failed = agents
        .harness
        .repository
        .wait_state(job.id, &failed(FailureReason::Deadline))
        .await;

    assert!(failed.may_have_been_charged());
    assert_eq!(agents.total().await, total(130, 0));
}

/// The agent's request ID names the job: queueing it again returns the same job.
#[tokio::test]
async fn a_known_job_id_is_returned_not_queued_again() {
    let agents = Agents::new();
    let engine = agents.engine().await;
    let id = Uuid::new_v4();
    let options = EnqueueOptions {
        id: Some(id),
        ..by_agent(origin(500, "r-1"))
    };

    let first = engine.enqueue(make_spec(1), options.clone()).await.unwrap();
    let second = engine.enqueue(make_spec(1), options).await.unwrap();
    agents.done(id).await;

    assert!(first.id == id && second.id == id);
    assert_eq!(agents.submits(), 1);
}

#[tokio::test]
async fn retry_forgets_the_persons_approval() {
    let agents = Agents::new();
    agents.harness.provider.with(|state| {
        state.submit_steps = [SubmitStep::Fail(PortError::Provider(ProviderError::Validation {
            field: None,
            message: "no".into(),
        }))]
        .into();
    });
    let engine = agents.engine().await;

    let job = engine
        .enqueue(make_spec(1), by_agent(origin(10, "r-1")))
        .await
        .unwrap();
    agents.held(job.id).await;
    engine.resume(job.id).await.unwrap();
    agents
        .harness
        .repository
        .wait_state(job.id, &failed(FailureReason::Message("no".into())))
        .await;
    engine.retry(job.id).await.unwrap();

    let held = agents.held(job.id).await;
    assert!(!held.agent.unwrap().approved_by_person);
    assert_eq!(agents.submits(), 1);
}

// The ledger file (`AgentSpendFileTests`).

fn day() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_791_892_800).unwrap()
}

fn ledger_file() -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join(AgentSpendFile::FILE_NAME);
    (folder, file)
}

fn ledger_at(file: &std::path::Path) -> AgentSpendFile {
    AgentSpendFile::with_clock(file.to_path_buf(), UtcOffset::UTC, Arc::new(day))
}

fn spend_of(amount: i64, at: OffsetDateTime) -> AgentSpend {
    AgentSpend::new(Uuid::new_v4(), Uuid::new_v4(), cents(amount), at, false)
}

#[tokio::test]
async fn reserving_stops_at_the_limit_and_survives_a_restart() {
    let (_folder, file) = ledger_file();
    let ledger = ledger_at(&file);
    assert!(ledger.reserve(spend_of(60, day()), Some(100)).await.unwrap());
    assert!(!ledger.reserve(spend_of(41, day()), Some(100)).await.unwrap());
    assert!(ledger.reserve(spend_of(40, day()), Some(100)).await.unwrap());

    let reopened = ledger_at(&file);
    assert_eq!(
        reopened.total(day(), AgentMoneyUnit::UsdCents).await.unwrap(),
        total(0, 100)
    );
    assert_eq!(
        reopened.total(day(), AgentMoneyUnit::Credits).await.unwrap(),
        AgentSpendTotal::default()
    );
}

/// Many submits at once can't all take the last of the budget.
#[tokio::test]
async fn concurrent_reservations_never_pass_the_limit() {
    let (_folder, file) = ledger_file();
    let ledger = Arc::new(ledger_at(&file));
    let tries: Vec<_> = (0..20)
        .map(|_| {
            let ledger = ledger.clone();
            tokio::spawn(async move {
                ledger
                    .reserve(spend_of(10, day()), Some(55))
                    .await
                    .unwrap_or(false)
            })
        })
        .collect();
    let mut granted = 0;
    for attempt in tries {
        granted += usize::from(attempt.await.unwrap());
    }
    assert_eq!(granted, 5);
}

#[tokio::test]
async fn settle_replaces_and_release_only_gives_back_what_is_unsettled() {
    let (_folder, file) = ledger_file();
    let ledger = ledger_at(&file);
    let charged = spend_of(100, day());
    let unsent = spend_of(30, day());
    let (charged_id, unsent_id) = (charged.id, unsent.id);
    ledger.reserve(charged, None).await.unwrap();
    ledger.reserve(unsent, None).await.unwrap();

    ledger.settle(charged_id, Some(80)).await.unwrap();
    ledger.release(unsent_id).await.unwrap();
    ledger.release(charged_id).await.unwrap();

    assert_eq!(
        ledger.total(day(), AgentMoneyUnit::UsdCents).await.unwrap(),
        total(80, 0)
    );
}

/// The numbers read back stay as they were written: only `settle` and `unbill` clamp what they write.
#[tokio::test]
async fn stored_numbers_are_read_and_kept_as_written() {
    let (_folder, file) = ledger_file();
    let huge = AgentMoney::CEILING * 4;
    let big = spend_of(huge, day());
    let other = spend_of(10, day());
    let (big_id, other_id) = (big.id, other.id);
    let ledger = ledger_at(&file);
    ledger.reserve(big, None).await.unwrap();
    ledger.reserve(other, None).await.unwrap();

    let reopened = ledger_at(&file);
    assert_eq!(
        reopened.total(day(), AgentMoneyUnit::UsdCents).await.unwrap(),
        total(0, AgentMoney::CEILING)
    );
    reopened.settle(other_id, Some(12)).await.unwrap();
    let stored = |id| {
        let entries: Vec<AgentSpend> = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        entries.into_iter().find(|entry| entry.id == id).unwrap()
    };
    assert_eq!(stored(big_id).money.amount, huge);
    reopened.settle(big_id, Some(huge)).await.unwrap();
    assert_eq!(stored(big_id).money.amount, AgentMoney::CEILING);
}

/// Parts that ended unbilled stop counting their share, each once, rounded up; settling after keeps that.
#[tokio::test]
async fn unbilled_parts_stop_counting_their_share() {
    let (_folder, file) = ledger_file();
    let ledger = ledger_at(&file);
    let spend = spend_of(10, day());
    let id = spend.id;
    ledger.reserve(spend, None).await.unwrap();

    ledger.unbill(id, "a".into(), 3).await.unwrap();
    ledger.unbill(id, "a".into(), 3).await.unwrap();
    assert_eq!(
        ledger.total(day(), AgentMoneyUnit::UsdCents).await.unwrap(),
        total(0, 7)
    );
    ledger.settle(id, None).await.unwrap();
    assert_eq!(
        ledger.total(day(), AgentMoneyUnit::UsdCents).await.unwrap(),
        total(7, 0)
    );
    ledger.unbill(id, "b".into(), 3).await.unwrap();
    ledger.unbill(id, "c".into(), 3).await.unwrap();

    let reopened = ledger_at(&file);
    assert_eq!(
        reopened.total(day(), AgentMoneyUnit::UsdCents).await.unwrap(),
        AgentSpendTotal::default()
    );
    reopened.unbill(Uuid::new_v4(), "x".into(), 1).await.unwrap();
}

#[tokio::test]
async fn an_unreadable_file_fails_rather_than_counting_from_zero() {
    let (_folder, file) = ledger_file();
    std::fs::write(&file, b"not json").unwrap();
    let ledger = ledger_at(&file);

    assert!(ledger.total(day(), AgentMoneyUnit::UsdCents).await.is_err());
    assert!(ledger.reserve(spend_of(1, day()), None).await.is_err());
}

#[tokio::test]
async fn old_days_are_dropped_and_do_not_count() {
    let (_folder, file) = ledger_file();
    let ledger = ledger_at(&file);
    let last_week = day() - Duration::days(7);
    ledger.reserve(spend_of(500, last_week), None).await.unwrap();
    ledger.reserve(spend_of(5, day()), None).await.unwrap();

    assert_eq!(
        ledger.total(day(), AgentMoneyUnit::UsdCents).await.unwrap(),
        total(0, 5)
    );
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved.as_array().map(Vec::len), Some(1));
}
