//! `ModelResolutionsTests.swift`: `models.json`'s per-model `resolutions`: decoded strictly, 480p, 720p and 1080p when
//! left out, and a job asking for another gets the nearest the model renders at.

use jutsu_core::*;

fn catalog(resolutions: &str) -> VideoModelCatalog {
    let json = format!(
        r#"{{"models": [{{"id": "m", "family": "M", "title": "M", "modes": ["motionTransfer"], {resolutions}
        "request": {{"shape": "genjutsu", "endpoints": {{"default": "e"}}}}}}]}}"#
    );
    VideoModelCatalog::from_json(&json).unwrap()
}

fn model(raw: &str) -> VideoModel {
    VideoModel::from_raw(raw).unwrap()
}

#[test]
fn left_out_it_is_every_resolution() {
    assert_eq!(
        catalog("").get("m").map(|model| model.resolutions.clone()),
        Some(Resolution::STANDARD.to_vec())
    );
}

/// In `Resolution`'s order, whatever the file's.
#[test]
fn listed_it_is_those_in_order() {
    assert_eq!(
        catalog(r#""resolutions": ["1080p", "720p"],"#)
            .get("m")
            .map(|model| model.resolutions.clone()),
        Some(vec![Resolution::P720, Resolution::P1080])
    );
}

/// An entry this version can't run as described is left out, like any other unreadable entry.
#[test]
fn an_unknown_or_empty_list_leaves_the_entry_out() {
    for field in [
        r#""resolutions": [],"#,
        r#""resolutions": ["4K"],"#,
        r#""aspectRatios": ["5:4"],"#,
    ] {
        let catalog = catalog(field);
        assert!(catalog.get("m").is_none(), "{field}");
        assert_eq!(catalog.entry_count(), 1);
    }
}

#[test]
fn fitting_picks_the_nearest_and_the_higher_on_a_tie() {
    let wan27 = model("wan27Reference");
    assert_eq!(wan27.resolutions(), [Resolution::P720, Resolution::P1080]);
    assert_eq!(wan27.fitted_any(Resolution::P480), Resolution::P720);
    assert_eq!(wan27.fitted_any(Resolution::P1080), Resolution::P1080);
    let cinema = model("cinemaStudio4");
    assert_eq!(cinema.fitted_any(Resolution::P1080), Resolution::P720);
    assert_eq!(cinema.fitted_any(Resolution::P480), Resolution::P480);
}

/// MiniMax's tiers: H3 renders at 2K only, H3 Max at 480p and 768p; the others take the nearest by frame height.
#[test]
fn mini_max_has_its_own_tiers() {
    let h3 = model("minimaxH3Reference");
    let max = model("minimaxH3MaxReference");
    assert!(
        h3.resolutions() == [Resolution::K2] && max.resolutions() == [Resolution::P480, Resolution::P768]
    );
    assert_eq!(
        Resolution::STANDARD.map(|resolution| h3.fitted_any(resolution)),
        [Resolution::K2; 3]
    );
    assert_eq!(
        Resolution::STANDARD.map(|resolution| max.fitted_any(resolution)),
        [Resolution::P480, Resolution::P768, Resolution::P768]
    );
    assert!(
        VideoModel::GENJUTSU.fitted_any(Resolution::P768) == Resolution::P720
            && VideoModel::GENJUTSU.fitted_any(Resolution::K2) == Resolution::P1080
    );
    // Gemini Omni Flash 1.2's 360p and 4k frame them.
    assert_eq!(
        Resolution::ALL
            .iter()
            .map(|resolution| resolution.raw_value())
            .collect::<Vec<_>>(),
        ["360p", "480p", "720p", "768p", "1080p", "2K", "4k"]
    );
    assert_eq!(
        Resolution::ALL
            .iter()
            .map(|resolution| resolution.standardized())
            .collect::<Vec<_>>(),
        [
            Resolution::P480,
            Resolution::P480,
            Resolution::P720,
            Resolution::P720,
            Resolution::P1080,
            Resolution::P1080,
            Resolution::P1080
        ]
    );
    assert_eq!(
        JobSpec::new(Mode::MotionTransfer, h3, None, Resolution::P720).resolution,
        Resolution::K2
    );
}

/// A spec is made at a resolution its model renders at.
#[test]
fn a_spec_is_made_at_one_of_the_models_resolutions() {
    assert_eq!(
        JobSpec::new(
            Mode::MotionTransfer,
            model("wan27Reference"),
            None,
            Resolution::P480
        )
        .resolution,
        Resolution::P720
    );
    assert_eq!(
        JobSpec::new(Mode::MotionTransfer, VideoModel::GENJUTSU, None, Resolution::P480).resolution,
        Resolution::P480
    );
}
