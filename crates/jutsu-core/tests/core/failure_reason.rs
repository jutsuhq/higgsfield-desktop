//! `FailureReasonTests.swift`.

use std::collections::HashSet;

use jutsu_core::*;

fn round_trip(state: &JobState) -> JobState {
    serde_json::from_str(&serde_json::to_string(state).unwrap()).unwrap()
}

fn failed(reason: FailureReason) -> JobState {
    JobState::Failed { reason }
}

#[test]
fn every_reason_round_trips_in_a_failed_state() {
    let reasons = FailureReason::CODED.into_iter().chain([
        FailureReason::Message("Prompt is too long".into()),
        FailureReason::Message(String::new()),
    ]);
    for reason in reasons {
        assert_eq!(round_trip(&failed(reason.clone())), failed(reason));
    }
}

#[test]
fn each_reason_of_the_app_has_its_own_code() {
    let encoded: Vec<String> = FailureReason::CODED
        .iter()
        .map(|reason| serde_json::to_string(reason).unwrap())
        .collect();
    assert_eq!(
        encoded.iter().collect::<HashSet<_>>().len(),
        FailureReason::CODED.len()
    );
    assert!(encoded.iter().all(|text| text.starts_with(r#"{"code":"#)));
}

/// Server text is stored as it came, so a message that reads like a code stays a message.
#[test]
fn a_message_is_stored_as_plain_text() {
    let state = failed(FailureReason::Message("deadline".into()));
    assert_eq!(
        serde_json::to_string(&state).unwrap(),
        r#"{"failed":{"reason":"deadline"}}"#
    );
    assert_eq!(round_trip(&state), state);
}

/// The shape of a failed job stored before the reasons had cases.
#[test]
fn a_reason_stored_as_text_reads_as_a_message() {
    let json = r#"{"failed":{"reason":"Higgsfield didn't finish this job in time."}}"#;
    assert_eq!(
        serde_json::from_str::<JobState>(json).unwrap(),
        failed(FailureReason::Message(
            "Higgsfield didn't finish this job in time.".into()
        ))
    );
}

/// A newer version's reason still lets the job load.
#[test]
fn an_unknown_code_reads_as_an_internal_error() {
    let json = r#"{"failed":{"reason":{"code":"somethingNew"}}}"#;
    assert_eq!(
        serde_json::from_str::<JobState>(json).unwrap(),
        failed(FailureReason::InternalError)
    );
}
