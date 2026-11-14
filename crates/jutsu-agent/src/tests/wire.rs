use super::{FINGERPRINT_A, error, round_trip};
use crate::{
    AgentBudgetUnit, AgentCommand, AgentJob, AgentLineBuffer, AgentPage, AgentRequest, Budget,
    CapabilitiesCommand, ErrorCode, MAX_LINE_LENGTH, ShowCommand, UploadCommand, UploadRequest, Uuid,
    is_older, json, wire,
};

#[test]
fn a_request_is_one_flat_line_and_reads_back() {
    let mut sent = AgentRequest::new(AgentCommand::Show(ShowCommand::page(AgentPage::Queue)));
    sent.id = Some("7".to_owned());
    sent.client = Some("Claude Code".to_owned());
    let line = wire::line(&sent).expect("line");
    assert_eq!(line.last(), Some(&b'\n'));
    assert!(!line[..line.len() - 1].contains(&b'\n'));
    let text = String::from_utf8(line).expect("text");
    assert_eq!(
        text,
        "{\"client\":\"Claude Code\",\"command\":\"show\",\"id\":\"7\",\"page\":\"queue\",\"version\":1}\n"
    );
    assert_eq!(wire::request(text.as_bytes()).1.expect("request"), sent);
}

#[test]
fn every_command_round_trips() {
    let commands = [
        AgentCommand::Capabilities(CapabilitiesCommand::default()),
        AgentCommand::Capabilities(CapabilitiesCommand {
            models: true,
            model: Some("seedance25".to_owned()),
            ..CapabilitiesCommand::default()
        }),
        AgentCommand::Show(ShowCommand::page(AgentPage::Settings)),
        AgentCommand::Show(ShowCommand::job(Uuid::new_v4())),
        AgentCommand::Upload(UploadCommand {
            name: "clip.mov".to_owned(),
            size: 12,
            request: Some(UploadRequest {
                command: "create_video".to_owned(),
                request_id: "r-1".to_owned(),
                fingerprint: FINGERPRINT_A.to_owned(),
                parameters: None,
            }),
        }),
    ];
    for command in &commands {
        assert_eq!(&round_trip(command), command);
    }
}

#[test]
fn the_version_is_checked_before_the_command() {
    let later = r#"{"version":2,"id":"a","command":"later"}"#;
    assert_eq!(
        error(later).map(|error| error.code),
        Some(ErrorCode::UNSUPPORTED_VERSION)
    );
    assert_eq!(wire::request(later.as_bytes()).0.as_deref(), Some("a"));
    assert_eq!(
        error(r#"{"id":"a","command":"show"}"#).map(|error| error.code),
        Some(ErrorCode::BAD_REQUEST)
    );
}

#[test]
fn malformed_lines_are_bad_requests_not_crashes() {
    let lines = [
        "",
        "not json",
        "[]",
        r#"{"version":"1"}"#,
        r#"{"version":1}"#,
        r#"{"version":1,"command":7}"#,
        r#"{"version":1,"id":3,"command":"show","page":"queue"}"#,
        r#"{"version":1,"command":"show"}"#,
        r#"{"version":1,"command":"show","page":"queue","job":"x"}"#,
        r#"{"version":1,"command":"show","page":"jobs"}"#,
        r#"{"version":1,"command":"show","job":"not-a-uuid"}"#,
        r#"{"version":1,"command":"capabilities","models":"yes"}"#,
        r#"{"version":1,"command":"capabilities","model":"../etc"}"#,
        r#"{"version":1,"command":"upload","name":".hidden","size":1}"#,
        r#"{"version":1,"command":"upload","name":"a/b","size":1}"#,
        r#"{"version":1,"command":"upload","name":"a","size":-1}"#,
        r#"{"version":1,"command":"upload","name":"a","size":1e30}"#,
        r#"{"version":1,"command":"upload","name":"a","size":99999999999999}"#,
        r#"{"version":1,"command":"upload","name":"a","size":1,"request":{"command":"x","requestId":".r","fingerprint":""}}"#,
    ];
    for line in lines {
        assert_eq!(
            error(line).map(|error| error.code),
            Some(ErrorCode::BAD_REQUEST),
            "{line}"
        );
    }
    let unknown = error(r#"{"version":1,"command":"delete_everything"}"#);
    assert_eq!(unknown.map(|error| error.code), Some(ErrorCode::UNKNOWN_COMMAND));
}

/// The refusals name the field the way the macOS app names it.
#[test]
fn a_refusal_names_the_field() {
    let message = |line: &str| error(line).map(|error| error.message).unwrap_or_default();
    assert_eq!(message(r#"{"version":1}"#), "Malformed request: no command.");
    assert_eq!(
        message(r#"{"version":1,"command":"show","page":"jobs"}"#),
        "Malformed request: bad page."
    );
    assert_eq!(
        message(r#"{"version":1,"command":"upload","size":1}"#),
        "Malformed request: no name."
    );
    assert_eq!(
        message(r#"{"version":1,"command":"quote","video":{"mode":"edit","source":{"start":"x"}}}"#),
        "Malformed request: bad video.source.start."
    );
    assert_eq!(
        message(r#"{"version":1,"command":"jobs","ids":["x"]}"#),
        "Malformed request: bad ids.Index 0."
    );
}

#[test]
fn the_line_buffer_splits_lines_and_hands_out_the_bytes_after_them() {
    let mut buffer = AgentLineBuffer::new();
    buffer.push(b"one\ntw");
    assert_eq!(buffer.next_line().as_deref(), Some(&b"one"[..]));
    assert_eq!(buffer.next_line(), None);
    buffer.push(b"o\nRAW");
    assert_eq!(buffer.next_line().as_deref(), Some(&b"two"[..]));
    assert_eq!(buffer.take(2), b"RA");
    assert_eq!(buffer.take(10), b"W");
    assert_eq!(buffer.take(10).len(), 0);
}

#[test]
fn a_line_longer_than_any_request_is_overlong() {
    let mut buffer = AgentLineBuffer::new();
    buffer.push(&vec![b'x'; MAX_LINE_LENGTH]);
    assert!(!buffer.is_overlong());
    buffer.push(b"x");
    assert!(buffer.is_overlong());
}

/// Versions compare as numbers; a development build or an unknown version is never called older.
#[test]
fn a_helper_is_older_only_by_its_release_number() {
    assert!(is_older("0.2.1", "0.2.2"));
    assert!(is_older("0.2.9", "0.2.10"));
    assert!(!is_older("0.2.10", "0.2.9"));
    assert!(!is_older("0.2.2", "0.2.2"));
    assert!(!is_older("development", "0.2.2"));
    assert!(!is_older("0.2.1", "unknown"));
}

/// The person's cap goes with the budget when there is one, and is left out of the answer when there isn't.
#[test]
fn the_budget_carries_the_persons_cap() {
    let capped = Budget {
        unit: AgentBudgetUnit::Credits,
        daily_limit: 500,
        spent_today: 0,
        reserved_today: 0,
        remaining: 500,
        unlimited_allowed: false,
        daily_cap: Some(300),
        cap_taken_today: Some(290),
    };
    let text = json::write(&json::to_value(&capped).expect("value"));
    assert!(text.contains("\"dailyCap\":300") && text.contains("\"capTakenToday\":290"));
    assert_eq!(serde_json::from_str::<Budget>(&text).expect("budget"), capped);
    let open = Budget {
        daily_cap: None,
        cap_taken_today: None,
        ..capped
    };
    assert!(!json::write(&json::to_value(&open).expect("value")).contains("dailyCap"));
}

/// A `create_video` wait reads the job again: a raise still held keeps the note saying why while the job waits for
/// the same reason, and drops it once the job moved on or got a note of its own.
#[test]
fn a_job_read_again_keeps_its_note_while_it_waits_the_same() {
    let id = Uuid::new_v4();
    let mut held = AgentJob::new(id, "paused", None);
    held.pause_reason = Some("agent_limit".to_owned());
    held.hold = Some("over_daily_budget".to_owned());
    held.needs_person = true;
    held.next = Some("Still held.".to_owned());
    let mut again = held.clone();
    again.next = None;
    assert_eq!(again.keeping_next(&held).next.as_deref(), Some("Still held."));
    let mut other_hold = again.clone();
    other_hold.hold = Some("over_max_cost".to_owned());
    assert_eq!(other_hold.keeping_next(&held).next, None);
    let running = AgentJob::new(id, "preparing", None);
    assert_eq!(running.keeping_next(&held).next, None);
    let mut done = AgentJob::new(id, "done", None);
    done.next = Some("Check it with frames.".to_owned());
    assert_eq!(
        done.keeping_next(&held).next.as_deref(),
        Some("Check it with frames.")
    );
}
