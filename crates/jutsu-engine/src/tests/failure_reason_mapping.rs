//! Port of `FailureReasonMappingTests`: the reason a job fails with for an error no step handles
//! (`StepError::failure_reason`).

use std::io;

use jutsu_core::{FailureReason, PortError};

use crate::EngineError;
use crate::error::StepError;

#[test]
fn a_full_disk_before_submit_is_its_own_reason() {
    let error = StepError::from(io::Error::from(io::ErrorKind::StorageFull));
    assert_eq!(error.failure_reason(), FailureReason::NotEnoughDiskSpace);
}

#[test]
fn an_error_with_a_description_fails_with_it() {
    let description = EngineError::NotConnected.to_string();
    assert_ne!(description, "");
    assert_eq!(
        StepError::from(EngineError::NotConnected).failure_reason(),
        FailureReason::Message(description)
    );
    let FailureReason::Message(text) =
        StepError::from(io::Error::from(io::ErrorKind::PermissionDenied)).failure_reason()
    else {
        panic!("A file system error should fail with its description");
    };
    assert_ne!(text, "");
}

/// Its type name and payload mean nothing to the user.
#[test]
fn an_error_without_a_description_is_an_internal_error() {
    let undescribed = StepError::from(PortError::Other(String::new()));
    assert_eq!(undescribed.failure_reason(), FailureReason::InternalError);
}
