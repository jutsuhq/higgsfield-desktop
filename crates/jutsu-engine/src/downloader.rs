//! Fetching a finished result (or any file a server hands out) in resumable chunks.

use std::io::{self, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use jutsu_core::{MediaLinkError, ProviderError};
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, HeaderMap, RANGE};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use url::Url;

/// Fetches a finished video.
#[async_trait]
pub trait Downloader: Send + Sync {
    /// Downloads `url` into `file`. Bytes already in `file` from an interrupted call are kept and only the rest is
    /// fetched, so calling again resumes. Fails with `DownloadError::Provider` (network, 5xx) for failures worth
    /// retrying.
    async fn download(&self, url: &Url, file: &Path) -> Result<(), DownloadError>;
}

/// Why a download failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DownloadError {
    /// The network failed, or the server failed (5xx): worth retrying.
    #[error(transparent)]
    Provider(ProviderError),
    /// The URL answered with a client error (4xx); the caller decides whether waiting can help.
    #[error("The server refused to send the file (HTTP {status}).")]
    Failed {
        /// The HTTP status.
        status: u16,
    },
    /// The download was stopped; retrying won't help.
    #[error("{0}")]
    Refused(DownloadRefusal),
    /// The file couldn't be written; a full disk is `StorageFull` or `QuotaExceeded`.
    #[error("{message}")]
    Io {
        /// What kind of failure.
        kind: io::ErrorKind,
        /// Its description.
        message: String,
    },
    /// The caller's check of the response refused it.
    #[error(transparent)]
    NotAccepted(MediaLinkError),
    /// The download was cancelled.
    #[error("cancelled")]
    Cancelled,
}

impl From<io::Error> for DownloadError {
    fn from(error: io::Error) -> Self {
        Self::Io {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

/// Why a download was stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DownloadRefusal {
    /// Not an `https:` address.
    NotHttps,
    /// Larger than allowed.
    TooLarge {
        /// The limit.
        max_bytes: i64,
    },
    /// A partial answer without a usable total size, or one that brought nothing.
    Stalled,
}

impl DownloadRefusal {
    /// The macOS app's wording.
    pub fn message(self) -> String {
        match self {
            Self::NotHttps => {
                "The file isn't on a secure (https) address, so it wasn't downloaded.".to_owned()
            }
            Self::TooLarge { max_bytes } => {
                format!(
                    "The file is larger than {}, so it wasn't downloaded.",
                    file_size(max_bytes)
                )
            }
            Self::Stalled => "The server stopped sending the file. Try again later.".to_owned(),
        }
    }
}

impl std::fmt::Display for DownloadRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

/// A size as Foundation's `ByteCountFormatter` writes a file size: decimal units, whole kilobytes, one decimal of a
/// megabyte and two of a gigabyte or more, without trailing zeros.
pub(crate) fn file_size(bytes: i64) -> String {
    if bytes.abs() < 1000 {
        return if bytes == 1 {
            "1 byte".to_owned()
        } else {
            format!("{bytes} bytes")
        };
    }
    #[allow(clippy::cast_precision_loss, reason = "a file size, shown rounded")]
    let mut value = bytes as f64 / 1000.0;
    let units = ["KB", "MB", "GB", "TB", "PB"];
    let mut unit = 0;
    while value.abs() >= 1000.0 && unit < units.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    let decimals = match unit {
        0 => 0,
        1 => 1,
        _ => 2,
    };
    let text = format!("{value:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    format!("{text} {}", units[unit])
}

/// The bytes free for a file, `None` when unknown.
pub trait FreeSpace: Send + Sync {
    /// The bytes free on the volume `file` is written to.
    fn free_bytes(&self, file: &Path) -> Option<u64>;
}

/// The free space the system reports for the volume of a file's folder.
#[derive(Debug, Clone, Copy, Default)]
pub struct VolumeFreeSpace;

impl FreeSpace for VolumeFreeSpace {
    fn free_bytes(&self, file: &Path) -> Option<u64> {
        fs4::available_space(file.parent().unwrap_or(file)).ok()
    }
}

/// An answer's status line and headers, before its body.
#[derive(Debug, Clone)]
pub struct HttpHead {
    /// The HTTP status.
    pub status: u16,
    /// The headers.
    pub headers: HeaderMap,
    /// Where the answer came from, after redirects.
    pub url: Url,
}

impl HttpHead {
    /// The header's value, when it is there and readable.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// `Content-Length`, when the answer says it.
    pub fn content_length(&self) -> Option<u64> {
        self.header(CONTENT_LENGTH.as_str())
            .and_then(|value| value.trim().parse().ok())
    }
}

/// An answer whose body arrives in pieces.
pub struct HttpReply {
    /// The status and headers.
    pub head: HttpHead,
    /// The body; an item that fails is a connection that broke off.
    pub body: BoxStream<'static, Result<Vec<u8>, ()>>,
}

/// One HTTP `GET` of a byte range: the seam tests replace, as Swift's tests replace `URLProtocol`.
#[async_trait]
pub trait HttpFetch: Send + Sync {
    /// Asks for bytes `first…last` of `url`. No answer at all is `DownloadError::Provider(Network)`.
    async fn get(&self, url: &Url, first: u64, last: u64) -> Result<HttpReply, DownloadError>;
}

/// `HttpFetch` over `reqwest`.
#[derive(Debug, Clone, Default)]
pub struct ReqwestFetch {
    client: reqwest::Client,
}

impl ReqwestFetch {
    /// Requests through `client`.
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl HttpFetch for ReqwestFetch {
    async fn get(&self, url: &Url, first: u64, last: u64) -> Result<HttpReply, DownloadError> {
        let response = self
            .client
            .get(url.clone())
            .header(RANGE, format!("bytes={first}-{last}"))
            .send()
            .await
            .map_err(|_| DownloadError::Provider(ProviderError::Network))?;
        let head = HttpHead {
            status: response.status().as_u16(),
            headers: response.headers().clone(),
            url: response.url().clone(),
        };
        let body = response
            .bytes_stream()
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()).map_err(|_| ()))
            .boxed();
        Ok(HttpReply { head, body })
    }
}

/// Sees every successful response before its body is written, and refuses it with an error.
pub type Accept<'a> = &'a (dyn Fn(&HttpHead) -> Result<(), DownloadError> + Send + Sync);
/// The bytes in the file so far and the whole file's size when the server says it, after each write.
pub type Progress<'a> = &'a (dyn Fn(u64, Option<u64>) + Send + Sync);

/// Fetches the file in `Range` requests of `chunk_size` bytes and streams each one to the end of the file, so a
/// failure loses at most one chunk and no body is held in memory. A server that ignores `Range` sends the whole
/// file at once. Only https URLs are fetched, and never more than `max_bytes` in all. A response whose remaining
/// size won't fit on the disk fails with `StorageFull` before anything is written.
#[derive(Clone)]
pub struct HttpDownloader {
    fetch: Arc<dyn HttpFetch>,
    chunk_size: u64,
    max_attempts: u32,
    max_bytes: u64,
    free_space: Arc<dyn FreeSpace>,
}

impl HttpDownloader {
    /// A downloader of at most `max_bytes`, in chunks of 8 MiB, giving up after 3 failures in a row.
    pub fn new(max_bytes: u64) -> Self {
        Self {
            fetch: Arc::new(ReqwestFetch::default()),
            chunk_size: 8 << 20,
            max_attempts: 3,
            max_bytes,
            free_space: Arc::new(VolumeFreeSpace),
        }
    }

    /// Requests through `fetch`.
    pub fn with_fetch(mut self, fetch: Arc<dyn HttpFetch>) -> Self {
        self.fetch = fetch;
        self
    }

    /// Chunks of `chunk_size` bytes, at least 1.
    pub fn with_chunk_size(mut self, chunk_size: u64) -> Self {
        self.chunk_size = chunk_size.max(1);
        self
    }

    /// The free space checked before writing.
    pub fn with_free_space(mut self, free_space: Arc<dyn FreeSpace>) -> Self {
        self.free_space = free_space;
        self
    }

    /// `download`, plus `accept`, which sees every successful response before its body is written, and `progress`.
    pub async fn download_with(
        &self,
        url: &Url,
        file: &Path,
        accept: Accept<'_>,
        progress: Progress<'_>,
    ) -> Result<(), DownloadError> {
        if !higgsfield::core::is_https(url) {
            return Err(DownloadError::Refused(DownloadRefusal::NotHttps));
        }
        let mut handle = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(file)
            .await?;
        let mut failures = 0;
        loop {
            match self.fetch_next(url, file, &mut handle, accept, progress).await {
                Ok(true) => return Ok(()),
                Ok(false) => failures = 0,
                Err(DownloadError::Provider(_)) if failures + 1 < self.max_attempts => failures += 1,
                Err(error) => return Err(error),
            }
        }
    }

    /// Appends the next chunk at the end of the file. Returns `true` once the file is complete.
    async fn fetch_next(
        &self,
        url: &Url,
        file: &Path,
        handle: &mut tokio::fs::File,
        accept: Accept<'_>,
        progress: Progress<'_>,
    ) -> Result<bool, DownloadError> {
        let offset = handle.metadata().await?.len();
        let reply = self.fetch.get(url, offset, offset + self.chunk_size - 1).await?;
        let head = reply.head;
        let range = ContentRange::of(&head);
        let base = self.base(&head, range, offset, file, handle, accept).await?;
        let written = match base {
            Some(base) => {
                self.write_body(reply.body, base, &head, range, handle, progress)
                    .await?
            }
            None => 0,
        };
        match head.status {
            206 => {
                if range.and_then(|range| range.start) != Some(offset) {
                    return Err(restart(handle).await);
                }
                let Some(total) = range.and_then(|range| range.total).filter(|_| written > 0) else {
                    return Err(DownloadError::Refused(DownloadRefusal::Stalled));
                };
                Ok(offset + written >= total)
            }
            416 => {
                if offset > 0 && range.and_then(|range| range.total) == Some(offset) {
                    return Ok(true);
                }
                Err(restart(handle).await)
            }
            200..300 => Ok(true),
            500.. => Err(DownloadError::Provider(ProviderError::Server {
                status: i64::from(head.status),
            })),
            status => Err(DownloadError::Failed { status }),
        }
    }

    /// Where the body of the answer goes in the file; `None` when it isn't part of the file.
    async fn base(
        &self,
        head: &HttpHead,
        range: Option<ContentRange>,
        offset: u64,
        file: &Path,
        handle: &mut tokio::fs::File,
        accept: Accept<'_>,
    ) -> Result<Option<u64>, DownloadError> {
        if (200..300).contains(&head.status) {
            accept(head)?;
        }
        match head.status {
            206 => {
                if range.and_then(|range| range.start) != Some(offset) {
                    return Ok(None);
                }
                let Some(total) = range.and_then(|range| range.total) else {
                    return Err(DownloadError::Refused(DownloadRefusal::Stalled));
                };
                if total > self.max_bytes {
                    return Err(self.too_large());
                }
                self.check_space(file, total - offset.min(total))?;
                Ok(Some(offset))
            }
            200..300 => {
                let length = head.content_length().filter(|length| *length > 0);
                if length.is_some_and(|length| length > self.max_bytes) {
                    return Err(self.too_large());
                }
                handle.set_len(0).await?;
                if let Some(length) = length {
                    self.check_space(file, length)?;
                }
                Ok(Some(0))
            }
            _ => Ok(None),
        }
    }

    /// Writes the body at `base` as it arrives. Returns how many bytes went into the file.
    async fn write_body(
        &self,
        mut body: BoxStream<'static, Result<Vec<u8>, ()>>,
        base: u64,
        head: &HttpHead,
        range: Option<ContentRange>,
        handle: &mut tokio::fs::File,
        progress: Progress<'_>,
    ) -> Result<u64, DownloadError> {
        let total = if head.status == 206 {
            range.and_then(|range| range.total)
        } else {
            head.content_length().filter(|length| *length > 0)
        };
        handle.seek(SeekFrom::Start(base)).await?;
        let mut written = 0;
        while let Some(chunk) = body.next().await {
            let Ok(data) = chunk else {
                handle.flush().await?;
                return Err(DownloadError::Provider(ProviderError::Network));
            };
            let count = data.len() as u64;
            if base + written + count > self.max_bytes {
                handle.flush().await?;
                return Err(self.too_large());
            }
            handle.write_all(&data).await?;
            written += count;
            progress(base + written, total);
        }
        handle.flush().await?;
        Ok(written)
    }

    fn too_large(&self) -> DownloadError {
        DownloadError::Refused(DownloadRefusal::TooLarge {
            max_bytes: i64::try_from(self.max_bytes).unwrap_or(i64::MAX),
        })
    }

    /// Fails before writing what can't fit, rather than filling the disk up to the last byte.
    fn check_space(&self, file: &Path, bytes: u64) -> Result<(), DownloadError> {
        match self.free_space.free_bytes(file) {
            Some(free) if free < bytes => Err(DownloadError::Io {
                kind: io::ErrorKind::StorageFull,
                message: "There isn't enough space on the disk for the file.".to_owned(),
            }),
            _ => Ok(()),
        }
    }
}

#[async_trait]
impl Downloader for HttpDownloader {
    async fn download(&self, url: &Url, file: &Path) -> Result<(), DownloadError> {
        self.download_with(url, file, &|_| Ok(()), &|_, _| {}).await
    }
}

/// The kept bytes don't match what the server has: start over, counted as a failed attempt.
async fn restart(handle: &mut tokio::fs::File) -> DownloadError {
    match handle.set_len(0).await {
        Ok(()) => DownloadError::Provider(ProviderError::Network),
        Err(error) => error.into(),
    }
}

/// `Content-Range: bytes <start>-<end>/<total>`, or `bytes */<total>` on 416. `total` is `None` for `*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContentRange {
    start: Option<u64>,
    total: Option<u64>,
}

impl ContentRange {
    fn of(head: &HttpHead) -> Option<Self> {
        let value = head.header(CONTENT_RANGE.as_str())?.strip_prefix("bytes ")?;
        let (span, total) = value.split_once('/')?;
        Some(Self {
            start: span.split('-').next().and_then(|start| start.parse().ok()),
            total: total.parse().ok(),
        })
    }
}
