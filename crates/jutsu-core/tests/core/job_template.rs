//! `JobTemplateTests.swift`.

use std::collections::HashSet;

use jutsu_core::*;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::common::*;

fn template(name: &str, mode: Mode, model: VideoModel, resolution: Resolution) -> JobTemplate {
    JobTemplate::new(
        Uuid::new_v4(),
        name,
        OffsetDateTime::now_utc(),
        mode,
        model,
        resolution,
    )
}

fn from_spec(name: &str, spec: &JobSpec) -> JobTemplate {
    JobTemplate::from_spec(Uuid::new_v4(), name, OffsetDateTime::now_utc(), spec)
}

fn round_trip(template: &JobTemplate) -> JobTemplate {
    serde_json::from_str(&serde_json::to_string(template).unwrap()).unwrap()
}

fn without(template: &JobTemplate, keys: &[&str]) -> JobTemplate {
    let mut object = serde_json::to_value(template).unwrap();
    for key in keys {
        assert!(object.as_object_mut().unwrap().remove(*key).is_some(), "{key}");
    }
    serde_json::from_value(object).unwrap()
}

#[test]
fn takes_the_spec_inputs_but_not_the_source() {
    let mut spec = spec_with(SpecArgs {
        mode: Mode::Restyle,
        references: 2,
        preset_id: Some(Uuid::new_v4()),
        prompt: "rain".into(),
        resolution: Resolution::P1080,
        ..SpecArgs::default()
    });
    spec.audio = AudioOption::Mute;
    let template = from_spec("Rain", &spec);
    assert_eq!(template.mode, Mode::Restyle);
    assert_eq!(template.references, spec.references);
    assert_eq!(template.preset_id, spec.preset_id);
    assert_eq!(template.prompt, "rain");
    assert_eq!(template.resolution, Resolution::P1080);
    assert_eq!(template.audio, AudioOption::Mute);
}

#[test]
fn drops_the_preset_outside_restyle() {
    let mut template = template("Swap", Mode::ObjectSwap, VideoModel::genjutsu(), Resolution::P720);
    template.preset_id = Some(Uuid::new_v4());
    template.normalize();
    assert_eq!(template.preset_id, None);
}

#[test]
fn round_trips_through_json() {
    let mut template = JobTemplate::new(
        Uuid::new_v4(),
        "Neon",
        OffsetDateTime::from_unix_timestamp(1_000).unwrap(),
        Mode::Restyle,
        VideoModel::genjutsu(),
        Resolution::P720,
    );
    template.references = vec![LocalImage::new(file_url("/tmp/a.png"))];
    template.preset_id = Some(Uuid::new_v4());
    template.prompt = "neon".into();
    template.audio = AudioOption::FadeOut;
    assert_eq!(round_trip(&template), template);

    let kling = template_kling();
    assert_eq!(round_trip(&kling).model, VideoModel::KLING_MOTION_CONTROL);
}

fn template_kling() -> JobTemplate {
    template(
        "Dance",
        Mode::MotionTransfer,
        VideoModel::KLING_MOTION_CONTROL,
        Resolution::P1080,
    )
}

#[test]
fn keeps_the_spec_model() {
    let mut spec = spec_with(SpecArgs {
        mode: Mode::MotionTransfer,
        ..SpecArgs::default()
    });
    spec.model = VideoModel::SEEDANCE25;
    assert_eq!(from_spec("Dance", &spec).model, VideoModel::SEEDANCE25);
}

#[test]
fn a_model_that_doesnt_serve_the_mode_is_genjutsu() {
    let template = template("Swap", Mode::ObjectSwap, VideoModel::SEEDANCE25, Resolution::P720);
    assert_eq!(template.model, VideoModel::GENJUTSU);
}

/// Templates stored before models, the model parameters and kinds decode as Genjutsu video templates without them.
#[test]
fn stored_before_later_fields_decodes_with_defaults() {
    let mut current = template(
        "Old",
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        Resolution::P720,
    );
    current.bitrate = Some(SeedanceBitrate::Standard);
    current.character_orientation = Some(CharacterOrientation::Image);
    current.references_are_frames = true;
    current.image_model_id = Some("soul".into());
    current.image_model_title = Some("Soul".into());
    current.image_aspect_ratio = Some("16:9".into());
    current.image_count = 4;
    assert_eq!(round_trip(&current), current);

    let template = without(
        &current,
        &[
            "model",
            "bitrate",
            "characterOrientation",
            "referencesAreFrames",
            "kind",
            "imageModelID",
            "imageModelTitle",
            "imageAspectRatio",
            "imageCount",
        ],
    );

    assert_eq!(template.model, VideoModel::GENJUTSU);
    assert!(
        template.bitrate.is_none()
            && template.character_orientation.is_none()
            && !template.references_are_frames
    );
    assert_eq!(template.kind, TemplateKind::Video);
    assert!(
        template.image_model_id.is_none()
            && template.image_aspect_ratio.is_none()
            && template.image_count == 1
    );
    assert_eq!(template.name, "Old");
    assert_eq!(template.resolution, Resolution::P720);
}

#[test]
fn image_template_round_trips_through_json() {
    let template = JobTemplate::image(
        Uuid::new_v4(),
        "Knight",
        OffsetDateTime::from_unix_timestamp(2_000).unwrap(),
        Some("soul".into()),
        Some("Soul 2".into()),
        Some("16:9".into()),
        4,
        "a knight",
        vec![LocalImage::new(file_url("/tmp/knight.png"))],
    );
    assert_eq!(template.kind, TemplateKind::Image);
    assert_eq!(round_trip(&template), template);
}

#[test]
fn file_names_keep_order_and_same_names_apart() {
    let sources = [
        file_url("/a/face.png"),
        file_url("/b/face.png"),
        file_url("/c/shirt.jpg"),
    ];
    assert_eq!(
        TemplateFiles::file_names(&sources),
        ["1-face.png", "2-face.png", "3-shirt.jpg"]
    );
    assert!(TemplateFiles::file_names(&[]).is_empty());
}

#[test]
fn orphans_are_folders_without_a_template() {
    let kept = Uuid::new_v4();
    let gone = Uuid::new_v4();
    let folders = [
        support::uuid_string(kept),
        support::uuid_string(gone),
        "notes".to_owned(),
    ];
    let mut expected = vec![support::uuid_string(gone), "notes".to_owned()];
    expected.sort();
    assert_eq!(
        TemplateFiles::orphaned_folders(&folders, &HashSet::from([kept])),
        expected
    );
    assert!(
        TemplateFiles::orphaned_folders(&[support::uuid_string(kept)], &HashSet::from([kept])).is_empty()
    );
}
