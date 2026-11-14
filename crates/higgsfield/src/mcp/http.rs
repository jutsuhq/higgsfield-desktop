//! The MCP and OAuth side of the HTTP transport: bounded event-stream reads, form posts and OAuth error bodies.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use reqwest::RequestBuilder;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use url::Url;

use super::error::{McpError, OAuthError};
use super::sse::{MAX_RESPONSE_BYTES, ReadError, read_body, read_event_stream};
use crate::core::{HiggsfieldError, HttpResponse, send};

/// Sends `request`; an event-stream body is read only until `is_complete` accepts its events, and the whole
/// exchange fails as `HiggsfieldError::Network` after `timeout`. A stream the server keeps alive without ever
/// answering would otherwise hold the call forever. A body over 16 MiB fails as `Server` with its status.
pub(crate) async fn send_within(
    request: RequestBuilder,
    timeout: Duration,
    is_complete: impl FnMut(&[String]) -> bool,
) -> Result<HttpResponse, HiggsfieldError> {
    tokio::time::timeout(timeout, send_until(request, is_complete))
        .await
        .unwrap_or(Err(HiggsfieldError::Network))
}

async fn send_until(
    request: RequestBuilder,
    is_complete: impl FnMut(&[String]) -> bool,
) -> Result<HttpResponse, HiggsfieldError> {
    let Ok(response) = request.send().await else {
        return Err(HiggsfieldError::Network);
    };
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let chunks = response.bytes_stream();
    let read = if is_event_stream(&headers) {
        read_event_stream(chunks, MAX_RESPONSE_BYTES, is_complete).await
    } else {
        read_body(chunks, MAX_RESPONSE_BYTES).await
    };
    match read {
        Ok(body) => Ok(HttpResponse {
            status,
            headers,
            body,
        }),
        Err(ReadError::TooLarge) => Err(HiggsfieldError::Server { status }),
        Err(ReadError::Transport) => Err(HiggsfieldError::Network),
    }
}

pub(crate) fn is_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/event-stream"))
}

/// Posts an `application/x-www-form-urlencoded` body; OAuth token and revocation endpoints take this.
pub(crate) async fn post_form(
    http: &reqwest::Client,
    url: &Url,
    parameters: &BTreeMap<String, String>,
) -> Result<HttpResponse, HiggsfieldError> {
    send(
        http.post(url.clone())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(form_encoded(parameters)),
    )
    .await
}

/// The parameters by key, each escaped except for ASCII letters, digits and `-._~`.
pub(crate) fn form_encoded(parameters: &BTreeMap<String, String>) -> String {
    parameters
        .iter()
        .map(|(key, value)| format!("{}={}", escape(key), escape(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            escaped.push(char::from(byte));
        } else {
            let _ = write!(escaped, "%{byte:02X}");
        }
    }
    escaped
}

/// Maps an OAuth error body (`{"error": ..., "error_description": ...}`) or a bare status to an error.
pub(crate) fn oauth_error(response: &HttpResponse) -> McpError {
    #[derive(Deserialize)]
    struct Body {
        error: String,
        error_description: Option<String>,
    }
    match serde_json::from_slice::<Body>(&response.body) {
        Ok(body) if body.error == "access_denied" => OAuthError::AccessDenied.into(),
        Ok(body) => OAuthError::Server {
            code: body.error,
            description: body.error_description,
        }
        .into(),
        Err(_) => McpError::server(response.status),
    }
}

/// The query of `url` as name and value pairs in order, percent-decoded; `+` stays a plus, as in a URI.
pub(crate) fn query_items(url: &Url) -> Vec<(String, Option<String>)> {
    let Some(query) = url.query() else {
        return Vec::new();
    };
    query
        .split('&')
        .map(|item| match item.split_once('=') {
            Some((name, value)) => (percent_decoded(name), Some(percent_decoded(value))),
            None => (percent_decoded(item), None),
        })
        .collect()
}

/// The value of the first query item named `name`.
pub(crate) fn query_value(url: &Url, name: &str) -> Option<String> {
    query_items(url)
        .into_iter()
        .find(|(item, _)| item == name)
        .and_then(|(_, value)| value)
}

fn percent_decoded(text: &str) -> String {
    percent_encoding::percent_decode_str(text)
        .decode_utf8_lossy()
        .into_owned()
}

/// Equal up to a trailing slash, which servers add or drop inconsistently.
pub(crate) fn is_same_resource(url: &Url, other: &Url) -> bool {
    trimming_trailing_slash(url) == trimming_trailing_slash(other)
}

pub(crate) fn trimming_trailing_slash(url: &Url) -> &str {
    let text = url.as_str();
    text.strip_suffix('/').unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_encoding_sorts_keys_and_escapes_reserved_characters() {
        let parameters = BTreeMap::from([
            (
                "redirect_uri".to_owned(),
                "http://127.0.0.1:47613/callback".to_owned(),
            ),
            ("a b".to_owned(), "x+y=z&~._-".to_owned()),
        ]);
        assert_eq!(
            form_encoded(&parameters),
            "a%20b=x%2By%3Dz%26~._-&redirect_uri=http%3A%2F%2F127.0.0.1%3A47613%2Fcallback"
        );
    }

    #[test]
    fn query_items_keep_plus_and_decode_percent_escapes() {
        let url = Url::parse("http://127.0.0.1/callback?code=a+b%2Fc&flag&state=s%20t").unwrap();
        assert_eq!(
            query_items(&url),
            [
                ("code".to_owned(), Some("a+b/c".to_owned())),
                ("flag".to_owned(), None),
                ("state".to_owned(), Some("s t".to_owned())),
            ]
        );
    }

    #[test]
    fn oauth_error_bodies_map_to_errors() {
        let response = |status, body: &str| HttpResponse {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
        };
        assert_eq!(
            oauth_error(&response(400, r#"{"error":"access_denied"}"#)),
            OAuthError::AccessDenied.into()
        );
        assert_eq!(
            oauth_error(&response(
                400,
                r#"{"error":"invalid_grant","error_description":"gone"}"#
            )),
            OAuthError::Server {
                code: "invalid_grant".into(),
                description: Some("gone".into())
            }
            .into()
        );
        assert_eq!(oauth_error(&response(502, "<html>")), McpError::server(502));
    }
}
