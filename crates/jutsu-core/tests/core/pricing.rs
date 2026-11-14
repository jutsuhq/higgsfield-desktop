//! `PricingTests.swift`.

use jutsu_core::resources::PRICING_JSON;
use jutsu_core::*;

use crate::common::*;

fn spec(mode: Mode, duration: f64, resolution: Resolution) -> JobSpec {
    spec_with(SpecArgs {
        mode,
        duration,
        resolution,
        ..SpecArgs::default()
    })
}

fn model(id: &str) -> VideoModel {
    VideoModel::from_raw(id).unwrap()
}

#[test]
fn billed_seconds_round_up() {
    for (duration, expected) in [
        (8.1, 9),
        (12.0, 12),
        (12.000_000_1, 12),
        (11.999_999_9, 12),
        (4.0, 4),
        (29.5, 30),
        (0.0, 0),
        // Clamped instead of trapping.
        (f64::NAN, 0),
        (-5.0, 0),
        (f64::INFINITY, i64::MAX),
        (1e300, i64::MAX),
    ] {
        assert_eq!(Pricing::billed_seconds(duration), expected, "{duration}");
    }
}

#[test]
fn bundled_rates_are_exact_decimals() {
    let pricing = Pricing::bundled().unwrap();
    for mode in Mode::ALL
        .iter()
        .copied()
        .filter(|mode| VideoModel::GENJUTSU.serves(*mode))
    {
        assert_eq!(
            pricing.rate_for(&spec(mode, 10.0, Resolution::P480)).unwrap(),
            dec("0.318")
        );
        assert_eq!(
            pricing.rate_for(&spec(mode, 10.0, Resolution::P720)).unwrap(),
            dec("0.681")
        );
        assert_eq!(
            pricing.rate_for(&spec(mode, 10.0, Resolution::P1080)).unwrap(),
            dec("1.632")
        );
    }
}

/// Genjutsu Lite's list prices per second, -e.05, -e.10 and -e.20, bill the output and the input video alike, so a
/// job on a source pays twice the rate per second of its selection. Object Swap's are kept, though no job uses them.
#[test]
fn genjutsu_lite_has_its_list_prices() {
    let pricing = Pricing::bundled().unwrap();
    let rates = [
        (Resolution::P480, dec("0.05")),
        (Resolution::P720, dec("0.1")),
        (Resolution::P1080, dec("0.2")),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        pricing
            .rates
            .get("higgsfield/genjutsu-light/motion-transfer/v1.0"),
        Some(&rates)
    );
    assert_eq!(
        pricing.rates.get("higgsfield/genjutsu-light/object-swap/v1.0"),
        Some(&rates)
    );
    let lite = model("genjutsuLite");
    for (resolution, rate) in &rates {
        let mut spec = spec(Mode::MotionTransfer, 10.0, *resolution);
        spec.model = lite.clone();
        assert_eq!(pricing.rate_for(&spec).unwrap(), rate * dec("2"));
    }
}

/// Seedance's list price is per second of output, and a source clip is billed as input too, at 0.6 of the rate for
/// both: 1.2 times the rate. Kling's quality is its endpoint: Standard below 1080p, Pro at 1080p.
#[test]
fn other_models_are_priced_by_their_endpoint() {
    let pricing = Pricing::bundled().unwrap();
    for (model, has_source, has_references, resolution, rate) in [
        (VideoModel::SEEDANCE25, false, false, Resolution::P720, "0.4622"),
        (VideoModel::SEEDANCE25, false, true, Resolution::P480, "0.2056"),
        (VideoModel::SEEDANCE25, true, true, Resolution::P1080, "1.36464"),
        (
            VideoModel::KLING_MOTION_CONTROL,
            true,
            true,
            Resolution::P480,
            "0.126",
        ),
        (
            VideoModel::KLING_MOTION_CONTROL,
            true,
            true,
            Resolution::P720,
            "0.126",
        ),
        (
            VideoModel::KLING_MOTION_CONTROL,
            true,
            true,
            Resolution::P1080,
            "0.168",
        ),
    ] {
        let found = pricing
            .rate(
                &model,
                Mode::MotionTransfer,
                resolution,
                has_source,
                has_references,
                false,
            )
            .unwrap();
        assert_eq!(found, dec(rate), "{model:?} {resolution:?}");
    }
}

#[test]
fn other_models_give_an_estimate() {
    let mut spec = spec(Mode::MotionTransfer, 10.0, Resolution::P720);
    spec.model = VideoModel::KLING_MOTION_CONTROL;
    assert_eq!(
        Pricing::bundled().unwrap().cost(&spec).unwrap(),
        usd("1.26", true)
    );
}

#[test]
fn cost_is_billed_seconds_times_rate() {
    let pricing = Pricing::bundled().unwrap();
    assert_eq!(
        pricing
            .cost(&spec(Mode::MotionTransfer, 12.0, Resolution::P480))
            .unwrap(),
        usd("3.816", false)
    );
    assert_eq!(
        pricing
            .cost(&spec(Mode::MotionTransfer, 8.1, Resolution::P720))
            .unwrap(),
        usd("6.129", false)
    );
    assert_eq!(
        pricing
            .cost(&spec(Mode::ObjectSwap, 30.0, Resolution::P1080))
            .unwrap(),
        usd("48.96", false)
    );
}

/// open.higgsfield.ai: Edit and Extend need a video input and bill the input and generated seconds alike at 0.6 of
/// the rate. Edit's output is as long as its source, so it costs 1.2 times the rate per second of source; Extend
/// pays 0.6 times for the source selection plus the part it adds.
#[test]
fn edit_is_billed_on_the_source_and_extend_on_the_source_and_the_added_part() {
    let pricing = Pricing::bundled().unwrap();
    let mut edit = spec(Mode::Edit, 8.0, Resolution::P720);
    edit.model = VideoModel::SEEDANCE25;
    let mut extend = spec(Mode::Extend, 8.0, Resolution::P720);
    extend.model = VideoModel::SEEDANCE25;
    extend.length = Some(5.0);

    assert_eq!(Pricing::billed_seconds_for(&edit), 8);
    assert_eq!(Pricing::billed_seconds_for(&extend), 13);
    assert_eq!(pricing.cost(&edit).unwrap(), usd("4.43712", true));
    assert_eq!(pricing.cost(&extend).unwrap(), usd("3.60516", true));
    assert_eq!(pricing.rate_for(&extend).unwrap(), dec("0.27732"));
}

#[test]
fn missing_rate_is_rejected_at_load() {
    let json = r#"{"models": {"higgsfield/genjutsu/motion-transfer/v1.0": {"480p": 0.318}}}"#;
    assert_eq!(
        Pricing::from_json(json).unwrap_err(),
        PricingError::MissingRate(
            "higgsfield/genjutsu/motion-transfer/v1.0".into(),
            Resolution::P720
        )
    );
}

/// Each tariff is priced above $0, or listed as unpriced (`null`) where no page states its rate.
#[test]
fn bundled_pricing_covers_every_tariff_and_resolution() {
    let pricing = Pricing::bundled().unwrap();
    for (tariff, resolution) in Pricing::tariffs() {
        if pricing
            .unpriced
            .contains(&TariffKey::new(tariff.clone(), resolution))
        {
            continue;
        }
        let rate = pricing
            .rates
            .get(&tariff)
            .and_then(|rates| rates.get(&resolution));
        assert!(
            rate.is_some_and(|rate| *rate > dec("0")),
            "{tariff} {resolution:?}"
        );
    }
}

/// A `null` rate loads as unpriced: the job's list price is `Cost::UNKNOWN_USD`, never $0, and the rate fails.
#[test]
fn an_unpriced_rate_is_unknown_not_free() {
    let pricing = Pricing::bundled().unwrap();
    let source = SourceClip::new(
        file_url("/tmp/in.mp4"),
        30.0,
        PixelSize::new(1280, 720),
        ClosedRange::new(0.0, 5.0),
    );
    let mut spec = JobSpec::new(
        Mode::Edit,
        model("klingO3VideoEdit"),
        Some(source),
        Resolution::P1080,
    );
    spec.prompt = "dance".into();
    assert_eq!(pricing.cost(&spec).unwrap(), Cost::UNKNOWN_USD);
    assert_eq!(
        pricing.rate_for(&spec).unwrap_err(),
        PricingError::UnpricedRate("kling-video/o3/video-edit".into(), Resolution::P1080)
    );
    let json = r#"{"models": {"higgsfield/genjutsu/motion-transfer/v1.0": {"480p": null}}}"#;
    assert_eq!(
        Pricing::from_json(json).unwrap_err(),
        PricingError::MissingRate(
            "higgsfield/genjutsu/motion-transfer/v1.0".into(),
            Resolution::P720
        )
    );
}

/// A model's tariffs need only the resolutions it renders at: Wan 2.7 has no 480p, Cinema Studio no 1080p.
#[test]
fn tariffs_follow_each_models_resolutions() {
    let tariffs = Pricing::tariffs();
    let has = |tariff: &str, resolution| tariffs.iter().any(|(t, r)| t == tariff && *r == resolution);
    assert!(!has("wan/v2.7/reference-to-video", Resolution::P480));
    assert!(has("wan/v2.7/reference-to-video", Resolution::P720));
    assert!(!has("higgsfield/cinema-studio/4.0", Resolution::P1080));
    assert!(has("higgsfield/genjutsu/motion-transfer/v1.0", Resolution::P480));
}

/// The published list prices of the models added for issue #101, before discounts.
#[test]
fn list_prices_of_the_new_models() {
    let pricing = Pricing::bundled().unwrap();
    for (id, resolution, rate) in [
        ("klingMotionControl26", Resolution::P720, "0.07"),
        ("klingMotionControl26", Resolution::P1080, "0.112"),
        ("klingO3VideoEdit", Resolution::P720, "0.126"),
        ("wan27Reference", Resolution::P720, "0.1"),
        ("wan27Reference", Resolution::P1080, "0.15"),
    ] {
        let model = model(id);
        let mode = model.modes()[0];
        assert_eq!(
            pricing
                .rate(&model, mode, resolution, true, false, false)
                .unwrap(),
            dec(rate),
            "{id}"
        );
    }
}

/// Issue #113: the rates open.higgsfield.ai/pricing publishes (2026-10-15, before discounts). Wan 3.0 and Wan 3.0
/// Prime state one per resolution; Cinema Studio 4.0, Kling O3 and Kling O3 Edit only a range or a price naming no
/// mode, so they take its top and are marked as an upper bound.
#[test]
fn published_rates_of_issue_113() {
    let pricing = Pricing::bundled().unwrap();
    for (id, mode, resolution, rate, is_up_to) in [
        (
            "wan30Reference",
            Mode::MotionTransfer,
            Resolution::P480,
            "0.05",
            false,
        ),
        (
            "wan30Reference",
            Mode::MotionTransfer,
            Resolution::P720,
            "0.1",
            false,
        ),
        (
            "wan30Reference",
            Mode::MotionTransfer,
            Resolution::P1080,
            "0.2",
            false,
        ),
        (
            "wan30PrimeReference",
            Mode::MotionTransfer,
            Resolution::P480,
            "0.068",
            false,
        ),
        (
            "wan30PrimeReference",
            Mode::MotionTransfer,
            Resolution::P720,
            "0.14",
            false,
        ),
        (
            "wan30PrimeReference",
            Mode::MotionTransfer,
            Resolution::P1080,
            "0.28",
            false,
        ),
        (
            "cinemaStudio4",
            Mode::MotionTransfer,
            Resolution::P480,
            "0.4623",
            true,
        ),
        (
            "cinemaStudio4",
            Mode::MotionTransfer,
            Resolution::P720,
            "0.4623",
            true,
        ),
        (
            "klingO3VideoReference",
            Mode::MotionTransfer,
            Resolution::P720,
            "0.168",
            true,
        ),
        (
            "klingO3VideoReference",
            Mode::MotionTransfer,
            Resolution::P1080,
            "0.168",
            true,
        ),
        ("klingO3VideoEdit", Mode::Edit, Resolution::P720, "0.126", true),
    ] {
        let model = model(id);
        assert_eq!(
            pricing.rate(&model, mode, resolution, true, true, false).unwrap(),
            dec(rate),
            "{id}"
        );
        assert_eq!(
            pricing.is_upper_bound(&model, mode, resolution, true, true, false),
            is_up_to,
            "{id}"
        );
    }
}

/// `{"upTo": rate}` loads as that rate, marked as the top of a range: always an estimate, even on a model whose list
/// price is otherwise exact.
#[test]
fn an_upper_bound_loads_as_an_estimate() {
    let mut file: serde_json::Value = serde_json::from_str(PRICING_JSON).unwrap();
    file["models"]["higgsfield/genjutsu/motion-transfer/v1.0"] =
        serde_json::json!({"480p": {"upTo": 0.5}, "720p": 0.681, "1080p": 1.632});
    let pricing = Pricing::from_json(&file.to_string()).unwrap();

    let spec480 = spec(Mode::MotionTransfer, 10.0, Resolution::P480);
    assert!(pricing.is_upper_bound_for(&spec480));
    assert_eq!(pricing.rate_for(&spec480).unwrap(), dec("0.5"));
    assert_eq!(pricing.cost(&spec480).unwrap(), usd("5", true));
    let exact = spec(Mode::MotionTransfer, 10.0, Resolution::P720);
    assert!(!pricing.is_upper_bound_for(&exact));
    assert_eq!(pricing.cost(&exact).unwrap(), usd("6.81", false));
}

/// open.higgsfield.ai prices `image-to-video` like a clip without video input: images aren't billed as video.
#[test]
fn frames_are_priced_as_a_clip_without_video_input() {
    let pricing = Pricing::bundled().unwrap();
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        None,
        Resolution::P1080,
    );
    spec.references = vec![LocalImage::new(file_url("/tmp/start.png"))];
    spec.length = Some(8.0);
    spec.references_are_frames = true;
    let tariff = Pricing::tariff_id(
        &VideoModel::SEEDANCE25,
        Mode::MotionTransfer,
        Resolution::P1080,
        false,
        true,
        true,
    );
    assert_eq!(tariff, "bytedance/seedance-2.5/image-to-video");
    assert_eq!(pricing.rate_for(&spec).unwrap(), dec("1.1372"));
    assert_eq!(pricing.cost(&spec).unwrap(), usd("9.0976", true));
}
