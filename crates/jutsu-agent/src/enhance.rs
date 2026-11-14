//! `enhance`: the Library's Enhance menu, on Higgsfield (paid, a job) or on this computer (free, a run of its own).

use crate::{AgentAmount, AgentError, Uuid, values::wire_enum};
use serde::{Deserialize, Serialize};

wire_enum! {
    /// The operations of the Library's Enhance menu.
    pub enum AgentEnhancement {
        Upscale = "upscale",
        Reframe = "reframe",
        RemoveBackground = "remove_background",
        Dub = "dub",
        ChangeVoice = "change_voice",
        /// New frames between a video's own, on this computer only.
        SmoothMotion = "smooth_motion",
    }
}

wire_enum! {
    /// Smooth Motion's modes: 60 fps at the clip's own length and sound, or 2× and 4× slow motion without sound.
    pub enum AgentSmoothMotion { Fps60 = "60fps", Slow2x = "slow_2x", Slow4x = "slow_4x" }
}

/// One Enhance operation on a Library result, idempotent by its request ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceCommand {
    pub request_id: String,
    pub fingerprint: String,
    /// The Library item: a video result, or an image for an upscale on this computer.
    pub item: Uuid,
    pub operation: AgentEnhancement,
    pub target: Option<String>,
    pub frame_rate: Option<i64>,
    pub preset: Option<String>,
    pub engine: Option<String>,
    pub mode: Option<String>,
    pub aspect_ratio: Option<String>,
    pub language: Option<String>,
    pub voice: Option<String>,
    /// Needed on Higgsfield; a run on this computer is free and goes without one.
    pub max_cost: Option<AgentAmount>,
}

impl EnhanceCommand {
    /// A voice ID's length, in UTF-8 bytes.
    pub const MAX_VOICE_LENGTH: usize = 128;
    /// `engine` for a run on this computer.
    pub const MAC_ENGINE: &str = "mac";
    /// The engines an upscale takes; topaz is refused later, with its own reason.
    pub const UPSCALE_ENGINES: [&str; 3] = ["standard", "topaz", Self::MAC_ENGINE];

    /// Runs on this computer rather than on Higgsfield.
    pub fn is_on_device(&self) -> bool {
        Self::runs_on_device(self.operation, self.engine.as_deref())
    }

    /// Whether `operation` with `engine` runs on this computer: the helper asks before it reads `max_cost`.
    pub fn runs_on_device(operation: AgentEnhancement, engine: Option<&str>) -> bool {
        operation == AgentEnhancement::SmoothMotion
            || (operation == AgentEnhancement::Upscale && engine == Some(Self::MAC_ENGINE))
    }

    pub fn problem(&self) -> Option<String> {
        if !crate::names::is_valid_request_id(&self.request_id) {
            return Some(crate::names::REQUEST_ID_RULE.to_owned());
        }
        if !crate::names::is_valid_fingerprint(&self.fingerprint) {
            return Some("fingerprint is 64 lowercase hex digits".to_owned());
        }
        let operation = self.operation.as_str();
        if let Some(extra) = self.extra_option() {
            let name = if self.operation == AgentEnhancement::Upscale && self.is_on_device() {
                "upscale with engine mac"
            } else {
                operation
            };
            return Some(format!("{name} takes no {extra}"));
        }
        match self.operation {
            AgentEnhancement::Reframe if self.aspect_ratio.is_none() => {
                return Some("reframe needs aspect_ratio: 9:16, 1:1 or 16:9".to_owned());
            }
            AgentEnhancement::Dub if self.language.is_none() => {
                return Some("dub needs language, an ISO 639-3 code such as fra".to_owned());
            }
            AgentEnhancement::ChangeVoice if self.voice.is_none() => {
                return Some("change_voice needs voice, a voice ID from capabilities".to_owned());
            }
            AgentEnhancement::SmoothMotion if self.mode.is_none() => {
                return Some(format!(
                    "smooth_motion needs mode: {}",
                    AgentSmoothMotion::names()
                ));
            }
            AgentEnhancement::SmoothMotion
                if self
                    .engine
                    .as_deref()
                    .is_some_and(|engine| engine != Self::MAC_ENGINE) =>
            {
                return Some(
                    "smooth_motion runs on this computer only: engine is mac or left out".to_owned(),
                );
            }
            _ => {}
        }
        if self
            .mode
            .as_deref()
            .is_some_and(|mode| AgentSmoothMotion::parse(mode).is_none())
        {
            return Some(format!("mode is one of {}", AgentSmoothMotion::names()));
        }
        for value in [
            &self.target,
            &self.preset,
            &self.engine,
            &self.aspect_ratio,
            &self.language,
        ]
        .into_iter()
        .flatten()
        {
            if value.len() > 16 {
                return Some(format!(
                    "{} is not an option of {operation}",
                    crate::names::prefix(value, 16)
                ));
            }
        }
        if self
            .voice
            .as_deref()
            .is_some_and(|voice| !(1..=Self::MAX_VOICE_LENGTH).contains(&voice.len()))
        {
            return Some("voice is a voice ID from capabilities".to_owned());
        }
        if self.frame_rate.is_some_and(|rate| ![24, 30, 60].contains(&rate)) {
            return Some("frame_rate is 24, 30 or 60".to_owned());
        }
        if self.max_cost.is_none() && !self.is_on_device() {
            return Some(format!(
                "{operation} on Higgsfield needs max_cost; only smooth_motion and upscale with engine mac run free on \
                 this computer without one"
            ));
        }
        None
    }

    /// The first option sent that the operation doesn't take.
    fn extra_option(&self) -> Option<&'static str> {
        let options = [
            ("target", self.target.is_some()),
            ("frame_rate", self.frame_rate.is_some()),
            ("preset", self.preset.is_some()),
            ("engine", self.engine.is_some()),
            ("mode", self.mode.is_some()),
            ("aspect_ratio", self.aspect_ratio.is_some()),
            ("language", self.language.is_some()),
            ("voice", self.voice.is_some()),
        ];
        let allowed: &[&str] = match self.operation {
            AgentEnhancement::Upscale if self.is_on_device() => &["target", "engine"],
            AgentEnhancement::Upscale => &["target", "frame_rate", "preset", "engine"],
            AgentEnhancement::SmoothMotion => &["mode", "engine"],
            AgentEnhancement::Reframe => &["aspect_ratio"],
            AgentEnhancement::RemoveBackground => &[],
            AgentEnhancement::Dub => &["language"],
            AgentEnhancement::ChangeVoice => &["voice"],
        };
        options
            .iter()
            .find(|(name, sent)| *sent && !allowed.contains(name))
            .map(|(name, _)| *name)
    }

    pub(crate) fn check(&self) -> Result<(), AgentError> {
        if let Some(amount) = &self.max_cost {
            amount.check()?;
        }
        self.problem()
            .map_or(Ok(()), |problem| Err(AgentError::bad_request(problem)))
    }
}
