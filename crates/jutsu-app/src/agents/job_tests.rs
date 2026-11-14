//! A video job as agents see it, what they may do to one, and the waits (`AgentVideoTests.swift`, the job cases).

use std::cell::Cell;
use std::str::FromStr;
use std::time::Duration;

use jutsu_agent::AgentAmount;
use jutsu_core::{
    AgentHold, AgentMoney, AgentMoneyUnit, AgentOrigin, ClosedRange, Cost, FailureReason, OutputMedia,
    PixelSize, SourceClip, SubmitAttempt,
};
use rust_decimal::Decimal;

use super::*;
use crate::queue::fixture;

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn agent_job_in(state: JobState, hold: Option<AgentHold>, cost: Cost) -> Job {
    let mut job = fixture::JobFixture {
        cost,
        ..fixture::JobFixture::default()
    }
    .job(state);
    let mut origin = AgentOrigin::new(
        Some("claude-code".to_owned()),
        "r1",
        AgentMoney::new(250, AgentMoneyUnit::UsdCents),
    );
    origin.hold = hold;
    job.agent = Some(origin);
    job
}

fn agent(state: JobState) -> Job {
    agent_job_in(state, None, Cost::usd(dec("1.3"), false))
}

fn source(width: i64, height: i64) -> SourceClip {
    SourceClip::new(
        crate::testing::file_url("/tmp/in.mov"),
        5.0,
        PixelSize { width, height },
        ClosedRange::new(0.0, 5.0),
    )
}

#[test]
fn agents_see_stable_names_and_who_must_act() {
    let held = agent_job(&agent_job_in(
        JobState::Paused(PauseReason::AgentLimit),
        Some(AgentHold::OverDailyBudget),
        Cost::usd(dec("1.3"), false),
    ));
    assert_eq!(held.state, "paused");
    assert_eq!(held.pause_reason.as_deref(), Some("agent_limit"));
    assert_eq!(held.hold.as_deref(), Some("over_daily_budget"));
    assert!(held.needs_person);
    assert_eq!(
        held.price,
        Some(AgentAmount::new(130, jutsu_agent::AgentBudgetUnit::UsdCents))
    );
    assert!(held.is_price_exact);
    let by_user = agent_job(&agent(JobState::Paused(PauseReason::ByUser)));
    assert!(!by_user.needs_person && by_user.hold.is_none());
    assert_eq!(
        agent_job(&agent_job_in(JobState::WaitingForSlot, None, Cost::UNKNOWN_USD)).price,
        None
    );
    assert!(
        agent_job(&agent(JobState::Failed {
            reason: FailureReason::OutcomeUnknown
        }))
        .error
        .is_some()
    );
    assert_eq!(
        agent_job(&agent(JobState::Rejected)).error.as_deref(),
        Some("Rejected by content moderation")
    );
}

/// The wire job carries who asked for it and what it is, as `jobs` lists it.
#[test]
fn the_wire_job_names_its_request_mode_and_model() {
    let job = agent(JobState::Queued);
    let wire = agent_job(&job);
    assert_eq!(wire.id, wire_id(job.id));
    assert_eq!(wire.kind, "video");
    assert_eq!(wire.created_at, Some(Timestamp(job.created_at)));
    assert_eq!(wire.request_id.as_deref(), Some("r1"));
    assert_eq!(wire.client.as_deref(), Some("claude-code"));
    assert_eq!(wire.mode.as_deref(), Some(mode_name(job.spec.mode)));
    assert_eq!(wire.model.as_deref(), Some(job.spec.model.raw_value()));
    for mode in [
        Mode::MotionTransfer,
        Mode::Restyle,
        Mode::ObjectSwap,
        Mode::Edit,
        Mode::Extend,
    ] {
        assert_eq!(mode_named(mode_name(mode)), Some(mode));
    }
    assert_eq!(mode_named("dance"), None);
}

/// A raise that didn't lift the job changed nothing: the note says it stays held over its old `max_cost` and how the
/// same request can lift it later, not that it now waits for the person.
#[test]
fn a_raise_still_held_says_the_job_stays_over_its_old_max_cost() {
    for hold in [
        Some(AgentHold::OverDailyBudget),
        Some(AgentHold::LedgerUnavailable),
        None,
    ] {
        let note = AgentJobRules::still_held_note(hold);
        assert!(note.contains("stays held over its old max_cost") && note.contains("Send the request again"));
    }
}

/// A finished video points the agent to frames, against its source when it has one.
#[test]
fn a_finished_video_says_how_to_check_it_with_frames() {
    let mut done = agent(JobState::Done);
    done.spec.source = None;
    done.output_file = Some(crate::testing::file_url("/tmp/out.mp4"));
    let plain = AgentJobRules::check_hint(&done).unwrap();
    assert!(
        plain.contains(&format!("frames with item {}", wire_id(done.id)))
            && !plain.contains("compare_source")
    );
    done.spec.source = Some(source(576, 1024));
    assert!(
        AgentJobRules::check_hint(&done)
            .unwrap()
            .contains("compare_source true")
    );
    assert_eq!(AgentJobRules::check_hint(&agent(JobState::Processing)), None);
    assert_eq!(AgentJobRules::check_hint(&agent(JobState::Done)), None);
}

/// A finished video says its frame and length, and says so when the model answered in another shape than the
/// source's: a 3:4 draft from a 9:16 source, scaled to 1080×1920, stretched.
#[test]
fn a_finished_video_says_its_frame_and_when_it_is_not_the_sources() {
    let mut done = agent(JobState::Done);
    done.output_file = Some(crate::testing::file_url("/tmp/out.mp4"));
    done.spec.source = Some(source(576, 1024));
    done.output_media = Some(OutputMedia {
        duration: 5.04,
        pixel_size: PixelSize {
            width: 554,
            height: 740,
        },
    });
    let wire = agent_job(&done);
    assert_eq!(
        (wire.width, wire.height, wire.duration),
        (Some(554), Some(740), Some(5.04))
    );
    assert_eq!(
        wire.result.as_deref(),
        Some(crate::testing::native("/tmp/out.mp4").as_str())
    );
    let note = wire.frame_note.unwrap();
    assert!(note.contains("554×740 (3:4)") && note.contains("576×1024 (9:16)"));

    // The same shape at another size is the source's frame.
    done.output_media = Some(OutputMedia {
        duration: 5.0,
        pixel_size: PixelSize {
            width: 720,
            height: 1280,
        },
    });
    assert_eq!(agent_job(&done).frame_note, None);
    assert_eq!(agent_job(&done).width, Some(720));
    // Without a source there is nothing to compare; before it is done nothing is known.
    assert_eq!(
        AgentJobRules::frame_note(
            None,
            PixelSize {
                width: 554,
                height: 740
            }
        ),
        None
    );
    done.state = JobState::Processing;
    assert_eq!(
        (agent_job(&done).width, agent_job(&done).frame_note),
        (None, None)
    );
}

/// A `create_video` wait ends once the guard can read the price, the job is past the guard, or it is held.
#[test]
fn a_create_video_wait_ends_once_the_price_is_known_or_the_job_is_held() {
    let estimate = Cost::credits(dec("24"), true);
    assert!(!is_priced(&agent_job_in(
        JobState::Preparing,
        None,
        estimate.clone()
    )));
    assert!(!is_priced(&agent_job_in(
        JobState::Uploading,
        None,
        Cost::UNKNOWN_USD
    )));
    let mut quoted = agent_job_in(JobState::WaitingForSlot, None, estimate.clone());
    quoted.server_price = Some(Cost::credits(dec("24"), false));
    assert!(is_priced(&quoted));
    assert!(is_priced(&agent(JobState::Preparing)));
    assert!(is_priced(&agent_job_in(
        JobState::Paused(PauseReason::AgentLimit),
        Some(AgentHold::UnknownPrice),
        estimate.clone()
    )));
    assert!(is_priced(&agent_job_in(
        JobState::Failed {
            reason: FailureReason::GenerationFailed
        },
        None,
        estimate.clone()
    )));
    let mut sending = agent_job_in(JobState::WaitingForSlot, None, estimate);
    sending.submit_attempt = Some(SubmitAttempt { is_idempotent: true });
    assert!(is_priced(&sending));
}

#[test]
fn agents_act_only_on_their_own_jobs_and_never_retry_what_may_be_charged() {
    let own = agent(JobState::Failed {
        reason: FailureReason::GenerationFailed,
    });
    assert_eq!(AgentJobRules::retry_refusal(&own, |_| true), None);
    let mut maybe_charged = agent(JobState::Failed {
        reason: FailureReason::OutcomeUnknown,
    });
    maybe_charged.submit_attempt = Some(SubmitAttempt { is_idempotent: false });
    assert!(AgentJobRules::retry_refusal(&maybe_charged, |_| true).is_some());
    assert!(AgentJobRules::retry_refusal(&own, |_| false).is_some());
    let mut persons = own;
    persons.agent = None;
    assert!(AgentJobRules::retry_refusal(&persons, |_| true).is_some());
    assert!(AgentJobRules::cancel_refusal(&persons).is_some());
    assert!(AgentJobRules::run_next_refusal(&persons).is_some());
}

/// Pages run newest first and a cursor never repeats or drops a job, even at the same millisecond.
#[test]
fn pages_run_newest_first_without_repeats() {
    let mut jobs: Vec<Job> = (0..5).map(|_| agent(JobState::Queued)).collect();
    let same = jobs[0].created_at;
    for job in &mut jobs {
        job.created_at = same;
    }
    let mut persons = agent(JobState::Queued);
    persons.agent = None;
    jobs.push(persons);
    let (first, next) = job_page(&jobs, 2, None);
    assert_eq!(first.len(), 2);
    let (second, next) = job_page(&jobs, 2, next.as_deref());
    let (third, last) = job_page(&jobs, 2, next.as_deref());
    assert_eq!(third.len(), 1);
    assert_eq!(last, None);
    let mut seen: Vec<Uuid> = first
        .iter()
        .chain(&second)
        .chain(&third)
        .map(|job| job.id)
        .collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 5);
    assert!(AgentCursor::parse("nonsense").is_none());
}

/// A wait ends once settled, or when its time is up; it never stops anything itself.
#[tokio::test]
async fn a_wait_ends_when_settled_or_timed_out() {
    let sleeps = Cell::new(0);
    let clock = Cell::new(Duration::ZERO);
    let sleep = |step: Duration| {
        sleeps.set(sleeps.get() + 1);
        clock.set(clock.get() + step);
        std::future::ready(())
    };
    let settled_at_once = agent_wait_with(
        50,
        Duration::from_millis(500),
        || std::future::ready(true),
        || clock.get(),
        sleep,
    )
    .await;
    assert!(!settled_at_once && sleeps.get() == 0);

    let checks = Cell::new(0);
    let settled_later = agent_wait_with(
        50,
        Duration::from_millis(500),
        || {
            checks.set(checks.get() + 1);
            std::future::ready(checks.get() == 3)
        },
        || clock.get(),
        sleep,
    )
    .await;
    assert!(!settled_later && sleeps.get() == 2);

    sleeps.set(0);
    let timed_out = agent_wait_with(
        2,
        Duration::from_millis(500),
        || std::future::ready(false),
        || clock.get(),
        sleep,
    )
    .await;
    assert!(timed_out && sleeps.get() == 4);
}

/// Each 500 ms sleep takes 3 s, as in the background: the wait still ends at 50 s, not after 100 sleeps.
#[tokio::test]
async fn a_wait_ends_by_the_clock_when_sleeps_run_long() {
    let sleeps = Cell::new(0);
    let clock = Cell::new(Duration::ZERO);
    let timed_out = agent_wait_with(
        50,
        Duration::from_millis(500),
        || std::future::ready(false),
        || clock.get(),
        |_| {
            sleeps.set(sleeps.get() + 1);
            clock.set(clock.get() + Duration::from_secs(3));
            std::future::ready(())
        },
    )
    .await;
    assert!(timed_out && sleeps.get() == 17 && clock.get() <= Duration::from_secs(51));
}
