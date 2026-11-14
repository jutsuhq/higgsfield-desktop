//! How downloaded results are named.

use time::UtcOffset;
use unicode_segmentation::UnicodeSegmentation;
use url::Url;

use crate::models::Job;
use crate::support::string_enum;
use crate::support::text::{drop_last_char, is_control};
use crate::support::url_ext::url_extension;
use crate::support::uuid_string;

string_enum! {
    /// A placeholder of a pattern.
    pub enum OutputNameToken {
        /// The day the job was made.
        Date = "date",
        /// The time the job was made.
        Time = "time",
        /// The mode.
        Mode = "mode",
        /// The style preset's name.
        Preset = "preset",
        /// The resolution.
        Res = "res",
        /// "draft" for a draft.
        Draft = "draft",
        /// The start of the job's ID.
        Id = "id",
    }
}

impl OutputNameToken {
    /// "{date}".
    pub fn placeholder(self) -> String {
        format!("{{{}}}", self.raw_value())
    }
}

enum Part<'a> {
    Text(&'a str),
    Token(OutputNameToken),
}

/// How downloaded results are named, e.g. `{date}_{mode}_{id}` → `2026-03-07_restyle_1a2b3c4d.mp4`.
/// An empty pattern, or one with an unknown or unclosed token, means `DEFAULT_PATTERN`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutputNamePattern {
    pattern: String,
}

impl OutputNamePattern {
    /// The pattern used when none or an invalid one is set.
    pub const DEFAULT_PATTERN: &'static str = "{date}_{mode}_{id}";
    /// The extension of a result.
    pub const FILE_EXTENSION: &'static str = "mp4";
    /// UTF-8 bytes of the name before the counter and extension; macOS allows 255 per path component.
    pub const MAX_BASE_BYTES: usize = 200;
    /// Characters an empty token must not leave doubled, or at either end of the name.
    const SEPARATORS: [char; 4] = ['_', '-', '.', ' '];

    /// `pattern`, or `DEFAULT_PATTERN` when it isn't valid.
    pub fn new(pattern: &str) -> Self {
        let pattern = if Self::is_valid(pattern) {
            pattern
        } else {
            Self::DEFAULT_PATTERN
        };
        Self {
            pattern: pattern.to_owned(),
        }
    }

    /// The default pattern.
    pub fn standard() -> Self {
        Self::new(Self::DEFAULT_PATTERN)
    }

    /// The pattern in use.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Not empty, with only known and closed tokens.
    pub fn is_valid(pattern: &str) -> bool {
        !crate::support::text::trim(pattern).is_empty() && Self::parse(pattern).is_some()
    }

    /// The sanitized name with its extension, dated at `offset` (the Mac's time zone in Swift). `preset_name` is
    /// `None` when the catalog doesn't know the preset; a name that sanitizes to nothing falls back to the default.
    /// `file_extension` is `None` for `FILE_EXTENSION`.
    pub fn file_name(
        &self,
        job: &Job,
        preset_name: Option<&str>,
        offset: UtcOffset,
        file_extension: Option<&str>,
    ) -> String {
        let mut base = Self::base(&self.expand(job, preset_name, offset));
        if base.is_empty() {
            base = Self::base(&Self::standard().expand(job, preset_name, offset));
        }
        format!("{base}.{}", file_extension.unwrap_or(Self::FILE_EXTENSION))
    }

    /// The lowercased extension of a result's download URL, so an operation's result in another container keeps it
    /// (every one seen live was `.mp4`, a background cut-out included); `FILE_EXTENSION` when the URL names none or a
    /// suspicious one.
    pub fn result_extension(url: &Url) -> String {
        let candidate = url_extension(url).to_lowercase();
        let is_plain = (1..=5).contains(&candidate.chars().count())
            && candidate.chars().all(|c| c.is_ascii_alphanumeric());
        if is_plain {
            candidate
        } else {
            Self::FILE_EXTENSION.to_owned()
        }
    }

    fn base(expanded: &str) -> String {
        Self::sanitize(expanded)
            .trim_matches(|c| Self::SEPARATORS.contains(&c))
            .to_owned()
    }

    /// Safe as one path component: `/` and `:` become `-`, control characters, leading dots and
    /// surrounding spaces go, and the result is cut to `MAX_BASE_BYTES`.
    pub fn sanitize(name: &str) -> String {
        let cleaned: String = name
            .chars()
            .filter(|c| !is_control(*c))
            .map(|c| if c == '/' || c == ':' { '-' } else { c })
            .collect();
        let start = cleaned
            .grapheme_indices(true)
            .find(|(_, grapheme)| !(*grapheme == "." || is_whitespace_grapheme(grapheme)))
            .map_or(cleaned.len(), |(index, _)| index);
        let mut result = cleaned[start..].to_owned();
        while result.len() > Self::MAX_BASE_BYTES {
            let kept = drop_last_char(&result).len();
            result.truncate(kept);
        }
        while result
            .graphemes(true)
            .next_back()
            .is_some_and(is_whitespace_grapheme)
        {
            let kept = drop_last_char(&result).len();
            result.truncate(kept);
        }
        result
    }

    /// `file_name` if `is_taken` rejects it, otherwise the first free `name-2.ext`, `name-3.ext`, …
    pub fn available(file_name: &str, is_taken: impl Fn(&str) -> bool) -> String {
        if !is_taken(file_name) {
            return file_name.to_owned();
        }
        let dot = match file_name.rfind('.') {
            Some(0) | None => file_name.len(),
            Some(dot) => dot,
        };
        let (stem, suffix) = file_name.split_at(dot);
        let mut counter = 2;
        while is_taken(&format!("{stem}-{counter}{suffix}")) {
            counter += 1;
        }
        format!("{stem}-{counter}{suffix}")
    }

    fn parse(pattern: &str) -> Option<Vec<Part<'_>>> {
        let mut parts = Vec::new();
        let mut rest = pattern;
        while let Some(open) = rest.find('{') {
            parts.push(Part::Text(&rest[..open]));
            let close = open + rest[open..].find('}')?;
            parts.push(Part::Token(OutputNameToken::from_raw(&rest[open + 1..close])?));
            rest = &rest[close + 1..];
        }
        parts.push(Part::Text(rest));
        Some(parts)
    }

    fn expand(&self, job: &Job, preset_name: Option<&str>, offset: UtcOffset) -> String {
        let mut result = String::new();
        let mut follows_empty_token = false;
        for part in Self::parse(&self.pattern).unwrap_or_default() {
            match part {
                Part::Text(mut text) => {
                    if text.is_empty() {
                        continue;
                    }
                    if follows_empty_token
                        && let Some(first) = text.chars().next()
                        && Self::SEPARATORS.contains(&first)
                        && result.ends_with(first)
                    {
                        text = &text[first.len_utf8()..];
                    }
                    result.push_str(text);
                    follows_empty_token = false;
                }
                Part::Token(token) => {
                    let value = Self::value(token, job, preset_name, offset);
                    result.push_str(&value);
                    follows_empty_token = value.is_empty();
                }
            }
        }
        result
    }

    fn value(token: OutputNameToken, job: &Job, preset_name: Option<&str>, offset: UtcOffset) -> String {
        let created = job.created_at.to_offset(offset);
        match token {
            OutputNameToken::Date => {
                format!(
                    "{:04}-{:02}-{:02}",
                    created.year(),
                    u8::from(created.month()),
                    created.day()
                )
            }
            OutputNameToken::Time => {
                format!(
                    "{:02}-{:02}-{:02}",
                    created.hour(),
                    created.minute(),
                    created.second()
                )
            }
            OutputNameToken::Mode => job.spec.mode.raw_value().to_owned(),
            OutputNameToken::Preset => match job.spec.preset_id {
                None => String::new(),
                Some(_) => preset_name.unwrap_or("preset").to_owned(),
            },
            OutputNameToken::Res => job.spec.resolution.raw_value().to_owned(),
            OutputNameToken::Draft => if job.spec.is_draft { "draft" } else { "" }.to_owned(),
            OutputNameToken::Id => uuid_string(job.id)[..8].to_lowercase(),
        }
    }
}

/// Swift's `Character.isWhitespace`: the character's first scalar is white space.
fn is_whitespace_grapheme(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(char::is_whitespace)
}
