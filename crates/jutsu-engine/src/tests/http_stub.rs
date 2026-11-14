//! The stub servers the download tests talk to: wiremock answering as the Swift tests' `URLProtocol` stubs do,
//! reached through an `HttpFetch` that sends each https request to the stub over plain HTTP.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use jutsu_core::ProviderError;
use url::Url;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use crate::downloader::{DownloadError, FreeSpace, HttpFetch, HttpReply, ReqwestFetch};

/// Sends `https://<host>/<path>?<query>` to the stub at the same path and query, and hands the answer on as coming
/// from the https URL. A `dropAt=<offset>` query option loses the connection the first time a range starts there,
/// after the stub got the request, as `RangeStub`'s `dropAt` does in Swift.
pub(crate) struct StubFetch {
    server: Url,
    inner: ReqwestFetch,
    dropped: Mutex<HashSet<u64>>,
}

impl StubFetch {
    pub(crate) fn new(server: &MockServer) -> Arc<Self> {
        Arc::new(Self {
            server: Url::parse(&server.uri()).unwrap(),
            inner: ReqwestFetch::new(reqwest::Client::builder().no_proxy().build().unwrap()),
            dropped: Mutex::default(),
        })
    }
}

#[async_trait]
impl HttpFetch for StubFetch {
    async fn get(&self, url: &Url, first: u64, last: u64) -> Result<HttpReply, DownloadError> {
        let mut stub = self.server.clone();
        stub.set_path(url.path());
        stub.set_query(url.query());
        let mut reply = self.inner.get(&stub, first, last).await?;
        reply.head.url = url.clone();
        let drop_at = option(url, "dropAt").and_then(|offset| offset.parse::<u64>().ok());
        if drop_at == Some(first) && self.dropped.lock().unwrap().insert(first) {
            return Err(DownloadError::Provider(ProviderError::Network));
        }
        Ok(reply)
    }
}

/// The free space a test sets: `None` is unknown.
pub(crate) struct Free(pub(crate) Option<u64>);

impl FreeSpace for Free {
    fn free_bytes(&self, _file: &Path) -> Option<u64> {
        self.0
    }
}

fn option(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// What `RangeStub` serves.
pub(crate) const RANGE_BODY: &[u8] = b"video-bytes";

/// Serves `RANGE_BODY` and honors `Range`. Query options: `status=<code>` answers every request with that status,
/// `ignoreRange=1` always sends the whole body, `noTotal=1` answers 206 with an unknown total (`/*`), `empty=1`
/// answers 206 without a body.
struct RangeStub;

impl Respond for RangeStub {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let url = &request.url;
        if let Some(status) = option(url, "status").and_then(|status| status.parse::<u16>().ok()) {
            return ResponseTemplate::new(status);
        }
        let bounds = request
            .headers
            .get("range")
            .and_then(|value| value.to_str().ok())
            .and_then(bounds);
        let Some((lower, upper)) = bounds.filter(|_| option(url, "ignoreRange").is_none()) else {
            return ResponseTemplate::new(200).set_body_bytes(RANGE_BODY);
        };
        let count = RANGE_BODY.len();
        if option(url, "empty").is_some() {
            return ResponseTemplate::new(206)
                .insert_header("Content-Range", format!("bytes {lower}-{upper}/{count}"));
        }
        if lower >= count {
            return ResponseTemplate::new(416).insert_header("Content-Range", format!("bytes */{count}"));
        }
        let end = upper.min(count - 1);
        let total = if option(url, "noTotal").is_none() {
            count.to_string()
        } else {
            "*".to_owned()
        };
        ResponseTemplate::new(206)
            .insert_header("Content-Range", format!("bytes {lower}-{end}/{total}"))
            .set_body_bytes(&RANGE_BODY[lower..=end])
    }
}

/// "bytes=4-7" → (4, 7)
fn bounds(header: &str) -> Option<(usize, usize)> {
    let (lower, upper) = header.strip_prefix("bytes=")?.split_once('-')?;
    let (lower, upper) = (lower.parse().ok()?, upper.parse().ok()?);
    (lower <= upper).then_some((lower, upper))
}

/// Answers every request with the whole body, ignoring `Range`. Query options: `type=<Content-Type>` (none when
/// left out), `status=<code>`, `size=<bytes>` of body (default 12, all 7s).
struct MediaLinkStub;

impl Respond for MediaLinkStub {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let url = &request.url;
        let status = option(url, "status")
            .and_then(|status| status.parse::<u16>().ok())
            .unwrap_or(200);
        let size = option(url, "size")
            .and_then(|size| size.parse::<usize>().ok())
            .unwrap_or(12);
        let mut response = ResponseTemplate::new(status);
        if let Some(content_type) = option(url, "type") {
            response = response.insert_header("Content-Type", content_type);
        }
        if status < 300 {
            response = response.set_body_bytes(vec![7_u8; size]);
        }
        response
    }
}

/// A server answering as Swift's `RangeStub`.
pub(crate) async fn range_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(RangeStub)
        .mount(&server)
        .await;
    server
}

/// A server answering as Swift's `MediaLinkStub`.
pub(crate) async fn media_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(MediaLinkStub)
        .mount(&server)
        .await;
    server
}

/// The `Range` header of every request the server got, in order; `-` for none.
pub(crate) async fn ranges(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|request| {
            request
                .headers
                .get("range")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("-")
                .to_owned()
        })
        .collect()
}
