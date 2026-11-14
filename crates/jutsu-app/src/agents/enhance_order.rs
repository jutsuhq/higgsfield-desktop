//! `enhance` on Higgsfield (`AgentEnhanceOrder.swift`): an agent's fields as the Enhance sheet's options, and whether
//! the sheet would queue them for this result. The job is then queued like the sheet's (`operation_spec`), with the
//! agent's origin, so the spending guard checks it before its submit.

use jutsu_core::{
    DubLanguage, PostAction, PostActionKind, ReframeAspect, UpscaleEngine, UpscaleFrameRate, UpscaleOptions,
    UpscalePreset, UpscaleTarget, VoiceChoice,
};

use crate::library::enhance::EnhanceAvailability;
use crate::library::item::LibraryItem;

/// Why an agent's `enhance` isn't queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefusalKind {
    /// Fields the operation doesn't take.
    Invalid,
    /// Topaz: Higgsfield prices it only once it has run, so no guard could check it first.
    PricedAfterRun,
    /// The result or the connection can't run it now.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentEnhanceRefusal {
    pub kind: RefusalKind,
    pub message: String,
}

fn invalid(message: &str) -> AgentEnhanceRefusal {
    AgentEnhanceRefusal {
        kind: RefusalKind::Invalid,
        message: message.to_owned(),
    }
}

/// The fields of an agent's `enhance` on Higgsfield.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentEnhanceOrder {
    pub kind: PostActionKind,
    pub target: Option<String>,
    pub frame_rate: Option<i64>,
    pub preset: Option<String>,
    pub engine: Option<String>,
    pub aspect_ratio: Option<String>,
    pub language: Option<String>,
    pub voice: Option<String>,
}

impl AgentEnhanceOrder {
    pub fn new(kind: PostActionKind) -> Self {
        Self {
            kind,
            target: None,
            frame_rate: None,
            preset: None,
            engine: None,
            aspect_ratio: None,
            language: None,
            voice: None,
        }
    }

    /// The operation the fields name, with the sheet's defaults for what is left out.
    /// - `voices`: the account's voices, for Change Voice; `None` when they can't be read.
    pub fn action(&self, voices: Option<&[VoiceChoice]>) -> Result<PostAction, AgentEnhanceRefusal> {
        match self.kind {
            PostActionKind::Upscale => self.upscale(),
            PostActionKind::Reframe => self
                .aspect_ratio
                .as_deref()
                .and_then(ReframeAspect::from_raw)
                .map(PostAction::Reframe)
                .ok_or_else(|| invalid("aspect_ratio is one of 9:16, 1:1, 16:9")),
            PostActionKind::RemoveBackground => Ok(PostAction::RemoveBackground),
            PostActionKind::Dub => self
                .language
                .as_deref()
                .and_then(DubLanguage::from_raw)
                .map(PostAction::Dub)
                .ok_or_else(|| {
                    let codes: Vec<&str> = DubLanguage::ALL
                        .iter()
                        .map(|language| language.raw_value())
                        .collect();
                    invalid(&format!("language is one of {}", codes.join(", ")))
                }),
            PostActionKind::ChangeVoice => {
                let Some(voices) = voices else {
                    return Err(AgentEnhanceRefusal {
                        kind: RefusalKind::Unavailable,
                        message: "The account's voices can't be read right now.".to_owned(),
                    });
                };
                voices
                    .iter()
                    .find(|choice| Some(choice.id.as_str()) == self.voice.as_deref())
                    .cloned()
                    .map(PostAction::ChangeVoice)
                    .ok_or_else(|| invalid("voice is a voice ID from capabilities with voices true"))
            }
        }
    }

    fn upscale(&self) -> Result<PostAction, AgentEnhanceRefusal> {
        let engine = match self.engine.as_deref() {
            None => UpscaleEngine::Standard,
            Some(raw) => UpscaleEngine::from_raw(raw).ok_or_else(|| invalid("engine is standard"))?,
        };
        if engine == UpscaleEngine::Topaz {
            return Err(AgentEnhanceRefusal {
                kind: RefusalKind::PricedAfterRun,
                message: "Topaz is priced only after it runs, so an agent can't queue it within max_cost."
                    .to_owned(),
            });
        }
        let target = match self.target.as_deref() {
            None => Some(UpscaleTarget::P1080),
            Some(raw) => UpscaleTarget::from_raw(raw),
        }
        .filter(|target| engine.targets().contains(target))
        .ok_or_else(|| invalid("target is one of 1080p, 2k, 4k"))?;
        let frame_rate = match self.frame_rate {
            None => Some(UpscaleFrameRate::Fps24),
            Some(raw) => UpscaleFrameRate::try_from(raw).ok(),
        }
        .filter(|rate| UpscaleFrameRate::OFFERED.contains(rate))
        .ok_or_else(|| invalid("frame_rate is 24, 30 or 60"))?;
        let preset = match self.preset.as_deref() {
            None => UpscalePreset::Aigc,
            Some(raw) => UpscalePreset::from_raw(raw).ok_or_else(|| {
                let names: Vec<&str> = UpscalePreset::ALL
                    .iter()
                    .map(|preset| preset.raw_value())
                    .collect();
                invalid(&format!("preset is one of {}", names.join(", ")))
            })?,
        };
        Ok(PostAction::Upscale(UpscaleOptions {
            target,
            frame_rate,
            engine,
            preset,
        }))
    }

    /// Why the sheet wouldn't queue `action` on `item`; `None` when it would.
    /// - `duration`: the result's length, as probed or stored.
    pub fn refusal(
        action: &PostAction,
        _item: &LibraryItem,
        availability: &EnhanceAvailability,
        duration: f64,
    ) -> Option<AgentEnhanceRefusal> {
        match availability {
            EnhanceAvailability::Hidden => {
                return Some(AgentEnhanceRefusal {
                    kind: RefusalKind::Unavailable,
                    message: "Enhance runs on a downloaded video result with a Higgsfield account connected."
                        .to_owned(),
                });
            }
            EnhanceAvailability::Disabled(reason) => {
                return Some(AgentEnhanceRefusal {
                    kind: RefusalKind::Unavailable,
                    message: (*reason).to_owned(),
                });
            }
            EnhanceAvailability::Available => {}
        }
        if matches!(action, PostAction::Reframe(_)) && duration > ReframeAspect::MAX_SOURCE_LENGTH {
            return Some(AgentEnhanceRefusal {
                kind: RefusalKind::Unavailable,
                message: "Reframe takes results up to a minute long.".to_owned(),
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    //! `AgentEnhanceOrderTests`.

    use super::*;
    use crate::library::fixture::item;

    fn order(kind: PostActionKind, change: impl FnOnce(&mut AgentEnhanceOrder)) -> AgentEnhanceOrder {
        let mut order = AgentEnhanceOrder::new(kind);
        change(&mut order);
        order
    }

    #[test]
    fn left_out_options_are_the_sheets_defaults() {
        let upscale = AgentEnhanceOrder::new(PostActionKind::Upscale)
            .action(None)
            .unwrap();
        assert_eq!(
            upscale,
            PostAction::Upscale(UpscaleOptions {
                target: UpscaleTarget::P1080,
                frame_rate: UpscaleFrameRate::Fps24,
                engine: UpscaleEngine::Standard,
                ..UpscaleOptions::default()
            })
        );
        let reframe = order(PostActionKind::Reframe, |o| {
            o.aspect_ratio = Some("9:16".to_owned());
        });
        assert_eq!(
            reframe.action(None).unwrap(),
            PostAction::Reframe(ReframeAspect::Portrait)
        );
        let dub = order(PostActionKind::Dub, |o| o.language = Some("fra".to_owned()));
        assert_eq!(dub.action(None).unwrap(), PostAction::Dub(DubLanguage::French));
    }

    #[test]
    fn an_upscale_is_tuned_for_ai_video_unless_the_preset_is_named() {
        assert_eq!(
            AgentEnhanceOrder::new(PostActionKind::Upscale)
                .action(None)
                .unwrap(),
            PostAction::Upscale(UpscaleOptions::default())
        );
        let ugc = order(PostActionKind::Upscale, |o| o.preset = Some("ugc".to_owned()));
        assert_eq!(
            ugc.action(None).unwrap(),
            PostAction::Upscale(UpscaleOptions {
                preset: UpscalePreset::Ugc,
                ..UpscaleOptions::default()
            })
        );
        let anime = order(PostActionKind::Upscale, |o| o.preset = Some("anime".to_owned()));
        assert_eq!(anime.action(None).unwrap_err().kind, RefusalKind::Invalid);
    }

    #[test]
    fn topaz_is_refused_for_its_price_comes_after_the_run() {
        let topaz = order(PostActionKind::Upscale, |o| o.engine = Some("topaz".to_owned()));
        assert_eq!(topaz.action(None).unwrap_err().kind, RefusalKind::PricedAfterRun);
    }

    #[test]
    fn a_voice_must_be_the_accounts_own() {
        let voice = VoiceChoice {
            id: "v1".to_owned(),
            kind: "preset".to_owned(),
            name: "Ava".to_owned(),
        };
        let voices = [voice.clone()];
        let known = order(PostActionKind::ChangeVoice, |o| o.voice = Some("v1".to_owned()));
        assert_eq!(
            known.action(Some(&voices)).unwrap(),
            PostAction::ChangeVoice(voice)
        );
        let unknown = order(PostActionKind::ChangeVoice, |o| o.voice = Some("v2".to_owned()));
        assert_eq!(
            unknown.action(Some(&voices)).unwrap_err().kind,
            RefusalKind::Invalid
        );
    }

    #[test]
    fn the_sheets_own_availability_guards_it() {
        let video = item();
        let hidden = AgentEnhanceOrder::refusal(
            &PostAction::RemoveBackground,
            &video,
            &EnhanceAvailability::Hidden,
            5.0,
        );
        assert_eq!(hidden.map(|refusal| refusal.kind), Some(RefusalKind::Unavailable));
        let waiting = AgentEnhanceOrder::refusal(
            &PostAction::RemoveBackground,
            &video,
            &EnhanceAvailability::Disabled("Waiting"),
            5.0,
        );
        assert_eq!(waiting.map(|refusal| refusal.message).as_deref(), Some("Waiting"));
        let square = PostAction::Reframe(ReframeAspect::Square);
        assert!(AgentEnhanceOrder::refusal(&square, &video, &EnhanceAvailability::Available, 61.0).is_some());
        assert!(AgentEnhanceOrder::refusal(&square, &video, &EnhanceAvailability::Available, 10.0).is_none());
    }
}
