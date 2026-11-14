//! Port of `URLSessionDownloaderTests`, against a wiremock `RangeStub`.

use std::path::PathBuf;
use std::sync::Arc;

use jutsu_core::ProviderError;
use url::Url;
use wiremock::MockServer;

use super::http_stub::{Free, RANGE_BODY, StubFetch, range_server, ranges};
use crate::downloader::{
    DownloadError, DownloadRefusal, Downloader, FreeSpace, HttpDownloader, VolumeFreeSpace,
};
use crate::error::is_out_of_space;

struct Setup {
    server: MockServer,
    folder: tempfile::TempDir,
}

impl Setup {
    async fn new() -> Self {
        Self {
            server: range_server().await,
            folder: tempfile::tempdir().unwrap(),
        }
    }

    fn file(&self) -> PathBuf {
        self.folder.path().join("download.partial")
    }

    fn url(query: &str) -> Url {
        Url::parse(&format!("https://cdn.example/video.mp4{query}")).unwrap()
    }

    fn downloader(&self, max_bytes: u64, free_space: Option<u64>) -> HttpDownloader {
        HttpDownloader::new(max_bytes)
            .with_fetch(StubFetch::new(&self.server))
            .with_chunk_size(4)
            .with_free_space(Arc::new(Free(free_space)))
    }

    fn default_downloader(&self) -> HttpDownloader {
        self.downloader(1 << 20, None)
    }

    async fn ranges(&self) -> Vec<String> {
        ranges(&self.server).await
    }

    fn contents(&self) -> Vec<u8> {
        std::fs::read(self.file()).unwrap()
    }
}

fn is_disk_full(error: &Result<(), DownloadError>) -> bool {
    matches!(error, Err(DownloadError::Io { kind, .. }) if is_out_of_space(*kind))
}

#[tokio::test]
async fn downloads_in_chunks() {
    let setup = Setup::new().await;

    setup
        .default_downloader()
        .download(&Setup::url(""), &setup.file())
        .await
        .unwrap();

    assert_eq!(setup.contents(), RANGE_BODY);
    assert_eq!(setup.ranges().await, ["bytes=0-3", "bytes=4-7", "bytes=8-11"]);
}

#[tokio::test]
async fn a_dropped_chunk_is_fetched_again_without_the_earlier_ones() {
    let setup = Setup::new().await;

    setup
        .default_downloader()
        .download(&Setup::url("?dropAt=4"), &setup.file())
        .await
        .unwrap();

    assert_eq!(setup.contents(), RANGE_BODY);
    assert_eq!(
        setup.ranges().await,
        ["bytes=0-3", "bytes=4-7", "bytes=4-7", "bytes=8-11"]
    );
}

#[tokio::test]
async fn resumes_from_the_partial_file() {
    let setup = Setup::new().await;
    std::fs::write(setup.file(), b"video").unwrap();

    setup
        .default_downloader()
        .download(&Setup::url(""), &setup.file())
        .await
        .unwrap();

    assert_eq!(setup.contents(), RANGE_BODY);
    assert_eq!(setup.ranges().await, ["bytes=5-8", "bytes=9-12"]);
}

#[tokio::test]
async fn a_complete_partial_file_is_kept() {
    let setup = Setup::new().await;
    std::fs::write(setup.file(), RANGE_BODY).unwrap();

    setup
        .default_downloader()
        .download(&Setup::url(""), &setup.file())
        .await
        .unwrap();

    assert_eq!(setup.contents(), RANGE_BODY);
    assert_eq!(setup.ranges().await, ["bytes=11-14"]);
}

#[tokio::test]
async fn a_server_ignoring_the_range_replaces_the_partial_file() {
    let setup = Setup::new().await;
    std::fs::write(setup.file(), b"stale").unwrap();

    setup
        .default_downloader()
        .download(&Setup::url("?ignoreRange=1"), &setup.file())
        .await
        .unwrap();

    assert_eq!(setup.contents(), RANGE_BODY);
}

#[tokio::test]
async fn server_errors_are_retried_then_passed_on() {
    let setup = Setup::new().await;

    let result = setup
        .default_downloader()
        .download(&Setup::url("?status=503"), &setup.file())
        .await;

    assert_eq!(
        result,
        Err(DownloadError::Provider(ProviderError::Server { status: 503 }))
    );
    assert_eq!(setup.ranges().await.len(), 3);
}

#[tokio::test]
async fn client_errors_fail_the_download_at_once() {
    let setup = Setup::new().await;

    let result = setup
        .default_downloader()
        .download(&Setup::url("?status=403"), &setup.file())
        .await;

    assert_eq!(result, Err(DownloadError::Failed { status: 403 }));
    assert_eq!(setup.ranges().await.len(), 1);
}

#[tokio::test]
async fn only_https_is_fetched() {
    let setup = Setup::new().await;
    for address in ["http://cdn.example/video.mp4", "file:///home/me/Movies/x.mov"] {
        let result = setup
            .default_downloader()
            .download(&Url::parse(address).unwrap(), &setup.file())
            .await;
        assert_eq!(result, Err(DownloadError::Refused(DownloadRefusal::NotHttps)));
    }
    assert_eq!(setup.ranges().await, Vec::<String>::new());
}

#[tokio::test]
async fn a_declared_total_above_the_cap_stops_at_once() {
    let setup = Setup::new().await;

    let result = setup
        .downloader(6, None)
        .download(&Setup::url(""), &setup.file())
        .await;

    assert_eq!(
        result,
        Err(DownloadError::Refused(DownloadRefusal::TooLarge { max_bytes: 6 }))
    );
    assert_eq!(setup.ranges().await.len(), 1);
    assert_eq!(setup.contents(), b"");
}

#[tokio::test]
async fn a_whole_body_above_the_cap_is_cut_off() {
    let setup = Setup::new().await;

    let result = setup
        .downloader(6, None)
        .download(&Setup::url("?ignoreRange=1"), &setup.file())
        .await;

    assert_eq!(
        result,
        Err(DownloadError::Refused(DownloadRefusal::TooLarge { max_bytes: 6 }))
    );
    assert_eq!(setup.ranges().await.len(), 1);
    assert!(setup.contents().len() <= 6);
}

#[tokio::test]
async fn a_whole_body_within_the_cap_is_streamed_to_the_file() {
    let setup = Setup::new().await;

    setup
        .downloader(11, None)
        .download(&Setup::url("?ignoreRange=1"), &setup.file())
        .await
        .unwrap();

    assert_eq!(setup.contents(), RANGE_BODY);
}

#[tokio::test]
async fn a_result_that_does_not_fit_on_the_disk_stops_before_writing() {
    let setup = Setup::new().await;

    let result = setup
        .downloader(1 << 20, Some(10))
        .download(&Setup::url(""), &setup.file())
        .await;

    assert!(is_disk_full(&result), "{result:?}");
    assert_eq!(setup.ranges().await.len(), 1);
    assert_eq!(setup.contents(), b"");
}

#[tokio::test]
async fn only_the_missing_part_has_to_fit() {
    let setup = Setup::new().await;
    std::fs::write(setup.file(), b"video").unwrap();

    setup
        .downloader(1 << 20, Some(6))
        .download(&Setup::url(""), &setup.file())
        .await
        .unwrap();

    assert_eq!(setup.contents(), RANGE_BODY);
}

#[tokio::test]
async fn a_whole_body_that_does_not_fit_stops() {
    let setup = Setup::new().await;

    let result = setup
        .downloader(1 << 20, Some(10))
        .download(&Setup::url("?ignoreRange=1"), &setup.file())
        .await;

    assert!(is_disk_full(&result), "{result:?}");
}

#[test]
fn free_space_is_read_for_the_files_volume() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("download.partial");
    assert!(VolumeFreeSpace.free_bytes(&file).unwrap_or(0) > 0);
}

#[tokio::test]
async fn a_partial_answer_without_a_total_stops() {
    let setup = Setup::new().await;

    let result = setup
        .default_downloader()
        .download(&Setup::url("?noTotal=1"), &setup.file())
        .await;

    assert_eq!(result, Err(DownloadError::Refused(DownloadRefusal::Stalled)));
    assert_eq!(setup.ranges().await.len(), 1);
}

#[tokio::test]
async fn a_partial_answer_without_bytes_stops() {
    let setup = Setup::new().await;

    let result = setup
        .default_downloader()
        .download(&Setup::url("?empty=1"), &setup.file())
        .await;

    assert_eq!(result, Err(DownloadError::Refused(DownloadRefusal::Stalled)));
    assert_eq!(setup.ranges().await.len(), 1);
}
