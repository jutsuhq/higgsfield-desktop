//! Downloads a model's pinned files (`ModelManifest`) into a folder of the app's own, a piece at a time, so a download
//! that stops resumes from its last piece; each file is checked against its hash before it takes its name (Swift's
//! `ModelDownloader`).

use std::io::Write;
use std::path::Path;

use async_trait::async_trait;
use jutsu_core::{ManifestFile, ModelManifest, PortError, Progress, SpeechModelDownloading};
use sha1::Sha1;
use sha2::Digest;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::hash::{hex, read_chunks, sha256_hex_of_file};

/// Why a model download failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelDownloadError {
    /// The file came down with other content than the pinned revision has; it was deleted.
    #[error("{0} didn't match the model's checksum and was deleted. Try again.")]
    ChecksumMismatch(String),
    /// The server answered with something other than the piece asked for.
    #[error("The server didn't send {0} as asked. Try again later.")]
    BadResponse(String),
    /// The request failed: the network's description.
    #[error("{0}")]
    Network(String),
    /// Reading or writing the folder failed.
    #[error("{0}")]
    Io(String),
    /// The download was cancelled.
    #[error("cancelled")]
    Cancelled,
}

impl From<std::io::Error> for ModelDownloadError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<ModelDownloadError> for PortError {
    fn from(error: ModelDownloadError) -> Self {
        match error {
            ModelDownloadError::Cancelled => Self::Cancelled,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Downloads a model's files over HTTP `Range` requests.
#[derive(Debug, Clone, Default)]
pub struct ModelDownloader {
    client: reqwest::Client,
    /// Where requests go instead of the manifest's host: a mirror, or a test server.
    host: Option<Url>,
}

impl ModelDownloader {
    /// Bytes asked for at once: what a stopped download loses at most.
    pub const PIECE_SIZE: u64 = 8 << 20;

    /// A downloader on `client`.
    pub fn new(client: reqwest::Client) -> Self {
        Self { client, host: None }
    }

    /// A downloader that asks `host` (scheme, host and port) for every file, at the manifest's paths.
    pub fn with_host(client: reqwest::Client, host: Url) -> Self {
        Self {
            client,
            host: Some(host),
        }
    }

    fn url(&self, manifest: &ModelManifest, file: &ManifestFile) -> Option<Url> {
        let mut url = manifest.url(file)?;
        if let Some(host) = &self.host {
            url.set_scheme(host.scheme()).ok()?;
            url.set_host(host.host_str()).ok()?;
            url.set_port(host.port()).ok()?;
        }
        Some(url)
    }

    /// Whether every file is in `folder` at its size; reads no content.
    pub fn is_complete(manifest: &ModelManifest, folder: &Path) -> bool {
        manifest
            .files
            .iter()
            .all(|file| file_size(&folder.join(file.path)) == Some(file.size))
    }

    /// Downloads the files that aren't complete yet, each resuming where an earlier download stopped, and checks each
    /// against its hash; a file that doesn't match is deleted and the download fails. `progress` is 0…1 of the whole
    /// model.
    pub async fn download(
        &self,
        manifest: &ModelManifest,
        folder: &Path,
        progress: Option<Progress<f64>>,
        cancel: &CancellationToken,
    ) -> Result<(), ModelDownloadError> {
        let total = manifest.size().max(1) as f64;
        let report = |bytes: i64| {
            if let Some(progress) = &progress {
                progress(bytes as f64 / total);
            }
        };
        let mut done: i64 = 0;
        for file in &manifest.files {
            let destination = folder.join(file.path);
            if file_size(&destination) == Some(file.size) {
                done += file.size;
                continue;
            }
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut partial_name = destination.as_os_str().to_owned();
            partial_name.push(".part");
            let partial = std::path::PathBuf::from(partial_name);
            let url = self.url(manifest, file);
            self.fetch(url, file.path, file.size, &partial, cancel, |written| {
                report(done + written);
            })
            .await?;
            let checked = {
                let partial = partial.clone();
                let file = file.clone();
                tokio::task::spawn_blocking(move || matches(&partial, &file))
                    .await
                    .map_err(|error| ModelDownloadError::Io(error.to_string()))??
            };
            if !checked {
                let _ = std::fs::remove_file(&partial);
                return Err(ModelDownloadError::ChecksumMismatch(file.path.to_owned()));
            }
            let _ = std::fs::remove_file(&destination);
            std::fs::rename(&partial, &destination)?;
            done += file.size;
            report(done);
        }
        Ok(())
    }

    /// Downloads the file at `url` (`size` bytes) to `destination`, resuming a `.part` file an earlier download left,
    /// and checks it against `sha256` before it takes its name; a file that doesn't match is deleted and the download
    /// fails. `progress` is 0…1 of the file. A complete file already at `destination` is checked again only by size.
    pub async fn download_file(
        &self,
        url: &Url,
        size: i64,
        sha256: &str,
        destination: &Path,
        progress: Option<Progress<f64>>,
        cancel: &CancellationToken,
    ) -> Result<(), ModelDownloadError> {
        let name = destination
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if file_size(destination) == Some(size) {
            return Ok(());
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut partial_name = destination.as_os_str().to_owned();
        partial_name.push(".part");
        let partial = std::path::PathBuf::from(partial_name);
        let total = size.max(1) as f64;
        let report = |bytes: i64| {
            if let Some(progress) = &progress {
                progress(bytes as f64 / total);
            }
        };
        self.fetch(self.redirect(url.clone()), &name, size, &partial, cancel, report)
            .await?;
        let checked = {
            let partial = partial.clone();
            let sha256 = sha256.to_owned();
            tokio::task::spawn_blocking(move || sha256_hex_of_file(&partial).map(|hash| hash == sha256))
                .await
                .map_err(|error| ModelDownloadError::Io(error.to_string()))??
        };
        if !checked {
            let _ = std::fs::remove_file(&partial);
            return Err(ModelDownloadError::ChecksumMismatch(name));
        }
        let _ = std::fs::remove_file(destination);
        std::fs::rename(&partial, destination)?;
        report(size);
        Ok(())
    }

    /// `url` sent to `host` instead, when there is one.
    fn redirect(&self, mut url: Url) -> Option<Url> {
        if let Some(host) = &self.host {
            url.set_scheme(host.scheme()).ok()?;
            url.set_host(host.host_str()).ok()?;
            url.set_port(host.port()).ok()?;
        }
        Some(url)
    }

    /// Appends the rest of the file `name` (`size` bytes, at `url`) to `partial`, a piece at a time, from where it
    /// ends.
    async fn fetch(
        &self,
        url: Option<Url>,
        name: &str,
        size: i64,
        partial: &Path,
        cancel: &CancellationToken,
        progress: impl Fn(i64),
    ) -> Result<(), ModelDownloadError> {
        let bad = || ModelDownloadError::BadResponse(name.to_owned());
        let url = url.ok_or_else(bad)?;
        let expected = size;
        let size = u64::try_from(size).map_err(|_| bad())?;
        if file_size(partial).is_some_and(|length| length > expected) {
            std::fs::remove_file(partial)?;
        }
        let mut handle = std::fs::File::options().create(true).append(true).open(partial)?;
        let mut offset = handle.metadata()?.len();
        progress(offset as i64);
        while offset < size {
            if cancel.is_cancelled() {
                return Err(ModelDownloadError::Cancelled);
            }
            let end = (offset + Self::PIECE_SIZE).min(size) - 1;
            let request = self
                .client
                .get(url.clone())
                .header(reqwest::header::RANGE, format!("bytes={offset}-{end}"))
                .send();
            let response = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(ModelDownloadError::Cancelled),
                response = request => response.map_err(|error| ModelDownloadError::Network(error.to_string()))?,
            };
            let status = response.status().as_u16();
            let body = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(ModelDownloadError::Cancelled),
                body = response.bytes() => body.map_err(|error| ModelDownloadError::Network(error.to_string()))?,
            };
            let length = body.len() as u64;
            let whole_file = status == 200 && offset == 0 && length == size;
            if !whole_file && (status != 206 || length != end - offset + 1) {
                return Err(bad());
            }
            handle.write_all(&body)?;
            offset += length;
            progress(offset as i64);
        }
        handle.flush()?;
        Ok(())
    }

    /// What `folder` takes on disk, in bytes.
    pub fn size(folder: &Path) -> i64 {
        fn walk(folder: &Path) -> i64 {
            let Ok(entries) = std::fs::read_dir(folder) else {
                return 0;
            };
            entries
                .filter_map(Result::ok)
                .map(|entry| match entry.file_type() {
                    Ok(kind) if kind.is_dir() => walk(&entry.path()),
                    Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |metadata| allocated(&metadata)),
                    _ => 0,
                })
                .sum()
        }
        walk(folder)
    }

    /// Removes `folder`; nothing to do when it isn't there.
    pub fn remove(folder: &Path) -> std::io::Result<()> {
        if !folder.exists() {
            return Ok(());
        }
        std::fs::remove_dir_all(folder)
    }
}

#[cfg(unix)]
fn allocated(metadata: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    (metadata.blocks() * 512) as i64
}

#[cfg(not(unix))]
fn allocated(metadata: &std::fs::Metadata) -> i64 {
    metadata.len() as i64
}

fn file_size(path: &Path) -> Option<i64> {
    std::fs::metadata(path).ok().map(|metadata| metadata.len() as i64)
}

/// Whether the file's content hashes to what the manifest says: SHA-256, or Git's blob hash.
pub fn matches(path: &Path, file: &ManifestFile) -> std::io::Result<bool> {
    if file_size(path) != Some(file.size) {
        return Ok(false);
    }
    if let Some(sha256) = file.sha256 {
        return Ok(sha256_hex_of_file(path)? == sha256);
    }
    if let Some(git_blob) = file.git_blob {
        let mut hasher = Sha1::new();
        hasher.update(format!("blob {}\0", file.size).as_bytes());
        read_chunks(path, |chunk| hasher.update(chunk))?;
        return Ok(hex(&hasher.finalize()) == git_blob);
    }
    Ok(false)
}

#[async_trait]
impl SpeechModelDownloading for ModelDownloader {
    fn is_complete(&self, manifest: &ModelManifest, folder: &Path) -> bool {
        Self::is_complete(manifest, folder)
    }

    async fn download(
        &self,
        manifest: &ModelManifest,
        folder: &Path,
        progress: Progress<f64>,
    ) -> Result<(), PortError> {
        Ok(Self::download(self, manifest, folder, Some(progress), &CancellationToken::new()).await?)
    }

    fn size(&self, folder: &Path) -> i64 {
        Self::size(folder)
    }

    fn remove(&self, folder: &Path) -> Result<(), PortError> {
        Self::remove(folder).map_err(|error| PortError::Other(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use jutsu_core::ParakeetUltra;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;
    use crate::test_support::dir;

    /// "hello", by SHA-256 and by Git's blob hash.
    const SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    const GIT_BLOB: &str = "b6fc4c620b67d95f953a5c1c1230aaab5db5a1b0";

    /// Serves "hello" for every file but one named "bad", which has other content of the same size, and answers a
    /// `Range` request with just that part, as Hugging Face does.
    struct ModelFileStub;

    impl Respond for ModelFileStub {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let body: &[u8] = if request.url.path().ends_with("/bad") {
                b"jello"
            } else {
                b"hello"
            };
            let range = request
                .headers
                .get("range")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("bytes="))
                .and_then(|value| value.split_once('-'))
                .and_then(|(first, last)| Some((first.parse::<usize>().ok()?, last.parse::<usize>().ok()?)));
            match range {
                Some((first, last)) => ResponseTemplate::new(206)
                    .set_body_bytes(body[first..(last + 1).min(body.len())].to_vec()),
                None => ResponseTemplate::new(200).set_body_bytes(body.to_vec()),
            }
        }
    }

    async fn downloader() -> (MockServer, ModelDownloader) {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ModelFileStub)
            .mount(&server)
            .await;
        let host = Url::parse(&server.uri()).unwrap();
        (server, ModelDownloader::with_host(reqwest::Client::new(), host))
    }

    fn manifest(files: Vec<ManifestFile>) -> ModelManifest {
        ModelManifest {
            repository: "owner/model",
            revision: "0123abcd",
            files,
        }
    }

    fn file(
        path: &'static str,
        size: i64,
        sha256: Option<&'static str>,
        git_blob: Option<&'static str>,
    ) -> ManifestFile {
        ManifestFile {
            path,
            size,
            sha256,
            git_blob,
        }
    }

    fn write(folder: &Path, text: &str, path: &str) -> std::path::PathBuf {
        let url = folder.join(path);
        std::fs::create_dir_all(url.parent().unwrap()).unwrap();
        std::fs::write(&url, text).unwrap();
        url
    }

    #[test]
    fn a_file_matches_by_either_hash_and_at_its_size_only() {
        let folder = dir();
        let good = write(folder.path(), "hello", "good");
        let other = write(folder.path(), "jello", "other");

        assert!(matches(&good, &file("good", 5, Some(SHA256), None)).unwrap());
        assert!(matches(&good, &file("good", 5, None, Some(GIT_BLOB))).unwrap());
        assert!(!matches(&other, &file("other", 5, Some(SHA256), None)).unwrap());
        assert!(!matches(&other, &file("other", 5, None, Some(GIT_BLOB))).unwrap());
        assert!(!matches(&good, &file("good", 6, Some(SHA256), None)).unwrap());
        assert!(!matches(&good, &file("good", 5, None, None)).unwrap());
    }

    #[tokio::test]
    async fn downloads_checked_files_and_resumes_a_partial_one() {
        let folder = dir();
        let (_server, downloader) = downloader().await;
        let files = manifest(vec![
            file("Model.mlmodelc/weights/weight.bin", 5, Some(SHA256), None),
            file("vocab.json", 5, None, Some(GIT_BLOB)),
        ]);
        write(folder.path(), "he", "vocab.json.part");
        assert!(!ModelDownloader::is_complete(&files, folder.path()));

        let fractions = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&fractions);
        let progress: Progress<f64> = Arc::new(move |value| log.lock().unwrap().push(value));
        downloader
            .download(&files, folder.path(), Some(progress), &CancellationToken::new())
            .await
            .unwrap();

        assert!(ModelDownloader::is_complete(&files, folder.path()));
        assert_eq!(
            std::fs::read_to_string(folder.path().join("vocab.json")).unwrap(),
            "hello"
        );
        assert!(!folder.path().join("vocab.json.part").exists());
        assert_eq!(fractions.lock().unwrap().last().copied(), Some(1.0));
        assert!(ModelDownloader::size(folder.path()) > 0);
        ModelDownloader::remove(folder.path()).unwrap();
        assert_eq!(ModelDownloader::size(folder.path()), 0);
        ModelDownloader::remove(folder.path()).unwrap();
    }

    #[tokio::test]
    async fn a_file_with_other_content_is_deleted_and_fails() {
        let folder = dir();
        let (_server, downloader) = downloader().await;
        let files = manifest(vec![file("bad", 5, Some(SHA256), None)]);

        let outcome = downloader
            .download(&files, folder.path(), None, &CancellationToken::new())
            .await;

        assert_eq!(
            outcome,
            Err(ModelDownloadError::ChecksumMismatch("bad".to_owned()))
        );
        assert!(!folder.path().join("bad").exists());
        assert!(!folder.path().join("bad.part").exists());
    }

    #[test]
    fn each_file_comes_from_the_pinned_revision() {
        let one = file("Encoder v3.mlmodelc/model.mil", 1, None, None);
        let url = manifest(vec![one.clone()]).url(&one).unwrap();
        assert_eq!(
            url.as_str(),
            "https://huggingface.co/owner/model/resolve/0123abcd/Encoder%20v3.mlmodelc/model.mil"
        );
        let parakeet = ParakeetUltra::manifest();
        assert_eq!(parakeet.files.len(), 18);
        assert_eq!(parakeet.size(), 632_162_964);
        assert!(
            parakeet
                .files
                .iter()
                .all(|file| file.sha256.is_none() != file.git_blob.is_none())
        );
        assert_eq!(parakeet.revision.len(), 40);
    }

    #[test]
    fn messages_name_the_file() {
        assert_eq!(
            ModelDownloadError::ChecksumMismatch("vocab.json".into()).to_string(),
            "vocab.json didn't match the model's checksum and was deleted. Try again."
        );
        assert_eq!(
            ModelDownloadError::BadResponse("vocab.json".into()).to_string(),
            "The server didn't send vocab.json as asked. Try again later."
        );
    }
}
