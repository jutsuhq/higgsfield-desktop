//! `PostOperationTests.swift`.

use jutsu_core::*;
use url::Url;
use uuid::Uuid;

use crate::common::*;

fn spec() -> JobSpec {
    let source = SourceClip::new(
        file_url("/tmp/out.mp4"),
        5.0,
        PixelSize::new(640, 360),
        ClosedRange::new(0.0, 5.0),
    );
    JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::genjutsu(),
        Some(source),
        Resolution::P720,
    )
}

fn operation(action: PostAction, source_job_id: Option<&str>) -> PostOperation {
    PostOperation {
        action,
        source_job_id: source_job_id.map(str::to_owned),
    }
}

fn decode(json: &str) -> PostOperation {
    serde_json::from_str(json).unwrap()
}

#[test]
fn an_operation_survives_a_save_and_relaunch() {
    let mut spec = spec();
    let options = UpscaleOptions {
        target: UpscaleTarget::K4,
        frame_rate: UpscaleFrameRate::Fps30,
        engine: UpscaleEngine::Topaz,
        ..UpscaleOptions::default()
    };
    spec.operation = Some(operation(PostAction::Upscale(options), Some("job-1")));
    let mut job = new_job(spec, ProviderKind::Account, Cost::UNKNOWN_CREDITS);
    job.parent_id = Some(Uuid::new_v4());

    let decoded: Job = serde_json::from_str(&serde_json::to_string(&job).unwrap()).unwrap();

    assert_eq!(decoded, job);
    assert!(decoded.price_is_unknown());
}

/// Jobs, templates' specs and drafts stored before operations existed have no `operation` key.
#[test]
fn a_spec_without_operation_decodes_as_a_generation() {
    let data = serde_json::to_string(&spec()).unwrap();
    assert!(!data.contains("operation"));
    assert_eq!(serde_json::from_str::<JobSpec>(&data).unwrap().operation, None);
}

/// Operations as jobs stored them before Reframe, Dub and Change Voice existed, written out as literal JSON so a
/// change to the coding shows here rather than in a user's library.
#[test]
fn operations_stored_before_the_new_cases_still_decode() {
    let upscale = r#"{"action":{"upscale":{"_0":{"target":"4k","frameRate":0,"engine":"topaz"}}},"sourceJobID":"job-1"}"#;
    let cutout = r#"{"action":{"removeBackground":{}}}"#;

    let options = UpscaleOptions {
        target: UpscaleTarget::K4,
        frame_rate: UpscaleFrameRate::Keep,
        engine: UpscaleEngine::Topaz,
        preset: UpscalePreset::Common,
    };
    assert_eq!(
        decode(upscale),
        operation(PostAction::Upscale(options), Some("job-1"))
    );
    assert_eq!(decode(cutout), operation(PostAction::RemoveBackground, None));
}

/// An upscale stored before the preset was sent ran with the server's default, so it reads as `common`; a new
/// one keeps the preset it was queued with.
#[test]
fn an_upscale_preset_survives_a_save_and_old_jobs_read_as_common() {
    let old = r#"{"target":"1080p","frameRate":24,"engine":"standard"}"#;
    assert_eq!(
        serde_json::from_str::<UpscaleOptions>(old).unwrap().preset,
        UpscalePreset::Common
    );

    let options = UpscaleOptions {
        preset: UpscalePreset::Ugc,
        ..UpscaleOptions::default()
    };
    assert_eq!(
        serde_json::from_str::<UpscaleOptions>(&serde_json::to_string(&options).unwrap()).unwrap(),
        options
    );
    assert_eq!(UpscaleOptions::default().preset, UpscalePreset::Aigc);
}

#[test]
fn the_new_operations_keep_their_stored_shape() {
    let stored = [
        r#"{"action":{"reframe":{"_0":"9:16"}}}"#,
        r#"{"action":{"dub":{"_0":"fra"}}}"#,
        r#"{"action":{"changeVoice":{"_0":{"id":"v-1","type":"element","name":"Mine"}}}}"#,
    ];
    let expected = [
        PostAction::Reframe(ReframeAspect::Portrait),
        PostAction::Dub(DubLanguage::French),
        PostAction::ChangeVoice(VoiceChoice {
            id: "v-1".into(),
            kind: "element".into(),
            name: "Mine".into(),
        }),
    ];

    for (json, action) in stored.into_iter().zip(expected) {
        let decoded = decode(json);
        assert_eq!(decoded, operation(action, None));
        assert_eq!(decode(&serde_json::to_string(&decoded).unwrap()), decoded);
    }
}

#[test]
fn a_result_keeps_its_own_extension() {
    for (url, expected) in [
        ("https://cdn.example/a.MOV?sig=x", "mov"),
        ("https://cdn.example/a.webm", "webm"),
        ("https://cdn.example/a", "mp4"),
        ("https://cdn.example/a.mp4%2Fx", "mp4"),
        ("https://cdn.example/a.toolongext", "mp4"),
    ] {
        assert_eq!(
            OutputNamePattern::result_extension(&Url::parse(url).unwrap()),
            expected,
            "{url}"
        );
    }
}

#[test]
fn a_file_name_takes_the_results_extension() {
    let job = new_job(spec(), ProviderKind::Account, Cost::UNKNOWN_CREDITS);
    let offset = time::UtcOffset::UTC;
    assert!(
        OutputNamePattern::standard()
            .file_name(&job, None, offset, Some("mov"))
            .rsplit_once('.')
            .is_some_and(|(_, ext)| ext == "mov")
    );
    assert!(
        OutputNamePattern::standard()
            .file_name(&job, None, offset, None)
            .rsplit_once('.')
            .is_some_and(|(_, ext)| ext == "mp4")
    );
}
