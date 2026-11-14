//! Pictures a server links to, read into memory for a thumbnail or a preview.

use std::path::PathBuf;
use std::sync::Arc;

use jutsu_core::{MediaLink, MediaLinkError};
use url::Url;
use uuid::Uuid;

use crate::downloader::{DownloadError, HttpDownloader, HttpFetch, HttpHead};
use crate::media_link_downloader::{MediaDownloadError, link_error};

/// Removes the scratch file when the read ends, however it ends.
struct ScratchGuard(PathBuf);

impl Drop for ScratchGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Reads a picture a server links to into memory, for a thumbnail or a preview. Only an https URL is requested, and
/// only a successful answer that may be an image (`MediaLink::check_image`) of at most `max_bytes` is returned.
/// Failures are `MediaLinkError`s; dropping the read stops it.
#[derive(Clone)]
pub struct ServerImageReader {
    downloader: Arc<HttpDownloader>,
    max_bytes: u64,
}

impl Default for ServerImageReader {
    fn default() -> Self {
        Self::new(Self::MAX_BYTES)
    }
}

impl ServerImageReader {
    /// Far above any thumbnail or character sheet, and low enough that a wrong link can't fill the memory.
    pub const MAX_BYTES: u64 = 64 << 20;

    /// A reader refusing pictures over `max_bytes`.
    pub fn new(max_bytes: u64) -> Self {
        Self {
            downloader: Arc::new(HttpDownloader::new(max_bytes)),
            max_bytes,
        }
    }

    /// The same reader over `fetch`, for tests.
    pub fn with_fetch(max_bytes: u64, fetch: Arc<dyn HttpFetch>) -> Self {
        Self {
            downloader: Arc::new(HttpDownloader::new(max_bytes).with_fetch(fetch)),
            max_bytes,
        }
    }

    /// Streams to a bounded scratch file before loading the accepted bytes into memory.
    pub async fn data(&self, url: &Url) -> Result<Vec<u8>, MediaDownloadError> {
        if !higgsfield::core::is_https(url) {
            return Err(MediaLinkError::NotHttps.into());
        }
        let file = std::env::temp_dir().join(format!(
            "preview-{}",
            jutsu_core::support::uuid_string(Uuid::new_v4())
        ));
        let _guard = ScratchGuard(file.clone());
        let max_bytes = self.max_bytes;
        let accept = |head: &HttpHead| -> Result<(), DownloadError> {
            MediaLink::check_image(head.header("Content-Type")).map_err(DownloadError::NotAccepted)?;
            if head.content_length().is_some_and(|length| length > max_bytes) {
                return Err(DownloadError::NotAccepted(MediaLinkError::TooLarge {
                    max_bytes: i64::try_from(max_bytes).unwrap_or(i64::MAX),
                }));
            }
            Ok(())
        };
        self.downloader
            .download_with(url, &file, &accept, &|_, _| {})
            .await
            .map_err(link_error)?;
        Ok(tokio::fs::read(&file).await?)
    }
}
