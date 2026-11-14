//! `subtitles` on the wire: make, get and export, the sheet `show` opens, and what `capabilities` says of the models.

use super::{FINGERPRINT_A, round_trip};
use crate::{
    AgentCommand, AgentCue, AgentOnDeviceEngine, AgentRequest, AgentResponse, AgentSheet,
    AgentSubtitleCapabilities, AgentSubtitleEngine, AgentSubtitleEngineState, AgentSubtitleFormat,
    AgentSubtitles, AgentSubtitlesAction, ErrorCode, OnDevice, ShowCommand, SubtitlesCommand, ToolGroup,
    Uuid, wire,
};

fn command(action: AgentSubtitlesAction, item: Uuid) -> SubtitlesCommand {
    SubtitlesCommand {
        action,
        item,
        request_id: None,
        fingerprint: None,
        language: None,
        engine: None,
        format: None,
    }
}

fn make(item: Uuid, request_id: Option<&str>, fingerprint: Option<&str>) -> SubtitlesCommand {
    SubtitlesCommand {
        request_id: request_id.map(str::to_owned),
        fingerprint: fingerprint.map(str::to_owned),
        ..command(AgentSubtitlesAction::Make, item)
    }
}

#[test]
fn each_action_round_trips() {
    let item = Uuid::new_v4();
    let commands = [
        AgentCommand::Subtitles(SubtitlesCommand {
            language: Some("de-DE".to_owned()),
            engine: Some(AgentSubtitleEngine::Parakeet),
            ..make(item, Some("subs-1"), Some(FINGERPRINT_A))
        }),
        AgentCommand::Subtitles(make(item, Some("subs-2"), Some(FINGERPRINT_A))),
        AgentCommand::Subtitles(command(AgentSubtitlesAction::Get, item)),
        AgentCommand::Subtitles(SubtitlesCommand {
            format: Some(AgentSubtitleFormat::Vtt),
            ..command(AgentSubtitlesAction::Get, item)
        }),
        AgentCommand::Subtitles(SubtitlesCommand {
            format: Some(AgentSubtitleFormat::Srt),
            ..command(AgentSubtitlesAction::Export, item)
        }),
        AgentCommand::Show(ShowCommand::sheet(AgentSheet::Subtitles, item)),
    ];
    for command in &commands {
        assert_eq!(&round_trip(command), command);
    }
    let text =
        String::from_utf8(wire::line(&AgentRequest::new(commands[0].clone())).expect("line")).expect("text");
    assert!(text.contains(r#""command":"subtitles""#) && text.contains(r#""requestId":"subs-1""#));
    assert!(text.contains(r#""engine":"parakeet""#) && text.contains(r#""action":"make""#));
}

#[test]
fn each_action_takes_only_its_own_fields() {
    let item = Uuid::new_v4();
    let full = make(item, Some("subs-1"), Some(FINGERPRINT_A));
    let refused = [
        make(item, None, Some(FINGERPRINT_A)),
        make(item, Some("subs-1"), None),
        SubtitlesCommand {
            format: Some(AgentSubtitleFormat::Srt),
            ..full.clone()
        },
        SubtitlesCommand {
            language: Some("en US".to_owned()),
            ..full
        },
        SubtitlesCommand {
            request_id: Some("subs-1".to_owned()),
            ..command(AgentSubtitlesAction::Get, item)
        },
        SubtitlesCommand {
            language: Some("en-US".to_owned()),
            ..command(AgentSubtitlesAction::Get, item)
        },
        SubtitlesCommand {
            engine: Some(AgentSubtitleEngine::Auto),
            format: Some(AgentSubtitleFormat::Srt),
            ..command(AgentSubtitlesAction::Export, item)
        },
        command(AgentSubtitlesAction::Export, item),
    ];
    for command in &refused {
        assert!(command.problem().is_some(), "{command:?}");
    }
    let export = SubtitlesCommand {
        format: Some(AgentSubtitleFormat::Vtt),
        ..command(AgentSubtitlesAction::Export, item)
    };
    assert_eq!(export.problem(), None);
    assert_eq!(command(AgentSubtitlesAction::Get, item).problem(), None);
    assert!(SubtitlesCommand::is_language("zh-Hant-TW") && SubtitlesCommand::is_language("de"));
    assert!(!SubtitlesCommand::is_language("1de") && !SubtitlesCommand::is_language("d"));
}

#[test]
fn a_refused_command_never_reaches_the_app() {
    let line = format!(
        r#"{{"version":1,"command":"subtitles","action":"export","item":"{}"}}"#,
        Uuid::new_v4()
    );
    let error = wire::request(line.as_bytes()).1.expect_err("refused");
    assert!(error.code == ErrorCode::BAD_REQUEST && error.message.contains("format"));
}

#[test]
fn the_answer_and_capabilities_keep_their_shape() {
    let answer = AgentSubtitles {
        item: Uuid::new_v4(),
        version_id: Uuid::new_v4(),
        language: "en-US".to_owned(),
        model: "speech_transcriber".to_owned(),
        cues: vec![AgentCue {
            start: 0.2,
            end: 1.1,
            text: "Hello\nthere.".to_owned(),
        }],
        text: Some("WEBVTT\n".to_owned()),
        file: None,
    };
    let response = AgentResponse {
        id: Some("1".to_owned()),
        subtitles: Some(answer.clone()),
        ..AgentResponse::default()
    };
    assert_eq!(
        wire::decode::<AgentResponse>(&wire::line(&response).expect("line")).expect("response"),
        response
    );
    assert!(String::from_utf8_lossy(&wire::line(&answer).expect("line")).contains(r#""versionId""#));

    let engine = |available, model: &str| AgentOnDeviceEngine {
        available,
        max_long_side: None,
        max_short_side: None,
        model: model.to_owned(),
    };
    let mut on_device = OnDevice::new(
        engine(true, "none"),
        engine(true, "agreed"),
        engine(true, "agreed"),
    );
    on_device.subtitles = Some(AgentSubtitleCapabilities {
        default_engine: "auto".to_owned(),
        engines: vec![AgentSubtitleEngineState {
            engine: "parakeet".to_owned(),
            available: true,
            languages: vec!["de".to_owned(), "en".to_owned()],
            model: "not_downloaded".to_owned(),
            needs_consent: None,
        }],
    });
    assert_eq!(
        wire::decode::<OnDevice>(&wire::line(&on_device).expect("line")).expect("engines"),
        on_device
    );
    assert!(
        ToolGroup::all()
            .iter()
            .flat_map(|group| &group.tools)
            .any(|tool| tool == "subtitles")
    );
}
