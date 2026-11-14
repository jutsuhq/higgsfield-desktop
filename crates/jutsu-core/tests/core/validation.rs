//! `ValidationTests.swift`.

use jutsu_core::*;
use uuid::Uuid;

use crate::common::*;

fn check(spec: &JobSpec) -> Vec<ValidationIssue> {
    Validation::check(spec, &ModelConstraintTable::built_in())
}

fn args(mode: Mode) -> SpecArgs {
    SpecArgs {
        mode,
        ..SpecArgs::default()
    }
}

fn restyle(references: usize) -> JobSpec {
    spec_with(SpecArgs {
        references,
        preset_id: Some(Uuid::new_v4()),
        ..args(Mode::Restyle)
    })
}

fn with_duration(duration: f64) -> JobSpec {
    spec_with(SpecArgs {
        duration,
        ..SpecArgs::default()
    })
}

fn with_prompt(prompt: String) -> JobSpec {
    spec_with(SpecArgs {
        prompt,
        ..SpecArgs::default()
    })
}

#[test]
fn valid_specs_have_no_issues() {
    assert!(check(&spec_with(args(Mode::MotionTransfer))).is_empty());
    assert!(check(&spec_with(args(Mode::ObjectSwap))).is_empty());
    assert!(check(&restyle(0)).is_empty());
}

#[test]
fn duration_outside_four_to_thirty_seconds() {
    for duration in [4.0, 30.0, 3.999_999_9, 30.000_000_1] {
        assert!(check(&with_duration(duration)).is_empty(), "{duration}");
    }
    assert_eq!(
        check(&with_duration(3.9)),
        [ValidationIssue::DurationTooShort(3.9)]
    );
    assert_eq!(
        check(&with_duration(30.1)),
        [ValidationIssue::DurationTooLong(30.1)]
    );
}

#[test]
fn reference_counts_for_motion_and_swap() {
    for mode in [Mode::MotionTransfer, Mode::ObjectSwap] {
        let spec = |references| {
            spec_with(SpecArgs {
                references,
                ..args(mode)
            })
        };
        assert_eq!(
            check(&spec(0)),
            [ValidationIssue::TooFewReferences { minimum: 1 }]
        );
        assert!(check(&spec(8)).is_empty());
        assert_eq!(
            check(&spec(9)),
            [ValidationIssue::TooManyReferences { maximum: 8 }]
        );
    }
}

#[test]
fn reference_counts_for_restyle() {
    assert!(check(&restyle(5)).is_empty());
    assert_eq!(
        check(&restyle(6)),
        [ValidationIssue::TooManyReferences { maximum: 5 }]
    );
}

#[test]
fn restyle_requires_a_preset() {
    assert_eq!(
        check(&spec_with(SpecArgs {
            references: 0,
            ..args(Mode::Restyle)
        })),
        [ValidationIssue::PresetRequired]
    );
}

#[test]
fn object_swap_needs_enough_pixels_unless_upscale_is_allowed() {
    let small = PixelSize::new(720, 480);
    let spec = |mode, size, allows_upscale| {
        spec_with(SpecArgs {
            size,
            allows_upscale,
            ..args(mode)
        })
    };
    assert_eq!(
        check(&spec(Mode::ObjectSwap, small, false)),
        [ValidationIssue::FrameTooSmall(small)]
    );
    assert!(check(&spec(Mode::ObjectSwap, small, true)).is_empty());
    assert!(check(&spec(Mode::ObjectSwap, PixelSize::new(640, 640), false)).is_empty());
    assert!(check(&spec(Mode::MotionTransfer, small, false)).is_empty());
}

#[test]
fn prompt_length_counts_unicode_scalars() {
    assert!(check(&with_prompt("a".repeat(10_000))).is_empty());
    assert_eq!(
        check(&with_prompt("a".repeat(10_001))),
        [ValidationIssue::PromptTooLong { length: 10_001 }]
    );
    // One grapheme, two scalars: 5 001 flags are 10 002 scalars.
    assert_eq!(
        check(&with_prompt("🇯🇵".repeat(5_001))),
        [ValidationIssue::PromptTooLong { length: 10_002 }]
    );
}

#[test]
fn all_issues_are_reported_together() {
    let spec = spec_with(SpecArgs {
        duration: 2.0,
        references: 7,
        prompt: "a".repeat(10_001),
        ..args(Mode::Restyle)
    });
    assert_eq!(
        check(&spec),
        [
            ValidationIssue::DurationTooShort(2.0),
            ValidationIssue::TooManyReferences { maximum: 5 },
            ValidationIssue::PresetRequired,
            ValidationIssue::PromptTooLong { length: 10_001 },
        ]
    );
}

#[test]
fn upscaled_size() {
    for (width, height, expected_width, expected_height) in [
        (720, 480, 784, 524),
        (640, 360, 854, 480),
        (480, 480, 640, 640),
        (1280, 720, 1280, 720),
        (640, 640, 640, 640),
    ] {
        let size = Validation::upscaled_size(width, height);
        assert_eq!(size, PixelSize::new(expected_width, expected_height));
        assert!(size.pixel_count() >= Validation::OBJECT_SWAP_MINIMUM_PIXELS);
        assert!(size.width % 2 == 0 && size.height % 2 == 0);
    }
}

fn source_spec(duration: f64, range: ClosedRange<f64>, size: Option<PixelSize>) -> JobSpec {
    let mut spec = make_spec();
    let source = spec.source.as_mut().unwrap();
    source.duration = duration;
    source.range = range;
    if let Some(size) = size {
        source.pixel_size = size;
    }
    spec
}

#[test]
fn selection_must_be_finite_and_inside_the_source() {
    for (duration, range) in [
        (60.0, ClosedRange::new(50.0, 70.0)),
        (60.0, ClosedRange::new(-1.0, 10.0)),
        (f64::NAN, ClosedRange::new(0.0, 10.0)),
        (f64::INFINITY, ClosedRange::new(0.0, 10.0)),
        (f64::INFINITY, ClosedRange::new(f64::INFINITY, f64::INFINITY)),
        (60.0, ClosedRange::new(0.0, f64::INFINITY)),
    ] {
        assert_eq!(
            check(&source_spec(duration, range, None)),
            [ValidationIssue::SelectionOutsideSource],
            "{range:?}"
        );
    }
}

#[test]
fn selection_may_end_at_the_source_end() {
    assert!(check(&source_spec(10.0, ClosedRange::new(0.0, 10.000_000_1), None)).is_empty());
}

#[test]
fn frame_sides_must_be_positive() {
    for size in [PixelSize::new(0, 720), PixelSize::new(-1280, -720)] {
        let spec = source_spec(60.0, ClosedRange::new(0.0, 10.0), Some(size));
        assert_eq!(check(&spec), [ValidationIssue::InvalidFrameSize(size)]);
        let mut swap = spec;
        swap.mode = Mode::ObjectSwap;
        assert_eq!(check(&swap), [ValidationIssue::InvalidFrameSize(size)]);
    }
}

#[test]
fn pixel_count_never_traps() {
    assert_eq!(PixelSize::new(-1280, -720).pixel_count(), 0);
    assert_eq!(PixelSize::new(i64::MAX, 2).pixel_count(), i64::MAX);
    assert_eq!(PixelSize::new(1280, 720).pixel_count(), 921_600);
}

fn clip_without_source(model: VideoModel, prompt: &str, references: usize, length: f64) -> JobSpec {
    let mut spec = JobSpec::new(Mode::MotionTransfer, model, None, Resolution::P720);
    spec.references = (0..references)
        .map(|i| LocalImage::new(file_url(&format!("/tmp/ref{i}.png"))))
        .collect();
    spec.prompt = prompt.into();
    spec.length = Some(length);
    spec.aspect_ratio = Some(AspectRatio::Landscape);
    spec
}

fn seedance_clip(prompt: &str, references: usize, length: f64) -> JobSpec {
    clip_without_source(VideoModel::SEEDANCE25, prompt, references, length)
}

#[test]
fn seedance_makes_a_clip_without_a_source_from_a_prompt_or_an_image() {
    assert!(check(&seedance_clip("a fox in the snow", 0, 5.0)).is_empty());
    assert!(check(&seedance_clip("", 1, 5.0)).is_empty());
    assert_eq!(
        check(&seedance_clip(" \n", 0, 5.0)),
        [ValidationIssue::PromptRequired]
    );
    assert_eq!(
        check(&seedance_clip("a fox in the snow", 9, 5.0)),
        [ValidationIssue::TooManyReferences { maximum: 8 }]
    );
}

#[test]
fn a_clip_without_a_source_keeps_to_the_models_length() {
    assert!(check(&seedance_clip("a fox in the snow", 0, 30.0)).is_empty());
    assert_eq!(
        check(&seedance_clip("a fox in the snow", 0, 3.0)),
        [ValidationIssue::DurationTooShort(3.0)]
    );
    assert_eq!(
        check(&seedance_clip("a fox in the snow", 0, 31.0)),
        [ValidationIssue::DurationTooLong(31.0)]
    );
}

#[test]
fn other_models_need_a_source() {
    for model in [VideoModel::GENJUTSU, VideoModel::KLING_MOTION_CONTROL] {
        let spec = clip_without_source(model.clone(), "a fox in the snow", 0, 5.0);
        assert!(
            check(&spec).contains(&ValidationIssue::SourceRequired),
            "{model:?}"
        );
    }
}

fn video_operation(mode: Mode, prompt: &str, selection: f64) -> JobSpec {
    let mut spec = spec_with(SpecArgs {
        duration: selection,
        references: 0,
        prompt: prompt.into(),
        ..args(mode)
    });
    spec.model = VideoModel::SEEDANCE25;
    if mode == Mode::Extend {
        spec.length = Some(5.0);
    }
    spec
}

#[test]
fn edit_and_extend_need_a_source_and_a_prompt() {
    for mode in [Mode::Edit, Mode::Extend] {
        assert!(
            check(&video_operation(mode, "make it night", 8.0)).is_empty(),
            "{mode:?}"
        );
        assert_eq!(
            check(&video_operation(mode, " \n", 8.0)),
            [ValidationIssue::PromptRequired]
        );
        let mut without_source = video_operation(mode, "make it night", 8.0);
        without_source.source = None;
        assert!(check(&without_source).contains(&ValidationIssue::SourceRequired));
    }
}

#[test]
fn extend_checks_the_selection_and_the_added_length() {
    assert_eq!(
        check(&video_operation(Mode::Extend, "make it night", 31.0)),
        [ValidationIssue::DurationTooLong(31.0)]
    );
    let mut spec = video_operation(Mode::Extend, "make it night", 8.0);
    spec.length = Some(3.0);
    assert_eq!(check(&spec), [ValidationIssue::DurationTooShort(3.0)]);
    spec.length = None;
    assert_eq!(check(&spec), [ValidationIssue::DurationTooShort(0.0)]);
}
