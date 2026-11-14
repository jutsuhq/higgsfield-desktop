//! `ModelConstraintsTests.swift`.

use jutsu_core::*;

use crate::common::*;

fn table(model: VideoModel, constraints: ModelConstraints, mode: Mode) -> ModelConstraintTable {
    ModelConstraintTable::new(vec![ModelConstraintEntry {
        model,
        mode,
        constraints,
    }])
}

fn constraints(
    min: Option<f64>,
    max: Option<f64>,
    min_images: Option<i64>,
    max_images: Option<i64>,
) -> ModelConstraints {
    ModelConstraints::new(min, max, min_images, max_images)
}

fn limits(model: &VideoModel, mode: Mode, table: &ModelConstraintTable) -> ModeLimits {
    model.limits(mode, true, table)
}

#[test]
fn the_table_replaces_the_constants_it_states() {
    let table = table(
        VideoModel::SEEDANCE25,
        constraints(None, Some(45.0), None, Some(4)),
        Mode::MotionTransfer,
    );
    let limits = limits(&VideoModel::SEEDANCE25, Mode::MotionTransfer, &table);
    assert_eq!(limits.duration, ClosedRange::new(4.0, 45.0));
    assert_eq!(limits.reference_images, ClosedRange::new(1, 4));
}

#[test]
fn a_missing_bound_keeps_its_constant() {
    let kling = VideoModel::KLING_MOTION_CONTROL;
    let table = table(
        kling.clone(),
        constraints(None, None, None, Some(2)),
        Mode::MotionTransfer,
    );
    let limits = limits(&kling, Mode::MotionTransfer, &table);
    assert_eq!(limits.reference_images, ClosedRange::new(1, 2));
    assert_eq!(
        limits.duration,
        self::limits(&kling, Mode::MotionTransfer, &ModelConstraintTable::built_in()).duration
    );
}

#[test]
fn bounds_that_contradict_each_other_keep_the_constants() {
    let table = table(
        VideoModel::SEEDANCE25,
        constraints(Some(40.0), None, Some(9), None),
        Mode::MotionTransfer,
    );
    assert_eq!(
        limits(&VideoModel::SEEDANCE25, Mode::MotionTransfer, &table),
        ModeLimits {
            reference_images: ClosedRange::new(1, 8),
            requires_preset: false,
            duration: ClosedRange::new(4.0, 30.0)
        }
    );
}

#[test]
fn an_entry_applies_only_to_its_model_and_mode() {
    let table = table(
        VideoModel::GENJUTSU,
        constraints(None, None, None, Some(3)),
        Mode::ObjectSwap,
    );
    assert_eq!(
        limits(&VideoModel::GENJUTSU, Mode::ObjectSwap, &table).reference_images,
        ClosedRange::new(1, 3)
    );
    assert_eq!(
        limits(&VideoModel::GENJUTSU, Mode::MotionTransfer, &table).reference_images,
        ClosedRange::new(1, 8)
    );
    assert_eq!(
        limits(&VideoModel::SEEDANCE25, Mode::ObjectSwap, &table).reference_images,
        ClosedRange::new(1, 8)
    );
}

#[test]
fn without_a_source_no_image_is_required_whatever_the_table_says() {
    let table = table(
        VideoModel::SEEDANCE25,
        constraints(None, None, Some(2), Some(5)),
        Mode::MotionTransfer,
    );
    assert_eq!(
        VideoModel::SEEDANCE25
            .limits(Mode::MotionTransfer, false, &table)
            .reference_images,
        ClosedRange::new(0, 5)
    );
}

#[test]
fn validation_checks_against_the_table() {
    let source = SourceClip::new(
        file_url("/tmp/in.mp4"),
        60.0,
        PixelSize::new(1280, 720),
        ClosedRange::new(0.0, 40.0),
    );
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        Some(source),
        Resolution::P720,
    );
    spec.references = vec![LocalImage::new(file_url("/tmp/a.png"))];
    assert_eq!(
        Validation::check(&spec, &ModelConstraintTable::built_in()),
        [ValidationIssue::DurationTooLong(40.0)]
    );
    let table = table(
        VideoModel::SEEDANCE25,
        constraints(None, Some(45.0), None, None),
        Mode::MotionTransfer,
    );
    assert!(Validation::check(&spec, &table).is_empty());
}

#[test]
fn invalid_values_are_dropped() {
    let constraints = constraints(Some(-1.0), Some(f64::INFINITY), Some(-2), Some(3));
    assert_eq!(constraints, self::constraints(None, None, None, Some(3)));
}

#[test]
fn junk_decodes_to_what_can_be_read() {
    let json = r#"
        {"entries": [
          {"model": "seedance25", "mode": "motionTransfer",
           "constraints": {"maxDuration": "45", "minDuration": 5, "maxReferenceImages": 2.5, "extra": true}},
          {"model": "veo", "mode": "motionTransfer", "constraints": {"maxDuration": 8}},
          {"model": "genjutsu", "mode": "dance", "constraints": {}},
          {"model": "klingMotionControl", "mode": "motionTransfer"},
          "nonsense", 12, null
        ]}
        "#;
    let table: ModelConstraintTable = serde_json::from_str(json).unwrap();
    assert_eq!(table.entries().len(), 1);
    assert_eq!(
        table.get(&VideoModel::SEEDANCE25, Mode::MotionTransfer),
        Some(constraints(Some(5.0), None, None, None))
    );
}

#[test]
fn a_table_without_entries_is_the_constants() {
    assert_eq!(
        serde_json::from_str::<ModelConstraintTable>("{}").unwrap(),
        ModelConstraintTable::built_in()
    );
}

#[test]
fn differences_from_the_constants_are_listed() {
    let same = table(
        VideoModel::SEEDANCE25,
        constraints(None, Some(30.0), None, None),
        Mode::MotionTransfer,
    );
    assert!(same.differences_from_built_in().is_empty());
    let differences = table(
        VideoModel::SEEDANCE25,
        constraints(None, Some(15.0), None, None),
        Mode::MotionTransfer,
    )
    .differences_from_built_in();
    assert_eq!(differences.len(), 1);
    assert!(differences[0].contains("seedance25 motionTransfer"));
}
