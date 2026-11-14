//! Prompt history (`PromptHistory.swift`, `PromptHistoryPicker.swift`, `AppEnvironment+PromptHistory.swift`): the
//! prompts of submitted jobs, deduplicated, with starred ones on top and a search that ignores case and accents.
//! Only the stars and the clearing date are stored.

use std::collections::HashMap;
use std::sync::Arc;

use jutsu_store::Store;
use serde::Serialize;
use time::OffsetDateTime;
use unicode_normalization::UnicodeNormalization as _;
use unicode_normalization::char::is_combining_mark;

use crate::preferences::PreferencesStore;

/// A prompt and when it was used or starred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptUse {
    pub text: String,
    pub date: OffsetDateTime,
}

/// One row of the prompt picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptHistoryItem {
    /// Trimmed; also the identity, since history is deduplicated by it.
    pub text: String,
    /// The newest job with this prompt since the last clearing; `None` for a starred prompt with none.
    pub last_used: Option<OffsetDateTime>,
    pub is_starred: bool,
}

/// The text prompts are compared and starred by.
pub fn key(prompt: &str) -> String {
    prompt.trim().to_owned()
}

/// Starred prompts first, most recently starred first, then the rest by last use, newest first. Uses at or before
/// `cleared_at` are dropped; starred prompts stay whatever their date.
pub fn items(
    uses: &[PromptUse],
    starred: &[PromptUse],
    cleared_at: Option<OffsetDateTime>,
) -> Vec<PromptHistoryItem> {
    let mut last_used: HashMap<String, OffsetDateTime> = HashMap::new();
    for prompt in uses {
        let text = key(&prompt.text);
        if text.is_empty() || cleared_at.is_some_and(|cleared| prompt.date <= cleared) {
            continue;
        }
        let date = last_used
            .get(&text)
            .map_or(prompt.date, |known| (*known).max(prompt.date));
        last_used.insert(text, date);
    }
    let mut star_dates: HashMap<String, OffsetDateTime> = HashMap::new();
    for star in starred {
        let text = key(&star.text);
        if text.is_empty() {
            continue;
        }
        let date = star_dates
            .get(&text)
            .map_or(star.date, |known| (*known).max(star.date));
        star_dates.insert(text, date);
    }
    let mut pinned: Vec<(String, OffsetDateTime)> = star_dates
        .iter()
        .map(|(text, date)| (text.clone(), *date))
        .collect();
    pinned.sort_by(newest_first);
    let mut recent: Vec<(String, OffsetDateTime)> = last_used
        .iter()
        .filter(|(text, _)| !star_dates.contains_key(*text))
        .map(|(text, date)| (text.clone(), *date))
        .collect();
    recent.sort_by(newest_first);
    pinned
        .into_iter()
        .map(|(text, _)| PromptHistoryItem {
            last_used: last_used.get(&text).copied(),
            text,
            is_starred: true,
        })
        .chain(recent.into_iter().map(|(text, date)| PromptHistoryItem {
            text,
            last_used: Some(date),
            is_starred: false,
        }))
        .collect()
}

/// Ties on the date are ordered by text so the list doesn't reshuffle between renders.
fn newest_first(lhs: &(String, OffsetDateTime), rhs: &(String, OffsetDateTime)) -> std::cmp::Ordering {
    rhs.1.cmp(&lhs.1).then_with(|| lhs.0.cmp(&rhs.0))
}

/// Items containing every word of `query`, ignoring case and accents, in their order.
pub fn search(items: &[PromptHistoryItem], query: &str) -> Vec<PromptHistoryItem> {
    let words: Vec<String> = query.split_whitespace().map(folded).collect();
    if words.is_empty() {
        return items.to_vec();
    }
    items
        .iter()
        .filter(|item| {
            let text = folded(&item.text);
            words.iter().all(|word| text.contains(word.as_str()))
        })
        .cloned()
        .collect()
}

/// Lowercased with the marks taken off in every script ("Café" → "cafe", "й" → "и", "ά" → "α"), as
/// `.diacriticInsensitive` compares: the canonical decomposition without its combining marks.
pub(crate) fn folded(text: &str) -> String {
    text.to_lowercase()
        .nfd()
        .filter(|letter| !is_combining_mark(*letter))
        .collect()
}

/// When a prompt was last used, as `.relative(presentation: .named)` says it in English: "now", "5 minutes ago",
/// "2 hours ago", "yesterday", "3 days ago", "last week", "last month", "2 years ago". The largest whole unit wins.
// note: units come from the elapsed time (a week is 7 days, a month 30, a year 365), not from calendar dates as
// Foundation counts them, so "yesterday" means 24–48 h ago. Upgrade path: local calendar days if it matters.
pub fn named_relative_time(date: OffsetDateTime, now: OffsetDateTime) -> String {
    let seconds = (now - date).whole_seconds().max(0);
    let days = seconds / 86_400;
    let (count, unit, named) = if days >= 365 {
        (days / 365, "year", "last year")
    } else if days >= 30 {
        (days / 30, "month", "last month")
    } else if days >= 7 {
        (days / 7, "week", "last week")
    } else if days >= 1 {
        (days, "day", "yesterday")
    } else if seconds >= 3_600 {
        (seconds / 3_600, "hour", "1 hour ago")
    } else if seconds >= 60 {
        (seconds / 60, "minute", "1 minute ago")
    } else if seconds >= 1 {
        (seconds, "second", "1 second ago")
    } else {
        return "now".to_owned();
    };
    if count == 1 {
        named.to_owned()
    } else {
        format!("{count} {unit}s ago")
    }
}

/// The stored clearing date (seconds since 1970) as a date; `None` when history was never cleared.
pub fn cleared_at(stored: f64) -> Option<OffsetDateTime> {
    if stored <= 0.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, reason = "seconds since 1970 fit an i64")]
    let seconds = stored.trunc() as i64;
    OffsetDateTime::from_unix_timestamp(seconds).ok()
}

/// One row of the popover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptRow {
    pub text: String,
    /// "2 hours ago"; `None` for a starred prompt not used since the clearing.
    pub last_used: Option<String>,
    pub is_starred: bool,
}

/// The Prompt History popover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptHistoryView {
    pub rows: Vec<PromptRow>,
    /// Shown instead of the rows when there are none.
    pub empty_text: Option<String>,
}

/// Reads the history and stars and unstars prompts.
pub struct PromptService {
    store: Arc<Store>,
    preferences: Arc<PreferencesStore>,
}

impl std::fmt::Debug for PromptService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("PromptService").finish_non_exhaustive()
    }
}

impl PromptService {
    pub fn new(store: Arc<Store>, preferences: Arc<PreferencesStore>) -> Self {
        Self { store, preferences }
    }

    /// The rows matching `query`, starred first.
    pub async fn history(&self, query: &str, now: OffsetDateTime) -> Result<PromptHistoryView, String> {
        let cleared = cleared_at(self.preferences.get().prompt_history_cleared_at.unwrap_or(0.0));
        let uses: Vec<PromptUse> = self
            .store
            .prompt_history(cleared)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|prompt| PromptUse {
                text: prompt.text,
                date: prompt.created_at,
            })
            .collect();
        let stars: Vec<PromptUse> = self
            .store
            .starred_prompts()
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|star| PromptUse {
                text: star.text,
                date: star.starred_at,
            })
            .collect();
        let found = search(&items(&uses, &stars, cleared), query);
        let empty_text = found.is_empty().then(|| {
            if query.is_empty() {
                "Prompts of jobs you submit appear here."
            } else {
                "No prompt matches."
            }
            .to_owned()
        });
        Ok(PromptHistoryView {
            rows: found
                .into_iter()
                .map(|item| PromptRow {
                    last_used: item.last_used.map(|date| named_relative_time(date, now)),
                    text: item.text,
                    is_starred: item.is_starred,
                })
                .collect(),
            empty_text,
        })
    }

    /// Stars or unstars `prompt`; a starred prompt stays on top of the history and survives clearing it.
    pub async fn set_starred(&self, prompt: &str, is_starred: bool) -> Result<(), String> {
        let text = key(prompt);
        if text.is_empty() {
            return Ok(());
        }
        self.store
            .set_prompt_starred(&text, is_starred, OffsetDateTime::now_utc())
            .await
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
#[path = "prompts_tests.rs"]
mod tests;
