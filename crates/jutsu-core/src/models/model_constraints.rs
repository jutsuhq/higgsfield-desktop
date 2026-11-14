//! What the server says each model takes, over the built-in constants.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::models::{Mode, ModeLimits, VideoModel};
use crate::support::Seconds;
use crate::support::range::ClosedRange;

/// What the server says one model takes, over the constants in `VideoModel::limits`. Each bound is optional: a
/// missing one keeps the constant, and so does a pair that contradicts itself (a minimum above the maximum).
/// Decoded tolerantly, since it comes from a cache of `models_explore` answers whose shape no export has confirmed
/// yet: a field that is missing or of another type is `None`, never an error.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ModelConstraints {
    /// Seconds of source one job takes, or for a clip made without one, the seconds it runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_duration: Option<Seconds>,
    /// The longest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<Seconds>,
    /// The fewest reference images.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_reference_images: Option<i64>,
    /// The most reference images.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_reference_images: Option<i64>,
}

impl ModelConstraints {
    /// The bounds that make sense: a length above 0 s and at most an hour, a count of 0 or more.
    pub fn new(
        min_duration: Option<Seconds>,
        max_duration: Option<Seconds>,
        min_reference_images: Option<i64>,
        max_reference_images: Option<i64>,
    ) -> Self {
        Self {
            min_duration: seconds(min_duration),
            max_duration: seconds(max_duration),
            min_reference_images: count(min_reference_images),
            max_reference_images: count(max_reference_images),
        }
    }

    /// `limits` with the bounds this has, where they make a range.
    pub fn applied(&self, limits: ModeLimits) -> ModeLimits {
        let mut limits = limits;
        let duration = (
            self.min_duration.unwrap_or(limits.duration.lower()),
            self.max_duration.unwrap_or(limits.duration.upper()),
        );
        if duration.0 <= duration.1 {
            limits.duration = ClosedRange::new(duration.0, duration.1);
        }
        let images = (
            self.min_reference_images
                .map_or(limits.reference_images.lower(), |count| count as usize),
            self.max_reference_images
                .map_or(limits.reference_images.upper(), |count| count as usize),
        );
        if images.0 <= images.1 {
            limits.reference_images = ClosedRange::new(images.0, images.1);
        }
        limits
    }
}

fn seconds(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0 && *value <= 3600.0)
}

fn count(value: Option<i64>) -> Option<i64> {
    value.filter(|value| *value >= 0)
}

impl<'de> Deserialize<'de> for ModelConstraints {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let Value::Object(fields) = value else {
            return Err(serde::de::Error::custom("expected an object"));
        };
        let number = |key: &str| fields.get(key).and_then(Value::as_f64);
        let integer = |key: &str| fields.get(key).and_then(Value::as_i64);
        Ok(Self::new(
            number("minDuration"),
            number("maxDuration"),
            integer("minReferenceImages"),
            integer("maxReferenceImages"),
        ))
    }
}

/// One model and mode's constraints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ModelConstraintEntry {
    /// The model.
    pub model: VideoModel,
    /// The mode.
    pub mode: Mode,
    /// What the server says.
    pub constraints: ModelConstraints,
}

/// The server's constraints of the models the account runs, by model and mode; an empty table (`built_in`) leaves
/// every limit on its constant. Injected wherever limits are read (`VideoModel::limits`), so the rules stay pure:
/// the app reads it from a cache, never on a form edit.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ModelConstraintTable {
    entries: Vec<ModelConstraintEntry>,
}

impl<'de> Deserialize<'de> for ModelConstraintTable {
    /// An entry of an unknown model or mode, or that can't be read, is dropped; the others are kept.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Stored {
            #[serde(default)]
            entries: Option<Vec<Value>>,
        }
        let stored = Stored::deserialize(deserializer)?;
        let entries = stored
            .entries
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| serde_json::from_value(entry).ok());
        Ok(Self {
            entries: entries.collect(),
        })
    }
}

impl ModelConstraintTable {
    /// The constants alone.
    pub fn built_in() -> Self {
        Self::default()
    }

    /// A table of `entries`.
    pub fn new(entries: Vec<ModelConstraintEntry>) -> Self {
        Self { entries }
    }

    /// Its entries.
    pub fn entries(&self) -> &[ModelConstraintEntry] {
        &self.entries
    }

    /// The first entry for `model` in `mode`.
    pub fn get(&self, model: &VideoModel, mode: Mode) -> Option<ModelConstraints> {
        self.entries
            .iter()
            .find(|entry| entry.model == *model && entry.mode == mode)
            .map(|entry| entry.constraints)
    }

    /// Where these limits differ from the constants, one line per model and mode, for a Debug log that notices
    /// server changes. Empty when they agree.
    pub fn differences_from_built_in(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|entry| {
                let constant = entry.model.limits(entry.mode, true, &Self::built_in());
                let server = entry.model.limits(entry.mode, true, self);
                if constant == server {
                    return None;
                }
                Some(format!(
                    "{} {}: duration {} (built in {}), images {} (built in {})",
                    entry.model.raw_value(),
                    entry.mode.raw_value(),
                    server.duration,
                    constant.duration,
                    server.reference_images,
                    constant.reference_images
                ))
            })
            .collect()
    }
}
