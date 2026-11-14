//! `KlingO3Tests.swift`: Kling O3's video reference, added as a `models.json` entry.

use jutsu_core::*;

use crate::common::*;

fn model() -> VideoModel {
    VideoModel::from_raw("klingO3VideoReference").unwrap()
}

fn spec(seconds: f64, resolution: Resolution, prompt: &str) -> JobSpec {
    let source = SourceClip::new(
        file_url("/tmp/in.mp4"),
        30.0,
        PixelSize::new(1280, 720),
        ClosedRange::new(0.0, seconds),
    );
    let mut spec = JobSpec::new(Mode::MotionTransfer, model(), Some(source), resolution);
    prompt.clone_into(&mut spec.prompt);
    spec
}

#[test]
fn it_is_a_kling_with_its_own_limits() {
    let model = model();
    let limits = model.limits(Mode::MotionTransfer, true, &ModelConstraintTable::built_in());
    assert_eq!(model.title(), "Kling O3");
    assert_eq!(model.family(), VideoModel::KLING_MOTION_CONTROL.family());
    assert_eq!(model.modes(), [Mode::MotionTransfer]);
    assert_eq!(limits.duration, ClosedRange::new(3.0, 10.0));
    assert_eq!(limits.reference_images, ClosedRange::new(0, 4));
    assert!(
        model.declares(VideoControl::Quality) && !model.offers_draft() && !model.makes_clips_without_source()
    );
}

/// One endpoint for both qualities (Standard below 1080p, Pro at 1080p). Its page gives only a range, $0.126 to
/// $0.168 a second, naming no mode, so both are priced up to its top.
#[test]
fn it_is_priced_up_to_the_top_of_its_range() {
    let pricing = Pricing::bundled().unwrap();
    let standard = spec(5.0, Resolution::P720, "she spins");
    let pro = spec(5.0, Resolution::P1080, "she spins");
    assert_eq!(standard.endpoint_id(), "kling-video/o3/video-reference");
    assert_eq!(pro.endpoint_id(), "kling-video/o3/video-reference");
    for spec in [standard, pro] {
        assert_eq!(pricing.cost(&spec).unwrap(), usd("0.84", true));
        assert!(pricing.is_upper_bound_for(&spec));
    }
}

/// The schema requires a prompt, and takes 3–10 s.
#[test]
fn it_needs_a_prompt_and_a_short_selection() {
    let table = ModelConstraintTable::built_in();
    assert!(Validation::check(&spec(5.0, Resolution::P720, "she spins"), &table).is_empty());
    assert!(
        Validation::check(&spec(5.0, Resolution::P720, " "), &table)
            .contains(&ValidationIssue::PromptRequired)
    );
    assert!(
        Validation::check(&spec(12.0, Resolution::P720, "she spins"), &table)
            .contains(&ValidationIssue::DurationTooLong(12.0))
    );
}
