//! Keeps secrets out of the log: every line passes through `redact` before it is written, so a token, key or
//! authorization code that reaches a message by mistake is masked rather than stored.

use std::io::{self, Write};
use std::sync::LazyLock;

/// Words after which the rest of the value is secret, in headers (`Bearer x`) and in `key=value` or JSON pairs.
const SECRET_NAMES: &[&str] = &[
    "access_token",
    "refresh_token",
    "id_token",
    "accessToken",
    "refreshToken",
    "client_secret",
    "secret",
    "code",
    "code_verifier",
    "password",
    "api_key",
    "apiKey",
    "signature",
    "X-Amz-Signature",
    "token",
];
const SCHEMES: &[&str] = &["Bearer ", "Basic ", "Key "];
const MASK: &str = "[redacted]";

static NAMES: LazyLock<Vec<String>> = LazyLock::new(|| {
    SECRET_NAMES
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect()
});

fn is_value_end(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '&' | ',' | '"' | '\'' | '}' | ')' | ']' | ';' | '<' | '>'
        )
}

/// `line` with every secret value replaced by `[redacted]`.
pub fn redact(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        if let Some((prefix_len, value_start)) = secret_at(rest, out.chars().next_back()) {
            out.push_str(&rest[..prefix_len]);
            let after = &rest[value_start..];
            let value_len = after.find(is_value_end).unwrap_or(after.len());
            out.push_str(&rest[prefix_len..value_start]);
            if value_len > 0 {
                out.push_str(MASK);
            }
            rest = &after[value_len..];
            continue;
        }
        let mut characters = rest.chars();
        if let Some(character) = characters.next() {
            out.push(character);
        }
        rest = characters.as_str();
    }
    out
}

/// When `text` starts with a secret's name and separator (or an auth scheme), the length of the name and where its
/// value starts. `previous` keeps names from matching inside longer words (`postcode=`).
fn secret_at(text: &str, previous: Option<char>) -> Option<(usize, usize)> {
    if previous.is_some_and(|character| character.is_alphanumeric() || character == '_') {
        return None;
    }
    for scheme in SCHEMES {
        if text.len() >= scheme.len()
            && text.is_char_boundary(scheme.len())
            && text[..scheme.len()].eq_ignore_ascii_case(scheme)
        {
            return Some((scheme.len(), scheme.len()));
        }
    }
    let name_len =
        text.find(|character: char| !(character.is_alphanumeric() || character == '_' || character == '-'))?;
    if !NAMES.contains(&text[..name_len].to_ascii_lowercase()) {
        return None;
    }
    // `name=value`, `name: value`, `"name":"value"`, `name="value"`.
    let after = &text[name_len..];
    let skipped = after.len() - after.trim_start_matches(['"', '\'']).len();
    let separator = after[skipped..].chars().next()?;
    if separator != '=' && separator != ':' {
        return None;
    }
    let value = &after[skipped + 1..];
    let gap = value.len() - value.trim_start_matches([' ', '"', '\'']).len();
    Some((name_len, name_len + skipped + 1 + gap))
}

/// A writer that redacts each write before passing it on, for `tracing_subscriber`'s `with_writer`.
#[derive(Debug)]
pub struct RedactingWriter<W> {
    inner: W,
}

impl<W> RedactingWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buffer);
        self.inner.write_all(redact(&text).as_bytes())?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_headers_are_masked() {
        assert_eq!(
            redact("Authorization: Bearer abc.def-ghi end"),
            "Authorization: Bearer [redacted] end"
        );
        assert_eq!(
            redact("authorization=Key id123:sec456"),
            "authorization=Key [redacted]"
        );
    }

    #[test]
    fn query_and_json_values_are_masked() {
        assert_eq!(
            redact("GET /callback?code=xyz&state=s1"),
            "GET /callback?code=[redacted]&state=s1"
        );
        assert_eq!(
            redact(r#"{"access_token":"a1","refresh_token": "r1","expires_in":3600}"#),
            r#"{"access_token":"[redacted]","refresh_token": "[redacted]","expires_in":3600}"#
        );
        assert_eq!(redact("secret=hunter2 id=7"), "secret=[redacted] id=7");
        assert_eq!(
            redact("https://cdn/x.mp4?X-Amz-Signature=deadbeef&a=1"),
            "https://cdn/x.mp4?X-Amz-Signature=[redacted]&a=1"
        );
    }

    #[test]
    fn ordinary_text_is_untouched() {
        for line in [
            "postcode=12345",
            "connected to Higgsfield",
            "no secrets here: 42",
            "code review done",
            "unicode ✓ fine",
        ] {
            assert_eq!(redact(line), line);
        }
    }

    #[test]
    fn the_writer_redacts_what_it_writes() {
        let mut buffer = Vec::new();
        let mut writer = RedactingWriter::new(&mut buffer);
        writer.write_all(b"token=t0p\n").unwrap();
        assert_eq!(String::from_utf8(buffer).unwrap(), "token=[redacted]\n");
    }
}
