//! Cinema Studio 4.0's creative controls.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::resources::models_string;
use crate::support::text::uppercase_first;

/// One of Cinema Studio 4.0's creative controls, by its field in the REST body (`higgsfield/cinema-studio/4.0`). A
/// job sets it to one of the schema's values or leaves it out, and the director picks it then: the schema takes no
/// "auto". The account's `generate_video` names these by IDs its catalog doesn't list, so only an API key sends them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum CinemaControl {
    /// `camera_model`.
    CameraModel,
    /// `camera_lens`.
    CameraLens,
    /// `camera_aperture`.
    CameraAperture,
    /// `camera_movement`.
    CameraMovement,
    /// `genre`.
    Genre,
    /// `era`.
    Era,
    /// `light`.
    Light,
    /// `pacing`.
    Pacing,
    /// `color_palette`.
    ColorPalette,
}

const MOVEMENTS: &[&str] = &[
    "aerial-pullback",
    "arc-left",
    "arc-right",
    "bullet-time",
    "crane-down",
    "crane-up",
    "crush-zoom",
    "dolly-in",
    "dolly-out",
    "dolly-zoom",
    "drone-orbit",
    "handheld",
    "helicopter-shot",
    "pan-left",
    "pan-right",
    "pedestal-down",
    "pedestal-up",
    "pov",
    "rack-focus",
    "robot-arm",
    "side-tracking",
    "slider-left",
    "slider-right",
    "slow-zoom-in",
    "slow-zoom-out",
    "snorricam",
    "static-shot",
    "tilt-down",
    "tilt-up",
    "tracking",
    "truck-left",
    "truck-right",
    "whip-pan",
];

const PALETTES: &[&str] = &[
    "a-dream-in-color",
    "a-hotel-for-one",
    "after-dark",
    "amber-wasteland",
    "back-row-kissing-seats",
    "bioluminescent-night",
    "breakfast-on-schedule",
    "bubblegum-boulevard",
    "crimson-vigi",
    "dont-turn-it-off-im-watching",
    "everyone-speaks-in-whispers",
    "favela-gold",
    "field-post",
    "gasoline-sunset",
    "ghost-in-the-code",
    "glossy-flesh",
    "highway-standoff",
    "home-is-the-next-gas-station",
    "industrial-fog",
    "mirage-at-noon",
    "neon-rain-at-midnight",
    "oil-ochre",
    "on-the-other-side-of-the-porthole",
    "overtime",
    "pink-velvet",
    "playtime",
    "runaway-summer",
    "stairs-go-up",
    "static-noon",
    "the-butterfly",
    "the-circus",
    "the-crimson-ballet",
    "the-earth-keeps-things-reluctantly",
    "the-emerald-ambush",
    "the-faded-fresco",
    "the-grey-channel",
    "the-ground",
    "the-investigation",
    "the-iron-borough",
    "the-morning-after-rain",
    "the-mountain-convent",
    "the-neighbors-saw-everything",
    "the-silk-curtain-falls",
    "the-way-home-is-longer",
    "tropic-fever-dream",
    "turquoise-mirage",
    "twilight-fable",
    "two-days-to-the-horizon",
    "wallpaper-romance",
    "yellow-room",
];

impl CinemaControl {
    /// Every control, in order.
    pub const ALL: [Self; 9] = [
        Self::CameraModel,
        Self::CameraLens,
        Self::CameraAperture,
        Self::CameraMovement,
        Self::Genre,
        Self::Era,
        Self::Light,
        Self::Pacing,
        Self::ColorPalette,
    ];

    /// The REST body's field.
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::CameraModel => "camera_model",
            Self::CameraLens => "camera_lens",
            Self::CameraAperture => "camera_aperture",
            Self::CameraMovement => "camera_movement",
            Self::Genre => "genre",
            Self::Era => "era",
            Self::Light => "light",
            Self::Pacing => "pacing",
            Self::ColorPalette => "color_palette",
        }
    }

    /// The control of the field `raw`.
    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|control| control.raw_value() == raw)
    }

    /// The schema's values (docs.higgsfield.ai, 2026-10-15). The camera, lens and aperture go from the cleanest to
    /// the most stylized; the movements and palettes by name.
    pub fn options(self) -> &'static [&'static str] {
        match self {
            Self::CameraModel => &["modern", "35mm-film", "8mm-film", "dv-camcorder"],
            Self::CameraLens => &[
                "clean-sharp",
                "anamorphic",
                "vintage-anamorphic",
                "warm-vintage",
                "halation-vintage",
            ],
            Self::CameraAperture => &["f14-wide-open", "f4-moderate", "f11-deep-focus"],
            Self::CameraMovement => MOVEMENTS,
            Self::Genre => &["action", "comedy", "drama", "epic", "horror", "noir"],
            Self::Era => &["1960s", "1980s", "1990s", "2000s", "2020s"],
            Self::Light => &[
                "window",
                "practicals",
                "soft-cross",
                "overhead-fall",
                "contre-jour",
                "silhouette",
            ],
            Self::Pacing => &["calm", "dynamic", "chaotic", "single-shot"],
            Self::ColorPalette => PALETTES,
        }
    }

    /// "Camera", "Lens" and so on: the `cinemaStudio4.<field>` key in `Models.xcstrings`.
    pub fn title(self) -> String {
        let fallback = match self {
            Self::CameraModel => "Camera",
            Self::CameraLens => "Lens",
            Self::CameraAperture => "Aperture",
            Self::CameraMovement => "Movement",
            Self::Genre => "Genre",
            Self::Era => "Era",
            Self::Light => "Lighting",
            Self::Pacing => "Pacing",
            Self::ColorPalette => "Color palette",
        };
        localized(self.raw_value(), fallback)
    }

    /// The title of one of `options`: the `cinemaStudio4.<field>.<value>` key in `Models.xcstrings`, which holds
    /// the English; the value with spaces for hyphens stands in for a key it lacks.
    pub fn title_of(self, option: &str) -> String {
        let words = option.replace('-', " ");
        localized(
            &format!("{}.{option}", self.raw_value()),
            &uppercase_first(&words),
        )
    }
}

fn localized(key: &str, fallback: &str) -> String {
    models_string(&format!("cinemaStudio4.{key}"))
        .unwrap_or(fallback)
        .to_owned()
}

/// The creative controls a Cinema Studio job sets (`CinemaControl`); the director picks each one it leaves out.
///
/// Coded as the body's fields. A field or value this version doesn't know is dropped, so a job or a template
/// stored by another version still reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "specta", specta(type = BTreeMap<String, String>))]
#[serde(into = "BTreeMap<String, String>", from = "BTreeMap<String, String>")]
pub struct CinemaControls {
    values: BTreeMap<CinemaControl, String>,
}

impl CinemaControls {
    /// No control set.
    pub fn new() -> Self {
        Self::default()
    }

    /// The controls in `values`, each kept only with a value it has.
    pub fn from_values(values: impl IntoIterator<Item = (CinemaControl, String)>) -> Self {
        let mut controls = Self::new();
        for (control, value) in values {
            controls.set(control, Some(value));
        }
        controls
    }

    /// `None` leaves the control to the director.
    pub fn get(&self, control: CinemaControl) -> Option<&str> {
        self.values.get(&control).map(String::as_str)
    }

    /// Sets a control. A value the control doesn't have (`CinemaControl::options`) clears it rather than reaching a
    /// schema that refuses it.
    pub fn set(&mut self, control: CinemaControl, value: Option<String>) {
        match value.filter(|value| control.options().contains(&value.as_str())) {
            Some(value) => {
                self.values.insert(control, value);
            }
            None => {
                self.values.remove(&control);
            }
        }
    }

    /// How many controls are set.
    pub fn count(&self) -> usize {
        self.values.len()
    }

    /// Whether no control is set.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The REST body's fields, by key.
    pub fn fields(&self) -> BTreeMap<String, String> {
        self.values
            .iter()
            .map(|(control, value)| (control.raw_value().to_owned(), value.clone()))
            .collect()
    }
}

impl From<CinemaControls> for BTreeMap<String, String> {
    fn from(controls: CinemaControls) -> Self {
        controls.fields()
    }
}

impl From<BTreeMap<String, String>> for CinemaControls {
    fn from(fields: BTreeMap<String, String>) -> Self {
        Self::from_values(
            fields
                .into_iter()
                .filter_map(|(key, value)| CinemaControl::from_raw(&key).map(|control| (control, value))),
        )
    }
}
