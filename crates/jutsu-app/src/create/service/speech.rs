//! Speech on a model that voices a recording (`SpeechSection.swift`, `CreateView.importSpeech`): one audio file,
//! picked or dropped, with its length read from the file.

use std::path::PathBuf;
use std::sync::Arc;

use jutsu_core::LocalAudio;

use super::{AUDIO_EXTENSIONS, CreateError, CreateService, file_url};
use crate::create::form::Notice;

/// Whether `path` names a recording by its extension.
fn is_audio(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| AUDIO_EXTENSIONS.contains(&extension.to_lowercase().as_str()))
}

impl CreateService {
    /// The first recording of picked or dropped files becomes the speech; answers why it can't be read.
    pub async fn add_speech(self: &Arc<Self>, paths: &[PathBuf]) -> Result<Option<Notice>, CreateError> {
        let Some(path) = paths.iter().find(|path| is_audio(path)) else {
            return Ok(None);
        };
        let Some(duration) = self.deps.media.audio_duration(path).await else {
            return Ok(Some(Notice {
                title: "Can't read this audio file".to_owned(),
                detail: "Use an MP3, WAV or M4A recording.".to_owned(),
            }));
        };
        let speech = LocalAudio {
            file_url: file_url(path)?,
            duration,
        };
        self.locked().form.set_speech(Some(speech));
        self.edited();
        Ok(None)
    }
}
