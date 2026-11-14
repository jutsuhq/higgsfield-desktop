//! `SavedImageTests.swift`.

use jutsu_core::*;
use time::macros::datetime;
use time::{OffsetDateTime, UtcOffset};

/// 2026-10-15 10:41:03 UTC.
const DATE: OffsetDateTime = datetime!(2026-10-15 10:41:03 UTC);

fn name(prompt: &str) -> String {
    SavedImage::base_name(prompt, DATE, UtcOffset::UTC)
}

#[test]
fn the_name_starts_with_the_date_then_the_prompt() {
    assert_eq!(
        name("A knight in silver armor"),
        "2026-10-15 10.41.03 A knight in silver armor"
    );
}

#[test]
fn a_long_prompt_is_cut_at_a_word() {
    let prompt =
        "The Great Hall is grand and magical, with long wooden tables, floating candles and Gothic arches";
    assert_eq!(
        name(prompt),
        "2026-10-15 10.41.03 The Great Hall is grand and magical, with long"
    );
}

#[test]
fn separators_and_leading_dots_dont_reach_the_file_name() {
    assert_eq!(
        name("..a/b:c\\d\nnext line"),
        "2026-10-15 10.41.03 a-b-c-d next line"
    );
    assert_eq!(name("   "), "2026-10-15 10.41.03");
}
