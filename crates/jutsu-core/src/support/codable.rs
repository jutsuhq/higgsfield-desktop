//! Shapes of Swift's synthesized enum coding, for the stored forms that keep them.

use serde::{Deserialize, Serialize};

/// An empty payload: Swift codes a case without values as `{}`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Empty {}

/// The single unlabeled value of a Swift enum case, coded as `{"_0": value}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Unlabeled<T> {
    /// The value.
    #[serde(rename = "_0")]
    pub value: T,
}

/// Whether a value is its type's default, for `skip_serializing_if`.
pub fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// Swift's `Data?` fields, coded as base64 text.
pub mod base64_option {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Writes the bytes as base64, or null.
    pub fn serialize<S: Serializer>(data: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error> {
        match data {
            Some(bytes) => serializer.serialize_str(&STANDARD.encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    /// Reads base64 text, or null.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|text| STANDARD.decode(text).map_err(serde::de::Error::custom))
            .transpose()
    }
}
