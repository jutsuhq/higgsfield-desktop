//! `JobTransitionTests.swift`.

use std::collections::HashMap;

use jutsu_core::*;

fn x() -> JobState {
    JobState::Failed {
        reason: FailureReason::Message("x".into()),
    }
}

fn boom() -> FailureReason {
    FailureReason::Message("boom".into())
}

fn all_states() -> Vec<JobState> {
    use PauseReason as P;
    vec![
        JobState::Preparing,
        JobState::Uploading,
        JobState::WaitingForSlot,
        JobState::Queued,
        JobState::Processing,
        JobState::Downloading,
        JobState::Done,
        x(),
        JobState::Rejected,
        JobState::Canceled,
        JobState::Paused(P::InsufficientFunds),
        JobState::Paused(P::ConnectionNeedsAttention),
        JobState::Paused(P::PriceAboveCap),
        JobState::Paused(P::NotEnoughDiskSpace),
        JobState::Paused(P::KeptFailing),
        JobState::Paused(P::ByUser),
    ]
}

fn all_events() -> Vec<JobEvent> {
    use PauseReason as P;
    vec![
        JobEvent::Prepared,
        JobEvent::Uploaded,
        JobEvent::Submitted,
        JobEvent::HitConcurrencyLimit,
        JobEvent::Started,
        JobEvent::Completed,
        JobEvent::Downloaded,
        JobEvent::Failed { reason: boom() },
        JobEvent::Rejected,
        JobEvent::Paused(P::InsufficientFunds),
        JobEvent::Paused(P::ConnectionNeedsAttention),
        JobEvent::Paused(P::PriceAboveCap),
        JobEvent::Paused(P::NotEnoughDiskSpace),
        JobEvent::Paused(P::KeptFailing),
        JobEvent::Paused(P::ByUser),
        JobEvent::Cancel,
        JobEvent::Resume,
        JobEvent::Retry,
        JobEvent::RetryDownload,
        JobEvent::CheckAgain,
    ]
}

fn valid_transitions() -> HashMap<(JobState, JobEvent), JobState> {
    use JobEvent as E;
    use JobState as S;
    let mut table = HashMap::from([
        ((S::Preparing, E::Prepared), S::Uploading),
        ((S::Uploading, E::Uploaded), S::WaitingForSlot),
        ((S::WaitingForSlot, E::Submitted), S::Queued),
        ((S::WaitingForSlot, E::HitConcurrencyLimit), S::WaitingForSlot),
        ((S::Queued, E::Started), S::Processing),
        ((S::Queued, E::Completed), S::Downloading),
        ((S::Processing, E::Completed), S::Downloading),
        ((S::Downloading, E::Downloaded), S::Done),
        ((S::Queued, E::Rejected), S::Rejected),
        ((S::Processing, E::Rejected), S::Rejected),
        ((x(), E::Retry), S::Preparing),
        ((S::Canceled, E::Retry), S::Preparing),
        ((x(), E::RetryDownload), S::Downloading),
        ((S::Canceled, E::RetryDownload), S::Downloading),
        ((x(), E::CheckAgain), S::Queued),
    ]);
    for reason in [
        PauseReason::InsufficientFunds,
        PauseReason::ConnectionNeedsAttention,
        PauseReason::PriceAboveCap,
        PauseReason::KeptFailing,
        PauseReason::ByUser,
    ] {
        for state in [S::Preparing, S::Uploading, S::WaitingForSlot] {
            table.insert((state, E::Paused(reason)), S::Paused(reason));
        }
        table.insert((S::Paused(reason), E::Resume), S::Preparing);
        table.insert((S::Paused(reason), E::Cancel), S::Canceled);
    }
    table.insert(
        (
            S::Paused(PauseReason::ConnectionNeedsAttention),
            E::Paused(PauseReason::ByUser),
        ),
        S::Paused(PauseReason::ByUser),
    );
    // Only a paid result waits for disk space, and resuming continues its download.
    table.insert(
        (S::Downloading, E::Paused(PauseReason::NotEnoughDiskSpace)),
        S::Paused(PauseReason::NotEnoughDiskSpace),
    );
    table.insert(
        (S::Paused(PauseReason::NotEnoughDiskSpace), E::Resume),
        S::Downloading,
    );
    table.insert(
        (S::Paused(PauseReason::NotEnoughDiskSpace), E::Cancel),
        S::Canceled,
    );
    for state in [
        S::Preparing,
        S::Uploading,
        S::WaitingForSlot,
        S::Queued,
        S::Downloading,
    ] {
        table.insert((state, E::Cancel), S::Canceled);
    }
    for state in [
        S::Preparing,
        S::Uploading,
        S::WaitingForSlot,
        S::Queued,
        S::Processing,
        S::Downloading,
    ] {
        table.insert(
            (state, E::Failed { reason: boom() }),
            S::Failed { reason: boom() },
        );
    }
    table
}

#[test]
fn every_state_event_pair_matches_the_table() {
    let table = valid_transitions();
    for state in all_states() {
        for event in all_events() {
            let expected = table.get(&(state.clone(), event.clone())).cloned();
            assert_eq!(
                JobState::transition(&state, &event),
                expected,
                "{state:?} + {event:?}"
            );
        }
    }
}

#[test]
fn terminal_and_active_flags() {
    let terminal = [JobState::Done, x(), JobState::Rejected, JobState::Canceled];
    for state in all_states() {
        assert_eq!(state.is_terminal(), terminal.contains(&state), "{state:?}");
        assert_eq!(state.is_active(), !terminal.contains(&state), "{state:?}");
    }
}

#[test]
fn cancellable_except_while_processing() {
    use PauseReason as P;
    let cancellable = [
        JobState::Preparing,
        JobState::Uploading,
        JobState::WaitingForSlot,
        JobState::Queued,
        JobState::Downloading,
        JobState::Paused(P::InsufficientFunds),
        JobState::Paused(P::ConnectionNeedsAttention),
        JobState::Paused(P::PriceAboveCap),
        JobState::Paused(P::NotEnoughDiskSpace),
        JobState::Paused(P::KeptFailing),
        JobState::Paused(P::ByUser),
    ];
    for state in all_states() {
        assert_eq!(state.is_cancellable(), cancellable.contains(&state), "{state:?}");
    }
}

#[test]
fn user_pauses_only_what_is_not_submitted_or_held_by_the_user() {
    let pausable = [
        JobState::Preparing,
        JobState::Uploading,
        JobState::WaitingForSlot,
        JobState::Paused(PauseReason::ConnectionNeedsAttention),
    ];
    for state in all_states() {
        assert_eq!(
            state.is_pausable_by_user(),
            pausable.contains(&state),
            "{state:?}"
        );
    }
}
