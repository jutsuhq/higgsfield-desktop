//! The scalar values of the wire that need Swift's spelling: UUIDs in upper case, dates in ISO 8601 to the second,
//! and the string enums.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::fmt;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

/// A UUID as Swift's `UUID` writes it: upper case with hyphens. Read in either case, only in that form.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uuid(uuid::Uuid);

impl Uuid {
    /// A new random UUID.
    pub fn new_v4() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    /// The UUID `text` spells, as `UUID(uuidString:)` reads one: 36 characters with hyphens, in any case.
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        let hyphens = [8, 13, 18, 23];
        if bytes.len() != 36 || hyphens.iter().any(|&index| bytes[index] != b'-') {
            return None;
        }
        uuid::Uuid::try_parse(text).ok().map(Self)
    }
}

impl From<uuid::Uuid> for Uuid {
    fn from(id: uuid::Uuid) -> Self {
        Self(id)
    }
}

impl From<Uuid> for uuid::Uuid {
    fn from(id: Uuid) -> Self {
        id.0
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buffer = uuid::Uuid::encode_buffer();
        formatter.write_str(self.0.hyphenated().encode_upper(&mut buffer))
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Serialize for Uuid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Uuid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| de::Error::custom("invalid UUID string"))
    }
}

/// A moment as Swift's `.iso8601` date strategy writes it: UTC, to the second, `2026-10-21T09:00:00Z`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timestamp(pub OffsetDateTime);

impl Timestamp {
    pub fn now() -> Self {
        Self(OffsetDateTime::now_utc())
    }

    /// Seconds since 1970, as `Date(timeIntervalSince1970:)`.
    pub fn from_unix(seconds: i64) -> Self {
        Self(OffsetDateTime::from_unix_timestamp(seconds).unwrap_or(OffsetDateTime::UNIX_EPOCH))
    }

    /// The wire's form: `ISO8601DateFormatter` with its default options, which takes no fraction of a second.
    pub fn parse_wire(text: &str) -> Option<Self> {
        if text.contains('.') {
            return None;
        }
        OffsetDateTime::parse(text, &Rfc3339).ok().map(Self)
    }

    /// A tool argument: an ISO 8601 date and time, with or without a fraction of a second.
    pub fn parse_argument(text: &str) -> Option<Self> {
        OffsetDateTime::parse(text, &Rfc3339).ok().map(Self)
    }

    /// The wire's form of this moment; a fraction of a second is dropped, as Swift drops it.
    pub fn wire(&self) -> String {
        let utc = self.0.to_offset(UtcOffset::UTC);
        let whole = utc.replace_nanosecond(0).unwrap_or(utc);
        whole.format(&Rfc3339).unwrap_or_default()
    }

    /// This moment plus `seconds`, saturating.
    #[must_use]
    pub fn adding_seconds(self, seconds: f64) -> Self {
        Self(self.0.saturating_add(time::Duration::seconds_f64(seconds)))
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.wire())
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.wire())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse_wire(&text)
            .ok_or_else(|| de::Error::custom("Expected date string to be ISO8601-formatted."))
    }
}

/// An enum carried as one of a fixed set of strings, Swift's `String` raw values: `ALL` in declaration order (what
/// `CaseIterable` gives), `as_str`, `parse`, and serde as the string.
macro_rules! wire_enum {
    ($(#[$meta:meta])* $vis:vis enum $name:ident { $($(#[$variant_meta:meta])* $variant:ident = $raw:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        $vis enum $name {
            $($(#[$variant_meta])* $variant),+
        }

        impl $name {
            /// Every case, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The raw value on the wire.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $raw),+
                }
            }

            /// The case of a raw value.
            pub fn parse(raw: &str) -> Option<Self> {
                match raw {
                    $($raw => Some(Self::$variant),)+
                    _ => None,
                }
            }

            /// Every raw value, in order, joined by ", ".
            pub fn names() -> String {
                Self::ALL.iter().map(|case| case.as_str()).collect::<Vec<_>>().join(", ")
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(deserializer)?;
                Self::parse(&raw).ok_or_else(|| {
                    serde::de::Error::custom(format!("cannot initialize {} from invalid value {raw}", stringify!($name)))
                })
            }
        }
    };
}

pub(crate) use wire_enum;

#[cfg(test)]
mod tests {
    use super::{Timestamp, Uuid};

    #[test]
    fn a_uuid_is_written_in_upper_case_and_read_in_either() {
        let id = Uuid::parse("e621e1f8-c36c-495a-93fc-0c247a3e6e5f");
        assert_eq!(
            id.map(|id| id.to_string()).as_deref(),
            Some("E621E1F8-C36C-495A-93FC-0C247A3E6E5F")
        );
        assert!(Uuid::parse("e621e1f8c36c495a93fc0c247a3e6e5f").is_none());
        assert!(Uuid::parse("not-a-uuid").is_none());
    }

    #[test]
    fn a_date_is_whole_seconds_in_utc() {
        let date = Timestamp::from_unix(1);
        assert_eq!(date.wire(), "1970-01-01T00:00:01Z");
        assert_eq!(Timestamp::parse_wire("1970-01-01T00:00:01Z"), Some(date));
        assert!(Timestamp::parse_wire("2026-10-21T00:00:00.500Z").is_none());
        assert!(Timestamp::parse_argument("2026-10-21T00:00:00.500Z").is_some());
    }
}
