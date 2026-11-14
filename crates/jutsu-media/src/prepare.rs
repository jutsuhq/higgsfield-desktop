//! `MediaPreparing` on ffmpeg (Swift's `MediaPreparer`): results land in a working folder, named after the source
//! content and the options, so preparing the same input again returns the existing file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use jutsu_core::support::text::swift_double;
use jutsu_core::{
    JobSpec, LocalAudio, LocalImage, MediaPreparing, OutputMedia, PixelSize, PortError, PreparedFile,
    Progress, SourceClip, Validation,
};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::error::MediaError;
use crate::hash::{FileHashCache, sha256_hex, sha256_hex_of_file};
use crate::probe::{audio_duration, probe, probe_url};
use crate::still::{content_format, decode_upright, fit};
use crate::tools::{FfmpegRun, FfmpegTools, os_args};
use crate::video::{VideoRequest, export, reencode};

/// Turns the user's files into files the API accepts, in `working_directory`.
#[derive(Debug, Clone)]
pub struct MediaPreparer {
    tools: FfmpegTools,
    working_directory: PathBuf,
    /// Source and output hashes, so a cache hit or a resumed job doesn't read the files again.
    hashes: Arc<FileHashCache>,
}

/// A scratch file next to the cache entry it becomes; removed unless it was moved into place, so a cancelled or failed
/// run never leaves a half-written file under the cache name.
pub(crate) struct Partial(pub PathBuf);

impl Drop for Partial {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl MediaPreparer {
    /// Largest upload the API accepts.
    pub const FILE_LENGTH_LIMIT: i64 = 200 * 1024 * 1024;
    /// Longest side of a prepared image, in pixels.
    pub const MAXIMUM_IMAGE_PIXEL_SIZE: u32 = 4096;
    /// An export may end up to this much shorter than asked (last frame duration, rounding).
    const DURATION_TOLERANCE: f64 = 0.5;
    /// The audio types sent as they are, by file extension, with the content type they are uploaded as.
    const PASS_THROUGH_AUDIO: [(&'static str, &'static str); 3] =
        [("mp3", "audio/mpeg"), ("wav", "audio/wav"), ("m4a", "audio/mp4")];

    /// A preparer writing into `working_directory`, with its hashes saved there in `hashes.json`.
    pub fn new(tools: FfmpegTools, working_directory: impl Into<PathBuf>) -> Self {
        let working_directory = working_directory.into();
        let hashes = Arc::new(FileHashCache::new(Some(working_directory.join("hashes.json"))));
        Self::with_hashes(tools, working_directory, hashes)
    }

    /// A preparer sharing `hashes` with others.
    pub fn with_hashes(
        tools: FfmpegTools,
        working_directory: impl Into<PathBuf>,
        hashes: Arc<FileHashCache>,
    ) -> Self {
        Self {
            tools,
            working_directory: working_directory.into(),
            hashes,
        }
    }

    /// The tools it runs.
    pub fn tools(&self) -> &FfmpegTools {
        &self.tools
    }

    /// Where its results land.
    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    /// Trims the selected range, transcodes to MP4 H.264/AAC within `FILE_LENGTH_LIMIT`, and upscales when the clip
    /// allows it. A first pass at high quality stops at the limit; a result that came out short or large is encoded
    /// again at a bitrate computed from its length.
    pub async fn prepare_video(
        &self,
        source: &SourceClip,
        progress: Option<Progress<f64>>,
        cancel: &CancellationToken,
    ) -> Result<PreparedFile, MediaError> {
        let path = local_path(&source.file_url).ok_or(MediaError::UnsupportedFormat)?;
        let info = probe(&self.tools, &path).await?;
        let start = source.range.lower().max(0.0);
        let end = source.range.upper().min(info.duration);
        if end <= start {
            return Err(MediaError::InvalidRange);
        }
        let render_size = if source.allows_upscale {
            Validation::upscaled_size(info.pixel_size.width, info.pixel_size.height)
        } else {
            info.pixel_size
        };

        let source_hash = self.content_hash(&path).await?;
        let key = sha256_hex(&format!(
            "video|{source_hash}|{}|{}|{}x{}",
            swift_double(start),
            swift_double(end),
            render_size.width,
            render_size.height
        ));
        let output = self.working_directory.join(format!("{key}.mp4"));
        if let Some(cached) = self.cached_file(&output, "video/mp4").await? {
            if let Some(progress) = &progress {
                progress(1.0);
            }
            return Ok(cached);
        }

        let request = VideoRequest {
            source: path,
            start,
            end,
            upscaled_size: (render_size != info.pixel_size).then_some(render_size),
            byte_limit: Self::FILE_LENGTH_LIMIT,
        };
        let partial = self.partial(&output)?;
        export(&self.tools, &request, &partial.0, progress.clone(), cancel).await?;
        // The first pass stops at the size limit instead of failing, so a short result means it didn't fit.
        let exported = probe(&self.tools, &partial.0).await?;
        if exported.file_size > request.byte_limit
            || exported.duration < end - start - Self::DURATION_TOLERANCE
        {
            std::fs::remove_file(&partial.0)?;
            reencode(&self.tools, &request, &partial.0, progress, cancel).await?;
            let size = file_size(&partial.0)?;
            if size > request.byte_limit {
                return Err(MediaError::TooLarge { byte_count: size });
            }
        }
        self.finish(partial, &output, "video/mp4").await
    }

    /// JPEG and PNG within `MAXIMUM_IMAGE_PIXEL_SIZE` are passed through untouched. Everything else (HEIC, TIFF,
    /// oversized) is decoded with its orientation applied, downscaled and saved as JPEG.
    pub async fn prepare_image(&self, image: &LocalImage) -> Result<PreparedFile, MediaError> {
        let path = local_path(&image.file_url).ok_or(MediaError::UnreadableImage)?;
        let size = crate::still::size(&self.tools, &path)
            .await
            .ok_or(MediaError::UnreadableImage)?;
        let fits_as_is = size.width.max(size.height) <= i64::from(Self::MAXIMUM_IMAGE_PIXEL_SIZE);
        let pass_through = match content_format(&path) {
            Some(image::ImageFormat::Png) if fits_as_is => Some(("png", "image/png")),
            Some(image::ImageFormat::Jpeg) if fits_as_is => Some(("jpeg", "image/jpeg")),
            _ => None,
        };
        let (extension, content_type) = pass_through.unwrap_or(("jpeg", "image/jpeg"));

        let source_hash = self.content_hash(&path).await?;
        let key = sha256_hex(&format!("image|{source_hash}|{}", Self::MAXIMUM_IMAGE_PIXEL_SIZE));
        let output = self.working_directory.join(format!("{key}.{extension}"));
        if let Some(cached) = self.cached_file(&output, content_type).await? {
            return Ok(cached);
        }

        let partial = self.partial(&output)?;
        if pass_through.is_some() {
            std::fs::copy(&path, &partial.0)?;
        } else {
            let picture = decode_upright(&self.tools, &path).await?;
            let target = partial.0.clone();
            tokio::task::spawn_blocking(move || write_jpeg(picture, &target))
                .await
                .map_err(|error| MediaError::ExportFailed(error.to_string()))??;
        }
        self.finish(partial, &output, content_type).await
    }

    /// MP3, WAV and M4A are passed through untouched; any other file ffmpeg reads sound from (AIFF, CAF, a video's
    /// soundtrack) is converted to M4A (AAC).
    pub async fn prepare_audio(&self, audio: &LocalAudio) -> Result<PreparedFile, MediaError> {
        let path = local_path(&audio.file_url).ok_or(MediaError::UnsupportedFormat)?;
        audio_duration(&self.tools, &path).await?;
        let file_extension = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let pass_through = Self::PASS_THROUGH_AUDIO
            .iter()
            .find(|(extension, _)| *extension == file_extension)
            .map(|(_, content_type)| *content_type);
        let source_hash = self.content_hash(&path).await?;
        let key = sha256_hex(&format!("audio|{source_hash}"));
        let extension = if pass_through.is_some() {
            file_extension.as_str()
        } else {
            "m4a"
        };
        let output = self.working_directory.join(format!("{key}.{extension}"));
        let content_type = pass_through.unwrap_or("audio/mp4");
        if let Some(cached) = self.cached_file(&output, content_type).await? {
            return Ok(cached);
        }

        let partial = self.partial(&output)?;
        if pass_through.is_some() {
            std::fs::copy(&path, &partial.0)?;
        } else {
            let mut args = os_args(["-y", "-i"]);
            args.push(path.as_os_str().to_owned());
            args.extend(os_args([
                "-map", "0:a:0", "-vn", "-sn", "-dn", "-c:a", "aac", "-b:a", "128000", "-f", "ipod",
            ]));
            args.push(partial.0.as_os_str().to_owned());
            FfmpegRun {
                args,
                output: Some(&partial.0),
                duration: 0.0,
                progress: None,
                working_directory: None,
            }
            .run(&self.tools, &CancellationToken::new())
            .await?;
        }
        self.finish(partial, &output, content_type).await
    }

    /// The image's size as it shows, upright: EXIF orientations 5–8 turn it a quarter, so its sides swap. `None` when
    /// it can't be read.
    pub async fn image_size(&self, image: &LocalImage) -> Option<PixelSize> {
        let path = local_path(&image.file_url)?;
        crate::still::size(&self.tools, &path).await
    }

    /// A downloaded result's length and upright frame size, read from the file; `None` when it can't be read.
    pub async fn output_media(&self, file: &Url) -> Option<OutputMedia> {
        let info = probe_url(&self.tools, file).await.ok()?;
        (info.duration.is_finite() && info.duration > 0.0 && info.pixel_size.is_valid()).then_some(
            OutputMedia {
                duration: info.duration,
                pixel_size: info.pixel_size,
            },
        )
    }

    /// A clip made without a source keeps its own sound: the model made it, or none when the job asked for silence.
    /// So does Extend's new part, which no part of the source's sound belongs to. An operation's source is a whole
    /// result, Extend's too, whose sound is the one to put back.
    pub async fn restore_audio(
        &self,
        spec: &JobSpec,
        result: &Path,
        output: &Path,
    ) -> Result<bool, MediaError> {
        let Some(clip) = &spec.source else {
            return Ok(false);
        };
        if spec.mode == jutsu_core::Mode::Extend && spec.operation.is_none() {
            return Ok(false);
        }
        let source = local_path(&clip.file_url).ok_or(MediaError::UnsupportedFormat)?;
        crate::audio::restore(&self.tools, spec.audio, clip.range, &source, result, output).await
    }

    async fn cached_file(&self, path: &Path, content_type: &str) -> Result<Option<PreparedFile>, MediaError> {
        if !path.exists() {
            return Ok(None);
        }
        self.prepared_file(path, content_type).await.map(Some)
    }

    fn partial(&self, output: &Path) -> Result<Partial, MediaError> {
        std::fs::create_dir_all(&self.working_directory)?;
        let stem = output
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let extension = output
            .extension()
            .map(|extension| extension.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = format!(
            "{stem}-{}.partial.{extension}",
            jutsu_core::support::uuid_string(uuid::Uuid::new_v4())
        );
        Ok(Partial(self.working_directory.join(name)))
    }

    /// Moves the finished work into place in one step.
    async fn finish(
        &self,
        partial: Partial,
        output: &Path,
        content_type: &str,
    ) -> Result<PreparedFile, MediaError> {
        if let Err(error) = std::fs::rename(&partial.0, output) {
            // A concurrent call for the same input may have finished first; its file has the same content.
            if !output.exists() {
                return Err(error.into());
            }
        }
        drop(partial);
        self.prepared_file(output, content_type).await
    }

    async fn prepared_file(&self, path: &Path, content_type: &str) -> Result<PreparedFile, MediaError> {
        let file_url = Url::from_file_path(path)
            .map_err(|()| MediaError::Io(format!("{} isn't an absolute path", path.display())))?;
        Ok(PreparedFile {
            file_url,
            content_type: content_type.to_owned(),
            byte_count: file_size(path)?,
            content_hash: self.content_hash(path).await?,
        })
    }

    /// The file's SHA-256, from the hash cache when the file hasn't changed.
    pub async fn content_hash(&self, path: &Path) -> Result<String, MediaError> {
        let hashes = Arc::clone(&self.hashes);
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || hashes.hash(&path, sha256_hex_of_file))
            .await
            .map_err(|error| MediaError::Io(error.to_string()))?
            .map_err(MediaError::from)
    }
}

fn write_jpeg(picture: image::DynamicImage, path: &Path) -> Result<(), MediaError> {
    let rgb = fit(picture, MediaPreparer::MAXIMUM_IMAGE_PIXEL_SIZE).to_rgb8();
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    image::codecs::jpeg::JpegEncoder::new_with_quality(file, 90)
        .encode_image(&rgb)
        .map_err(|_| MediaError::ExportFailed("JPEG encoding failed".to_owned()))
}

/// The path of a `file:` URL.
pub(crate) fn local_path(url: &Url) -> Option<PathBuf> {
    url.to_file_path().ok()
}

fn file_size(path: &Path) -> Result<i64, MediaError> {
    Ok(std::fs::metadata(path)?.len() as i64)
}

#[async_trait]
impl MediaPreparing for MediaPreparer {
    async fn prepare_video(
        &self,
        source: &SourceClip,
        progress: Progress<f64>,
    ) -> Result<PreparedFile, PortError> {
        Ok(Self::prepare_video(self, source, Some(progress), &CancellationToken::new()).await?)
    }

    async fn prepare_image(&self, image: &LocalImage) -> Result<PreparedFile, PortError> {
        Ok(Self::prepare_image(self, image).await?)
    }

    async fn prepare_audio(&self, audio: &LocalAudio) -> Result<PreparedFile, PortError> {
        Ok(Self::prepare_audio(self, audio).await?)
    }

    async fn restore_audio(&self, spec: &JobSpec, result: &Url, output: &Url) -> Result<bool, PortError> {
        let (Some(result), Some(output)) = (local_path(result), local_path(output)) else {
            return Err(MediaError::UnsupportedFormat.into());
        };
        Ok(Self::restore_audio(self, spec, &result, &output).await?)
    }

    async fn image_size(&self, image: &LocalImage) -> Option<PixelSize> {
        Self::image_size(self, image).await
    }

    async fn output_media(&self, file: &Url) -> Option<OutputMedia> {
        Self::output_media(self, file).await
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use jutsu_core::ClosedRange;

    use super::*;
    use crate::test_support::{Still, Video, dir, image_size, make_image, tone, tools};

    async fn clip(
        tools: &FfmpegTools,
        path: &Path,
        range: ClosedRange<f64>,
        allows_upscale: bool,
    ) -> SourceClip {
        let info = probe(tools, path).await.unwrap();
        let mut clip = SourceClip::new(
            Url::from_file_path(path).unwrap(),
            info.duration,
            info.pixel_size,
            range,
        );
        clip.allows_upscale = allows_upscale;
        clip
    }

    fn modified(path: &Path) -> SystemTime {
        std::fs::metadata(path).unwrap().modified().unwrap()
    }

    fn path_of(file: &PreparedFile) -> PathBuf {
        file.file_url.to_file_path().unwrap()
    }

    #[tokio::test]
    async fn trims_and_transcodes_to_h264_mp4() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = Video::new(6).write(&tools, folder.path()).await;
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));

        let source = clip(&tools, &url, ClosedRange::new(1.0, 5.0), false).await;
        let file = preparer
            .prepare_video(&source, None, &CancellationToken::new())
            .await
            .unwrap();
        let info = probe(&tools, &path_of(&file)).await.unwrap();
        let output_hash = sha256_hex_of_file(&path_of(&file)).unwrap();

        assert!((info.duration - 4.0).abs() < 0.1, "{}", info.duration);
        assert_eq!(info.codec, "avc1");
        assert_eq!(info.pixel_size, PixelSize::new(720, 480));
        assert_eq!(path_of(&file).extension().unwrap(), "mp4");
        assert_eq!(file.content_type, "video/mp4");
        assert_eq!(file.byte_count, info.file_size);
        assert!(file.byte_count <= MediaPreparer::FILE_LENGTH_LIMIT);
        assert_eq!(file.content_hash, output_hash);
    }

    #[tokio::test]
    async fn transcodes_hevc_mov_to_h264() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        // A build without an HEVC encoder has nothing to test without the fixture.
        let Some(url) = Video::new(5)
            .size(1280, 720)
            .hevc_mov()
            .try_write(&tools, folder.path())
            .await
        else {
            eprintln!("skipped: no HEVC encoder for the fixture");
            return;
        };
        let input = probe(&tools, &url).await.unwrap();
        assert_eq!(input.codec, "hvc1");
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));
        let source = clip(&tools, &url, ClosedRange::new(0.0, 5.0), false).await;

        let file = preparer
            .prepare_video(&source, None, &CancellationToken::new())
            .await
            .unwrap();
        let output = probe(&tools, &path_of(&file)).await.unwrap();

        assert_eq!(output.codec, "avc1");
        assert_eq!(output.pixel_size, PixelSize::new(1280, 720));
    }

    #[tokio::test]
    async fn trims_long_clip_to_thirty_seconds() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = Video::new(35).rate(1).write(&tools, folder.path()).await;
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));

        let source = clip(&tools, &url, ClosedRange::new(2.0, 32.0), false).await;
        let file = preparer
            .prepare_video(&source, None, &CancellationToken::new())
            .await
            .unwrap();
        let info = probe(&tools, &path_of(&file)).await.unwrap();

        assert!((info.duration - 30.0).abs() < 0.1, "{}", info.duration);
        assert!(file.byte_count <= MediaPreparer::FILE_LENGTH_LIMIT);
    }

    #[tokio::test]
    async fn upscales_small_frame_for_object_swap() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = Video::new(6).write(&tools, folder.path()).await;
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));

        let source = clip(&tools, &url, ClosedRange::new(0.0, 4.0), true).await;
        let file = preparer
            .prepare_video(&source, None, &CancellationToken::new())
            .await
            .unwrap();
        let info = probe(&tools, &path_of(&file)).await.unwrap();

        assert_eq!(info.pixel_size, PixelSize::new(784, 524));
    }

    #[tokio::test]
    async fn second_call_returns_cached_file() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = Video::new(6).write(&tools, folder.path()).await;
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));
        let source = clip(&tools, &url, ClosedRange::new(0.0, 4.0), false).await;
        let cancel = CancellationToken::new();

        let first = preparer.prepare_video(&source, None, &cancel).await.unwrap();
        let created = modified(&path_of(&first));
        let second = preparer.prepare_video(&source, None, &cancel).await.unwrap();
        let reused = modified(&path_of(&second));
        let other_source = clip(&tools, &url, ClosedRange::new(1.0, 5.0), false).await;
        let other = preparer
            .prepare_video(&other_source, None, &cancel)
            .await
            .unwrap();

        assert_eq!(second, first);
        assert_eq!(reused, created);
        assert_ne!(other.file_url, first.file_url);
    }

    #[tokio::test]
    async fn a_range_outside_the_video_is_refused() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = Video::new(2).write(&tools, folder.path()).await;
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));
        let source = clip(&tools, &url, ClosedRange::new(3.0, 4.0), false).await;

        assert_eq!(
            preparer
                .prepare_video(&source, None, &CancellationToken::new())
                .await,
            Err(MediaError::InvalidRange)
        );
    }

    /// WAV is sent as it is; the speech's length is read from the file.
    #[tokio::test]
    async fn passes_wav_through_and_reads_its_length() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let wav = tone(&tools, folder.path(), 2.5, "wav").await;
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));

        let file = preparer
            .prepare_audio(&LocalAudio {
                file_url: Url::from_file_path(&wav).unwrap(),
                duration: 2.5,
            })
            .await
            .unwrap();

        assert_eq!(file.content_type, "audio/wav");
        assert_eq!(path_of(&file).extension().unwrap(), "wav");
        assert_eq!(
            std::fs::read(path_of(&file)).unwrap(),
            std::fs::read(&wav).unwrap()
        );
        assert!((audio_duration(&tools, &wav).await.unwrap() - 2.5).abs() < 0.05);
    }

    /// Another type is converted to M4A, as long as the original.
    #[tokio::test]
    async fn converts_caf_to_m4a() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let caf = tone(&tools, folder.path(), 2.0, "caf").await;
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));

        let file = preparer
            .prepare_audio(&LocalAudio {
                file_url: Url::from_file_path(&caf).unwrap(),
                duration: 2.0,
            })
            .await
            .unwrap();

        assert_eq!(file.content_type, "audio/mp4");
        assert_eq!(path_of(&file).extension().unwrap(), "m4a");
        assert!((audio_duration(&tools, &path_of(&file)).await.unwrap() - 2.0).abs() < 0.1);
    }

    /// A file without sound isn't a speech recording.
    #[tokio::test]
    async fn refuses_a_file_without_sound() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let image = make_image(folder.path(), 64, 64, Still::Png, 1);

        assert_eq!(
            audio_duration(&tools, &image).await,
            Err(MediaError::UnsupportedFormat)
        );
    }

    fn local(path: &Path) -> LocalImage {
        LocalImage::new(Url::from_file_path(path).unwrap())
    }

    /// A start frame's shape sets the account's frame for a clip from frames, so the size is the one the image shows.
    #[tokio::test]
    async fn image_size_is_upright() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));
        let upright = make_image(folder.path(), 480, 854, Still::Png, 1);
        // Orientation 6: stored landscape, shown turned a quarter.
        let turned = make_image(folder.path(), 854, 480, Still::Jpeg, 6);

        assert_eq!(
            preparer.image_size(&local(&upright)).await,
            Some(PixelSize::new(480, 854))
        );
        assert_eq!(
            preparer.image_size(&local(&turned)).await,
            Some(PixelSize::new(480, 854))
        );
        let missing = folder.path().join("missing.png");
        assert_eq!(preparer.image_size(&local(&missing)).await, None);
    }

    #[tokio::test]
    async fn converts_tiff_to_jpeg() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = make_image(folder.path(), 1200, 800, Still::Tiff, 1);
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));

        let file = preparer.prepare_image(&local(&url)).await.unwrap();

        assert_eq!(file.content_type, "image/jpeg");
        assert_eq!(path_of(&file).extension().unwrap(), "jpeg");
        assert_eq!(image_size(&path_of(&file)), (1200, 800));
    }

    /// The upright picture is what is sent, so a turned JPEG over the limit comes out turned.
    #[tokio::test]
    async fn a_converted_image_is_upright() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = make_image(folder.path(), 4200, 100, Still::Jpeg, 6);
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));

        let file = preparer.prepare_image(&local(&url)).await.unwrap();

        let (width, height) = image_size(&path_of(&file));
        assert_eq!(height, 4096);
        assert!(width == 97 || width == 98, "{width}");
    }

    #[tokio::test]
    async fn keeps_small_png_as_is() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = make_image(folder.path(), 300, 200, Still::Png, 1);
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));

        let file = preparer.prepare_image(&local(&url)).await.unwrap();

        assert_eq!(file.content_type, "image/png");
        assert_eq!(file.content_hash, sha256_hex_of_file(&url).unwrap());
    }

    #[tokio::test]
    async fn a_replaced_source_is_hashed_again() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = make_image(folder.path(), 300, 200, Still::Png, 1);
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));
        let first = preparer.prepare_image(&local(&url)).await.unwrap();

        let other = make_image(folder.path(), 320, 200, Still::Png, 1);
        std::fs::rename(&other, &url).unwrap();
        let second = preparer.prepare_image(&local(&url)).await.unwrap();

        assert_eq!(second.content_hash, sha256_hex_of_file(&url).unwrap());
        assert_ne!(second.content_hash, first.content_hash);
    }

    #[tokio::test]
    async fn downscales_oversized_image() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = make_image(folder.path(), 6000, 300, Still::Png, 1);
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));

        let file = preparer.prepare_image(&local(&url)).await.unwrap();
        let (width, height) = image_size(&path_of(&file));

        assert_eq!(file.content_type, "image/jpeg");
        assert_eq!(width, MediaPreparer::MAXIMUM_IMAGE_PIXEL_SIZE);
        assert!(height == 205 || height == 204, "{height}");
    }

    #[tokio::test]
    async fn rejects_non_image() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = folder.path().join("photo.heic");
        std::fs::write(&url, "plain text").unwrap();
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));

        assert_eq!(
            preparer.prepare_image(&local(&url)).await,
            Err(MediaError::UnreadableImage)
        );
    }

    #[tokio::test]
    async fn output_media_reads_a_result_or_nothing() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = Video::new(2).size(320, 240).write(&tools, folder.path()).await;
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));

        let media = preparer
            .output_media(&Url::from_file_path(&url).unwrap())
            .await
            .unwrap();
        assert_eq!(media.pixel_size, PixelSize::new(320, 240));
        assert!((media.duration - 2.0).abs() < 0.1);
        let broken = folder.path().join("broken.mp4");
        std::fs::write(&broken, "nope").unwrap();
        assert!(
            preparer
                .output_media(&Url::from_file_path(&broken).unwrap())
                .await
                .is_none()
        );
    }
}
