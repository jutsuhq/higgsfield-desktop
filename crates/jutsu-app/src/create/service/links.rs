//! Paste URL… (`CreateViewLinks.swift`): downloads a link, then takes the file like a dropped one, remembering the
//! link so a job that sends the file whole can pass the link instead of uploading it.

use std::sync::{Arc, Mutex, PoisonError};

use jutsu_core::{LocalImage, MediaLink, MediaLinkKind};
use jutsu_engine::MediaLinkDownloader;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::{CreateError, CreateService, file_url};
use crate::create::links::{LinkDownloadView, LinkOutcome, download_message, download_view, link_message};

/// The link being downloaded.
#[derive(Debug, Default)]
pub(super) struct LinkState {
    current: Option<Running>,
    /// Numbers the downloads, so a late report of an earlier link is dropped.
    count: u64,
}

#[derive(Debug)]
struct Running {
    id: u64,
    link: Url,
    expected: Option<MediaLinkKind>,
    fraction: Option<f64>,
    cancel: CancellationToken,
}

impl LinkState {
    pub(super) fn view(&self) -> Option<LinkDownloadView> {
        self.current
            .as_ref()
            .map(|running| download_view(&running.link, running.expected, running.fraction))
    }
}

impl CreateService {
    /// Downloads the typed link as `expected` (`None` takes either, as a pasted link does): a video becomes the
    /// source, an image a reference. A new link cancels the one running.
    pub async fn download_link(
        self: &Arc<Self>,
        text: &str,
        expected: Option<MediaLinkKind>,
    ) -> Result<LinkOutcome, CreateError> {
        let link = match MediaLink::parse(text) {
            Ok(link) => link,
            Err(error) => {
                return Ok(LinkOutcome::Failed {
                    message: link_message(&error),
                });
            }
        };
        let cancel = CancellationToken::new();
        let id = {
            let mut state = self.locked();
            if let Some(old) = state.link.current.take() {
                old.cancel.cancel();
            }
            state.link.count += 1;
            let id = state.link.count;
            state.link.current = Some(Running {
                id,
                link: link.clone(),
                expected,
                fraction: None,
                cancel: cancel.clone(),
            });
            id
        };
        self.notify();
        let shown = Arc::new(Mutex::new(None::<i64>));
        let weak = Arc::downgrade(self);
        let progress = move |fraction: Option<f64>| {
            // Only a new whole percent is shown (`ProgressThrottle`).
            #[allow(clippy::cast_possible_truncation, reason = "a percent fits")]
            let whole = fraction.map(|fraction| (fraction * 100.0).floor() as i64);
            {
                let mut last = shown.lock().unwrap_or_else(PoisonError::into_inner);
                if *last == whole && whole.is_some() {
                    return;
                }
                *last = whole;
            }
            let Some(service) = weak.upgrade() else { return };
            let changed = {
                let mut state = service.locked();
                match state.link.current.as_mut() {
                    Some(running) if running.id == id => {
                        running.fraction = fraction;
                        true
                    }
                    _ => false,
                }
            };
            if changed {
                service.notify();
            }
        };
        let folder = self.deps.layout.linked_media_folder();
        let downloader = MediaLinkDownloader::default();
        let finished = tokio::select! {
            () = cancel.cancelled() => None,
            result = downloader.download(&link, &folder, expected, &progress) => Some(result),
        };
        {
            let mut state = self.locked();
            if state
                .link
                .current
                .as_ref()
                .is_some_and(|running| running.id == id)
            {
                state.link.current = None;
            }
        }
        self.notify();
        let media = match finished {
            None | Some(Err(jutsu_engine::MediaDownloadError::Cancelled)) => {
                return Ok(LinkOutcome::Cancelled);
            }
            Some(Err(error)) => {
                return Ok(LinkOutcome::Failed {
                    message: download_message(&error),
                });
            }
            Some(Ok(media)) => media,
        };
        match media.kind {
            MediaLinkKind::Video => {
                self.open_source_from(&media.file, Some(media.origin)).await?;
                Ok(LinkOutcome::Done { notice: None })
            }
            MediaLinkKind::Image => {
                let mut image = LocalImage::new(file_url(&media.file)?);
                image.remote_origin = Some(media.origin);
                let notice = self.locked().form.add_opened_references(vec![image.clone()]);
                self.edited();
                self.make_thumbnails(vec![image]);
                Ok(LinkOutcome::Done { notice })
            }
        }
    }

    /// Cancel on the download row.
    pub fn cancel_link(&self) {
        if let Some(running) = self.locked().link.current.take() {
            running.cancel.cancel();
        }
        self.notify();
    }
}
