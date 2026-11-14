//! The speech model the app downloads itself, as Settings › Subtitles shows it (`SpeechModels.swift`): on macOS that
//! is Parakeet Ultra; here it is whisper's model (`WhisperModel`), which a run the person agreed to downloads too.
//! It is downloaded into the app's own folder only when the person asks, and removed on request. The folders of
//! earlier revisions count in its size until the current revision is downloaded or removed, which deletes them.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use jutsu_core::{ModelManifest, PortError, SpeechModelDownloading, WhisperModel};
use serde::Serialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::text;
use crate::connection::UserMessage;

/// Where the model is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SpeechModelState {
    NotDownloaded,
    /// 0…1.
    Downloading {
        fraction: f64,
    },
    Ready,
}

struct Inner {
    state: SpeechModelState,
    /// What the model's folders take on disk, earlier revisions' included.
    size: i64,
    failure: Option<UserMessage>,
    task: Option<CancellationToken>,
}

/// The model's download, size and removal.
pub struct SpeechModels {
    me: Weak<SpeechModels>,
    /// The model's folder: one per revision, so another revision never mixes with it.
    folder: PathBuf,
    root: PathBuf,
    manifest: ModelManifest,
    downloader: Arc<dyn SpeechModelDownloading>,
    inner: Mutex<Inner>,
    changes: watch::Sender<u64>,
}

impl std::fmt::Debug for SpeechModels {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SpeechModels")
            .field("folder", &self.folder)
            .finish_non_exhaustive()
    }
}

impl SpeechModels {
    /// The folder name's start, before the revision.
    pub const PREFIX: &'static str = "whisper-small-q5_1-";
    /// The app data dir's folder the models are kept in.
    pub const FOLDER: &'static str = "Speech Models";

    /// The model's folder under `root`, the folder the app keeps its models in.
    pub fn folder_in(root: &Path) -> PathBuf {
        root.join(format!("{}{}", Self::PREFIX, &WhisperModel::REVISION[..8]))
    }

    pub fn new(root: &Path, downloader: Arc<dyn SpeechModelDownloading>) -> Arc<Self> {
        let folder = Self::folder_in(root);
        let manifest = WhisperModel::manifest();
        let state = if downloader.is_complete(&manifest, &folder) {
            SpeechModelState::Ready
        } else {
            SpeechModelState::NotDownloaded
        };
        let models = Arc::new_cyclic(|me| Self {
            me: me.clone(),
            folder,
            root: root.to_path_buf(),
            manifest,
            downloader,
            inner: Mutex::new(Inner {
                state,
                size: 0,
                failure: None,
                task: None,
            }),
            changes: watch::channel(0).0,
        });
        let size = models.measured();
        models.locked().size = size;
        models
    }

    fn locked(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A revision that goes up when the model comes or goes, or its download moves on.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn changed(&self) {
        self.changes.send_modify(|revision| *revision += 1);
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn state(&self) -> SpeechModelState {
        self.locked().state
    }

    pub fn size(&self) -> i64 {
        self.locked().size
    }

    pub fn is_downloading(&self) -> bool {
        matches!(self.state(), SpeechModelState::Downloading { .. })
    }

    /// What went wrong with the last download or removal, until it is taken to be shown.
    pub fn take_failure(&self) -> Option<UserMessage> {
        self.locked().failure.take()
    }

    pub fn failure(&self) -> Option<UserMessage> {
        self.locked().failure.clone()
    }

    /// Downloads what is missing; a download stopped before goes on from where it was.
    pub fn download(&self) {
        let Some(me) = self.me.upgrade() else {
            return;
        };
        let token = {
            let mut inner = self.locked();
            if inner.task.is_some() || inner.state == SpeechModelState::Ready {
                return;
            }
            inner.state = SpeechModelState::Downloading { fraction: 0.0 };
            let token = CancellationToken::new();
            inner.task = Some(token.clone());
            token
        };
        self.changed();
        tokio::spawn(async move {
            let weak = Arc::downgrade(&me);
            let last = Mutex::new(None::<i64>);
            let progress = Arc::new(move |fraction: f64| {
                #[allow(clippy::cast_possible_truncation, reason = "a percent")]
                let percent = (fraction * 100.0).floor() as i64;
                {
                    let mut last = last.lock().unwrap_or_else(PoisonError::into_inner);
                    if *last == Some(percent) {
                        return;
                    }
                    *last = Some(percent);
                }
                if let Some(models) = weak.upgrade() {
                    models.progress(fraction);
                }
            });
            let result = tokio::select! {
                biased;
                () = token.cancelled() => Err(PortError::Cancelled),
                result = me.downloader.download(&me.manifest, &me.folder, progress) => result,
            };
            if let Err(error) = result
                && error != PortError::Cancelled
                && !token.is_cancelled()
            {
                me.locked().failure = Some(UserMessage::new(
                    text::DOWNLOAD_FAILED_TITLE,
                    error.to_string(),
                    None,
                ));
            }
            me.settle();
        });
    }

    /// Stops a download; what came down so far stays for the next one.
    pub fn cancel(&self) {
        if let Some(token) = &self.locked().task {
            token.cancel();
        }
    }

    /// Deletes the model's folder, and those of earlier revisions.
    pub fn remove(&self) {
        if self.locked().task.is_some() {
            return;
        }
        let removed = self
            .downloader
            .remove(&self.folder)
            .and_then(|()| self.remove_earlier_revisions());
        if let Err(error) = removed {
            self.locked().failure = Some(UserMessage::new(
                text::REMOVE_FAILED_TITLE,
                error.to_string(),
                None,
            ));
        }
        self.settle();
    }

    /// Waits for a download in flight.
    pub async fn finish(&self) {
        let mut changes = self.subscribe();
        loop {
            if self.locked().task.is_none() || changes.changed().await.is_err() {
                return;
            }
        }
    }

    /// Reads again whether the model is here, after a run downloaded it.
    pub fn refresh(&self) {
        if self.locked().task.is_some() {
            return;
        }
        let ready = self.downloader.is_complete(&self.manifest, &self.folder);
        let size = self.measured();
        let changed = {
            let mut inner = self.locked();
            let state = if ready {
                SpeechModelState::Ready
            } else {
                SpeechModelState::NotDownloaded
            };
            let changed = inner.state != state || inner.size != size;
            inner.state = state;
            inner.size = size;
            changed
        };
        if changed {
            self.changed();
        }
    }

    fn progress(&self, fraction: f64) {
        {
            let mut inner = self.locked();
            if !matches!(inner.state, SpeechModelState::Downloading { .. }) {
                return;
            }
            inner.state = SpeechModelState::Downloading { fraction };
        }
        self.changed();
    }

    fn settle(&self) {
        let ready = self.downloader.is_complete(&self.manifest, &self.folder);
        if ready && let Err(error) = self.remove_earlier_revisions() {
            self.locked().failure = Some(UserMessage::new(
                text::REMOVE_FAILED_TITLE,
                error.to_string(),
                None,
            ));
        }
        let size = self.measured();
        {
            let mut inner = self.locked();
            inner.task = None;
            inner.state = if ready {
                SpeechModelState::Ready
            } else {
                SpeechModelState::NotDownloaded
            };
            inner.size = size;
        }
        self.changed();
    }

    /// The folders other revisions of the model were downloaded into.
    fn earlier_revisions(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let own = self.folder.file_name();
        entries
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name();
                name.to_string_lossy().starts_with(Self::PREFIX) && Some(name.as_os_str()) != own
            })
            .map(|entry| entry.path())
            .collect()
    }

    fn remove_earlier_revisions(&self) -> Result<(), PortError> {
        for old in self.earlier_revisions() {
            self.downloader.remove(&old)?;
        }
        Ok(())
    }

    fn measured(&self) -> i64 {
        std::iter::once(self.folder.clone())
            .chain(self.earlier_revisions())
            .map(|folder| self.downloader.size(&folder))
            .sum()
    }
}
