//! The voices Change Voice takes.

use async_trait::async_trait;
use url::Url;

use crate::models::VoiceChoice;
use crate::ports::PortError;

/// A voice a result's speaker can be changed to (Change Voice).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountVoice {
    /// The voice.
    pub choice: VoiceChoice,
    /// A short sample to listen to; `None` when the provider gives none.
    pub preview_url: Option<Url>,
}

impl AccountVoice {
    /// "type/id".
    pub fn id(&self) -> String {
        format!("{}/{}", self.choice.kind, self.choice.id)
    }
}

/// A provider whose account lists the voices Change Voice takes: built-in ones and the account's own.
#[async_trait]
pub trait VoiceLibrary: Send + Sync {
    /// Every voice, in the provider's order. Read once per connection; later calls answer from that read.
    async fn voices(&self) -> Result<Vec<AccountVoice>, PortError>;
}
