//! Real-ESRGAN ncnn-vulkan as the upscaler (`FrameUpscaling`): its pinned release archive downloaded into the app's
//! data dir once the person agreed, checked by SHA-256, the executable and the x4plus model unpacked, and run on a
//! folder of frames. It needs a Vulkan driver; without one it reports the engine unavailable, with the reason.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;

use async_trait::async_trait;
use jutsu_core::{EngineArchive, Progress, RealEsrgan};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::upscale::FrameUpscaling;
use crate::download::ModelDownloader;
use crate::error::MediaError;
use crate::tools::{command, tail};

/// Real-ESRGAN ncnn-vulkan, installed under `<data dir>/Engines/realesrgan-ncnn-vulkan-<tag>`.
pub struct RealEsrganEngine {
    archive: Option<EngineArchive>,
    /// Why there is no archive for this computer.
    missing: String,
    folder: PathBuf,
    downloader: ModelDownloader,
    /// What a run learned: no Vulkan driver, with the engine's own words.
    learned: Mutex<Option<String>>,
}

impl std::fmt::Debug for RealEsrganEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RealEsrganEngine")
            .field("archive", &self.archive)
            .field("folder", &self.folder)
            .finish_non_exhaustive()
    }
}

impl RealEsrganEngine {
    /// The engine for this computer, kept in `data_dir`.
    pub fn new(data_dir: &Path, downloader: ModelDownloader) -> Self {
        Self::with_archive(
            RealEsrgan::current(),
            RealEsrgan::unavailable_reason(std::env::consts::OS, std::env::consts::ARCH),
            data_dir,
            downloader,
        )
    }

    /// The engine of `archive`; `missing` says why there is none.
    pub fn with_archive(
        archive: Option<EngineArchive>,
        missing: String,
        data_dir: &Path,
        downloader: ModelDownloader,
    ) -> Self {
        Self {
            archive,
            missing,
            folder: data_dir.join(RealEsrgan::FOLDER),
            downloader,
            learned: Mutex::new(None),
        }
    }

    /// Where the engines are kept: what Settings › Storage counts and removes.
    pub fn engines_folder(&self) -> &Path {
        &self.folder
    }

    fn install_folder(&self, archive: &EngineArchive) -> PathBuf {
        self.folder
            .join(format!("realesrgan-ncnn-vulkan-{}", archive.tag))
    }

    fn executable(&self) -> Option<PathBuf> {
        let archive = self.archive.as_ref()?;
        Some(self.install_folder(archive).join(archive.executable))
    }

    /// What the engines take on disk, in bytes.
    pub fn size_on_disk(&self) -> i64 {
        ModelDownloader::size(&self.folder)
    }

    /// Removes every engine downloaded; the next upscale downloads it again.
    pub fn remove(&self) -> std::io::Result<()> {
        ModelDownloader::remove(&self.folder)
    }
}

#[async_trait]
impl FrameUpscaling for RealEsrganEngine {
    fn unavailable_reason(&self) -> Option<String> {
        if self.archive.is_none() {
            return Some(self.missing.clone());
        }
        self.learned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn is_installed(&self) -> bool {
        let Some(archive) = self.archive.as_ref() else {
            return false;
        };
        let folder = self.install_folder(archive);
        std::iter::once(archive.executable)
            .chain(archive.files.iter().copied())
            .all(|file| folder.join(file).is_file())
    }

    fn download_size(&self) -> Option<i64> {
        self.archive.map(|archive| archive.size)
    }

    async fn install(&self, progress: Progress<f64>, cancel: &CancellationToken) -> Result<(), MediaError> {
        let Some(archive) = self.archive else {
            return Err(MediaError::EngineUnavailableBecause(self.missing.clone()));
        };
        let url = Url::parse(archive.url).map_err(|error| MediaError::Io(error.to_string()))?;
        let file_name = url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .unwrap_or("realesrgan.zip")
            .to_owned();
        let zip = self.folder.join(&file_name);
        self.downloader
            .download_file(&url, archive.size, archive.sha256, &zip, Some(progress), cancel)
            .await
            .map_err(|error| match error {
                crate::ModelDownloadError::Cancelled => MediaError::Cancelled,
                other => MediaError::ExportFailed(other.to_string()),
            })?;
        let destination = self.install_folder(&archive);
        let staging = self.folder.join(format!(".unpacking-{}", uuid::Uuid::new_v4()));
        let unpacked = {
            let zip = zip.clone();
            let staging = staging.clone();
            tokio::task::spawn_blocking(move || unpack(&zip, &staging, &archive))
                .await
                .map_err(|error| MediaError::Io(error.to_string()))?
        };
        if let Err(error) = unpacked {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        let _ = std::fs::remove_dir_all(&destination);
        std::fs::rename(&staging, &destination)?;
        let _ = std::fs::remove_file(&zip);
        Ok(())
    }

    async fn upscale(
        &self,
        input: &Path,
        output: &Path,
        cancel: &CancellationToken,
    ) -> Result<(), MediaError> {
        let Some(executable) = self.executable() else {
            return Err(MediaError::EngineUnavailableBecause(self.missing.clone()));
        };
        let models = executable
            .parent()
            .map_or_else(|| PathBuf::from("models"), |folder| folder.join("models"));
        let mut command = command(&executable);
        command
            .arg("-i")
            .arg(input)
            .arg("-o")
            .arg(output)
            .arg("-m")
            .arg(&models)
            .args(["-n", RealEsrgan::MODEL, "-s", "4", "-f", "png"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| MediaError::ExportFailed(format!("realesrgan-ncnn-vulkan: {error}")))?;
        let errors = tokio::spawn(tail(child.stderr.take()));
        let status = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(MediaError::Cancelled),
            status = child.wait() => status?,
        };
        let errors = errors.await.unwrap_or_default();
        let missing_output = std::fs::read_dir(input)?
            .filter_map(Result::ok)
            .any(|entry| !output.join(entry.file_name()).is_file());
        if status.success() && !missing_output {
            return Ok(());
        }
        let text = errors.iter().cloned().collect::<Vec<_>>().join("\n");
        if let Some(reason) = vulkan_failure(&text) {
            *self
                .learned
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason.clone());
            return Err(MediaError::EngineUnavailableBecause(reason));
        }
        Err(MediaError::ExportFailed(if text.is_empty() {
            format!("realesrgan-ncnn-vulkan ended with {status}")
        } else {
            text
        }))
    }
}

/// Real-ESRGAN's words for a computer without a usable Vulkan driver, as the reason it can't run.
fn vulkan_failure(stderr: &str) -> Option<String> {
    let line = stderr.lines().find(|line| {
        let lower = line.to_lowercase();
        lower.contains("vkcreateinstance")
            || lower.contains("vulkan")
            || lower.contains("invalid gpu device")
            || lower.contains("no gpu")
    })?;
    Some(format!(
        "it needs a graphics driver with Vulkan, which this computer doesn't have ({}).",
        line.trim()
    ))
}

/// Unpacks the executable and the files the engine needs from the archive into `folder`; the executable may run.
fn unpack(zip: &Path, folder: &Path, archive: &EngineArchive) -> Result<(), MediaError> {
    let file = std::fs::File::open(zip)?;
    let mut entries = zip::ZipArchive::new(file)
        .map_err(|error| MediaError::ExportFailed(format!("the archive: {error}")))?;
    for name in std::iter::once(archive.executable).chain(archive.files.iter().copied()) {
        let mut entry = entries
            .by_name(name)
            .map_err(|error| MediaError::ExportFailed(format!("{name}: {error}")))?;
        let destination = folder.join(name);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        std::fs::write(&destination, bytes)?;
        #[cfg(unix)]
        if name == archive.executable {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Arc;

    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;
    use crate::hash::sha256_hex_of_file;
    use crate::test_support::dir;

    /// A small archive laid out as the release's: the executable and the model at the root, among other files.
    fn archive_bytes(folder: &Path) -> (Vec<u8>, String) {
        let path = folder.join("release.zip");
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in [
            ("realesrgan-ncnn-vulkan", "#!/bin/sh\n"),
            ("models/realesrgan-x4plus.bin", "weights"),
            ("models/realesrgan-x4plus.param", "layers"),
            ("README_ubuntu.md", "readme"),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let hash = sha256_hex_of_file(&path).unwrap();
        (std::fs::read(&path).unwrap(), hash)
    }

    /// Serves `body` for any path, answering a `Range` request with that part.
    struct Archive(Vec<u8>);

    impl Respond for Archive {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let range = request
                .headers
                .get("range")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("bytes="))
                .and_then(|value| value.split_once('-'))
                .and_then(|(first, last)| Some((first.parse::<usize>().ok()?, last.parse::<usize>().ok()?)));
            match range {
                Some((first, last)) => ResponseTemplate::new(206)
                    .set_body_bytes(self.0[first..=last.min(self.0.len() - 1)].to_vec()),
                None => ResponseTemplate::new(200).set_body_bytes(self.0.clone()),
            }
        }
    }

    fn leaked(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    async fn engine(data: &Path, bytes: Vec<u8>, sha256: &str) -> (MockServer, RealEsrganEngine) {
        let server = MockServer::start().await;
        let size = bytes.len() as i64;
        Mock::given(any())
            .respond_with(Archive(bytes))
            .mount(&server)
            .await;
        let archive = EngineArchive {
            tag: "v0.0.1",
            url: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.0.1/release.zip",
            size,
            sha256: leaked(sha256.to_owned()),
            executable: "realesrgan-ncnn-vulkan",
            files: &["models/realesrgan-x4plus.bin", "models/realesrgan-x4plus.param"],
        };
        let downloader =
            ModelDownloader::with_host(reqwest::Client::new(), Url::parse(&server.uri()).unwrap());
        (
            server,
            RealEsrganEngine::with_archive(Some(archive), String::new(), data, downloader),
        )
    }

    #[tokio::test]
    async fn the_archive_is_downloaded_checked_and_unpacked_once_agreed() {
        let root = dir();
        let data = root.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let (bytes, hash) = archive_bytes(root.path());
        let (_server, engine) = engine(&data, bytes.clone(), &hash).await;
        assert!(!engine.is_installed());
        assert_eq!(engine.download_size(), Some(bytes.len() as i64));
        let fractions = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&fractions);

        engine
            .install(
                Arc::new(move |value| log.lock().unwrap().push(value)),
                &CancellationToken::new(),
            )
            .await
            .unwrap();

        assert!(engine.is_installed());
        assert_eq!(fractions.lock().unwrap().last().copied(), Some(1.0));
        let folder = data.join("Engines/realesrgan-ncnn-vulkan-v0.0.1");
        assert!(!folder.join("README_ubuntu.md").exists());
        // The archive itself goes once unpacked.
        assert!(!data.join("Engines/release.zip").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(folder.join("realesrgan-ncnn-vulkan"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        assert!(engine.size_on_disk() > 0);
        engine.remove().unwrap();
        assert!(!engine.is_installed());
    }

    #[tokio::test]
    async fn an_archive_with_other_content_is_deleted_and_nothing_is_installed() {
        let root = dir();
        let data = root.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let (bytes, _) = archive_bytes(root.path());
        let wrong = "0".repeat(64);
        let (_server, engine) = engine(&data, bytes, &wrong).await;

        let outcome = engine.install(Arc::new(|_| {}), &CancellationToken::new()).await;

        assert_eq!(
            outcome,
            Err(MediaError::ExportFailed(
                "release.zip didn't match the model's checksum and was deleted. Try again.".to_owned()
            ))
        );
        assert!(!engine.is_installed());
        assert!(!data.join("Engines/release.zip").exists());
        assert!(!data.join("Engines/release.zip.part").exists());
    }

    #[test]
    fn a_platform_without_a_build_says_why() {
        let root = dir();
        let engine = RealEsrganEngine::with_archive(
            None,
            RealEsrgan::unavailable_reason("linux", "aarch64"),
            root.path(),
            ModelDownloader::default(),
        );
        assert_eq!(
            engine.unavailable_reason().as_deref(),
            Some("Real-ESRGAN publishes no build for linux on aarch64.")
        );
        assert!(!engine.is_installed());
        assert_eq!(engine.download_size(), None);
    }

    #[test]
    fn a_missing_vulkan_driver_is_the_reason() {
        assert_eq!(
            vulkan_failure("[0 llvmpipe]  queueC=0[1]\nvkCreateInstance failed -9\n").as_deref(),
            Some(
                "it needs a graphics driver with Vulkan, which this computer doesn't have (vkCreateInstance failed -9)."
            )
        );
        assert_eq!(vulkan_failure("decode image failed"), None);
    }
}
