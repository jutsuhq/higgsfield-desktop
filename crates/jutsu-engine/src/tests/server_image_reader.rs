//! Port of `ServerImageReaderTests`, against a wiremock `MediaLinkStub`. The oversized stream is a fake `HttpFetch`
//! that hands out its body a chunk at a time, since a wiremock answer always says its length and arrives whole.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use jutsu_core::MediaLinkError;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderValue};
use url::Url;
use wiremock::MockServer;

use super::http_stub::{StubFetch, media_server};
use crate::downloader::{DownloadError, HttpFetch, HttpHead, HttpReply};
use crate::{MediaDownloadError, ServerImageReader};

/// Answers with 8 KiB in 1 KiB chunks, each made only when the reader asks for it, and counts what it sent.
struct ImageStream {
    declares_length: bool,
    sent: Arc<AtomicUsize>,
}

#[async_trait]
impl HttpFetch for ImageStream {
    async fn get(&self, url: &Url, _first: u64, _last: u64) -> Result<HttpReply, DownloadError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
        if self.declares_length {
            headers.insert(CONTENT_LENGTH, HeaderValue::from_static("8192"));
        }
        let sent = self.sent.clone();
        let body = futures::stream::unfold(8, move |left| {
            let sent = sent.clone();
            async move {
                if left == 0 {
                    return None;
                }
                sent.fetch_add(1024, Ordering::SeqCst);
                Some((Ok(vec![7_u8; 1024]), left - 1))
            }
        });
        Ok(HttpReply {
            head: HttpHead {
                status: 200,
                headers,
                url: url.clone(),
            },
            body: Box::pin(body),
        })
    }
}

#[tokio::test]
async fn an_oversized_stream_is_stopped_before_its_whole_body_arrives() {
    for declares_length in [true, false] {
        let sent = Arc::new(AtomicUsize::new(0));
        let fetch = Arc::new(ImageStream {
            declares_length,
            sent: sent.clone(),
        });
        let url = Url::parse(&format!("https://example.org/a?length={declares_length}")).unwrap();
        let reader = ServerImageReader::with_fetch(16, fetch);

        assert_eq!(
            reader.data(&url).await,
            Err(MediaDownloadError::Link(MediaLinkError::TooLarge {
                max_bytes: 16
            })),
            "declares its length: {declares_length}"
        );
        assert!(sent.load(Ordering::SeqCst) < 8192);
    }
}

fn reader(server: &MockServer, max_bytes: u64) -> ServerImageReader {
    ServerImageReader::with_fetch(max_bytes, StubFetch::new(server))
}

fn link(query: &str, scheme: &str) -> Url {
    Url::parse(&format!("{scheme}://cdn.example.org/thumbs/a?{query}")).unwrap()
}

#[tokio::test]
async fn an_image_is_read() {
    let server = media_server().await;
    // A generic type is left to the decoder.
    for content_type in ["image/webp", "image/avif", "application/octet-stream"] {
        let data = reader(&server, 1 << 20)
            .data(&link(&format!("type={content_type}"), "https"))
            .await
            .unwrap();
        assert_eq!(data.len(), 12);
    }
}

#[tokio::test]
async fn only_https_is_requested() {
    let server = media_server().await;
    for scheme in ["http", "file", "ftp"] {
        assert_eq!(
            reader(&server, 1 << 20)
                .data(&link("type=image/png", scheme))
                .await,
            Err(MediaDownloadError::Link(MediaLinkError::NotHttps)),
            "{scheme}"
        );
    }
}

#[tokio::test]
async fn a_page_is_refused() {
    let server = media_server().await;
    assert_eq!(
        reader(&server, 1 << 20)
            .data(&link("type=text/html;%20charset=utf-8", "https"))
            .await,
        Err(MediaDownloadError::Link(MediaLinkError::NotMedia {
            content_type: Some("text/html".into())
        }))
    );
}

#[tokio::test]
async fn a_picture_over_the_limit_is_refused() {
    let server = media_server().await;
    assert_eq!(
        reader(&server, 8)
            .data(&link("type=image/png&size=20", "https"))
            .await,
        Err(MediaDownloadError::Link(MediaLinkError::TooLarge {
            max_bytes: 8
        }))
    );
}

#[tokio::test]
async fn a_missing_picture_says_the_status() {
    let server = media_server().await;
    assert_eq!(
        reader(&server, 1 << 20)
            .data(&link("type=image/png&status=404", "https"))
            .await,
        Err(MediaDownloadError::Link(MediaLinkError::Unavailable {
            status: 404
        }))
    );
}
