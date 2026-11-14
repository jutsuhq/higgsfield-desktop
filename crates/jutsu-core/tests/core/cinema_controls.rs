//! `CinemaControlsTests.swift`: Cinema Studio 4.0's creative controls: only the schema's values, coded as the body's
//! fields, kept by jobs and templates, and titled from `Models.xcstrings`.

use std::collections::BTreeMap;

use jutsu_core::*;
use time::OffsetDateTime;
use uuid::Uuid;

fn cinema() -> VideoModel {
    VideoModel::from_raw("cinemaStudio4").unwrap()
}

fn controls(values: &[(CinemaControl, &str)]) -> CinemaControls {
    CinemaControls::from_values(
        values
            .iter()
            .map(|(control, value)| (*control, (*value).to_owned())),
    )
}

#[test]
fn only_the_schemas_values_are_kept() {
    let mut controls = controls(&[(CinemaControl::Genre, "noir"), (CinemaControl::Era, "1970s")]);
    assert_eq!(
        controls.fields(),
        BTreeMap::from([("genre".to_owned(), "noir".to_owned())])
    );
    controls.set(CinemaControl::Genre, Some("western".to_owned()));
    assert!(controls.is_empty());
    controls.set(CinemaControl::Light, Some("contre-jour".to_owned()));
    assert!(controls.get(CinemaControl::Light) == Some("contre-jour") && controls.count() == 1);
}

/// Stored as the body's fields; a field or value another version wrote and this one doesn't know is dropped.
#[test]
fn it_codes_as_the_bodys_fields() {
    let controls = controls(&[
        (CinemaControl::CameraLens, "anamorphic"),
        (CinemaControl::ColorPalette, "yellow-room"),
    ]);
    let data = serde_json::to_string(&controls).unwrap();
    let object: BTreeMap<String, String> = serde_json::from_str(&data).unwrap();
    assert_eq!(
        object,
        BTreeMap::from([
            ("camera_lens".to_owned(), "anamorphic".to_owned()),
            ("color_palette".to_owned(), "yellow-room".to_owned())
        ])
    );
    assert_eq!(serde_json::from_str::<CinemaControls>(&data).unwrap(), controls);

    let newer = r#"{"camera_lens": "fisheye", "genre": "epic", "weather": "rain"}"#;
    assert_eq!(
        serde_json::from_str::<CinemaControls>(newer).unwrap(),
        self::controls(&[(CinemaControl::Genre, "epic")])
    );
}

#[test]
fn jobs_and_templates_keep_them_and_older_ones_have_none() {
    let controls = controls(&[
        (CinemaControl::CameraModel, "8mm-film"),
        (CinemaControl::Pacing, "single-shot"),
    ]);
    let mut spec = JobSpec::new(Mode::MotionTransfer, cinema(), None, Resolution::P720);
    spec.prompt = "rain".to_owned();
    spec.cinema_controls = controls.clone();
    let decoded: JobSpec = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
    assert!(decoded.cinema_controls == controls && decoded == spec);
    let template = JobTemplate::from_spec(Uuid::new_v4(), "Night", OffsetDateTime::now_utc(), &spec);
    assert_eq!(template.cinema_controls, controls);

    let decoded_template: JobTemplate =
        serde_json::from_str(&serde_json::to_string(&template).unwrap()).unwrap();
    assert_eq!(decoded_template, template);

    let mut object = serde_json::to_value(&spec).unwrap();
    assert!(object.as_object_mut().unwrap().remove("cinemaControls").is_some());
    let old: JobSpec = serde_json::from_value(object).unwrap();
    assert!(old.cinema_controls.is_empty());
}

/// Every control and value has its English title in `Models.xcstrings`, not the stand-in made from the value.
#[test]
fn every_title_is_in_the_string_catalog() {
    for control in CinemaControl::ALL {
        let field = format!("cinemaStudio4.{}", control.raw_value());
        let keys = std::iter::once(field.clone())
            .chain(control.options().iter().map(|option| format!("{field}.{option}")));
        for key in keys {
            assert!(resources::models_string(&key).is_some(), "{key}");
        }
    }
    assert_eq!(
        CinemaControl::CameraAperture.title_of("f14-wide-open"),
        "f/1.4, wide open"
    );
    assert_eq!(
        CinemaControl::ColorPalette.title_of("dont-turn-it-off-im-watching"),
        "Don't Turn It Off, I'm Watching"
    );
    assert_eq!(CinemaControl::Light.title(), "Lighting");
    assert_eq!(CinemaControl::Genre.title_of("space-opera"), "Space opera");
}

#[test]
fn only_cinema_studio_declares_them() {
    let declaring: Vec<String> = VideoModel::all()
        .into_iter()
        .filter(|model| model.declares(VideoControl::CinemaControls))
        .map(|model| model.raw_value().to_owned())
        .collect();
    assert_eq!(declaring, ["cinemaStudio4"]);
}
