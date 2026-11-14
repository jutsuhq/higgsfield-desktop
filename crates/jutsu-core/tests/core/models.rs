//! `ModelsTests.swift`. Security-scoped bookmarks are macOS-only and not ported, so the bookmark parts of these tests
//! check that a stored `bookmark` key is ignored instead.

use jutsu_core::*;
use uuid::Uuid;

use crate::common::*;

fn round_trip_spec(spec: &JobSpec) -> JobSpec {
    serde_json::from_str(&serde_json::to_string(spec).unwrap()).unwrap()
}

fn built_in() -> ModelConstraintTable {
    ModelConstraintTable::built_in()
}

fn limits(model: &VideoModel, mode: Mode) -> ModeLimits {
    model.limits(mode, true, &built_in())
}

fn bare(model: VideoModel, resolution: Resolution, length: Option<f64>) -> JobSpec {
    let mut spec = JobSpec::new(Mode::MotionTransfer, model, None, resolution);
    spec.length = length;
    spec
}

#[test]
fn job_spec_round_trips_through_json() {
    let mut spec = spec_with(SpecArgs {
        mode: Mode::Restyle,
        references: 2,
        preset_id: Some(Uuid::new_v4()),
        prompt: "rain".into(),
        resolution: Resolution::P1080,
        ..SpecArgs::default()
    });
    spec.audio = AudioOption::FadeOut;
    spec.model = VideoModel::SEEDANCE25;
    assert_eq!(round_trip_spec(&spec), spec);

    let mut parameters = bare(VideoModel::SEEDANCE25, Resolution::P1080, Some(6.0));
    parameters.references = vec![LocalImage::new(file_url("/tmp/start.png"))];
    parameters.audio = AudioOption::Generated;
    parameters.bitrate = Some(SeedanceBitrate::Standard);
    parameters.character_orientation = Some(CharacterOrientation::Image);
    parameters.references_are_frames = true;
    assert_eq!(round_trip_spec(&parameters), parameters);
}

#[test]
fn job_state_round_trips_through_json() {
    for state in [
        JobState::Failed {
            reason: FailureReason::Message("boom".into()),
        },
        JobState::Paused(PauseReason::InsufficientFunds),
        JobState::Paused(PauseReason::ByUser),
        JobState::Done,
    ] {
        assert_eq!(
            serde_json::from_str::<JobState>(&serde_json::to_string(&state).unwrap()).unwrap(),
            state
        );
    }
}

#[test]
fn spec_stored_before_bookmarks_and_audio_decodes_with_defaults() {
    let json = r#"
        {"mode": "motionTransfer", "prompt": "", "resolution": "480p", "isDraft": false,
         "source": {"fileURL": "file:///tmp/source.mov", "duration": 60, "pixelSize": {"width": 1280, "height": 720},
                    "range": [0, 10], "allowsUpscale": false, "bookmark": "AQID"},
         "references": [{"fileURL": "file:///tmp/ref0.jpg", "bookmark": "BAU="}]}
        "#;
    let decoded: JobSpec = serde_json::from_str(json).unwrap();
    assert_eq!(decoded, make_spec());
    assert_eq!(decoded.audio, AudioOption::Original);
    assert_eq!(decoded.model, VideoModel::GENJUTSU);
    assert!(!decoded.uses_unlimited);
}

#[test]
fn missing_bookmark_is_left_out_of_the_json() {
    let image = LocalImage::new(file_url("/tmp/a.jpg"));
    let object = serde_json::to_value(&image).unwrap();
    assert_eq!(
        object.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["fileURL"]
    );
}

#[test]
fn cost_decodes_the_old_number_format() {
    let usd_json = r#"{"usd":{"_0":3.816,"isEstimate":false}}"#;
    assert_eq!(
        serde_json::from_str::<Cost>(usd_json).unwrap(),
        usd("3.816", false)
    );
    let credits_json = r#"{"credits":{"_0":96,"isEstimate":true}}"#;
    assert_eq!(
        serde_json::from_str::<Cost>(credits_json).unwrap(),
        credits("96", true)
    );
}

/// Whole credits stay a number, as every library saved before fractional credits has them.
#[test]
fn whole_credits_stay_a_number() {
    let object = serde_json::to_value(credits("96", true)).unwrap();
    assert_eq!(object["credits"]["_0"].as_i64(), Some(96));
}

#[test]
fn cost_stores_the_amount_as_a_string() {
    let amount = dec("0.318") * rust_decimal::Decimal::from(37);
    let object = serde_json::to_value(Cost::usd(amount, true)).unwrap();
    assert_eq!(object["usd"]["_0"].as_str(), Some("11.766"));
}

#[test]
fn cost_round_trips_exactly() {
    let costs = [
        "0.318",
        "11.766",
        "48.96",
        "0.0000001",
        "123456789.123456789",
        "-2.5",
    ]
    .into_iter()
    .map(|amount| usd(amount, false))
    .chain([credits("500", true), credits("0.5", false)]);
    for cost in costs {
        assert_eq!(
            serde_json::from_str::<Cost>(&serde_json::to_string(&cost).unwrap()).unwrap(),
            cost
        );
    }
}

#[test]
fn model_limits_without_a_source_are_genjutsus() {
    for mode in Mode::ALL {
        assert_eq!(limits(&VideoModel::GENJUTSU, *mode), mode.limits());
    }
    let kling = limits(&VideoModel::KLING_MOTION_CONTROL, Mode::MotionTransfer);
    assert_eq!(kling.reference_images, ClosedRange::new(1, 1));
    assert_eq!(kling.duration, Mode::MotionTransfer.limits().duration);
    let seedance = limits(&VideoModel::SEEDANCE25, Mode::MotionTransfer);
    assert_eq!(seedance.duration, ClosedRange::new(4.0, 30.0));
    assert_eq!(
        seedance.reference_images,
        Mode::MotionTransfer.limits().reference_images
    );
    assert_eq!(Mode::MotionTransfer.limits().duration, Validation::DURATION_RANGE);
}

#[test]
fn spec_limits_follow_its_model() {
    let mut spec = spec_with(SpecArgs {
        mode: Mode::Restyle,
        ..SpecArgs::default()
    });
    assert_eq!(spec.limits(&built_in()), Mode::Restyle.limits());
    spec.mode = Mode::MotionTransfer;
    spec.model = VideoModel::SEEDANCE25;
    assert_eq!(
        spec.limits(&built_in()),
        limits(&VideoModel::SEEDANCE25, Mode::MotionTransfer)
    );
}

#[test]
fn each_model_serves_its_modes() {
    let served = |model: &VideoModel| {
        Mode::ALL
            .iter()
            .copied()
            .filter(|mode| model.serves(*mode))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        served(&VideoModel::GENJUTSU),
        [Mode::MotionTransfer, Mode::Restyle, Mode::ObjectSwap]
    );
    assert_eq!(
        served(&VideoModel::SEEDANCE25),
        [Mode::MotionTransfer, Mode::Edit, Mode::Extend]
    );
    assert_eq!(served(&VideoModel::KLING_MOTION_CONTROL), [Mode::MotionTransfer]);
    assert!(Mode::ALL.iter().all(|mode| mode.default_model().serves(*mode)));
}

/// Edit and Extend have their own Seedance endpoints whatever the media, and Extend's length is the part it adds.
#[test]
#[allow(clippy::float_cmp, reason = "exact lengths, as the Swift test expects")]
fn edit_and_extend_run_on_their_own_endpoints() {
    let source = SourceClip::new(
        file_url("/tmp/in.mp4"),
        10.0,
        PixelSize::new(1280, 720),
        ClosedRange::new(2.0, 10.0),
    );
    let mut edit = JobSpec::new(
        Mode::Edit,
        VideoModel::SEEDANCE25,
        Some(source.clone()),
        Resolution::P720,
    );
    edit.prompt = "night".into();
    let mut extend = JobSpec::new(
        Mode::Extend,
        VideoModel::SEEDANCE25,
        Some(source),
        Resolution::P720,
    );
    extend.prompt = "she leaves".into();
    extend.length = Some(6.0);

    assert_eq!(edit.endpoint_id(), "bytedance/seedance-2.5/video-edit");
    assert_eq!(extend.endpoint_id(), "bytedance/seedance-2.5/video-extend");
    assert_eq!(edit.duration(), 8.0);
    assert_eq!(extend.duration(), 6.0);
    assert_eq!(extend.limits(&built_in()).duration, ClosedRange::new(4.0, 30.0));
}

/// Jobs stored before Edit and Extend existed decode unchanged, and the new modes round-trip.
#[test]
fn stored_modes_still_decode() {
    let decoded: Vec<Mode> = serde_json::from_str(r#"["motionTransfer","restyle","objectSwap"]"#).unwrap();
    assert_eq!(decoded, [Mode::MotionTransfer, Mode::Restyle, Mode::ObjectSwap]);
    let new: Vec<Mode> =
        serde_json::from_str(&serde_json::to_string(&[Mode::Edit, Mode::Extend]).unwrap()).unwrap();
    assert_eq!(new, [Mode::Edit, Mode::Extend]);
}

#[test]
#[allow(clippy::float_cmp, reason = "exact lengths, as the Swift test expects")]
fn a_clip_without_a_source_round_trips_and_needs_no_image() {
    let mut spec = bare(VideoModel::SEEDANCE25, Resolution::P720, Some(12.0));
    spec.prompt = "a fox in the snow".into();
    spec.aspect_ratio = Some(AspectRatio::Portrait);
    assert_eq!(round_trip_spec(&spec), spec);
    assert_eq!(spec.duration(), 12.0);
    assert_eq!(spec.limits(&built_in()).reference_images, ClosedRange::new(0, 8));
    assert_eq!(
        VideoModel::all()
            .iter()
            .filter(|model| model.makes_clips_without_source())
            .map(|model| model.raw_value().to_owned())
            .collect::<Vec<_>>(),
        [
            "seedance25",
            "kling40",
            "kling40Flash",
            "geminiOmniFlash12",
            "intone"
        ]
    );
}

#[test]
fn a_spec_is_named_by_its_source_else_its_prompt() {
    assert_eq!(make_spec().name(), "source");
    let mut spec = bare(VideoModel::SEEDANCE25, Resolution::P720, Some(5.0));
    assert_eq!(spec.name(), "Seedance 2.5");
    spec.prompt = "  A fox runs through fresh snow at dawn, slow motion, golden light\nsecond line".into();
    assert_eq!(spec.name(), "A fox runs through fresh snow at dawn, s…");
    assert_eq!(AspectRatio::Portrait.width_over_height(), Some(9.0 / 16.0));
    assert_eq!(AspectRatio::Auto.width_over_height(), None);
}

#[test]
fn a_title_names_the_spec_instead_of_its_source() {
    let mut spec = make_spec();
    spec.title = JobSpec::title_from(Some("  Knight, take 3 \n  rooftop  \n"));
    assert_eq!(spec.title.as_deref(), Some("Knight, take 3 rooftop"));
    assert_eq!(spec.name(), "Knight, take 3 rooftop");
    assert!(ExportNaming::base_name(&spec, None).starts_with("Knight, take 3 rooftop_00m00.0s"));
    assert_eq!(
        JobSpec::title_from(Some(&"a".repeat(200))).map(|title| title.chars().count()),
        Some(JobSpec::MAX_TITLE_LENGTH)
    );
    assert_eq!(JobSpec::title_from(Some(" \n ")), None);
    assert_eq!(JobSpec::title_from(None), None);
}

/// An agent's title names rows, cards and files: no control or bidi-override characters, single spaces.
#[test]
fn a_title_drops_control_and_format_characters() {
    assert_eq!(
        JobSpec::title_from(Some("Knight\t\ttake\u{0}3")).as_deref(),
        Some("Knight take 3")
    );
    assert_eq!(
        JobSpec::title_from(Some("invoice\u{202E}4pm.exe")).as_deref(),
        Some("invoice4pm.exe")
    );
    assert_eq!(
        JobSpec::title_from(Some("a\u{200B}b \u{2066}c\u{2069}\u{2028}d")).as_deref(),
        Some("ab c d")
    );
    assert_eq!(JobSpec::title_from(Some("\u{202E}\t\u{0}")), None);
    assert_eq!(
        JobSpec::title_from(Some("Family 👨\u{200D}👩\u{200D}👧")).as_deref(),
        Some("Family 👨\u{200D}👩\u{200D}👧")
    );
}

#[test]
fn a_spec_stored_before_titles_decodes_named_by_its_source() {
    let mut spec = make_spec();
    spec.title = Some("Knight".into());
    let mut object = serde_json::to_value(&spec).unwrap();
    assert_eq!(
        object
            .as_object_mut()
            .unwrap()
            .remove("title")
            .and_then(|title| title.as_str().map(str::to_owned))
            .as_deref(),
        Some("Knight")
    );

    let decoded: JobSpec = serde_json::from_value(object).unwrap();

    assert_eq!(decoded.title, None);
    assert_eq!(decoded.name(), "source");
    assert_eq!(round_trip_spec(&spec), spec);
}

#[test]
fn a_spec_stored_before_the_model_parameters_decodes_without() {
    let json = r#"
        {"mode": "motionTransfer", "model": "seedance25", "references": [], "prompt": "a fox",
         "resolution": "720p", "isDraft": false, "length": 5}
        "#;
    let spec: JobSpec = serde_json::from_str(json).unwrap();
    assert!(spec.bitrate.is_none() && spec.character_orientation.is_none());
    assert!(!spec.references_are_frames && !spec.uses_frames());
    assert_eq!(spec.endpoint_id(), "bytedance/seedance-2.5/text-to-video");
}

/// The references are the start and end frame only on Seedance without a source.
#[test]
fn frames_run_on_image_to_video() {
    let mut spec = bare(VideoModel::SEEDANCE25, Resolution::P720, Some(5.0));
    spec.references = (0..2)
        .map(|i| LocalImage::new(file_url(&format!("/tmp/frame{i}.png"))))
        .collect();
    spec.references_are_frames = true;
    assert!(spec.uses_frames());
    assert_eq!(spec.endpoint_id(), "bytedance/seedance-2.5/image-to-video");
    assert_eq!(spec.limits(&built_in()).reference_images, ClosedRange::new(1, 2));

    spec.source = Some(SourceClip::new(
        file_url("/tmp/in.mp4"),
        10.0,
        PixelSize::new(1280, 720),
        ClosedRange::new(0.0, 5.0),
    ));
    assert!(!spec.uses_frames());
    assert_eq!(spec.endpoint_id(), "bytedance/seedance-2.5/reference-to-video");
    spec.source = None;
    spec.model = VideoModel::KLING_MOTION_CONTROL;
    assert!(!spec.uses_frames());
}
