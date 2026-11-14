use super::{FINGERPRINT_A, FINGERPRINT_B, error, round_trip};
use crate::{
    ANSWER_DEADLINE, AgentAmount, AgentBudgetUnit, AgentCommand, AgentJobAction, AgentQuote, AgentRequest,
    AgentSource, AgentVideoRequest, CreateVideoCommand, ErrorCode, HELPER_TIMEOUT, JobActionCommand,
    JobsCommand, QuoteCommand, Uuid, fingerprint, names, wait_seconds, wire,
};
use std::time::Duration;

fn create(video: AgentVideoRequest, max_cost: AgentAmount) -> CreateVideoCommand {
    CreateVideoCommand {
        request_id: "r-1".to_owned(),
        fingerprint: FINGERPRINT_B.to_owned(),
        parameters: None,
        video,
        max_cost,
        wait_seconds: 0,
    }
}

#[test]
fn every_video_command_round_trips() {
    let video = AgentVideoRequest {
        model: Some("seedance25".to_owned()),
        prompt: Some("dance".to_owned()),
        source: Some(AgentSource {
            path: Some("/tmp/a.mov".to_owned()),
            start: None,
            end: Some(5.0),
        }),
        images: vec!["/tmp/b.png".to_owned()],
        resolution: Some("720p".to_owned()),
        draft: true,
        audio: Some("mute".to_owned()),
        variants: Some(2),
        ..AgentVideoRequest::new("motion_transfer")
    };
    let quote = AgentVideoRequest {
        image_count: Some(2),
        style: Some(Uuid::new_v4()),
        ..AgentVideoRequest::new("restyle")
    };
    let commands = [
        AgentCommand::Quote(QuoteCommand { video: quote }),
        AgentCommand::CreateVideo(create(video, AgentAmount::new(250, AgentBudgetUnit::UsdCents))),
        AgentCommand::Jobs(JobsCommand {
            ids: vec![Uuid::new_v4()],
            wait_seconds: 50,
            ..JobsCommand::default()
        }),
        AgentCommand::Jobs(JobsCommand {
            limit: Some(5),
            ..JobsCommand::default()
        }),
        AgentCommand::JobAction(JobActionCommand {
            action: AgentJobAction::Cancel,
            job: Some(Uuid::new_v4()),
        }),
        AgentCommand::JobAction(JobActionCommand {
            action: AgentJobAction::PauseQueue,
            job: None,
        }),
    ];
    for command in &commands {
        assert_eq!(&round_trip(command), command);
    }
}

/// Numbers are checked before anything formats or computes with them.
#[test]
fn out_of_range_fields_are_bad_requests() {
    let base =
        format!(r#""version":1,"command":"create_video","requestId":"r","fingerprint":"{FINGERPRINT_A}""#);
    let job = Uuid::new_v4();
    let lines = [
        format!(r#"{{{base},"video":{{"mode":"restyle"}},"maxCost":{{"amount":-1,"unit":"usd_cents"}}}}"#),
        format!(
            r#"{{{base},"video":{{"mode":"restyle"}},"maxCost":{{"amount":100000000,"unit":"usd_cents"}}}}"#
        ),
        format!(r#"{{{base},"video":{{"mode":"restyle"}},"maxCost":{{"amount":1e30,"unit":"credits"}}}}"#),
        format!(
            r#"{{{base},"video":{{"mode":"restyle","source":{{"end":1e300}}}},"maxCost":{{"amount":1,"unit":"credits"}}}}"#
        ),
        format!(
            r#"{{{base},"video":{{"mode":"restyle","source":{{"start":4,"end":2}}}},"maxCost":{{"amount":1,"unit":"credits"}}}}"#
        ),
        format!(
            r#"{{{base},"video":{{"mode":"restyle","variants":99}},"maxCost":{{"amount":1,"unit":"credits"}}}}"#
        ),
        format!(r#"{{{base},"video":{{"mode":"nope"}},"maxCost":{{"amount":1,"unit":"credits"}}}}"#),
        format!(
            r#"{{"version":1,"command":"create_video","requestId":"../x","fingerprint":"{FINGERPRINT_A}","video":{{"mode":"restyle"}},"maxCost":{{"amount":1,"unit":"credits"}}}}"#
        ),
        format!(
            r#"{{{base},"video":{{"mode":"restyle"}},"maxCost":{{"amount":1,"unit":"credits"}},"waitSeconds":-1}}"#
        ),
        format!(
            r#"{{{base},"video":{{"mode":"restyle"}},"maxCost":{{"amount":1,"unit":"credits"}},"waitSeconds":51}}"#
        ),
        r#"{"version":1,"command":"jobs","waitSeconds":51}"#.to_owned(),
        r#"{"version":1,"command":"jobs","limit":0}"#.to_owned(),
        r#"{"version":1,"command":"job_action","action":"retry"}"#.to_owned(),
        format!(r#"{{"version":1,"command":"job_action","action":"resume_queue","job":"{job}"}}"#),
        format!(r#"{{"version":1,"command":"job_action","action":"delete","job":"{job}"}}"#),
    ];
    for line in &lines {
        assert_eq!(
            error(line).map(|error| error.code),
            Some(ErrorCode::BAD_REQUEST),
            "{line}"
        );
    }
}

#[test]
fn the_fingerprint_is_stable_and_changes_with_the_parameters() {
    let video = AgentVideoRequest {
        prompt: Some("a".to_owned()),
        style: Some(Uuid::new_v4()),
        ..AgentVideoRequest::new("restyle")
    };
    let first = fingerprint::of(&video).expect("fingerprint");
    assert_eq!(fingerprint::of(&video).expect("fingerprint"), first);
    assert!(names::is_valid_fingerprint(&first));
    let other = AgentVideoRequest {
        prompt: Some("b".to_owned()),
        ..video
    };
    assert_ne!(fingerprint::of(&other).expect("fingerprint"), first);
}

fn waiting_video() -> AgentVideoRequest {
    AgentVideoRequest {
        source: Some(AgentSource {
            path: Some("/tmp/a.mov".to_owned()),
            ..AgentSource::default()
        }),
        ..AgentVideoRequest::new("motion_transfer")
    }
}

fn line_of(command: &CreateVideoCommand) -> Vec<u8> {
    let mut request = AgentRequest::new(AgentCommand::CreateVideo(command.clone()));
    request.id = Some("1".to_owned());
    wire::line(&request).expect("line")
}

/// `create_video`'s wait: carried only when asked for, and cut to what is left of the answer deadline; one outside 0 to
/// 50 s is refused with the other out-of-range fields.
#[test]
fn a_wait_round_trips_and_is_left_out_when_none() {
    let waiting = CreateVideoCommand {
        wait_seconds: 30,
        ..create(waiting_video(), AgentAmount::new(40, AgentBudgetUnit::Credits))
    };
    let line = line_of(&waiting);
    let back = wire::request(&line[..line.len() - 1]).1.expect("request").command;
    assert_eq!(back, AgentCommand::CreateVideo(waiting.clone()));
    let plain = CreateVideoCommand {
        wait_seconds: 0,
        ..waiting
    };
    assert!(!String::from_utf8_lossy(&line_of(&plain)).contains("waitSeconds"));
    assert!(String::from_utf8_lossy(&line).contains("\"waitSeconds\":30"));
}

#[test]
fn a_wait_leaves_time_to_answer_within_the_deadline() {
    assert_eq!(wait_seconds(50, Duration::ZERO), 50);
    assert_eq!(wait_seconds(50, Duration::from_secs(10)), 40);
    assert_eq!(wait_seconds(20, Duration::from_secs(10)), 20);
    assert_eq!(wait_seconds(50, Duration::from_secs(60)), 0);
    let elapsed = Duration::from_secs(12);
    let waited = Duration::from_secs(u64::try_from(wait_seconds(50, elapsed)).expect("seconds"));
    assert!(waited + elapsed < ANSWER_DEADLINE);
}

#[test]
fn the_helper_waits_longer_than_the_app_takes_to_answer() {
    assert!(ANSWER_DEADLINE <= Duration::from_secs(60));
    assert!(HELPER_TIMEOUT > ANSWER_DEADLINE);
}

/// What a quote says about a job before upload: an estimate doesn't hold it, and says how its price is checked.
#[test]
fn an_estimate_within_the_budget_is_no_blocker_and_says_how_it_runs() {
    assert_eq!(AgentQuote::blocker(Some(24), Some(500), false, false), None);
    let note = AgentQuote::price_note(true, true, false).expect("note");
    assert!(note.contains("runs on its own") && note.contains("max_cost"));
    assert_eq!(AgentQuote::price_note(false, true, false), None);
    assert_eq!(AgentQuote::price_note(true, false, false), None);
    assert_eq!(AgentQuote::price_note(true, true, true), None);
}

#[test]
fn an_unknown_price_or_one_over_the_budget_still_waits_for_the_person() {
    let unknown = AgentQuote::blocker(None, Some(500), false, false);
    assert!(unknown.is_some_and(|blocker| blocker.contains("isn't known")));
    let over = AgentQuote::blocker(Some(501), Some(500), false, false);
    assert!(over.is_some_and(|blocker| blocker.contains("more than is left")));
    let unlimited = AgentQuote::blocker(Some(0), Some(0), true, false);
    assert!(unlimited.is_some_and(|blocker| blocker.contains("unlimited")));
    assert_eq!(AgentQuote::blocker(Some(0), Some(0), true, true), None);
}
