//! Swift and Foundation behaviour the rules rely on: closed ranges, dates, text and paths.

pub mod codable;
mod string_enum;
pub(crate) use string_enum::string_enum;
pub mod date;
pub mod range;
pub mod swift_json;
pub mod text;
pub mod url_ext;

/// Seconds of media: a position or a length in a clip, as Swift's `TimeInterval`.
pub type Seconds = f64;

/// A UUID as Swift's `uuidString` writes it: uppercase with hyphens.
pub fn uuid_string(id: uuid::Uuid) -> String {
    id.hyphenated()
        .encode_upper(&mut uuid::Uuid::encode_buffer())
        .to_owned()
}
