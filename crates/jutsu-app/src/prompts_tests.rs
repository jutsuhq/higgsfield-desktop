//! `PromptHistoryTests`, ported.

use time::OffsetDateTime;

use super::*;

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap()
}

fn used(text: &str, seconds: i64) -> PromptUse {
    PromptUse {
        text: text.to_owned(),
        date: at(seconds),
    }
}

fn texts(items: &[PromptHistoryItem]) -> Vec<&str> {
    items.iter().map(|item| item.text.as_str()).collect()
}

#[test]
fn deduplicates_by_trimmed_text_newest_first_and_skips_empty_prompts() {
    let found = items(
        &[
            used("rainy neon", 10),
            used("clay", 20),
            used("  rainy neon\n", 30),
            used("   ", 40),
            used("", 50),
        ],
        &[],
        None,
    );
    assert_eq!(texts(&found), ["rainy neon", "clay"]);
    assert_eq!(found[0].last_used, Some(at(30)));
    assert!(found.iter().all(|item| !item.is_starred));
}

#[test]
fn different_case_is_a_different_prompt() {
    let found = items(&[used("Neon", 1), used("neon", 2)], &[], None);
    assert_eq!(texts(&found), ["neon", "Neon"]);
}

#[test]
fn equal_dates_are_ordered_by_text() {
    let found = items(&[used("b", 5), used("a", 5)], &[], None);
    assert_eq!(texts(&found), ["a", "b"]);
}

#[test]
fn starred_prompts_come_first_most_recently_starred_first() {
    let found = items(
        &[used("old", 1), used("mid", 2), used("new", 3)],
        &[used("old", 100), used(" mid ", 200)],
        None,
    );
    assert_eq!(texts(&found), ["mid", "old", "new"]);
    let stars: Vec<bool> = found.iter().map(|item| item.is_starred).collect();
    assert_eq!(stars, [true, true, false]);
    assert_eq!(found[0].last_used, Some(at(2)));
}

#[test]
fn clearing_drops_uses_up_to_the_date_but_keeps_starred_prompts() {
    let found = items(
        &[
            used("before", 10),
            used("at", 20),
            used("after", 30),
            used("kept", 5),
        ],
        &[used("kept", 6)],
        Some(at(20)),
    );
    assert_eq!(texts(&found), ["kept", "after"]);
    assert!(found[0].is_starred);
    assert_eq!(found[0].last_used, None);
}

#[test]
fn a_prompt_used_again_after_clearing_returns() {
    let found = items(&[used("neon", 10), used("neon", 40)], &[], Some(at(20)));
    assert_eq!(
        found,
        [PromptHistoryItem {
            text: "neon".to_owned(),
            last_used: Some(at(40)),
            is_starred: false,
        }]
    );
}

#[test]
fn search_ignores_case_and_accents_and_matches_every_word() {
    let found = items(
        &[
            used("Café at night, rainy", 3),
            used("Sunny beach", 2),
            used("night city", 1),
        ],
        &[],
        None,
    );
    assert_eq!(texts(&search(&found, "CAFE")), ["Café at night, rainy"]);
    assert_eq!(
        texts(&search(&found, "night")),
        ["Café at night, rainy", "night city"]
    );
    assert_eq!(texts(&search(&found, " rainy  NIGHT ")), ["Café at night, rainy"]);
    assert_eq!(search(&found, "snow").len(), 0);
}

#[test]
fn empty_search_keeps_everything_in_order() {
    let found = items(&[used("a", 1), used("b", 2)], &[used("a", 3)], None);
    assert_eq!(search(&found, "  \n"), found);
}

#[test]
fn cleared_at_is_unset_until_first_clearing() {
    assert_eq!(cleared_at(0.0), None);
    assert_eq!(cleared_at(42.0), Some(at(42)));
}

#[test]
fn search_ignores_marks_in_every_script() {
    let found = items(&[used("Йога на крыше", 2), used("Άλφα beach", 1)], &[], None);
    assert_eq!(texts(&search(&found, "иога")), ["Йога на крыше"]);
    assert_eq!(texts(&search(&found, "αλφα")), ["Άλφα beach"]);
}

#[test]
fn last_use_reads_as_a_named_relative_date() {
    let now = at(10_000_000);
    for (seconds, text) in [
        (0, "now"),
        (30, "30 seconds ago"),
        (60, "1 minute ago"),
        (7_200, "2 hours ago"),
        (90_000, "yesterday"),
        (3 * 86_400, "3 days ago"),
        (8 * 86_400, "last week"),
        (61 * 86_400, "2 months ago"),
        (400 * 86_400, "last year"),
    ] {
        assert_eq!(
            named_relative_time(now - time::Duration::seconds(seconds), now),
            text
        );
    }
}
