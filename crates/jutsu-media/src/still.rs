//! Still images read upright and scaled down (Swift's `ImageThumbnail`): the `image` crate for the common formats,
//! ffmpeg for what it can't read (HEIC and other camera formats).

use std::ffi::OsString;
use std::path::Path;

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage};
use jutsu_core::PixelSize;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::error::MediaError;
use crate::tools::{FfmpegTools, os_args, run_capture};

/// The format of the image in `path` by its content, as the `image` crate tells it; `None` for one it doesn't know.
pub(crate) fn content_format(path: &Path) -> Option<image::ImageFormat> {
    ImageReader::open(path).ok()?.with_guessed_format().ok()?.format()
}

/// The image in `path` at full size, upright (its EXIF orientation applied).
pub(crate) async fn decode_upright(tools: &FfmpegTools, path: &Path) -> Result<DynamicImage, MediaError> {
    decode_upright_with_profile(tools, path)
        .await
        .map(|(image, _)| image)
}

/// `decode_upright`, with the ICC profile the file embeds; ffmpeg's pictures come as sRGB, without one.
pub(crate) async fn decode_upright_with_profile(
    tools: &FfmpegTools,
    path: &Path,
) -> Result<(DynamicImage, Option<Vec<u8>>), MediaError> {
    let owned = path.to_path_buf();
    let native = tokio::task::spawn_blocking(move || decode_upright_native(&owned))
        .await
        .map_err(|_| MediaError::UnreadableImage)?;
    match native {
        Ok(decoded) => Ok(decoded),
        Err(_) => decode_with_ffmpeg(tools, path).await.map(|image| (image, None)),
    }
}

fn decode_upright_native(path: &Path) -> Result<(DynamicImage, Option<Vec<u8>>), MediaError> {
    let mut decoder = ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()
        .map_err(|_| MediaError::UnreadableImage)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let profile = decoder.icc_profile().ok().flatten();
    let mut image = DynamicImage::from_decoder(decoder).map_err(|_| MediaError::UnreadableImage)?;
    image.apply_orientation(orientation);
    Ok((image, profile))
}

/// The upright size of a still picture only ffmpeg reads: an image demuxer's, or a HEIF/AVIF item in the ISO media
/// container. `None` for anything else, a text file ffmpeg would render as a picture among them.
async fn ffmpeg_still_size(tools: &FfmpegTools, path: &Path) -> Option<PixelSize> {
    if !path.is_file() {
        return None;
    }
    let report = crate::probe::probe_streams(tools, path.as_os_str()).await.ok()?;
    let is_still = report.is_still_image() || report.format_name.starts_with("mov,");
    report.video.filter(|_| is_still).map(|video| video.display)
}

/// The first picture ffmpeg decodes from `path`, turned for display, as a PNG it hands back.
async fn decode_with_ffmpeg(tools: &FfmpegTools, path: &Path) -> Result<DynamicImage, MediaError> {
    if ffmpeg_still_size(tools, path).await.is_none() {
        return Err(MediaError::UnreadableImage);
    }
    let mut args: Vec<OsString> = os_args(["-hide_banner", "-nostdin", "-v", "error", "-i"]);
    args.push(path.as_os_str().to_owned());
    args.extend(os_args([
        "-frames:v",
        "1",
        "-f",
        "image2pipe",
        "-c:v",
        "png",
        "pipe:1",
    ]));
    let png = run_capture(&tools.ffmpeg, args, &CancellationToken::new())
        .await
        .map_err(|error| match error {
            MediaError::Cancelled => MediaError::Cancelled,
            _ => MediaError::UnreadableImage,
        })?;
    image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .map_err(|_| MediaError::UnreadableImage)
}

/// `image` at most `max_pixel_size` on its long side, never upscaled.
pub(crate) fn fit(image: DynamicImage, max_pixel_size: u32) -> DynamicImage {
    let (width, height) = (image.width(), image.height());
    let longest = width.max(height);
    if longest <= max_pixel_size || max_pixel_size == 0 {
        return image;
    }
    let scale = f64::from(max_pixel_size) / f64::from(longest);
    let side = |value: u32| ((f64::from(value) * scale).round() as u32).max(1);
    image.resize_exact(side(width), side(height), image::imageops::FilterType::Lanczos3)
}

/// The image in the file at `url`, upright and at most `max_pixel_size` on its long side (never upscaled); `None` when
/// it can't be read. Any other URL than a `file:` one gives `None` without being opened: a picture a server links to
/// is read by the engine, which checks the link, and decoded with `image_from_bytes`.
pub async fn image(tools: &FfmpegTools, url: &Url, max_pixel_size: u32) -> Option<RgbaImage> {
    let path = url.to_file_path().ok()?;
    image_at(tools, &path, max_pixel_size).await
}

/// `image` for a path.
pub async fn image_at(tools: &FfmpegTools, path: &Path, max_pixel_size: u32) -> Option<RgbaImage> {
    let picture = decode_upright(tools, path).await.ok()?;
    tokio::task::spawn_blocking(move || fit(picture, max_pixel_size).to_rgba8())
        .await
        .ok()
}

/// The image encoded in `data`, as `image` gives it.
pub async fn image_from_bytes(data: Vec<u8>, max_pixel_size: u32) -> Option<RgbaImage> {
    tokio::task::spawn_blocking(move || {
        let mut decoder = ImageReader::new(std::io::Cursor::new(data))
            .with_guessed_format()
            .ok()?
            .into_decoder()
            .ok()?;
        let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
        let mut picture = DynamicImage::from_decoder(decoder).ok()?;
        picture.apply_orientation(orientation);
        Some(fit(picture, max_pixel_size).to_rgba8())
    })
    .await
    .ok()
    .flatten()
}

/// The upright size of the image in the file at `path`, read from its header: EXIF orientations 5–8 turn the image a
/// quarter, so its sides swap. `None` when it can't be read.
pub async fn size(tools: &FfmpegTools, path: &Path) -> Option<PixelSize> {
    let owned = path.to_path_buf();
    let native = tokio::task::spawn_blocking(move || {
        let mut decoder = ImageReader::open(&owned)
            .ok()?
            .with_guessed_format()
            .ok()?
            .into_decoder()
            .ok()?;
        let (width, height) = decoder.dimensions();
        let turned = matches!(
            decoder.orientation().unwrap_or(Orientation::NoTransforms),
            Orientation::Rotate90
                | Orientation::Rotate270
                | Orientation::Rotate90FlipH
                | Orientation::Rotate270FlipH
        );
        let size = if turned {
            PixelSize::new(i64::from(height), i64::from(width))
        } else {
            PixelSize::new(i64::from(width), i64::from(height))
        };
        Some(size)
    })
    .await
    .ok()
    .flatten();
    let size = match native {
        Some(size) => size,
        None => ffmpeg_still_size(tools, path).await?,
    };
    size.is_valid().then_some(size)
}

/// Whether the image in the file at `path` has an alpha channel, read from its header; `false` when unread.
pub fn has_alpha(path: &Path) -> bool {
    ImageReader::open(path)
        .ok()
        .and_then(|reader| reader.with_guessed_format().ok())
        .and_then(|reader| reader.into_decoder().ok())
        .is_some_and(|decoder| decoder.color_type().has_alpha())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Still, dir, make_image, tools};

    #[tokio::test]
    async fn thumbnail_fits_the_maximum_pixel_size() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let large = make_image(folder.path(), 2000, 1000, Still::Jpeg, 1);
        let small = make_image(folder.path(), 100, 50, Still::Png, 1);
        let missing = folder.path().join("missing.png");

        let thumbnail = image_at(&tools, &large, 256).await.unwrap();
        let unscaled = image_at(&tools, &small, 256).await.unwrap();
        let none = image_at(&tools, &missing, 256).await;

        assert_eq!(thumbnail.dimensions(), (256, 128));
        assert_eq!(unscaled.dimensions(), (100, 50));
        assert!(none.is_none());
    }

    /// A server's link is never opened here: the engine checks it first.
    #[tokio::test]
    async fn a_thumbnail_is_never_read_from_a_link() {
        let Some(tools) = tools() else { return };
        for link in ["http://cdn.example.org/a.png", "https://cdn.example.org/a.png"] {
            assert!(image(&tools, &Url::parse(link).unwrap(), 256).await.is_none());
        }
    }

    #[tokio::test]
    async fn a_thumbnail_is_decoded_from_bytes() {
        let folder = dir();
        let file = make_image(folder.path(), 2000, 1000, Still::Png, 1);

        let thumbnail = image_from_bytes(std::fs::read(&file).unwrap(), 256)
            .await
            .unwrap();
        let none = image_from_bytes(b"<html>".to_vec(), 256).await;

        assert_eq!(thumbnail.dimensions(), (256, 128));
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn alpha_is_read_from_the_header() {
        let folder = dir();
        let png = make_image(folder.path(), 8, 8, Still::Png, 1);
        let jpeg = make_image(folder.path(), 8, 8, Still::Jpeg, 1);
        assert!(has_alpha(&png));
        assert!(!has_alpha(&jpeg));
        assert!(!has_alpha(&folder.path().join("missing.png")));
    }
}
