//! `VideoModelCatalogTests.swift`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use jutsu_core::resources::MODELS_JSON;
use jutsu_core::*;

use crate::common::*;

fn model(id: &str) -> VideoModel {
    VideoModel::from_raw(id).unwrap()
}

fn built_in_limits(model: &VideoModel, mode: Mode) -> ModeLimits {
    model.limits(mode, true, &ModelConstraintTable::built_in())
}

/// Every entry of the shipped file is one this version runs; a typo would otherwise drop a model silently.
#[test]
fn the_bundled_catalog_reads_every_entry() {
    let catalog = VideoModelCatalog::from_json(MODELS_JSON).unwrap();
    assert_eq!(catalog.descriptors().len(), catalog.entry_count());
    assert_eq!(&catalog, VideoModelCatalog::bundled());
    let ids: HashSet<_> = catalog
        .descriptors()
        .iter()
        .map(|descriptor| &descriptor.id)
        .collect();
    assert_eq!(ids.len(), catalog.descriptors().len());
}

/// The three models stored jobs, templates and settings name keep their IDs, order and behaviour.
#[test]
fn the_first_models_keep_their_ids_and_order() {
    let first = [
        VideoModel::GENJUTSU,
        VideoModel::SEEDANCE25,
        VideoModel::KLING_MOTION_CONTROL,
    ];
    let stored: Vec<VideoModel> = VideoModel::all()
        .into_iter()
        .filter(|model| first.contains(model))
        .collect();
    assert_eq!(stored, first);
    assert_eq!(VideoModel::all().first(), Some(&VideoModel::GENJUTSU));
    assert_eq!(
        stored.iter().map(VideoModel::raw_value).collect::<Vec<_>>(),
        ["genjutsu", "seedance25", "klingMotionControl"]
    );
    assert_eq!(
        stored.iter().map(VideoModel::title).collect::<Vec<_>>(),
        ["Genjutsu", "Seedance 2.5", "Kling 3.0"]
    );
    assert_eq!(
        stored.iter().map(VideoModel::family).collect::<Vec<_>>(),
        ["Genjutsu", "Seedance", "Kling"]
    );
    assert_eq!(VideoModel::GENJUTSU.detail(), "Higgsfield's own");
    assert!(VideoModel::GENJUTSU.offers_draft() && VideoModel::SEEDANCE25.offers_draft());
    assert!(!VideoModel::KLING_MOTION_CONTROL.offers_draft());
    assert!(
        VideoModel::SEEDANCE25.declares(VideoControl::Frames)
            && VideoModel::SEEDANCE25.declares(VideoControl::Bitrate)
    );
    assert!(VideoModel::KLING_MOTION_CONTROL.declares(VideoControl::Quality));
    assert!(VideoModel::KLING_MOTION_CONTROL.declares(VideoControl::CharacterOrientation));
    assert!(!VideoModel::GENJUTSU.declares(VideoControl::VideoBackground));
}

/// The endpoint is the first key that applies: the mode, frames, the quality, then media or none.
#[test]
fn endpoints_come_from_the_catalog() {
    let genjutsu: Vec<String> = Mode::ALL
        .iter()
        .filter(|mode| VideoModel::GENJUTSU.serves(**mode))
        .map(|mode| VideoModel::GENJUTSU.endpoint_id(*mode, Resolution::P720, true, false))
        .collect();
    assert_eq!(
        genjutsu,
        [
            "higgsfield/genjutsu/motion-transfer/v1.0",
            "higgsfield/genjutsu/restyle/v1.0",
            "higgsfield/genjutsu/object-swap/v1.0"
        ]
    );
    let seedance = VideoModel::SEEDANCE25;
    assert_eq!(
        seedance.endpoint_id(Mode::Extend, Resolution::P720, true, true),
        "bytedance/seedance-2.5/video-extend"
    );
    assert_eq!(
        seedance.endpoint_id(Mode::MotionTransfer, Resolution::P720, true, true),
        "bytedance/seedance-2.5/image-to-video"
    );
    assert_eq!(
        seedance.endpoint_id(Mode::MotionTransfer, Resolution::P720, true, false),
        "bytedance/seedance-2.5/reference-to-video"
    );
    assert_eq!(
        seedance.endpoint_id(Mode::MotionTransfer, Resolution::P720, false, false),
        "bytedance/seedance-2.5/text-to-video"
    );
    let kling = VideoModel::KLING_MOTION_CONTROL;
    let endpoints: Vec<String> = Resolution::STANDARD
        .iter()
        .map(|resolution| kling.endpoint_id(Mode::MotionTransfer, *resolution, true, false))
        .collect();
    assert_eq!(
        endpoints,
        [
            "kling-video/v3/motion-control/std",
            "kling-video/v3/motion-control/std",
            "kling-video/v3/motion-control/pro"
        ]
    );
}

/// Every mode an API key runs reaches an endpoint, and every endpoint is priced: `Pricing` refuses to load with a
/// tariff of the catalog missing.
#[test]
fn every_models_endpoints_are_priced() {
    for model in VideoModel::all().iter().filter(|model| model.runs_with_api_key()) {
        for mode in model
            .modes()
            .into_iter()
            .filter(|mode| model.runs_with_api_key_in(*mode))
        {
            for has_media in [true, false] {
                let endpoint = model.endpoint_id(mode, Resolution::P720, has_media, false);
                assert!(!endpoint.is_empty(), "{} {}", model.raw_value(), mode.raw_value());
            }
        }
    }
    Pricing::bundled().unwrap();
}

/// MiniMax H3 Max isn't in the public REST docs: it has no endpoint, so no tariff, and runs on the account only.
/// Genjutsu Lite's REST endpoint is Motion Transfer's alone: its Object Swap runs on the account only.
#[test]
fn a_model_without_endpoints_runs_on_the_account_only() {
    let max = model("minimaxH3MaxReference");
    let lite = model("genjutsuLite");
    assert!(!max.runs_with_api_key());
    assert!(
        max.endpoint_id(Mode::MotionTransfer, Resolution::P768, true, false)
            .is_empty()
    );
    assert_eq!(
        max.account_model_id(Mode::MotionTransfer).as_deref(),
        Some("minimax_h3_max")
    );
    assert!(!Pricing::tariffs().iter().any(|(tariff, _)| tariff.is_empty()));
    let gemini = model("geminiOmniFlash12");
    let intone = model("intone");
    let account_only: Vec<VideoModel> = VideoModel::all()
        .into_iter()
        .filter(|model| !model.runs_with_api_key())
        .collect();
    assert_eq!(account_only, [max, gemini, intone]);
    assert!(lite.runs_with_api_key_in(Mode::MotionTransfer) && !lite.runs_with_api_key_in(Mode::ObjectSwap));
    assert_eq!(
        lite.endpoint_id(Mode::MotionTransfer, Resolution::P480, true, false),
        "higgsfield/genjutsu-light/motion-transfer/v1.0"
    );
    assert!(
        lite.endpoint_id(Mode::ObjectSwap, Resolution::P720, true, false)
            .is_empty()
    );
    // Every other model runs each of its modes with an API key.
    for model in VideoModel::all()
        .iter()
        .filter(|model| **model != lite && model.runs_with_api_key())
    {
        assert!(
            model
                .modes()
                .into_iter()
                .all(|mode| model.runs_with_api_key_in(mode)),
            "{}",
            model.raw_value()
        );
    }
}

/// Genjutsu Lite: Genjutsu's request and draft for Motion and Swap (no Restyle) on models of its own, up to 15 s
/// with up to 10 images.
#[test]
fn genjutsu_lite_is_genjutsus_second_model() {
    let lite = model("genjutsuLite");
    assert_eq!(VideoModel::all()[..2], [VideoModel::GENJUTSU, lite.clone()]);
    assert_eq!(lite.title(), "Genjutsu Lite");
    assert_eq!(lite.family(), VideoModel::GENJUTSU.family());
    assert_eq!(lite.modes(), [Mode::MotionTransfer, Mode::ObjectSwap]);
    assert_eq!(lite.descriptor().shape, RequestShape::Genjutsu);
    assert!(lite.offers_draft() && lite.resolutions() == Resolution::STANDARD);
    assert_eq!(
        lite.account_model_id(Mode::MotionTransfer).as_deref(),
        Some("hf_mult_motion_control_flash")
    );
    assert_eq!(
        lite.account_model_id(Mode::ObjectSwap).as_deref(),
        Some("hf_mult_replace_object_flash")
    );
    for mode in lite.modes() {
        assert_eq!(built_in_limits(&lite, mode).duration, ClosedRange::new(4.0, 15.0));
        assert_eq!(
            built_in_limits(&lite, mode).reference_images,
            ClosedRange::new(1, 10)
        );
    }
    // Genjutsu stays each of its modes' default.
    assert!(
        Mode::MotionTransfer.default_model() == VideoModel::GENJUTSU
            && Mode::ObjectSwap.default_model() == VideoModel::GENJUTSU
    );
}

/// Each mode renders at what its `models_explore get` lists (snapshot of 2026-10-28): Lite's Object Swap at
/// 720p and 1080p only, so a job asking it for 480p gets 720p; its Motion Transfer and every Genjutsu mode at
/// 480p, 720p and 1080p.
#[test]
fn genjutsu_lite_object_swap_starts_at_720p() {
    let lite = model("genjutsuLite");
    assert_eq!(lite.resolutions_for(Mode::MotionTransfer), Resolution::STANDARD);
    assert_eq!(
        lite.resolutions_for(Mode::ObjectSwap),
        [Resolution::P720, Resolution::P1080]
    );
    assert_eq!(lite.fitted(Resolution::P480, Mode::ObjectSwap), Resolution::P720);
    assert_eq!(
        lite.fitted(Resolution::P480, Mode::MotionTransfer),
        Resolution::P480
    );
    assert_eq!(
        lite.fitted(Resolution::P1080, Mode::ObjectSwap),
        Resolution::P1080
    );
    for mode in VideoModel::GENJUTSU.modes() {
        assert_eq!(VideoModel::GENJUTSU.resolutions_for(mode), Resolution::STANDARD);
    }
    let source = SourceClip::new(
        file_url("/tmp/in.mov"),
        30.0,
        PixelSize::new(1280, 720),
        ClosedRange::new(0.0, 10.0),
    );
    let swap = JobSpec::new(Mode::ObjectSwap, lite.clone(), Some(source), Resolution::P480);
    assert_eq!(swap.resolution, Resolution::P720);
    // Every other model renders at the same resolutions in each of its modes.
    for model in VideoModel::all().iter().filter(|model| **model != lite) {
        for mode in model.modes() {
            assert_eq!(
                model.resolutions_for(mode),
                model.resolutions(),
                "{}",
                model.raw_value()
            );
        }
    }
}

#[test]
fn each_mode_defaults_to_the_first_model_serving_it() {
    let defaults: Vec<VideoModel> = Mode::ALL.iter().map(|mode| mode.default_model()).collect();
    assert_eq!(
        defaults,
        [
            VideoModel::GENJUTSU,
            VideoModel::GENJUTSU,
            VideoModel::GENJUTSU,
            VideoModel::SEEDANCE25,
            VideoModel::SEEDANCE25
        ]
    );
}

#[test]
fn the_account_models_are_named_by_mode() {
    assert_eq!(
        VideoModel::GENJUTSU
            .account_model_id(Mode::MotionTransfer)
            .as_deref(),
        Some("hf_mult_motion_control")
    );
    assert_eq!(
        VideoModel::GENJUTSU.account_model_id(Mode::ObjectSwap).as_deref(),
        Some("hf_mult_replace_object")
    );
    assert_eq!(VideoModel::GENJUTSU.account_model_id(Mode::Restyle), None);
    assert_eq!(
        VideoModel::SEEDANCE25.account_model_id(Mode::Extend).as_deref(),
        Some("seedance_2_5")
    );
    assert_eq!(
        VideoModel::KLING_MOTION_CONTROL
            .account_model_id(Mode::MotionTransfer)
            .as_deref(),
        Some("kling3_0_motion_control")
    );
}

// Compatibility

/// A model is coded as its bare ID, exactly as the enum was.
#[test]
fn a_model_is_coded_as_its_id() {
    let models = [
        VideoModel::GENJUTSU,
        VideoModel::SEEDANCE25,
        VideoModel::KLING_MOTION_CONTROL,
    ];
    assert_eq!(
        serde_json::to_string(&models).unwrap(),
        r#"["genjutsu","seedance25","klingMotionControl"]"#
    );
    let decoded: Vec<VideoModel> =
        serde_json::from_str(r#"["klingMotionControl","seedance25","genjutsu"]"#).unwrap();
    assert_eq!(
        decoded,
        [
            VideoModel::KLING_MOTION_CONTROL,
            VideoModel::SEEDANCE25,
            VideoModel::GENJUTSU
        ]
    );
}

/// An ID the catalog doesn't have fails like an unknown enum case did, so lossy readers drop it.
#[test]
fn an_unknown_model_doesnt_decode() {
    assert_eq!(VideoModel::from_raw("veo"), None);
    assert_eq!(VideoModel::from_raw("seedance25"), Some(VideoModel::SEEDANCE25));
    assert!(serde_json::from_str::<VideoModel>(r#""veo""#).is_err());
}

/// Jobs stored by earlier versions, one per model, decode unchanged and encode the same ID back.
#[test]
fn a_stored_job_keeps_its_model() {
    for id in ["genjutsu", "seedance25", "klingMotionControl"] {
        let json = format!(
            r#"{{"mode":"motionTransfer","model":"{id}","source":{{"fileURL":"file:///tmp/in.mp4","duration":10,"pixelSize":{{"width":1280,"height":720}},"range":[0,5],"allowsUpscale":false}},"references":[],"prompt":"","resolution":"720p","isDraft":false,"audio":"original"}}"#
        );
        let spec: JobSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec.model.raw_value(), id);
        let object = serde_json::to_value(&spec).unwrap();
        assert_eq!(object["model"], id);
    }
}

/// Templates saved by earlier versions keep their model.
#[test]
fn a_stored_template_keeps_its_model() {
    let json = r#"{"id":"6F9619FF-8B86-D011-B42D-00C04FC964FF","name":"Fox","createdAt":0,"mode":"motionTransfer","model":"klingMotionControl","references":[],"prompt":"","resolution":"1080p","audio":"mute"}"#;
    let template: JobTemplate = serde_json::from_str(json).unwrap();
    assert_eq!(template.model, VideoModel::KLING_MOTION_CONTROL);
    assert_eq!(template.resolution, Resolution::P1080);
}

/// The cached server limits keep naming the models by ID; an unknown one is still dropped.
#[test]
fn cached_constraints_keep_their_models() {
    let json = r#"{"entries":[{"model":"seedance25","mode":"motionTransfer","constraints":{"maxDuration":20}},{"model":"veo","mode":"motionTransfer","constraints":{"maxDuration":8}}]}"#;
    let table: ModelConstraintTable = serde_json::from_str(json).unwrap();
    let models: Vec<&VideoModel> = table.entries().iter().map(|entry| &entry.model).collect();
    assert_eq!(models, [&VideoModel::SEEDANCE25]);
    assert_eq!(
        VideoModel::SEEDANCE25
            .limits(Mode::MotionTransfer, true, &table)
            .duration,
        ClosedRange::new(4.0, 20.0)
    );
}

// Reading the file

/// An entry this version can't run (an unknown shape, mode or control, a reversed range, a mode's resolutions
/// outside the model's or none) is left out and the others load; a repeated ID keeps the first.
#[test]
fn entries_this_version_cant_run_are_left_out() {
    let entry = |id: &str, extra: &str| {
        format!(
            r#"{{"id":"{id}","family":"F","title":"{id}","modes":["motionTransfer"],"request":{{"shape":"seedance","endpoints":{{"default":"x/{id}"}}}}{extra}}}"#
        )
    };
    let models = [
        entry(
            "ok",
            r#","limits":{"duration":[2,10],"referenceImages":[0,3]},"controls":["draft"]"#,
        ),
        entry("shape", "").replace(r#""seedance""#, r#""cinema""#),
        entry("mode", "").replace("motionTransfer", "dance"),
        entry("control", r#","controls":["camera"]"#),
        entry("range", r#","limits":{"duration":[10,2]}"#),
        entry("ok", r#","controls":["frames"]"#),
        entry(
            "modeResolution",
            r#","resolutions":["720p"],"modeResolutions":{"motionTransfer":["480p"]}"#,
        ),
        entry("noModeResolution", r#","modeResolutions":{"motionTransfer":[]}"#),
        entry(
            "byMode",
            r#","modeResolutions":{"motionTransfer":["1080p","720p"]}"#,
        ),
    ];
    let catalog = VideoModelCatalog::from_json(&format!(r#"{{"models":[{}]}}"#, models.join(","))).unwrap();
    assert_eq!(catalog.entry_count(), 9);
    let ids: Vec<&str> = catalog
        .descriptors()
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect();
    assert_eq!(ids, ["ok", "byMode"]);
    assert_eq!(
        catalog.get("byMode").unwrap().mode_resolutions,
        BTreeMap::from([(Mode::MotionTransfer, vec![Resolution::P720, Resolution::P1080])])
    );
    let model = catalog.get("ok").unwrap();
    assert_eq!(model.duration, Some(ClosedRange::new(2.0, 10.0)));
    assert_eq!(model.reference_images, Some(ClosedRange::new(0, 3)));
    assert_eq!(model.controls, BTreeSet::from([VideoControl::Draft]));
    assert_eq!(
        model.endpoints,
        HashMap::from([("default".to_owned(), "x/ok".to_owned())])
    );
    assert!(model.account_models.is_empty());
}
