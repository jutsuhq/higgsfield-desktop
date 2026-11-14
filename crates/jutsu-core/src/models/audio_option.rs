//! What a result sounds like.

use serde::{Deserialize, Serialize};

/// What a result sounds like. A clip made without a source has no source audio: `Mute` asks the model for no
/// sound, the others for the sound it makes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum AudioOption {
    /// The source range's audio.
    #[default]
    Original,
    /// No sound, even when the API kept some.
    Mute,
    /// The source range's audio, faded out at the end.
    FadeOut,
    /// The model makes the soundtrack even though there is a source (Seedance only); the source's audio isn't used.
    Generated,
}

impl AudioOption {
    /// Every option, in order.
    pub const ALL: [Self; 4] = [Self::Original, Self::Mute, Self::FadeOut, Self::Generated];
}
