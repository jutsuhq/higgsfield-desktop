//! Port of `MediaLinkDownloaderTests`, against a wiremock `MediaLinkStub`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use jutsu_core::{MediaLinkError, MediaLinkKind};
use url::Url;
use wiremock::MockServer;

use super::http_stub::{Free, StubFetch, media_server};
use crate::downloader::FreeSpace;
use crate::{MediaDownloadError, MediaLinkDownloader};

struct Setup {
    server: MockServer,
    root: tempfile::TempDir,
}

impl Setup {
    async fn new() -> Self {
        Self {
            server: media_server().await,
            root: tempfile::tempdir().unwrap(),
        }
    }

    /// The folder downloads go into; it doesn't exist until a download makes it.
    fn folder(&self) -> PathBuf {
        self.root.path().join("MediaLinks")
    }

    fn downloader(&self, max_bytes: u64, free_space: Option<u64>) -> MediaLinkDownloader {
        MediaLinkDownloader::with(
            max_bytes,
            StubFetch::new(&self.server),
            Arc::new(Free(free_space)),
        )
    }

    fn default_downloader(&self) -> MediaLinkDownloader {
        self.downloader(1 << 20, None)
    }

    /// What is left in the folder after a download, relative to it: nothing after a failure.
    fn leftovers(&self) -> Vec<String> {
        let mut found = Vec::new();
        subpaths(&self.folder(), &self.folder(), &mut found);
        found.sort();
        found
    }
}

fn subpaths(root: &Path, folder: &Path, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        found.push(
            path.strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/"),
        );
        if path.is_dir() {
            subpaths(root, &path, found);
        }
    }
}

fn link(path: &str, query: &str) -> Url {
    Url::parse(&format!("https://cdn.example.org/{path}?{query}")).unwrap()
}

fn link_error(error: MediaLinkError) -> MediaDownloadError {
    MediaDownloadError::Link(error)
}

#[tokio::test]
async fn a_direct_video_link_becomes_a_file_named_after_it() {
    let setup = Setup::new().await;
    let fractions = Mutex::new(Vec::new());
    let link = link("clips/Dance%20Take.mp4", "type=video/mp4");

    let media = setup
        .default_downloader()
        .download(&link, &setup.folder(), None, &|fraction| {
            fractions.lock().unwrap().push(fraction);
        })
        .await
        .unwrap();

    assert_eq!(media.kind, MediaLinkKind::Video);
    assert_eq!(media.origin, link);
    assert_eq!(media.file.file_name().unwrap(), "Dance Take.mp4");
    assert_eq!(media.file.parent().unwrap().parent().unwrap(), setup.folder());
    assert_eq!(std::fs::read(&media.file).unwrap().len(), 12);
    assert_eq!(fractions.lock().unwrap().last(), Some(&Some(1.0)));
}

#[tokio::test]
async fn a_generic_type_is_named_by_the_links_extension() {
    let setup = Setup::new().await;

    let media = setup
        .default_downloader()
        .download(
            &link("photo.PNG", "type=application/octet-stream"),
            &setup.folder(),
            None,
            &|_| {},
        )
        .await
        .unwrap();

    assert_eq!(media.kind, MediaLinkKind::Image);
    assert_eq!(media.file.extension().unwrap(), "png");
}

#[tokio::test]
async fn a_web_page_is_refused_and_nothing_is_kept() {
    let setup = Setup::new().await;
    let watch_page = link("watch", "type=text/html;%20charset=utf-8");

    let result = setup
        .default_downloader()
        .download(&watch_page, &setup.folder(), None, &|_| {})
        .await;

    assert_eq!(
        result,
        Err(link_error(MediaLinkError::NotMedia {
            content_type: Some("text/html".into())
        }))
    );
    assert_eq!(setup.leftovers(), Vec::<String>::new());
}

#[tokio::test]
async fn a_video_where_an_image_was_asked_for_is_refused() {
    let setup = Setup::new().await;

    let result = setup
        .default_downloader()
        .download(
            &link("a.mp4", "type=video/mp4"),
            &setup.folder(),
            Some(MediaLinkKind::Image),
            &|_| {},
        )
        .await;

    assert_eq!(
        result,
        Err(link_error(MediaLinkError::WrongKind {
            expected: MediaLinkKind::Image
        }))
    );
    assert_eq!(setup.leftovers(), Vec::<String>::new());
}

#[tokio::test]
async fn a_file_over_the_limit_is_refused() {
    let setup = Setup::new().await;

    let result = setup
        .downloader(8, None)
        .download(
            &link("big.mp4", "type=video/mp4&size=20"),
            &setup.folder(),
            None,
            &|_| {},
        )
        .await;

    assert_eq!(result, Err(link_error(MediaLinkError::TooLarge { max_bytes: 8 })));
    assert_eq!(setup.leftovers(), Vec::<String>::new());
}

#[tokio::test]
async fn a_full_disk_is_said() {
    let setup = Setup::new().await;

    let result = setup
        .downloader(1 << 20, Some(4))
        .download(&link("a.mp4", "type=video/mp4"), &setup.folder(), None, &|_| {})
        .await;

    assert_eq!(result, Err(link_error(MediaLinkError::NotEnoughSpace)));
}

#[tokio::test]
async fn a_missing_file_says_the_status() {
    let setup = Setup::new().await;

    let result = setup
        .default_downloader()
        .download(&link("gone.mp4", "status=404"), &setup.folder(), None, &|_| {})
        .await;

    assert_eq!(
        result,
        Err(link_error(MediaLinkError::Unavailable { status: 404 }))
    );
    assert_eq!(setup.leftovers(), Vec::<String>::new());
}

#[tokio::test]
async fn plain_http_is_never_requested() {
    let setup = Setup::new().await;
    let insecure = Url::parse("http://cdn.example.org/a.mp4?type=video/mp4").unwrap();

    let result = setup
        .default_downloader()
        .download(&insecure, &setup.folder(), None, &|_| {})
        .await;

    assert_eq!(result, Err(link_error(MediaLinkError::NotHttps)));
}

#[tokio::test]
async fn an_empty_answer_is_not_media() {
    let setup = Setup::new().await;

    let result = setup
        .default_downloader()
        .download(
            &link("a.mp4", "type=video/mp4&size=0"),
            &setup.folder(),
            None,
            &|_| {},
        )
        .await;

    assert_eq!(
        result,
        Err(link_error(MediaLinkError::NotMedia { content_type: None }))
    );
}

#[tokio::test]
async fn a_file_already_saved_is_not_downloaded_again() {
    let setup = Setup::new().await;
    let file = setup.folder().join("clip.mp4");
    std::fs::create_dir_all(setup.folder()).unwrap();
    std::fs::write(&file, b"kept").unwrap();

    // Requested, the link would fail.
    setup
        .default_downloader()
        .download_to(&link("clip.mp4", "status=404"), &file, MediaLinkKind::Video)
        .await
        .unwrap();

    assert_eq!(std::fs::read(&file).unwrap(), b"kept");
}

/// Writes another download's file when the downloader checks the free space, and says it is unknown.
struct Racing(PathBuf);

impl FreeSpace for Racing {
    fn free_bytes(&self, _file: &Path) -> Option<u64> {
        let _ = std::fs::write(&self.0, b"other");
        None
    }
}

/// Another download of the same file finishing while this one runs: the file is replaced whole, not refused.
#[tokio::test]
async fn a_file_saved_during_the_download_is_replaced_whole() {
    let setup = Setup::new().await;
    let file = setup.folder().join("account-media").join("photo.png");
    let racing = MediaLinkDownloader::with(
        MediaLinkDownloader::MAX_BYTES,
        StubFetch::new(&setup.server),
        Arc::new(Racing(file.clone())),
    );

    racing
        .download_to(&link("photo.png", "type=image/png"), &file, MediaLinkKind::Image)
        .await
        .unwrap();

    assert_eq!(std::fs::read(&file).unwrap(), vec![7_u8; 12]);
    assert_eq!(setup.leftovers(), ["account-media", "account-media/photo.png"]);
}

#[tokio::test]
async fn a_file_of_the_wrong_kind_is_not_saved() {
    let setup = Setup::new().await;
    let file = setup.folder().join("photo.png");

    let result = setup
        .default_downloader()
        .download_to(&link("photo.mp4", "type=video/mp4"), &file, MediaLinkKind::Image)
        .await;

    assert_eq!(
        result,
        Err(link_error(MediaLinkError::WrongKind {
            expected: MediaLinkKind::Image
        }))
    );
    assert_eq!(setup.leftovers(), Vec::<String>::new());
}
