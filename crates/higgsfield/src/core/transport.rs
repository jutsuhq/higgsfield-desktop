//! The one `reqwest` transport of the clients and the app's downloader. Each user names the error a transport
//! failure becomes, so the clients keep returning `HiggsfieldError` and the downloader its own error.

use reqwest::RequestBuilder;
use reqwest::header::HeaderMap;
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::Value;
use url::Url;

use super::HiggsfieldError;

/// An HTTP answer, read whole.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl HttpResponse {
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The header's value, when it is there and readable.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

/// Sends `request` and reads the answer, with a transport failure as `HiggsfieldError::Network`.
pub async fn send(request: RequestBuilder) -> Result<HttpResponse, HiggsfieldError> {
    send_or(request, HiggsfieldError::Network).await
}

/// Sends `request` and reads the answer, with a transport failure (no answer, or a body that broke off) as
/// `network_error`. Dropping the future cancels the request.
pub async fn send_or<E>(request: RequestBuilder, network_error: E) -> Result<HttpResponse, E> {
    let Ok(response) = request.send().await else {
        return Err(network_error);
    };
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let Ok(body) = response.bytes().await else {
        return Err(network_error);
    };
    Ok(HttpResponse {
        status,
        headers,
        body: body.to_vec(),
    })
}

/// Every URL a server hands out must pass this before it is opened, read or downloaded: a `file:` or other
/// scheme from the server must never reach the user's files.
#[must_use]
pub fn is_https(url: &Url) -> bool {
    url.scheme().eq_ignore_ascii_case("https") && url.host_str().is_some_and(|host| !host.is_empty())
}

/// `value` as JSON with its object keys sorted at every level and slashes unescaped, as the Swift clients'
/// `JSONEncoder` (`.sortedKeys`, `.withoutEscapingSlashes`) writes it, whatever map order `serde_json` keeps.
#[must_use]
pub fn sorted_json(value: &Value) -> Vec<u8> {
    serde_json::to_vec(&Sorted(value)).unwrap_or_default()
}

/// `value` as `sorted_json` writes it, pretty-printed as `JSONEncoder`'s `.prettyPrinted` does: two-space indents
/// and `" : "` between a key and its value.
#[must_use]
pub fn sorted_pretty_json(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, SwiftPretty::default());
    match Sorted(value).serialize(&mut serializer) {
        Ok(()) => out,
        Err(_) => Vec::new(),
    }
}

#[derive(Default)]
struct SwiftPretty<'a>(serde_json::ser::PrettyFormatter<'a>);

impl serde_json::ser::Formatter for SwiftPretty<'_> {
    fn begin_array<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.0.begin_array(writer)
    }

    fn end_array<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.0.end_array(writer)
    }

    fn begin_array_value<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        self.0.begin_array_value(writer, first)
    }

    fn end_array_value<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.0.end_array_value(writer)
    }

    fn begin_object<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.0.begin_object(writer)
    }

    fn end_object<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.0.end_object(writer)
    }

    fn begin_object_key<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        self.0.begin_object_key(writer, first)
    }

    fn begin_object_value<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(b" : ")
    }

    fn end_object_value<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.0.end_object_value(writer)
    }
}

struct Sorted<'a>(&'a Value);

impl Serialize for Sorted<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Object(object) => {
                let mut entries: Vec<_> = object.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, &Sorted(value))?;
                }
                map.end()
            }
            Value::Array(items) => serializer.collect_seq(items.iter().map(Sorted)),
            other => other.serialize(serializer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_urls_count_as_server_files() {
        let cases = [
            ("https://cdn.example/a.mp4", true),
            ("HTTPS://cdn.example/a.mp4", true),
            ("http://cdn.example/a.mp4", false),
            ("file:///home/me/Movies/x.mov", false),
            ("ftp://cdn.example/a.mp4", false),
            ("data:video/mp4,abc", false),
        ];
        for (text, expected) in cases {
            assert_eq!(is_https(&Url::parse(text).unwrap()), expected, "{text}");
        }
    }

    #[derive(Debug, PartialEq)]
    struct DownloadFailed;

    /// No HTTP answer is the caller's network error; any HTTP answer is a response, 2xx a success.
    #[tokio::test]
    async fn a_missing_http_response_is_the_callers_network_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let http = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}/a.mp4");
        assert_eq!(send(http.get(&url)).await.unwrap_err(), HiggsfieldError::Network);
        assert_eq!(
            send_or(http.get(&url), DownloadFailed).await.unwrap_err(),
            DownloadFailed
        );
        let response = HttpResponse {
            status: 204,
            headers: HeaderMap::new(),
            body: Vec::new(),
        };
        assert!(response.is_success());
    }
}
