//! Speech to timed words on this computer.

use async_trait::async_trait;
use url::Url;

use crate::models::{SpeechAvailability, SpeechModel, Transcript};
use crate::ports::{PortError, Progress};
use crate::rules::OnDeviceProgress;

/// Speech to timed words on-device (`Transcript`), from the sound of a video or audio file only.
#[async_trait]
pub trait SpeechTranscribing: Send + Sync {
    /// The models this computer has and the languages each takes; asks nothing that downloads.
    async fn availability(&self) -> SpeechAvailability;
    /// Transcribes the sound of `file` in `language` with `model`. A file without sound, or with only silence,
    /// fails with `TranscriptionError::NoSpeech` before any model is touched. `may_download` says whether the
    /// language's model may be downloaded first, as the person agreed to; without it a missing model fails with
    /// `TranscriptionError::ModelNeeded`.
    async fn transcribe(
        &self,
        file: &Url,
        language: &str,
        model: SpeechModel,
        may_download: bool,
        progress: Progress<OnDeviceProgress>,
    ) -> Result<Transcript, PortError>;
}
