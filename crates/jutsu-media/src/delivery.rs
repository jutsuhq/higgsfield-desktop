//! Writes the files of a delivery set (`DeliveryPlan`), Swift's `DeliveryExporter`: images with the `image` crate,
//! videos through the editing exporter.

use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageEncoder, RgbaImage};
use jutsu_core::{
    CropAspect, CropFrame, DeliveryPlan, DeliveryPreset, DeliverySetError, DeliverySubtitles, DeliveryTask,
    ImageColorSpace, ImageExportPreset, ImageFormat, MaterialKind, PixelRect, PixelSize, Progress,
    SubtitleCue,
};
use moxcms::{ColorProfile, Layout, TransformOptions};
use tokio_util::sync::CancellationToken;

use crate::editing::move_no_replace;
use crate::error::MediaError;
use crate::still::decode_upright_with_profile;
use crate::tools::FfmpegTools;

/// Writes `task`'s file into `folder` from the item's file `source`, which is only read. The file, and the SRT and
/// VTT of a video output with subtitle files, are written aside first and moved in together when all are complete;
/// they replace existing files only when the set wrote them before (`DeliveryTask::replaces_existing`), and a failed
/// or cancelled export leaves the folder as it was. A video output with subtitles burns `subtitles` in, or writes
/// them as SRT and VTT next to the video, timed as it is. An output with subtitles fails with
/// `DeliverySetError::NoSubtitles` without them. Returns the written file.
#[allow(
    clippy::too_many_arguments,
    reason = "the Swift function's parameters, plus the tools and the token"
)]
pub async fn export(
    tools: &FfmpegTools,
    task: &DeliveryTask,
    source: &Path,
    subtitles: Option<&[SubtitleCue]>,
    folder: &Path,
    progress: Option<Progress<f64>>,
    cancel: &CancellationToken,
) -> Result<PathBuf, MediaError> {
    let cues = subtitles.unwrap_or(&[]);
    let output_spec = &task.output;
    if output_spec.kind() == MaterialKind::Video && output_spec.subtitles.is_some() && cues.is_empty() {
        return Err(DeliverySetError::NoSubtitles.into());
    }
    let output = folder.join(&task.file_name);
    let mut staged = Staged(vec![(
        staging_file(folder, output_spec.file_extension()),
        output.clone(),
    )]);
    let crop = output_spec
        .aspect
        .map(|aspect| (aspect, task.frame.unwrap_or(CropFrame::CENTERED)));
    match output_spec.preset {
        DeliveryPreset::Image(preset) => {
            write_image(tools, source, preset, crop, &staged.0[0].0).await?;
            if let Some(progress) = &progress {
                progress(1.0);
            }
        }
        DeliveryPreset::Video(preset) => {
            let burn_in = if output_spec.subtitles == Some(DeliverySubtitles::BurnIn) {
                cues
            } else {
                &[]
            };
            crate::editing::export(
                tools,
                source,
                None,
                preset,
                crop,
                burn_in,
                &staged.0[0].0,
                progress,
                cancel,
            )
            .await?;
        }
    }
    let names = DeliveryPlan::names(&task.file_name, output_spec);
    for (name, format) in names.iter().skip(1).zip(output_spec.subtitle_formats()) {
        let file = staging_file(folder, format.file_extension());
        staged.0.push((file.clone(), folder.join(name)));
        std::fs::write(&file, format.text(cues))?;
    }
    if cancel.is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    move_in(&staged.0, task.replaces_existing)?;
    Ok(output)
}

/// Staging files, removed when the export ends; those moved in are gone already.
struct Staged(Vec<(PathBuf, PathBuf)>);

impl Drop for Staged {
    fn drop(&mut self) {
        for (staging, _) in &self.0 {
            let _ = std::fs::remove_file(staging);
        }
    }
}

fn staging_file(folder: &Path, extension: &str) -> PathBuf {
    folder.join(format!(
        ".{}.partial.{extension}",
        jutsu_core::support::uuid_string(uuid::Uuid::new_v4())
    ))
}

/// Moves every staged file to its name, or none: a name taken by a file the set didn't write fails before anything
/// moves, and a move that fails takes back the files this export added.
pub(crate) fn move_in(staged: &[(PathBuf, PathBuf)], replacing: bool) -> Result<(), MediaError> {
    if !replacing && let Some((_, taken)) = staged.iter().find(|(_, target)| target.exists()) {
        return Err(MediaError::FileExists(taken.clone()));
    }
    let mut added: Vec<&Path> = Vec::new();
    for (staging, target) in staged {
        let moved = if replacing && target.exists() {
            std::fs::rename(staging, target).map_err(MediaError::from)
        } else {
            move_no_replace(staging, target).map(|()| added.push(target))
        };
        if let Err(error) = moved {
            for file in added {
                let _ = std::fs::remove_file(file);
            }
            return Err(error);
        }
    }
    Ok(())
}

/// Decodes the image upright at full size, crops it, scales it down to the preset's long edge and writes it in the
/// preset's format and color space. Formats without alpha are drawn over the preset's background.
pub(crate) async fn write_image(
    tools: &FfmpegTools,
    source: &Path,
    preset: ImageExportPreset,
    crop: Option<(CropAspect, CropFrame)>,
    output: &Path,
) -> Result<(), MediaError> {
    if preset.format == ImageFormat::Heic {
        // Neither the `image` crate nor an LGPL ffmpeg build writes HEIC.
        return Err(MediaError::FormatUnavailable(ImageFormat::Heic));
    }
    let (upright, profile) = decode_upright_with_profile(tools, source).await?;
    let output = output.to_path_buf();
    tokio::task::spawn_blocking(move || render_and_write(&upright, profile.as_deref(), preset, crop, &output))
        .await
        .map_err(|error| MediaError::ExportFailed(error.to_string()))?
}

fn render_and_write(
    upright: &DynamicImage,
    source_profile: Option<&[u8]>,
    preset: ImageExportPreset,
    crop: Option<(CropAspect, CropFrame)>,
    output: &Path,
) -> Result<(), MediaError> {
    let size = PixelSize::new(i64::from(upright.width()), i64::from(upright.height()));
    let rect = crop.map_or(
        PixelRect {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        },
        |(aspect, frame)| frame.rect(aspect, size),
    );
    if rect.x < 0
        || rect.y < 0
        || rect.width <= 0
        || rect.height <= 0
        || rect.x + rect.width > size.width
        || rect.y + rect.height > size.height
    {
        return Err(MediaError::ExportFailed(
            "The crop lies outside the image".to_owned(),
        ));
    }
    let cropped = upright
        .crop_imm(
            rect.x as u32,
            rect.y as u32,
            rect.width as u32,
            rect.height as u32,
        )
        .to_rgba8();
    let target = preset.output_size(rect.size());
    let scaled = if i64::from(cropped.width()) == target.width && i64::from(cropped.height()) == target.height
    {
        cropped
    } else {
        image::imageops::resize(
            &cropped,
            target.width as u32,
            target.height as u32,
            image::imageops::FilterType::Lanczos3,
        )
    };
    let (pixels, profile) = convert(scaled, source_profile, preset.color_space)?;
    write_encoded(&pixels, &profile, preset, output)
}

/// `image`'s colors in the preset's space, converted from the source's profile (sRGB when it has none), with the ICC
/// profile the file is tagged with.
fn convert(
    image: RgbaImage,
    source_profile: Option<&[u8]>,
    space: ImageColorSpace,
) -> Result<(RgbaImage, Vec<u8>), MediaError> {
    let destination = match space {
        ImageColorSpace::Srgb => ColorProfile::new_srgb(),
        ImageColorSpace::DisplayP3 => ColorProfile::new_display_p3(),
    };
    let tag = destination
        .encode()
        .map_err(|error| MediaError::ExportFailed(format!("ICC profile: {error}")))?;
    let source = source_profile.and_then(|bytes| ColorProfile::new_from_slice(bytes).ok());
    let source = match (source, space) {
        (Some(source), _) => source,
        (None, ImageColorSpace::Srgb) => return Ok((image, tag)),
        (None, ImageColorSpace::DisplayP3) => ColorProfile::new_srgb(),
    };
    let Ok(transform) = source.create_transform_8bit(
        Layout::Rgba,
        &destination,
        Layout::Rgba,
        TransformOptions::default(),
    ) else {
        // A profile that can't be converted from (CMYK, a broken one): the pixels as they are.
        return Ok((image, tag));
    };
    let (width, height) = image.dimensions();
    let mut converted = vec![0_u8; image.as_raw().len()];
    transform
        .transform(image.as_raw(), &mut converted)
        .map_err(|error| MediaError::ExportFailed(format!("Color conversion: {error}")))?;
    let converted = RgbaImage::from_raw(width, height, converted)
        .ok_or_else(|| MediaError::ExportFailed("Color conversion".to_owned()))?;
    Ok((converted, tag))
}

fn write_encoded(
    image: &RgbaImage,
    profile: &[u8],
    preset: ImageExportPreset,
    output: &Path,
) -> Result<(), MediaError> {
    let extension = preset.format.file_extension().to_uppercase();
    let failed = |_| MediaError::ExportFailed(format!("{extension} encoding failed"));
    let file = std::io::BufWriter::new(std::fs::File::create(output)?);
    let (width, height) = image.dimensions();
    match preset.format {
        ImageFormat::Png => {
            let mut encoder = image::codecs::png::PngEncoder::new(file);
            let _ = encoder.set_icc_profile(profile.to_vec());
            encoder
                .write_image(image.as_raw(), width, height, image::ExtendedColorType::Rgba8)
                .map_err(failed)
        }
        ImageFormat::Jpeg => {
            let flat = flatten(image, preset);
            let quality = (preset.quality.clamp(0.0, 1.0) * 100.0).round().clamp(1.0, 100.0) as u8;
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality);
            let _ = encoder.set_icc_profile(profile.to_vec());
            encoder
                .write_image(flat.as_raw(), width, height, image::ExtendedColorType::Rgb8)
                .map_err(failed)
        }
        ImageFormat::Heic => Err(MediaError::FormatUnavailable(ImageFormat::Heic)),
    }
}

/// `image` drawn over the preset's background, without alpha.
fn flatten(image: &RgbaImage, preset: ImageExportPreset) -> image::RgbImage {
    let background = [
        preset.background.red,
        preset.background.green,
        preset.background.blue,
    ]
    .map(|component| component.clamp(0.0, 1.0) * 255.0);
    image::RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        let alpha = f64::from(pixel[3]) / 255.0;
        let channel = |index: usize| {
            (f64::from(pixel[index]) * alpha + background[index] * (1.0 - alpha)).round() as u8
        };
        image::Rgb([channel(0), channel(1), channel(2)])
    })
}

#[cfg(test)]
mod tests {
    use image::Rgba;
    use jutsu_core::{
        DeliveryItem, DeliveryOutput, DeliveryScope, DeliverySet, ExportPreset, Material, MaterialReference,
        MaterialVersion, RgbColor, SubtitleFormat,
    };
    use uuid::Uuid;

    use super::*;
    use crate::probe::probe_streams;
    use crate::test_support::{Video, dir, image_size, tools};

    /// A PNG whose left half is opaque red and right half fully transparent.
    fn half_transparent_png(folder: &Path, width: u32, height: u32) -> PathBuf {
        let path = folder.join(format!("{}.png", Uuid::new_v4()));
        let image = RgbaImage::from_fn(width, height, |x, _| {
            if x < width / 2 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 0, 0])
            }
        });
        image.save(&path).unwrap();
        path
    }

    /// The RGBA of the pixel at `x`, `y` (from the top left).
    fn pixel(path: &Path, x: u32, y: u32) -> [u8; 4] {
        image::open(path).unwrap().to_rgba8().get_pixel(x, y).0
    }

    fn format_of(path: &Path) -> image::ImageFormat {
        image::ImageReader::open(path)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .format()
            .unwrap()
    }

    fn task(
        output: DeliveryOutput,
        file_name: &str,
        frame: Option<CropFrame>,
        replaces: bool,
    ) -> DeliveryTask {
        let item = DeliveryItem::new(
            MaterialReference::new(Uuid::new_v4(), Uuid::new_v4()),
            output.kind(),
            "item",
        );
        let frame = frame.or_else(|| item.frame(output.aspect));
        DeliveryTask {
            item,
            output,
            file_name: file_name.to_owned(),
            frame,
            replaces_existing: replaces,
        }
    }

    fn image_output(preset: ImageExportPreset) -> DeliveryOutput {
        DeliveryOutput::new(DeliveryPreset::Image(preset))
    }

    fn preset(format: ImageFormat, long_edge: Option<i64>) -> ImageExportPreset {
        let mut preset = ImageExportPreset::new(format);
        preset.long_edge = long_edge;
        preset
    }

    async fn run(
        tools: &FfmpegTools,
        task: &DeliveryTask,
        source: &Path,
        cues: Option<&[SubtitleCue]>,
        folder: &Path,
    ) -> Result<PathBuf, MediaError> {
        export(tools, task, source, cues, folder, None, &CancellationToken::new()).await
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn png_keeps_alpha_and_is_scaled_down_to_its_long_edge() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = half_transparent_png(folder.path(), 400, 200);
        let output = image_output(preset(ImageFormat::Png, Some(100)));

        let file = run(
            &tools,
            &task(output, "a.png", None, false),
            &source,
            None,
            folder.path(),
        )
        .await
        .unwrap();

        assert_eq!(format_of(&file), image::ImageFormat::Png);
        assert_eq!(image_size(&file), (100, 50));
        assert_eq!(pixel(&file, 10, 25), [255, 0, 0, 255]);
        assert_eq!(pixel(&file, 90, 25)[3], 0);
    }

    #[tokio::test]
    async fn jpeg_flattens_onto_its_background() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = half_transparent_png(folder.path(), 400, 200);
        let mut jpeg = preset(ImageFormat::Jpeg, None);
        jpeg.quality = 1.0;
        jpeg.background = RgbColor {
            red: 0.0,
            green: 0.0,
            blue: 1.0,
        };

        let file = run(
            &tools,
            &task(image_output(jpeg), "a.jpg", None, false),
            &source,
            None,
            folder.path(),
        )
        .await
        .unwrap();

        assert_eq!(format_of(&file), image::ImageFormat::Jpeg);
        assert_eq!(image_size(&file), (400, 200));
        let transparent = pixel(&file, 350, 100);
        assert!(transparent[0] < 20 && transparent[1] < 20 && transparent[2] > 235 && transparent[3] == 255);
        let red = pixel(&file, 50, 100);
        assert!(red[0] > 235 && red[2] < 20);
    }

    /// HEIC has no encoder here: the export says so and writes nothing.
    #[tokio::test]
    async fn heic_is_unavailable_and_writes_nothing() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = half_transparent_png(folder.path(), 400, 200);
        let mut heic = preset(ImageFormat::Heic, Some(200));
        heic.color_space = ImageColorSpace::DisplayP3;
        let before = names(folder.path());

        let outcome = run(
            &tools,
            &task(image_output(heic), "a.heic", None, false),
            &source,
            None,
            folder.path(),
        )
        .await;

        assert_eq!(outcome, Err(MediaError::FormatUnavailable(ImageFormat::Heic)));
        assert_eq!(names(folder.path()), before);
    }

    #[tokio::test]
    async fn display_p3_is_converted_and_tagged() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = half_transparent_png(folder.path(), 40, 20);
        let mut p3 = preset(ImageFormat::Png, None);
        p3.color_space = ImageColorSpace::DisplayP3;

        let file = run(
            &tools,
            &task(image_output(p3), "p3.png", None, false),
            &source,
            None,
            folder.path(),
        )
        .await
        .unwrap();

        // sRGB's pure red lies inside P3, so it is less than P3's full red there.
        let red = pixel(&file, 5, 10);
        assert!(red[0] < 245 && red[1] > 20, "{red:?}");
        let mut decoder = image::ImageReader::open(&file)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap();
        assert!(image::ImageDecoder::icc_profile(&mut decoder).unwrap().is_some());
    }

    #[tokio::test]
    async fn each_item_is_cropped_to_its_own_frame() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = half_transparent_png(folder.path(), 400, 200);
        let mut output = image_output(preset(ImageFormat::Png, None));
        output.aspect = Some(CropAspect::new(1, 1));
        let left_frame = CropFrame {
            center_x: 0.0,
            ..CropFrame::CENTERED
        };
        let right_frame = CropFrame {
            center_x: 1.0,
            ..CropFrame::CENTERED
        };

        let left = run(
            &tools,
            &task(output.clone(), "left.png", Some(left_frame), false),
            &source,
            None,
            folder.path(),
        )
        .await
        .unwrap();
        let right = run(
            &tools,
            &task(output, "right.png", Some(right_frame), false),
            &source,
            None,
            folder.path(),
        )
        .await
        .unwrap();

        assert_eq!(image_size(&left), (200, 200));
        assert_eq!(pixel(&left, 190, 100), [255, 0, 0, 255]);
        assert_eq!(pixel(&right, 10, 100)[3], 0);
    }

    #[tokio::test]
    async fn twenty_images_in_three_outputs_write_sixty_files() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let sources = folder.path().join("sources");
        let delivered = folder.path().join("delivery");
        std::fs::create_dir_all(&sources).unwrap();
        std::fs::create_dir_all(&delivered).unwrap();
        let mut files = std::collections::HashMap::new();
        let now = time::OffsetDateTime::now_utc();
        let materials: Vec<Material> = (1..=20)
            .map(|number| {
                let file = sources.join(format!("look{number:02}.png"));
                std::fs::rename(half_transparent_png(&sources, 60, 40), &file).unwrap();
                let mut version = MaterialVersion::new(Uuid::new_v4(), now);
                version.file_url = Some(url::Url::from_file_path(&file).unwrap());
                files.insert(version.id, file);
                Material::new(Uuid::new_v4(), MaterialKind::Image, now, version)
            })
            .collect();
        let named = |name: &str, preset: ImageExportPreset, aspect: Option<CropAspect>| {
            let mut output = image_output(preset);
            output.name = name.to_owned();
            output.aspect = aspect;
            output
        };
        let delivery = DeliverySet::new(
            Uuid::new_v4(),
            "Lookbook",
            now,
            DeliverySet::items_from(&materials),
            vec![
                named("web", preset(ImageFormat::Jpeg, Some(32)), None),
                named("alpha", preset(ImageFormat::Png, None), None),
                named(
                    "square",
                    preset(ImageFormat::Jpeg, None),
                    Some(CropAspect::new(1, 1)),
                ),
            ],
        );

        for task in DeliveryPlan::tasks(&delivery, DeliveryScope::All, |_| false) {
            let source = &files[&task.item.reference.version_id];
            run(&tools, &task, source, None, &delivered).await.unwrap();
        }

        let written = names(&delivered);
        assert_eq!(written.len(), 60);
        assert_eq!(written.first().unwrap(), "Lookbook_01_look01_alpha.png");
        assert_eq!(written.last().unwrap(), "Lookbook_20_look20_web.jpg");
        assert_eq!(
            image_size(&delivered.join("Lookbook_07_look07_square.jpg")),
            (40, 40)
        );
    }

    #[tokio::test]
    async fn only_a_file_the_set_wrote_is_replaced() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = half_transparent_png(folder.path(), 400, 200);
        let output = image_output(preset(ImageFormat::Png, Some(40)));
        let existing = folder.path().join("taken.png");
        std::fs::write(&existing, "someone else's").unwrap();

        let refused = run(
            &tools,
            &task(output.clone(), "taken.png", None, false),
            &source,
            None,
            folder.path(),
        )
        .await;
        assert!(refused.is_err());
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "someone else's");

        run(
            &tools,
            &task(output, "taken.png", None, true),
            &source,
            None,
            folder.path(),
        )
        .await
        .unwrap();
        assert_eq!(image_size(&existing), (40, 20));
        assert!(!names(folder.path()).iter().any(|name| name.contains(".partial")));
    }

    #[tokio::test]
    async fn an_unreadable_source_fails_and_leaves_nothing() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let broken = folder.path().join("broken.png");
        std::fs::write(&broken, "not an image").unwrap();
        let output = image_output(preset(ImageFormat::Jpeg, None));

        let outcome = run(
            &tools,
            &task(output, "out.jpg", None, false),
            &broken,
            None,
            folder.path(),
        )
        .await;

        assert_eq!(outcome, Err(MediaError::UnreadableImage));
        assert_eq!(names(folder.path()), ["broken.png"]);
    }

    #[tokio::test]
    async fn a_video_is_cropped_to_its_outputs_aspect() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = Video::new(1).write(&tools, folder.path()).await;
        let mut output = DeliveryOutput::new(DeliveryPreset::Video(ExportPreset::EditingH264));
        output.aspect = Some(CropAspect::new(1, 1));
        let frame = CropFrame {
            center_x: 0.0,
            ..CropFrame::CENTERED
        };

        let file = run(
            &tools,
            &task(output, "clip.mp4", Some(frame), false),
            &source,
            None,
            folder.path(),
        )
        .await
        .unwrap();

        let size = probe_streams(&tools, file.as_os_str())
            .await
            .unwrap()
            .video
            .unwrap()
            .display;
        let original = probe_streams(&tools, source.as_os_str())
            .await
            .unwrap()
            .video
            .unwrap()
            .display;
        let shortest = original.width.min(original.height);
        assert_eq!(size.width, shortest - shortest % 2);
        assert_eq!(size.width, size.height);
    }

    // Subtitles in real exports: burned into the frames, or written next to the video, timed as it is.

    fn cues() -> Vec<SubtitleCue> {
        vec![
            SubtitleCue::new(0.0, 1.0, "HELLO THERE"),
            SubtitleCue::new(1.2, 1.8, "Second line"),
        ]
    }

    fn video_output(subtitles: Option<DeliverySubtitles>, aspect: Option<CropAspect>) -> DeliveryOutput {
        let mut output = DeliveryOutput::new(DeliveryPreset::Video(ExportPreset::EditingH264));
        output.subtitles = subtitles;
        output.aspect = aspect;
        output
    }

    /// How many pixels of the frame at `seconds` are near white inside the rectangle (from the top left).
    async fn bright_pixels(
        tools: &FfmpegTools,
        path: &Path,
        seconds: f64,
        rect: (u32, u32, u32, u32),
    ) -> usize {
        let frame = crate::frames::grab(tools, path, seconds, None).await.unwrap();
        let (x, y, width, height) = rect;
        (y..y + height)
            .flat_map(|row| (x..x + width).map(move |column| (column, row)))
            .filter(|(column, row)| {
                let pixel = frame.get_pixel(*column, *row);
                pixel[0] > 235 && pixel[1] > 235 && pixel[2] > 235
            })
            .count()
    }

    #[tokio::test]
    async fn burned_in_cues_show_in_the_bottom_of_the_frame_only_during_their_time() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = Video::new(2)
            .size(640, 360)
            .shade(|_| 40)
            .write(&tools, folder.path())
            .await;
        let output = video_output(Some(DeliverySubtitles::BurnIn), None);
        let cues = cues();

        let (subtitles, drawtext) = tools.text_filters().await.unwrap();
        if !subtitles && !drawtext {
            // This ffmpeg can't draw text: the export says so and writes nothing.
            let before = names(folder.path());
            let outcome = run(
                &tools,
                &task(output, "burned.mp4", None, false),
                &source,
                Some(&cues),
                folder.path(),
            )
            .await;
            assert_eq!(outcome, Err(MediaError::BurnInUnavailable));
            assert_eq!(names(folder.path()), before);
            eprintln!("burn-in unavailable: this ffmpeg has neither the subtitles nor the drawtext filter");
            return;
        }

        let file = run(
            &tools,
            &task(output, "burned.mp4", None, false),
            &source,
            Some(&cues),
            folder.path(),
        )
        .await
        .unwrap();
        let bottom = (0, 240, 640, 120);
        let top = (0, 0, 640, 120);
        assert!(bright_pixels(&tools, &file, 0.5, bottom).await > 100);
        assert_eq!(bright_pixels(&tools, &file, 0.5, top).await, 0);
        assert_eq!(bright_pixels(&tools, &file, 1.1, bottom).await, 0);

        let square = video_output(Some(DeliverySubtitles::BurnIn), Some(CropAspect::new(1, 1)));
        let cropped = run(
            &tools,
            &task(square, "square.mp4", None, false),
            &source,
            Some(&cues),
            folder.path(),
        )
        .await
        .unwrap();
        assert!(bright_pixels(&tools, &cropped, 0.5, (0, 240, 360, 120)).await > 100);
        assert_eq!(bright_pixels(&tools, &cropped, 0.5, (0, 0, 360, 120)).await, 0);
    }

    #[tokio::test]
    async fn subtitle_files_go_next_to_the_video_under_its_name() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = Video::new(2).size(320, 180).write(&tools, folder.path()).await;
        let output = video_output(Some(DeliverySubtitles::Files), None);
        let cues = cues();

        run(
            &tools,
            &task(output, "set_01_item.mp4", None, false),
            &source,
            Some(&cues),
            folder.path(),
        )
        .await
        .unwrap();

        let srt = std::fs::read_to_string(folder.path().join("set_01_item.srt")).unwrap();
        let vtt = std::fs::read_to_string(folder.path().join("set_01_item.vtt")).unwrap();
        assert_eq!(srt, SubtitleFormat::Srt.text(&cues));
        assert!(vtt.starts_with("WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nHELLO THERE\n"));
    }

    #[tokio::test]
    async fn a_taken_subtitle_name_leaves_the_folder_as_it_was() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = Video::new(1).size(320, 180).write(&tools, folder.path()).await;
        std::fs::write(folder.path().join("set_01_item.vtt"), "theirs").unwrap();
        let before = names(folder.path());
        let output = video_output(Some(DeliverySubtitles::Files), None);

        let outcome = run(
            &tools,
            &task(output, "set_01_item.mp4", None, false),
            &source,
            Some(&cues()),
            folder.path(),
        )
        .await;

        assert!(outcome.is_err());
        assert_eq!(names(folder.path()), before);
        assert_eq!(
            std::fs::read_to_string(folder.path().join("set_01_item.vtt")).unwrap(),
            "theirs"
        );
    }

    #[test]
    fn a_failed_move_takes_back_what_it_added() {
        let folder = dir();
        let video = folder.path().join(".a.partial.mp4");
        std::fs::write(&video, "video").unwrap();
        let staged = [
            (video, folder.path().join("take.mp4")),
            (
                folder.path().join(".gone.partial.srt"),
                folder.path().join("take.srt"),
            ),
        ];

        assert!(move_in(&staged, false).is_err());
        assert!(!folder.path().join("take.mp4").exists());
    }

    #[tokio::test]
    async fn an_output_with_subtitles_needs_them() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = Video::new(1).size(320, 180).write(&tools, folder.path()).await;
        let output = video_output(Some(DeliverySubtitles::BurnIn), None);

        let outcome = run(
            &tools,
            &task(output, "none.mp4", None, false),
            &source,
            None,
            folder.path(),
        )
        .await;

        assert_eq!(
            outcome,
            Err(MediaError::DeliverySet(DeliverySetError::NoSubtitles))
        );
        assert!(!folder.path().join("none.mp4").exists());
    }
}
