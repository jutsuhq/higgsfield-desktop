//! Pasted links to a video or image, downloaded for Create.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use jutsu_core::{MediaLink, MediaLinkError, MediaLinkKind, ProviderError, ServerFileName};
use url::Url;
use uuid::Uuid;

use crate::downloader::{DownloadError, DownloadRefusal, FreeSpace, HttpDownloader, HttpFetch, HttpHead};
use crate::error::is_out_of_space;
use crate::file_replacement::FileReplacement;

/// A video or image downloaded from a link for Create.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DownloadedMedia {
    /// The file.
    pub file: PathBuf,
    /// What it is.
    pub kind: MediaLinkKind,
    /// The link it came from: `SourceClip::remote_origin` / `LocalImage::remote_origin`.
    pub origin: Url,
}

/// Why a link wasn't downloaded.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MediaDownloadError {
    /// What the person is told.
    #[error(transparent)]
    Link(#[from] MediaLinkError),
    /// A provider failure not about the link.
    #[error(transparent)]
    Provider(ProviderError),
    /// A file system failure.
    #[error("{message}")]
    Io {
        /// The kind.
        kind: std::io::ErrorKind,
        /// The description.
        message: String,
    },
    /// The download was cancelled.
    #[error("cancelled")]
    Cancelled,
}

impl From<std::io::Error> for MediaDownloadError {
    fn from(error: std::io::Error) -> Self {
        link_error(DownloadError::Io {
            kind: error.kind(),
            message: error.to_string(),
        })
    }
}

/// The downloader's errors as `MediaLinkError`; cancellation and anything unexpected pass through.
pub(crate) fn link_error(error: DownloadError) -> MediaDownloadError {
    match error {
        DownloadError::NotAccepted(error) => MediaDownloadError::Link(error),
        DownloadError::Refused(DownloadRefusal::TooLarge { max_bytes }) => {
            MediaDownloadError::Link(MediaLinkError::TooLarge { max_bytes })
        }
        DownloadError::Refused(DownloadRefusal::NotHttps) => {
            MediaDownloadError::Link(MediaLinkError::NotHttps)
        }
        DownloadError::Refused(DownloadRefusal::Stalled)
        | DownloadError::Provider(ProviderError::Network) => {
            MediaDownloadError::Link(MediaLinkError::Network)
        }
        DownloadError::Failed { status } => MediaDownloadError::Link(MediaLinkError::Unavailable {
            status: i64::from(status),
        }),
        DownloadError::Provider(ProviderError::Server { status }) => {
            MediaDownloadError::Link(MediaLinkError::Unavailable { status })
        }
        DownloadError::Provider(other) => MediaDownloadError::Provider(other),
        DownloadError::Io { kind, .. } if is_out_of_space(kind) => {
            MediaDownloadError::Link(MediaLinkError::NotEnoughSpace)
        }
        DownloadError::Io { kind, message } => MediaDownloadError::Io { kind, message },
        DownloadError::Cancelled => MediaDownloadError::Cancelled,
    }
}

/// Removes a download's folder unless the download kept it.
struct FolderGuard {
    folder: PathBuf,
    keep: bool,
}

impl Drop for FolderGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
    }
}

/// Downloads a pasted link with `HttpDownloader` (resume, size limit, free-space check) into a folder of its own,
/// and keeps it only when the server says it is a video or an image (`MediaLink::classify`), so a web page is
/// refused before its body is written. Failures are `MediaLinkError`s; dropping the download stops it. Either way
/// nothing is left behind.
#[derive(Clone)]
pub struct MediaLinkDownloader {
    downloader: Arc<HttpDownloader>,
}

impl Default for MediaLinkDownloader {
    fn default() -> Self {
        Self::new(Self::MAX_BYTES)
    }
}

impl MediaLinkDownloader {
    /// Larger files are almost always the wrong link: a job takes at most 30 s of a source.
    pub const MAX_BYTES: u64 = 2 << 30;

    /// A downloader refusing files over `max_bytes`.
    pub fn new(max_bytes: u64) -> Self {
        Self {
            downloader: Arc::new(HttpDownloader::new(max_bytes)),
        }
    }

    /// The same downloader over `fetch` and `free_space`, for tests.
    pub fn with(max_bytes: u64, fetch: Arc<dyn HttpFetch>, free_space: Arc<dyn FreeSpace>) -> Self {
        Self {
            downloader: Arc::new(
                HttpDownloader::new(max_bytes)
                    .with_fetch(fetch)
                    .with_free_space(free_space),
            ),
        }
    }

    /// Downloads `link` to `folder/<new id>/<name>.<ext>`. `expected` is the kind the caller can use; another kind
    /// fails with `MediaLinkError::WrongKind` before the body is downloaded. `None` takes either. `progress` gets the
    /// fraction done, `None` while the size is unknown.
    pub async fn download(
        &self,
        link: &Url,
        folder: &Path,
        expected: Option<MediaLinkKind>,
        progress: &(dyn Fn(Option<f64>) + Send + Sync),
    ) -> Result<DownloadedMedia, MediaDownloadError> {
        if !higgsfield::core::is_https(link) {
            return Err(MediaLinkError::NotHttps.into());
        }
        let directory = folder.join(jutsu_core::support::uuid_string(Uuid::new_v4()));
        let mut guard = FolderGuard {
            folder: directory.clone(),
            keep: false,
        };
        let partial = directory.join(".download");
        let found: Mutex<Option<(MediaLinkKind, String)>> = Mutex::new(None);
        std::fs::create_dir_all(&directory)?;
        let accept = |head: &HttpHead| -> Result<(), DownloadError> {
            let (kind, extension) = MediaLink::classify(head.header("Content-Type"), &head.url)
                .map_err(DownloadError::NotAccepted)?;
            if let Some(expected) = expected
                && kind != expected
            {
                return Err(DownloadError::NotAccepted(MediaLinkError::WrongKind { expected }));
            }
            *found.lock().unwrap_or_else(PoisonError::into_inner) = Some((kind, extension));
            Ok(())
        };
        let report = |written: u64, total: Option<u64>| {
            #[allow(clippy::cast_precision_loss, reason = "a fraction for a progress bar")]
            progress(total.map(|total| {
                if total > 0 {
                    (written as f64 / total as f64).min(1.0)
                } else {
                    1.0
                }
            }));
        };
        self.downloader
            .download_with(link, &partial, &accept, &report)
            .await
            .map_err(link_error)?;
        let found = found.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let size = std::fs::metadata(&partial).map_or(0, |metadata| metadata.len());
        let Some((kind, extension)) = found.filter(|_| size > 0) else {
            return Err(MediaLinkError::NotMedia { content_type: None }.into());
        };
        let extension_of =
            Url::parse(&format!("file:///file.{extension}")).map_err(|error| MediaDownloadError::Io {
                kind: std::io::ErrorKind::InvalidInput,
                message: error.to_string(),
            })?;
        let Some(file) = ServerFileName::file(
            &directory,
            &[MediaLink::base_name(link)],
            &extension_of,
            &[extension.as_str()],
        ) else {
            return Err(MediaDownloadError::Io {
                kind: std::io::ErrorKind::InvalidInput,
                message: "The file name is invalid.".to_owned(),
            });
        };
        std::fs::rename(&partial, &file)?;
        guard.keep = true;
        Ok(DownloadedMedia {
            file,
            kind,
            origin: link.clone(),
        })
    }

    /// Downloads `link` to `file`, unless a file is already there: for media a server keeps, saved once under a name
    /// the caller chose. The download goes into a folder of its own beside `file` and is renamed into place, so
    /// another download of the same file finishing first is replaced whole instead of failing this one.
    pub async fn download_to(
        &self,
        link: &Url,
        file: &Path,
        expected: MediaLinkKind,
    ) -> Result<(), MediaDownloadError> {
        if file.exists() {
            return Ok(());
        }
        let folder = file.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(folder)?;
        let media = self.download(link, folder, Some(expected), &|_| {}).await?;
        let _guard = FolderGuard {
            folder: media.file.parent().unwrap_or(folder).to_path_buf(),
            keep: false,
        };
        FileReplacement::move_file(&media.file, file)?;
        Ok(())
    }
}
