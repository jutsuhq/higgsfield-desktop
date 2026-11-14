//! Dates as the macOS app stores them, and the calendar helpers the rules need.

use serde::{Deserialize, Deserializer, Serializer};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

/// Seconds from 1970-01-01 to 2001-01-01, the reference date of Swift's default `Date` coding.
const SWIFT_REFERENCE_UNIX: i64 = 978_307_200;

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredDate {
    Text(String),
    Seconds(f64),
}

fn read(stored: StoredDate) -> Result<OffsetDateTime, String> {
    match stored {
        StoredDate::Text(text) => OffsetDateTime::parse(&text, &Rfc3339).map_err(|error| error.to_string()),
        StoredDate::Seconds(seconds) => {
            let nanos = ((seconds + SWIFT_REFERENCE_UNIX as f64) * 1e9).round();
            #[allow(
                clippy::cast_possible_truncation,
                reason = "a stored date is far inside the i128 range"
            )]
            OffsetDateTime::from_unix_timestamp_nanos(nanos as i128).map_err(|error| error.to_string())
        }
    }
}

/// Writes a date as RFC 3339 text and reads RFC 3339 text or Swift's default form, seconds since 2001-01-01.
pub mod rfc3339 {
    use super::{Deserialize, Deserializer, OffsetDateTime, Rfc3339, Serializer, StoredDate, read};

    /// Writes `date` as RFC 3339 text.
    pub fn serialize<S: Serializer>(date: &OffsetDateTime, serializer: S) -> Result<S::Ok, S::Error> {
        let text = date.format(&Rfc3339).map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&text)
    }

    /// Reads RFC 3339 text or seconds since 2001-01-01.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<OffsetDateTime, D::Error> {
        read(StoredDate::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// `rfc3339` for an optional date.
pub mod rfc3339_option {
    use super::{Deserialize, Deserializer, OffsetDateTime, Serializer, StoredDate, read};

    /// Writes `date` as RFC 3339 text, or null.
    pub fn serialize<S: Serializer>(date: &Option<OffsetDateTime>, serializer: S) -> Result<S::Ok, S::Error> {
        match date {
            Some(date) => super::rfc3339::serialize(date, serializer),
            None => serializer.serialize_none(),
        }
    }

    /// Reads RFC 3339 text, seconds since 2001-01-01, or null.
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<OffsetDateTime>, D::Error> {
        Option::<StoredDate>::deserialize(deserializer)?
            .map(read)
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}

/// Writes a date as `ISO8601DateFormatter` does, whole seconds in UTC, and reads RFC 3339 text, as Swift's `.iso8601`
/// date strategies.
pub mod iso8601 {
    use super::{Deserialize, Deserializer, OffsetDateTime, Rfc3339, Serializer};

    /// Writes `date` as "2026-03-07T23:30:05Z".
    pub fn serialize<S: Serializer>(date: &OffsetDateTime, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::iso8601_seconds(*date))
    }

    /// Reads RFC 3339 text.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<OffsetDateTime, D::Error> {
        let text = String::deserialize(deserializer)?;
        OffsetDateTime::parse(&text, &Rfc3339).map_err(serde::de::Error::custom)
    }
}

/// `iso8601` for an optional date.
pub mod iso8601_option {
    use super::{Deserialize, Deserializer, OffsetDateTime, Rfc3339, Serializer};

    /// Writes `date` as "2026-03-07T23:30:05Z", or null.
    pub fn serialize<S: Serializer>(date: &Option<OffsetDateTime>, serializer: S) -> Result<S::Ok, S::Error> {
        match date {
            Some(date) => super::iso8601::serialize(date, serializer),
            None => serializer.serialize_none(),
        }
    }

    /// Reads RFC 3339 text or null.
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<OffsetDateTime>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|text| OffsetDateTime::parse(&text, &Rfc3339))
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}

/// The date at `seconds` past the Unix epoch, or the epoch itself for a value out of range.
pub fn from_unix_seconds(seconds: f64) -> OffsetDateTime {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "out-of-range values fall back to the epoch"
    )]
    let nanos = (seconds * 1e9).round() as i128;
    OffsetDateTime::from_unix_timestamp_nanos(nanos).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// Seconds past the Unix epoch, with fractions.
pub fn unix_seconds(date: OffsetDateTime) -> f64 {
    date.unix_timestamp_nanos() as f64 / 1e9
}

/// Whether `a` and `b` fall on the same calendar day at `offset`. Swift uses the calendar of the Mac; a fixed offset
/// stands in for it here.
pub fn same_day(a: OffsetDateTime, b: OffsetDateTime, offset: UtcOffset) -> bool {
    a.to_offset(offset).date() == b.to_offset(offset).date()
}

/// The start of the day holding `date`, at `offset`.
pub fn start_of_day(date: OffsetDateTime, offset: UtcOffset) -> OffsetDateTime {
    date.to_offset(offset).replace_time(time::Time::MIDNIGHT)
}

/// ISO 8601 at whole seconds in UTC, "2026-03-07T23:30:05Z", as Swift's `ISO8601DateFormatter` writes it.
pub fn iso8601_seconds(date: OffsetDateTime) -> String {
    let utc = date.to_offset(UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        utc.year(),
        u8::from(utc.month()),
        utc.day(),
        utc.hour(),
        utc.minute(),
        utc.second()
    )
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use time::macros::datetime;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Holder {
        #[serde(with = "super::rfc3339")]
        at: time::OffsetDateTime,
    }

    #[test]
    fn reads_swift_reference_seconds_and_text() {
        let swift: Holder = serde_json::from_str(r#"{"at": 0}"#).unwrap();
        assert_eq!(swift.at, datetime!(2001-01-01 0:00 UTC));
        let text: Holder = serde_json::from_str(r#"{"at": "2026-03-07T23:30:05Z"}"#).unwrap();
        assert_eq!(super::iso8601_seconds(text.at), "2026-03-07T23:30:05Z");
        let back: Holder = serde_json::from_str(&serde_json::to_string(&text).unwrap()).unwrap();
        assert_eq!(back, text);
    }
}
