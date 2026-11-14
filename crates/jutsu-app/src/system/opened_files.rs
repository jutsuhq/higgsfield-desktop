//! Files handed to the app from outside (`OpenedFiles.swift`, `AppDelegate.application(_:open:)`, File › Open…): the
//! file manager's "Open with", files passed to a second launch, and File › Open…. A video becomes Create's source,
//! images become references. Files opened before Create is on screen wait in `PendingOpens` until it is.
//!
//! macOS also asks the file's type (`public.movie`, `public.image`) for an extension the tables don't know; here the
//! extension alone decides.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;

use crate::create::CreateService;

/// What a file handed to the app becomes in Create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenedFileKind {
    Video,
    Image,
    Unsupported,
}

/// `OpenedFileKind.videoExtensions`.
pub const VIDEO_EXTENSIONS: [&str; 11] = [
    "mov", "mp4", "m4v", "avi", "mkv", "webm", "mpg", "mpeg", "3gp", "mts", "m2ts",
];
/// `OpenedFileKind.imageExtensions`.
pub const IMAGE_EXTENSIONS: [&str; 10] = [
    "jpg", "jpeg", "png", "heic", "heif", "webp", "gif", "tif", "tiff", "bmp",
];

impl OpenedFileKind {
    pub fn of(path: &Path) -> Self {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default();
        if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
            Self::Video
        } else if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
            Self::Image
        } else {
            Self::Unsupported
        }
    }
}

/// Opened files sorted for Create: one source video and the images for references.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenRequest {
    /// Create has one source, so of several videos the last one opened wins.
    pub video: Option<PathBuf>,
    pub images: Vec<PathBuf>,
    pub unsupported: Vec<PathBuf>,
}

/// An alert about opened files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenedFilesAlert {
    pub title: String,
    pub message: String,
}

impl OpenRequest {
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut request = Self::default();
        for path in paths {
            match OpenedFileKind::of(&path) {
                OpenedFileKind::Video => request.video = Some(path),
                OpenedFileKind::Image if !request.images.contains(&path) => request.images.push(path),
                OpenedFileKind::Image => {}
                OpenedFileKind::Unsupported => request.unsupported.push(path),
            }
        }
        request
    }

    pub fn is_empty(&self) -> bool {
        self.video.is_none() && self.images.is_empty() && self.unsupported.is_empty()
    }

    /// Adds files opened after these, keeping the order they were opened in.
    pub fn merge(&mut self, later: Self) {
        if later.video.is_some() {
            self.video = later.video;
        }
        for image in later.images {
            if !self.images.contains(&image) {
                self.images.push(image);
            }
        }
        self.unsupported.extend(later.unsupported);
    }

    /// The alert for files Create can't use; `None` when every file was a video or an image.
    pub fn unsupported_message(&self) -> Option<OpenedFilesAlert> {
        let first = self.unsupported.first()?;
        let title = if self.unsupported.len() == 1 {
            let name = first
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("Can't open “{name}”")
        } else {
            format!("Can't open {} files", self.unsupported.len())
        };
        Some(OpenedFilesAlert {
            title,
            message: "Jutsu Studio opens videos and images.".to_owned(),
        })
    }
}

/// Files opened before Create could take them, e.g. while the app was still launching or signed out. Nothing is
/// dropped: requests merge until `take`.
#[derive(Debug, Default)]
pub struct PendingOpens {
    request: Mutex<Option<OpenRequest>>,
}

impl PendingOpens {
    pub fn is_empty(&self) -> bool {
        self.locked().is_none()
    }

    pub fn add(&self, new: OpenRequest) {
        if new.is_empty() {
            return;
        }
        let mut request = self.locked();
        match request.as_mut() {
            Some(waiting) => waiting.merge(new),
            None => *request = Some(new),
        }
    }

    /// Everything opened so far, oldest first; nothing waits afterwards.
    pub fn take(&self) -> Option<OpenRequest> {
        self.locked().take()
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, Option<OpenRequest>> {
        self.request.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The files among a launch's arguments: what isn't an option (`--started-at-login`), resolved against the folder the
/// launch ran in, that names a file. The first argument is the program itself.
pub fn file_arguments(arguments: &[String], directory: &Path) -> Vec<PathBuf> {
    arguments
        .iter()
        .skip(1)
        .filter(|argument| !argument.starts_with('-'))
        .map(|argument| {
            let path = Path::new(argument);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                directory.join(path)
            }
        })
        .filter(|path| path.is_file())
        .collect()
}

/// Hands `request` to Create (`CreateView.takePendingOpens`): images become references, a video the source. Answers
/// with the alert to show, if any: a video that can't be opened wins over images that didn't fit, which win over files
/// Create can't use.
pub async fn open_in_create(create: &Arc<CreateService>, request: OpenRequest) -> Option<OpenedFilesAlert> {
    let mut alert = request.unsupported_message();
    if !request.images.is_empty() {
        match create.add_references(&request.images) {
            Ok(outcome) => {
                if let Some(notice) = outcome.notice {
                    alert = Some(OpenedFilesAlert {
                        title: notice,
                        message: String::new(),
                    });
                }
            }
            Err(error) => {
                alert = Some(OpenedFilesAlert {
                    title: "Can't add the images".to_owned(),
                    message: error.to_string(),
                });
            }
        }
    }
    if let Some(video) = request.video
        && let Err(error) = create.open_source(&video).await
    {
        alert = Some(OpenedFilesAlert {
            title: "Can't open the video".to_owned(),
            message: error.to_string(),
        });
    }
    alert
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|name| PathBuf::from(format!("/tmp/{name}")))
            .collect()
    }

    #[test]
    fn sorts_by_extension_ignoring_case() {
        assert_eq!(OpenedFileKind::of(Path::new("/tmp/a.MOV")), OpenedFileKind::Video);
        assert_eq!(OpenedFileKind::of(Path::new("/tmp/a.mkv")), OpenedFileKind::Video);
        assert_eq!(
            OpenedFileKind::of(Path::new("/tmp/a.HEIC")),
            OpenedFileKind::Image
        );
        assert_eq!(
            OpenedFileKind::of(Path::new("/tmp/a.txt")),
            OpenedFileKind::Unsupported
        );
        assert_eq!(
            OpenedFileKind::of(Path::new("/tmp/noextension")),
            OpenedFileKind::Unsupported
        );
    }

    #[test]
    fn the_last_video_wins_and_images_keep_their_order_once() {
        let request = OpenRequest::new(paths(&["a.mp4", "x.png", "b.mov", "y.jpg", "x.png", "notes.txt"]));
        assert_eq!(request.video, Some(PathBuf::from("/tmp/b.mov")));
        assert_eq!(request.images, paths(&["x.png", "y.jpg"]));
        assert_eq!(request.unsupported, paths(&["notes.txt"]));
        assert!(!request.is_empty());
        assert!(OpenRequest::new(Vec::new()).is_empty());
    }

    #[test]
    fn says_which_files_it_cant_open() {
        assert_eq!(OpenRequest::new(paths(&["a.mp4"])).unsupported_message(), None);
        let one = OpenRequest::new(paths(&["notes.txt"])).unsupported_message();
        assert_eq!(
            one,
            Some(OpenedFilesAlert {
                title: "Can't open “notes.txt”".to_owned(),
                message: "Jutsu Studio opens videos and images.".to_owned(),
            })
        );
        let two = OpenRequest::new(paths(&["a.txt", "b.pdf"])).unsupported_message();
        assert_eq!(
            two.map(|alert| alert.title).as_deref(),
            Some("Can't open 2 files")
        );
    }

    #[test]
    fn later_opens_merge_until_taken() {
        let pending = PendingOpens::default();
        assert!(pending.is_empty());
        pending.add(OpenRequest::new(Vec::new()));
        assert!(pending.is_empty());
        pending.add(OpenRequest::new(paths(&["a.mp4", "x.png"])));
        pending.add(OpenRequest::new(paths(&["y.png", "x.png", "z.doc"])));
        pending.add(OpenRequest::new(paths(&["y.png"])));
        let taken = pending.take();
        assert_eq!(
            taken,
            Some(OpenRequest {
                video: Some(PathBuf::from("/tmp/a.mp4")),
                images: paths(&["x.png", "y.png"]),
                unsupported: paths(&["z.doc"]),
            })
        );
        assert!(pending.is_empty());
        assert_eq!(pending.take(), None);

        pending.add(OpenRequest::new(paths(&["a.mp4"])));
        pending.add(OpenRequest::new(paths(&["b.mp4"])));
        assert_eq!(
            pending.take().and_then(|request| request.video),
            Some(PathBuf::from("/tmp/b.mp4"))
        );
    }

    #[test]
    fn launch_arguments_name_existing_files() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("clip.mp4"), b"").unwrap();
        std::fs::write(folder.path().join("still.png"), b"").unwrap();
        let absolute = folder.path().join("still.png").to_string_lossy().into_owned();
        let arguments = [
            "jutsu-studio".to_owned(),
            "--started-at-login".to_owned(),
            "clip.mp4".to_owned(),
            absolute,
            "missing.mov".to_owned(),
        ];
        assert_eq!(
            file_arguments(&arguments, folder.path()),
            [folder.path().join("clip.mp4"), folder.path().join("still.png")]
        );
        assert_eq!(
            file_arguments(&["jutsu-studio".to_owned()], folder.path()),
            Vec::<PathBuf>::new()
        );
    }
}
