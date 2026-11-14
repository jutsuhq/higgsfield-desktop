//! The one error type every command returns, serialized as `{ kind, message }`.

use jutsu_app::MediaServerError;
use jutsu_app::preferences::PreferencesError;
use serde::{Serialize, Serializer};

/// An error a command returns to the React app.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Something failed inside the app; the message says what.
    #[error("{0}")]
    Internal(String),
    /// What the command names doesn't exist.
    #[error("{0}")]
    NotFound(String),
    /// What the command names exists but is off limits, such as a file outside the folders the app shows.
    #[error("{0}")]
    Forbidden(String),
}

/// What kind of error it is, so the UI can choose how to show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AppErrorKind {
    Internal,
    NotFound,
    Forbidden,
}

/// The wire shape of [`AppError`].
#[derive(Serialize, specta::Type)]
#[serde(rename = "AppError")]
struct AppErrorPayload {
    kind: AppErrorKind,
    message: String,
}

impl AppError {
    #[must_use]
    pub fn kind(&self) -> AppErrorKind {
        match self {
            Self::Internal(_) => AppErrorKind::Internal,
            Self::NotFound(_) => AppErrorKind::NotFound,
            Self::Forbidden(_) => AppErrorKind::Forbidden,
        }
    }
}

impl From<MediaServerError> for AppError {
    fn from(error: MediaServerError) -> Self {
        match error {
            MediaServerError::OutsideRoots => Self::Forbidden(error.to_string()),
            MediaServerError::NotFound(_) => Self::NotFound(error.to_string()),
        }
    }
}

impl From<PreferencesError> for AppError {
    fn from(error: PreferencesError) -> Self {
        Self::Internal(error.to_string())
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        AppErrorPayload {
            kind: self.kind(),
            message: self.to_string(),
        }
        .serialize(serializer)
    }
}

impl specta::Type for AppError {
    fn definition(types: &mut specta::Types) -> specta::datatype::DataType {
        AppErrorPayload::definition(types)
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;
    use jutsu_app::MediaServerError;

    #[test]
    fn serializes_as_kind_and_message() {
        let json = serde_json::to_value(AppError::Internal("disk full".to_owned())).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "internal", "message": "disk full" })
        );
    }

    #[test]
    fn a_file_outside_the_media_folders_is_forbidden() {
        let json = serde_json::to_value(AppError::from(MediaServerError::OutsideRoots)).unwrap();
        assert_eq!(json["kind"], "forbidden");
    }
}
