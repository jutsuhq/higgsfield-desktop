//! The rules for the names and IDs agents send (`AgentNames`), shared by the helper and the app, and the text
//! measures Swift uses: `Character` (an extended grapheme cluster), Unicode scalars and UTF-8 bytes.

use unicode_segmentation::UnicodeSegmentation;

/// `number` as the messages write it, with a space between thousands: 10 000.
pub fn spaced(number: i64) -> String {
    let mut digits: Vec<char> = number.to_string().chars().collect();
    let mut index = digits.len().saturating_sub(3);
    while index > 0 {
        digits.insert(index, ' ');
        index = index.saturating_sub(3);
    }
    digits.into_iter().collect()
}

pub const REQUEST_ID_RULE: &str =
    "request_id takes 1–64 letters, digits, '.', '_' or '-', not starting with '.'";

/// A request ID: no path separators and no leading dot, so it is also safe as a file or folder name.
pub fn is_valid_request_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && !id.starts_with('.') && id.bytes().all(is_name_byte)
}

/// A model ID as `models.json` spells them.
pub fn is_valid_model_id(id: &str) -> bool {
    is_valid_request_id(id)
}

/// One path component up to 255 bytes that isn't hidden.
pub fn is_valid_file_name(name: &str) -> bool {
    (1..=255).contains(&name.len()) && !name.starts_with('.') && !name.contains('/') && !name.contains('\0')
}

/// 64 lowercase hex digits.
pub fn is_valid_fingerprint(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// An image model's ID: the account's or an API endpoint's, which may have slashes.
pub fn is_valid_image_model_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && !id.starts_with('.')
        && !id.contains("..")
        && id.bytes().all(|byte| is_name_byte(byte) || byte == b'/')
}

/// A `nextCursor`: letters, digits, '_' and '-'.
pub fn is_valid_cursor(cursor: &str) -> bool {
    (1..=96).contains(&cursor.len()) && cursor.bytes().all(|byte| is_name_byte(byte) && byte != b'.')
}

pub(crate) fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
}

/// How many `Character`s `text` has, as Swift's `String.count`.
pub fn character_count(text: &str) -> usize {
    text.graphemes(true).count()
}

/// The first `count` `Character`s of `text`, as Swift's `prefix(_:)`.
pub fn prefix(text: &str, count: usize) -> &str {
    match text.grapheme_indices(true).nth(count) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}

/// How many Unicode scalars `text` has, as Swift's `unicodeScalars.count`.
pub fn scalar_count(text: &str) -> usize {
    text.chars().count()
}

/// `text` without the white space and line breaks around it (`.whitespacesAndNewlines`).
pub fn trim_whitespace_and_newlines(text: &str) -> &str {
    text.trim_matches(char::is_whitespace)
}

/// `text` without the spaces and tabs around it (`.whitespaces`: the space separators and the tab, no line breaks).
pub fn trim_whitespaces(text: &str) -> &str {
    text.trim_matches(is_space)
}

fn is_space(character: char) -> bool {
    character == '\t' || (character.is_whitespace() && !is_newline(character))
}

/// Swift's `Character.isNewline`.
pub fn is_newline(character: char) -> bool {
    matches!(
        character,
        '\n' | '\u{b}' | '\u{c}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

#[cfg(test)]
mod tests {
    use super::{
        is_valid_file_name, is_valid_fingerprint, is_valid_request_id, prefix, spaced, trim_whitespaces,
    };
    use crate::{AgentVideoRequest, MAX_PROMPT_LENGTH, MAX_TITLE_LENGTH};

    /// The limits in the messages are the constants', written as the agents have always read them.
    #[test]
    fn the_messages_spell_their_limits_with_spaced_thousands() {
        assert_eq!(spaced(MAX_PROMPT_LENGTH), "10 000");
        assert_eq!(spaced(MAX_TITLE_LENGTH), "1 000");
        assert!(spaced(999) == "999" && spaced(1_000_000) == "1 000 000");
        let long = AgentVideoRequest {
            prompt: Some("a".repeat(10_001)),
            ..AgentVideoRequest::new("restyle")
        };
        assert_eq!(
            long.problem().as_deref(),
            Some("prompt is at most 10 000 characters")
        );
    }

    #[test]
    fn request_ids_and_file_names_cant_name_paths() {
        assert!(is_valid_request_id("job-1.retry_2"));
        assert!(is_valid_request_id(&"a".repeat(64)));
        for bad in ["", ".hidden", "a/b", "a b", "ä", &"a".repeat(65), "..", "a\0b"] {
            assert!(!is_valid_request_id(bad), "{bad}");
        }
        assert!(is_valid_file_name("My clip (final).mov"));
        for bad in ["", ".partial", "a/b", "x\0", &"a".repeat(256)] {
            assert!(!is_valid_file_name(bad), "{bad}");
        }
        assert!(is_valid_fingerprint(&"0f".repeat(32)));
        assert!(!is_valid_fingerprint(&"0F".repeat(32)));
    }

    #[test]
    fn text_is_cut_by_character() {
        assert_eq!(prefix("e\u{301}xyz", 2), "e\u{301}x");
        assert_eq!(prefix("ab", 5), "ab");
        assert_eq!(trim_whitespaces(" \tMara \n"), "Mara \n");
    }
}
