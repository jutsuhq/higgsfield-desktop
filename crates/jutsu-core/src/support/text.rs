//! Swift's text semantics where the rules depend on them: characters are grapheme clusters, and character sets
//! follow Unicode general categories.
//!
//! The category tables cover the ranges the app meets (Latin, Cyrillic, Greek, Arabic, Hebrew, Indic, CJK and the
//! general punctuation blocks). They are not the full Unicode database Swift reads; a rare script's punctuation or
//! format character outside them reads as an ordinary letter.

use unicode_segmentation::UnicodeSegmentation;

/// How many characters (grapheme clusters) `text` has, as Swift's `String.count`.
pub fn char_count(text: &str) -> usize {
    text.graphemes(true).count()
}

/// The first `count` characters of `text`, as Swift's `String.prefix(_:)`.
pub fn prefix_chars(text: &str, count: usize) -> &str {
    match text.grapheme_indices(true).nth(count) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}

/// `text` without its last character.
pub fn drop_last_char(text: &str) -> &str {
    match text.grapheme_indices(true).next_back() {
        Some((start, _)) => &text[..start],
        None => text,
    }
}

/// Swift's `.newlines`: line feed through carriage return, next line, line and paragraph separators.
pub fn is_newline(c: char) -> bool {
    matches!(c, '\u{000A}'..='\u{000D}' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

/// Swift's `.whitespaces`: space separators and tab.
pub fn is_whitespace(c: char) -> bool {
    c.is_whitespace() && !is_newline(c)
}

/// Swift's `.whitespacesAndNewlines`.
pub fn is_whitespace_or_newline(c: char) -> bool {
    c.is_whitespace()
}

/// `text.trimmingCharacters(in: .whitespaces)`.
pub fn trim_whitespace(text: &str) -> &str {
    text.trim_matches(is_whitespace)
}

/// `text.trimmingCharacters(in: .whitespacesAndNewlines)`.
pub fn trim(text: &str) -> &str {
    text.trim_matches(is_whitespace_or_newline)
}

/// General category Cc (control).
pub fn is_control(c: char) -> bool {
    c.is_control()
}

/// General category Cf (format): soft hyphen, zero-width and direction marks, byte-order mark and tags.
pub fn is_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// General category P (punctuation), as Swift's `.punctuationCharacters`.
pub fn is_punctuation(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_punctuation() && !matches!(c, '$' | '+' | '<' | '=' | '>' | '^' | '`' | '|' | '~');
    }
    matches!(
        c,
        '¡' | '§'
            | '«'
            | '¶'
            | '·'
            | '»'
            | '¿'
            | '\u{037E}'
            | '\u{0387}'
            | '\u{055A}'..='\u{055F}'
            | '\u{0589}'..='\u{058A}'
            | '\u{05BE}'
            | '\u{05C0}'
            | '\u{05C3}'
            | '\u{05C6}'
            | '\u{05F3}'..='\u{05F4}'
            | '\u{0609}'..='\u{060A}'
            | '\u{060C}'..='\u{060D}'
            | '\u{061B}'
            | '\u{061D}'..='\u{061F}'
            | '\u{066A}'..='\u{066D}'
            | '\u{06D4}'
            | '\u{0964}'..='\u{0965}'
            | '\u{0970}'
            | '\u{0E4F}'
            | '\u{0E5A}'..='\u{0E5B}'
            | '\u{10FB}'
            | '\u{1360}'..='\u{1368}'
            | '\u{2010}'..='\u{2027}'
            | '\u{2030}'..='\u{2043}'
            | '\u{2045}'..='\u{2051}'
            | '\u{2053}'..='\u{205E}'
            | '\u{207D}'..='\u{207E}'
            | '\u{208D}'..='\u{208E}'
            | '\u{2308}'..='\u{230B}'
            | '\u{2329}'..='\u{232A}'
            | '\u{2768}'..='\u{2775}'
            | '\u{27C5}'..='\u{27C6}'
            | '\u{27E6}'..='\u{27EF}'
            | '\u{2983}'..='\u{2998}'
            | '\u{29D8}'..='\u{29DB}'
            | '\u{29FC}'..='\u{29FD}'
            | '\u{2CF9}'..='\u{2CFC}'
            | '\u{2CFE}'..='\u{2CFF}'
            | '\u{2E00}'..='\u{2E2E}'
            | '\u{2E30}'..='\u{2E4F}'
            | '\u{2E52}'..='\u{2E5D}'
            | '\u{3001}'..='\u{3003}'
            | '\u{3008}'..='\u{3011}'
            | '\u{3014}'..='\u{301F}'
            | '\u{3030}'
            | '\u{303D}'
            | '\u{30A0}'
            | '\u{30FB}'
            | '\u{FD3E}'..='\u{FD3F}'
            | '\u{FE10}'..='\u{FE19}'
            | '\u{FE30}'..='\u{FE52}'
            | '\u{FE54}'..='\u{FE61}'
            | '\u{FE63}'
            | '\u{FE68}'
            | '\u{FE6A}'..='\u{FE6B}'
            | '\u{FF01}'..='\u{FF03}'
            | '\u{FF05}'..='\u{FF0A}'
            | '\u{FF0C}'..='\u{FF0F}'
            | '\u{FF1A}'..='\u{FF1B}'
            | '\u{FF1F}'..='\u{FF20}'
            | '\u{FF3B}'..='\u{FF3D}'
            | '\u{FF3F}'
            | '\u{FF5B}'
            | '\u{FF5D}'
            | '\u{FF5F}'..='\u{FF65}'
    )
}

/// A `Double` as Swift's `description` writes it: "4.0", "0.5", "1e-05", "1e+16".
pub fn swift_double(value: f64) -> String {
    if !value.is_finite() {
        return if value.is_nan() {
            "nan".to_owned()
        } else if value > 0.0 {
            "inf".to_owned()
        } else {
            "-inf".to_owned()
        };
    }
    let magnitude = value.abs();
    if magnitude == 0.0 || (1e-4..1e16).contains(&magnitude) {
        return format!("{value:?}");
    }
    let scientific = format!("{value:e}");
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let (sign, digits) = exponent
        .strip_prefix('-')
        .map_or(("+", exponent), |digits| ("-", digits));
    format!("{mantissa}e{sign}{digits:0>2}")
}

/// `text` with its first character uppercased and the rest kept, as the app capitalises a fallback title.
pub fn uppercase_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_cuts_by_character() {
        assert_eq!(char_count("e\u{301}🇺🇸"), 2);
        assert_eq!(prefix_chars("e\u{301}🇺🇸x", 2), "e\u{301}🇺🇸");
        assert_eq!(drop_last_char("ab🇺🇸"), "ab");
    }

    #[test]
    fn doubles_read_as_swift_writes_them() {
        assert_eq!(swift_double(4.0), "4.0");
        assert_eq!(swift_double(0.5), "0.5");
        assert_eq!(swift_double(0.000_01), "1e-05");
        assert_eq!(swift_double(1e16), "1e+16");
    }

    #[test]
    fn categories() {
        assert!(is_punctuation(','));
        assert!(!is_punctuation('$'));
        assert!(is_punctuation('—'));
        assert!(is_format('\u{200B}'));
        assert!(is_whitespace('\t'));
        assert!(!is_whitespace('\n'));
        assert_eq!(trim(" \n a \t"), "a");
    }
}
