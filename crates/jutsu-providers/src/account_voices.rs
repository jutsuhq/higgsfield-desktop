//! The account's voices for Change Voice.

use async_trait::async_trait;
use higgsfield::mcp::account::Voice;
use jutsu_core::{AccountVoice, PortError, VoiceChoice, VoiceLibrary};
use serde_json::Value;

use crate::mapping::port;
use crate::mcp_provider::HiggsfieldMcpProvider;

/// The largest page `list_voices` takes.
pub(crate) const VOICE_PAGE_SIZE: i64 = 100;
/// note: stops after 500 voices; a longer list would need a search field and a paged list in the sheet.
pub(crate) const MAX_VOICE_PAGES: usize = 5;

pub(crate) fn voice(voice: &Voice) -> AccountVoice {
    AccountVoice {
        choice: VoiceChoice {
            id: voice.id.clone(),
            kind: voice.kind.clone(),
            name: voice.name.clone(),
        },
        preview_url: voice.preview_url.clone(),
    }
}

#[async_trait]
impl VoiceLibrary for HiggsfieldMcpProvider {
    /// Every page of `list_voices`, read once for this connection: a sign-out or another sign-in builds a new
    /// provider, so another account's voices never show. A failed read isn't kept.
    async fn voices(&self) -> Result<Vec<AccountVoice>, PortError> {
        if let Some(cached) = &self.state().cached_voices {
            return Ok(cached.clone());
        }
        let mut voices = Vec::new();
        let mut cursor: Option<Value> = None;
        let mut pages = 0;
        loop {
            let page = self
                .account
                .voices(cursor.as_ref(), VOICE_PAGE_SIZE)
                .await
                .map_err(port)?;
            voices.extend(page.items.iter().map(voice));
            pages += 1;
            // An empty page that still names a next one would never end.
            cursor = if page.items.is_empty() {
                None
            } else {
                page.next_cursor
            };
            if cursor.is_none() || pages >= MAX_VOICE_PAGES {
                break;
            }
        }
        self.state().cached_voices = Some(voices.clone());
        Ok(voices)
    }
}
