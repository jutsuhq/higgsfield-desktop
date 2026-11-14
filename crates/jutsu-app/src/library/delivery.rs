//! Delivery sets (`DeliveryExport.swift`, `DeliverySetsSheet`, `DeliverySetDetail`, `DeliveryOutputEditing`): their
//! texts, the run that writes a set's files one by one, and the views of the sheet.

use std::future::Future;

pub use jutsu_core::CropFrame;
use jutsu_core::{
    CropAspect, DeliveryOutput, DeliveryPreset, DeliveryResult, DeliveryResultStatus, DeliverySet,
    DeliverySetError, DeliverySetStatus, DeliverySubtitles, DeliveryTask, ExportPreset, ImageColorSpace,
    ImageExportPreset, ImageFormat, MaterialKind, MaterialReference,
};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, UtcOffset};

use super::editing_export::preset_item_title;
use super::service::date_text;
use super::view::MediaFile;

pub const TITLE: &str = "Delivery Sets";
pub const NEW_SET_TITLE: &str = "New Delivery Set";
pub const UNAVAILABLE_MESSAGE: &str =
    "The file isn't on this computer. Download it to the library or connect its drive.";
pub const NO_SUBTITLES_MESSAGE: &str = "This version has no subtitles. Make them with Subtitles… on its page, or turn subtitles off for this output.";
pub const FROZEN_MESSAGE: &str =
    "This set has been exported, so its versions stay as they are. Start a new set from it instead.";
pub const FOLDER_MESSAGE: &str = "Choose a folder for the delivery. Files made by others are never replaced.";
pub const MAKE_FAILED: &str = "Couldn't make the delivery set";
pub const SAVE_FAILED: &str = "Couldn't save the delivery set";
pub const DELETE_FAILED: &str = "Couldn't delete the delivery set";
pub const SOME_NOT_EXPORTED: &str = "Some files weren't exported";

/// Long sides an image output offers, besides full size.
pub const SIZES: [i64; 6] = [4096, 3000, 2048, 1600, 1080, 800];
/// Qualities a lossy image output offers, in percent.
pub const QUALITIES: [i32; 4] = [100, 90, 80, 70];

/// "Delivery 15 Oct 2026".
pub fn default_name(date: OffsetDateTime, offset: UtcOffset) -> String {
    format!("Delivery {}", date_text(date, offset))
}

/// "Delivery 15 Oct 2026 – next round": the set started from the one called `name`.
pub fn next_round_name(name: &str) -> String {
    format!("{name} – next round")
}

/// "Not exported", or "Exported 15 Oct 2026, 10:41 to Client".
pub fn status_text(set: &DeliverySet, offset: UtcOffset) -> String {
    match set.status() {
        DeliverySetStatus::NotExported => "Not exported".to_owned(),
        DeliverySetStatus::Exported(date, folder) => {
            let local = date.to_offset(offset);
            let name = folder
                .to_file_path()
                .ok()
                .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
                .unwrap_or_default();
            format!(
                "Exported {}, {:02}:{:02} to {name}",
                date_text(date, offset),
                local.hour(),
                local.minute()
            )
        }
    }
}

/// How many of a set's files are done, failed or skipped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeliveryCounts {
    pub done: usize,
    pub failed: usize,
    pub skipped: usize,
}

impl DeliveryCounts {
    pub fn new(set: &DeliverySet) -> Self {
        let mut counts = Self::default();
        for result in set.results() {
            match result.status {
                DeliveryResultStatus::Done => counts.done += 1,
                DeliveryResultStatus::Failed => counts.failed += 1,
                DeliveryResultStatus::Skipped => counts.skipped += 1,
            }
        }
        counts
    }

    /// "12 done · 1 failed · 2 skipped", leaving out what is zero; `None` before any export.
    pub fn summary(&self) -> Option<String> {
        let parts: Vec<String> = [
            (self.done, "done"),
            (self.failed, "failed"),
            (self.skipped, "skipped"),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, word)| format!("{count} {word}"))
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

pub fn format_title(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::Png => "PNG",
        ImageFormat::Heic => "HEIC",
    }
}

pub fn color_space_title(space: ImageColorSpace) -> &'static str {
    match space {
        ImageColorSpace::Srgb => "sRGB",
        ImageColorSpace::DisplayP3 => "Display P3",
    }
}

pub fn subtitles_title(subtitles: DeliverySubtitles) -> &'static str {
    match subtitles {
        DeliverySubtitles::Files => "SRT + VTT",
        DeliverySubtitles::BurnIn => "Burn In",
    }
}

/// "JPEG · 2048 px · 4:5 · sRGB", or "Editing H.264 · 9:16".
pub fn output_summary(output: &DeliveryOutput) -> String {
    let mut parts: Vec<String> = match output.preset {
        DeliveryPreset::Image(image) => {
            let mut parts = vec![format_title(image.format).to_owned()];
            parts.extend(image.long_edge.map(|edge| format!("{edge} px")));
            parts.push(color_space_title(image.color_space).to_owned());
            parts
        }
        DeliveryPreset::Video(video) => vec![super::editing_export::preset_title(video).to_owned()],
    };
    if let Some(aspect) = output.aspect {
        parts.insert(1, aspect.title());
    }
    if output.kind() == MaterialKind::Video
        && let Some(subtitles) = output.subtitles
    {
        parts.push(subtitles_title(subtitles).to_owned());
    }
    parts.join(" · ")
}

/// One task's state in a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "kind", content = "message")]
pub enum DeliveryTaskState {
    Waiting,
    Running,
    Done,
    Failed(String),
    Skipped(String),
}

/// Why one file of a run wasn't written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryRunError {
    /// The run was canceled: it stops here.
    Canceled,
    /// The file isn't on this computer: the file is skipped.
    SourceUnavailable,
    /// The output wants subtitles the version doesn't have.
    NoSubtitles,
    Other(String),
}

impl From<jutsu_media::MediaError> for DeliveryRunError {
    fn from(error: jutsu_media::MediaError) -> Self {
        match error {
            jutsu_media::MediaError::Cancelled => Self::Canceled,
            jutsu_media::MediaError::DeliverySet(DeliverySetError::SourceUnavailable) => {
                Self::SourceUnavailable
            }
            jutsu_media::MediaError::DeliverySet(DeliverySetError::NoSubtitles) => Self::NoSubtitles,
            error => Self::Other(error.to_string()),
        }
    }
}

/// Writes `tasks` one by one with `export`, telling `report` each task's state by `DeliveryTask::id`. A file whose
/// source is unavailable is skipped, any other failure is noted and the run goes on; a cancel stops it. Returns the
/// result of every task that finished, in order, for `DeliverySet::record`.
pub async fn run<F, Fut>(
    tasks: &[DeliveryTask],
    export: F,
    report: impl Fn(&str, DeliveryTaskState),
) -> Vec<DeliveryResult>
where
    F: Fn(DeliveryTask) -> Fut,
    Fut: Future<Output = Result<(), DeliveryRunError>>,
{
    for task in tasks {
        report(&task.id(), DeliveryTaskState::Waiting);
    }
    let mut results = Vec::with_capacity(tasks.len());
    for task in tasks {
        let id = task.id();
        report(&id, DeliveryTaskState::Running);
        match export(task.clone()).await {
            Ok(()) => {
                results.push(task.result(DeliveryResultStatus::Done, None));
                report(&id, DeliveryTaskState::Done);
            }
            Err(DeliveryRunError::Canceled) => break,
            Err(DeliveryRunError::SourceUnavailable) => {
                results.push(task.result(
                    DeliveryResultStatus::Skipped,
                    Some(UNAVAILABLE_MESSAGE.to_owned()),
                ));
                report(&id, DeliveryTaskState::Skipped(UNAVAILABLE_MESSAGE.to_owned()));
            }
            Err(DeliveryRunError::NoSubtitles) => {
                results.push(task.result(
                    DeliveryResultStatus::Failed,
                    Some(NO_SUBTITLES_MESSAGE.to_owned()),
                ));
                report(&id, DeliveryTaskState::Failed(NO_SUBTITLES_MESSAGE.to_owned()));
            }
            Err(DeliveryRunError::Other(message)) => {
                results.push(task.result(DeliveryResultStatus::Failed, Some(message.clone())));
                report(&id, DeliveryTaskState::Failed(message));
            }
        }
    }
    results
}

// MARK: Views

/// One choice of a picker.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryChoice {
    /// The raw value the edit sends back.
    pub value: String,
    pub title: String,
}

fn choice(value: impl Into<String>, title: impl Into<String>) -> DeliveryChoice {
    DeliveryChoice {
        value: value.into(),
        title: title.into(),
    }
}

/// A set in the sheet's list, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeliverySetRow {
    pub id: String,
    pub name: String,
    pub status: String,
}

/// One item of a set: a tile with its preview, its run badge and its frame.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryItemView {
    pub material_id: String,
    pub version_id: String,
    /// The name, or "Untitled".
    pub name: String,
    pub is_video: bool,
    pub preview: MediaFile,
    /// The aspects of the set's cropping outputs of its kind, for "Frame 4:5…".
    pub frame_aspects: Vec<DeliveryChoice>,
    /// The badge over its picture: its files taken together.
    pub state: Option<DeliveryTaskState>,
    /// "Waiting", "Exported" or why a file failed or was skipped.
    pub state_hint: Option<String>,
}

/// One output of a set, as its row edits it.
#[allow(clippy::struct_excessive_bools, reason = "the wire's fields")]
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryOutputView {
    pub id: String,
    pub name: String,
    /// The name field's placeholder: the part of the file names an unnamed output gets.
    pub placeholder: String,
    /// "JPEG · 2048 px · 4:5 · sRGB".
    pub summary: String,
    pub is_video: bool,
    pub format: String,
    /// "full" or a pixel count.
    pub size: String,
    /// Only for a lossy format.
    pub quality: Option<i32>,
    pub color_space: String,
    /// Only for a format without alpha: "#rrggbb".
    pub background: Option<String>,
    pub video_preset: String,
    /// `none`, `files` or `burn_in`.
    pub subtitles: String,
    /// "original" or an aspect key.
    pub aspect: String,
}

/// What every output row's pickers offer.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryOutputOptions {
    pub formats: Vec<DeliveryChoice>,
    pub sizes: Vec<DeliveryChoice>,
    pub qualities: Vec<DeliveryChoice>,
    pub color_spaces: Vec<DeliveryChoice>,
    pub video_presets: Vec<DeliveryChoice>,
    pub subtitles: Vec<DeliveryChoice>,
    pub aspects: Vec<DeliveryChoice>,
}

impl DeliveryOutputOptions {
    pub fn new() -> Self {
        Self {
            formats: ImageFormat::ALL
                .iter()
                .map(|format| choice(format.raw_value(), format_title(*format)))
                .collect(),
            sizes: std::iter::once(choice("full", "Full size"))
                .chain(
                    SIZES
                        .iter()
                        .map(|size| choice(size.to_string(), format!("{size} px"))),
                )
                .collect(),
            qualities: QUALITIES
                .iter()
                .map(|quality| choice(quality.to_string(), format!("{quality}%")))
                .collect(),
            color_spaces: ImageColorSpace::ALL
                .iter()
                .map(|space| choice(space.raw_value(), color_space_title(*space)))
                .collect(),
            video_presets: ExportPreset::ALL
                .iter()
                .map(|preset| choice(preset.raw_value(), preset_item_title(*preset)))
                .collect(),
            subtitles: std::iter::once(choice("none", "None"))
                .chain(
                    DeliverySubtitles::ALL
                        .iter()
                        .map(|subtitles| choice(subtitles.raw_value(), subtitles_title(*subtitles))),
                )
                .collect(),
            aspects: std::iter::once(choice("original", "Original frame"))
                .chain(
                    CropAspect::COMMON
                        .iter()
                        .map(|aspect| choice(aspect.key(), aspect.title())),
                )
                .collect(),
        }
    }
}

impl Default for DeliveryOutputOptions {
    fn default() -> Self {
        Self::new()
    }
}

fn hex(color: jutsu_core::RgbColor) -> String {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0…255"
    )]
    let byte = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(color.red),
        byte(color.green),
        byte(color.blue)
    )
}

fn parse_hex(text: &str) -> Option<jutsu_core::RgbColor> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| {
        u8::from_str_radix(digits.get(range)?, 16)
            .ok()
            .map(|value| f64::from(value) / 255.0)
    };
    Some(jutsu_core::RgbColor {
        red: channel(0..2)?,
        green: channel(2..4)?,
        blue: channel(4..6)?,
    })
}

impl DeliveryOutputView {
    pub fn new(output: &DeliveryOutput) -> Self {
        let image = match output.preset {
            DeliveryPreset::Image(image) => Some(image),
            DeliveryPreset::Video(_) => None,
        };
        let video = match output.preset {
            DeliveryPreset::Video(video) => video,
            DeliveryPreset::Image(_) => ExportPreset::EditingH264,
        };
        let image_or_default = image.unwrap_or_else(|| ImageExportPreset::new(ImageFormat::Jpeg));
        #[allow(clippy::cast_possible_truncation, reason = "a percentage")]
        let quality = (image_or_default.quality * 100.0).round() as i32;
        Self {
            id: output.id.to_string(),
            name: output.name.clone(),
            placeholder: output.summary(),
            summary: output_summary(output),
            is_video: output.kind() == MaterialKind::Video,
            format: image_or_default.format.raw_value().to_owned(),
            size: image_or_default
                .long_edge
                .map_or_else(|| "full".to_owned(), |edge| edge.to_string()),
            quality: image_or_default.format.is_lossy().then_some(quality),
            color_space: image_or_default.color_space.raw_value().to_owned(),
            background: (!image_or_default.format.keeps_alpha()).then(|| hex(image_or_default.background)),
            video_preset: video.raw_value().to_owned(),
            subtitles: output
                .subtitles
                .map_or_else(|| "none".to_owned(), |subtitles| subtitles.raw_value().to_owned()),
            aspect: output
                .aspect
                .map_or_else(|| "original".to_owned(), |aspect| aspect.key()),
        }
    }
}

/// A change to one output, from its row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "field", content = "value")]
pub enum DeliveryOutputChange {
    Name(String),
    Format(String),
    Size(String),
    Quality(i32),
    ColorSpace(String),
    Background(String),
    VideoPreset(String),
    Subtitles(String),
    Aspect(String),
}

fn aspect_of(key: &str) -> Option<CropAspect> {
    CropAspect::COMMON.into_iter().find(|aspect| aspect.key() == key)
}

impl DeliveryOutputChange {
    /// Applies the change; a value it doesn't know leaves the output as it was.
    pub fn apply(self, output: &mut DeliveryOutput) {
        let mut image = match output.preset {
            DeliveryPreset::Image(image) => Some(image),
            DeliveryPreset::Video(_) => None,
        };
        match self {
            Self::Name(name) => output.name = name,
            Self::Format(raw) => {
                if let (Some(image), Some(format)) = (image.as_mut(), ImageFormat::from_raw(&raw)) {
                    image.format = format;
                }
            }
            Self::Size(raw) => {
                if let Some(image) = image.as_mut() {
                    image.long_edge = raw.parse::<i64>().ok().filter(|size| SIZES.contains(size));
                }
            }
            Self::Quality(percent) => {
                if let Some(image) = image.as_mut()
                    && QUALITIES.contains(&percent)
                {
                    image.quality = f64::from(percent) / 100.0;
                }
            }
            Self::ColorSpace(raw) => {
                if let (Some(image), Some(space)) = (image.as_mut(), ImageColorSpace::from_raw(&raw)) {
                    image.color_space = space;
                }
            }
            Self::Background(text) => {
                if let (Some(image), Some(color)) = (image.as_mut(), parse_hex(&text)) {
                    image.background = color;
                }
            }
            Self::VideoPreset(raw) => {
                if let (DeliveryPreset::Video(_), Some(preset)) =
                    (output.preset, ExportPreset::from_raw(&raw))
                {
                    output.preset = DeliveryPreset::Video(preset);
                }
            }
            Self::Subtitles(raw) => {
                if output.kind() == MaterialKind::Video {
                    output.subtitles = DeliverySubtitles::from_raw(&raw);
                }
            }
            Self::Aspect(key) => output.aspect = aspect_of(&key),
        }
        if let Some(image) = image {
            output.preset = DeliveryPreset::Image(image);
        }
    }
}

/// A change to a set, from its detail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DeliverySetChange {
    Rename {
        name: String,
    },
    Note {
        note: String,
    },
    ReviewLink {
        link: String,
    },
    /// Remove from set; refused once the set is frozen.
    RemoveItem {
        version_id: String,
    },
    /// Add Image Output (JPEG) or Add Video Output (Editing H.264).
    AddOutput {
        is_video: bool,
    },
    RemoveOutput {
        output_id: String,
    },
    Output {
        output_id: String,
        change: DeliveryOutputChange,
    },
    /// The frame of one item for one aspect.
    Frame {
        version_id: String,
        aspect: String,
        frame: CropFrame,
    },
}

impl DeliverySetChange {
    /// Applies the change to `set`.
    pub fn apply(self, set: &mut DeliverySet) -> Result<(), DeliverySetError> {
        match self {
            Self::Rename { name } => set.name = name,
            Self::Note { note } => set.note = note,
            Self::ReviewLink { link } => set.review_link = link,
            Self::RemoveItem { version_id } => {
                let items = set
                    .items()
                    .iter()
                    .filter(|item| item.reference.version_id.to_string() != version_id)
                    .cloned()
                    .collect();
                set.set_items(items)?;
            }
            Self::AddOutput { is_video } => set.outputs.push(DeliveryOutput::new(if is_video {
                DeliveryPreset::Video(ExportPreset::EditingH264)
            } else {
                DeliveryPreset::Image(ImageExportPreset::new(ImageFormat::Jpeg))
            })),
            Self::RemoveOutput { output_id } => {
                set.outputs.retain(|output| output.id.to_string() != output_id);
            }
            Self::Output { output_id, change } => {
                if let Some(output) = set
                    .outputs
                    .iter_mut()
                    .find(|output| output.id.to_string() == output_id)
                {
                    change.apply(output);
                }
            }
            Self::Frame {
                version_id,
                aspect,
                frame,
            } => {
                let reference = set
                    .items()
                    .iter()
                    .find(|item| item.reference.version_id.to_string() == version_id)
                    .map(|item| item.reference);
                if let (Some(reference), Some(aspect)) = (reference, aspect_of(&aspect)) {
                    set.set_frame(frame, reference, aspect);
                }
            }
        }
        Ok(())
    }
}

/// The detail of one set.
#[allow(clippy::struct_excessive_bools, reason = "the wire's fields")]
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeliverySetView {
    pub id: String,
    pub name: String,
    pub status: String,
    pub is_frozen: bool,
    /// "Versions locked after export" or "Versions lock on first export".
    pub lock_hint: String,
    pub items: Vec<DeliveryItemView>,
    pub outputs: Vec<DeliveryOutputView>,
    pub options: DeliveryOutputOptions,
    pub note: String,
    pub review_link: String,
    /// "12 done · 1 failed", after an export.
    pub counts: Option<String>,
    pub failed_count: i32,
    pub pending_count: i32,
    /// Export… needs items and outputs.
    pub can_export: bool,
    pub has_folder: bool,
    /// A run of this set is going on.
    pub is_exporting: bool,
}

/// The frame of `item` for `aspect`, for the frame sheet.
pub fn item_frame(set: &DeliverySet, reference: MaterialReference, aspect: CropAspect) -> CropFrame {
    set.items()
        .iter()
        .find(|item| item.reference == reference)
        .and_then(|item| item.frame(Some(aspect)))
        .unwrap_or(CropFrame::CENTERED)
}

/// The aspects an item of `kind` is cropped to: those of the outputs of its kind, once each.
pub fn frame_aspects(outputs: &[DeliveryOutput], kind: MaterialKind) -> Vec<CropAspect> {
    let mut aspects: Vec<CropAspect> = Vec::new();
    for aspect in outputs
        .iter()
        .filter(|output| output.kind() == kind)
        .filter_map(|output| output.aspect)
    {
        if !aspects.contains(&aspect) {
            aspects.push(aspect);
        }
    }
    aspects
}

/// The state that matters most (`DeliveryItemBadge.combined`): running, then failed, skipped, waiting and done.
pub fn combined_state(states: &[DeliveryTaskState]) -> Option<DeliveryTaskState> {
    if states.contains(&DeliveryTaskState::Running) {
        return Some(DeliveryTaskState::Running);
    }
    if let Some(failed) = states
        .iter()
        .find(|state| matches!(state, DeliveryTaskState::Failed(_)))
    {
        return Some(failed.clone());
    }
    if let Some(skipped) = states
        .iter()
        .find(|state| matches!(state, DeliveryTaskState::Skipped(_)))
    {
        return Some(skipped.clone());
    }
    if states.contains(&DeliveryTaskState::Waiting) {
        return Some(DeliveryTaskState::Waiting);
    }
    (!states.is_empty()).then_some(DeliveryTaskState::Done)
}

/// An item's files taken together: the run's states (by task ID) while its set runs, else the recorded results.
pub fn item_state(
    set: &DeliverySet,
    item: &jutsu_core::DeliveryItem,
    running: Option<&std::collections::HashMap<String, DeliveryTaskState, impl std::hash::BuildHasher>>,
) -> Option<DeliveryTaskState> {
    let outputs = set.outputs.iter().filter(|output| output.kind() == item.kind);
    let states: Vec<DeliveryTaskState> = if let Some(running) = running {
        outputs
            .filter_map(|output| {
                let id = format!(
                    "{}/{}",
                    jutsu_core::support::uuid_string(item.reference.version_id),
                    jutsu_core::support::uuid_string(output.id)
                );
                running.get(&id).cloned()
            })
            .collect()
    } else {
        outputs
            .filter_map(|output| {
                let result = set.result(item, output)?;
                let message = result.message.clone().unwrap_or_default();
                Some(match result.status {
                    DeliveryResultStatus::Done => DeliveryTaskState::Done,
                    DeliveryResultStatus::Failed => DeliveryTaskState::Failed(message),
                    DeliveryResultStatus::Skipped => DeliveryTaskState::Skipped(message),
                })
            })
            .collect()
    };
    combined_state(&states)
}

/// The crop of the frame sheet in fractions of the picture: what the full-size export crops.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FrameRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// `frame` for the aspect keyed `aspect` on a picture `picture_aspect` wide over high; `None` for an unknown aspect
/// or picture.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "a 10 000 px stand-in picture"
)]
pub fn crop_rect(frame: CropFrame, aspect: &str, picture_aspect: f64) -> Option<FrameRect> {
    const WIDTH: i64 = 10_000;
    let aspect = aspect_of(aspect)?;
    if !picture_aspect.is_finite() || picture_aspect <= 0.0 {
        return None;
    }
    let height = ((WIDTH as f64) / picture_aspect).round().max(1.0) as i64;
    let rect = frame.rect(aspect, jutsu_core::PixelSize::new(WIDTH, height));
    Some(FrameRect {
        x: rect.x as f64 / WIDTH as f64,
        y: rect.y as f64 / height as f64,
        width: rect.width as f64 / WIDTH as f64,
        height: rect.height as f64 / height as f64,
    })
}

/// The badge's hover text.
pub fn state_hint(state: &DeliveryTaskState) -> Option<String> {
    match state {
        DeliveryTaskState::Running => None,
        DeliveryTaskState::Waiting => Some("Waiting".to_owned()),
        DeliveryTaskState::Done => Some("Exported".to_owned()),
        DeliveryTaskState::Failed(message) | DeliveryTaskState::Skipped(message) => Some(message.clone()),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};

    use jutsu_core::{DeliveryFolder, DeliveryPlan, DeliveryScope, Material, MaterialVersion};
    use uuid::Uuid;

    use super::*;

    fn outputs() -> Vec<DeliveryOutput> {
        let mut web = DeliveryOutput::new(DeliveryPreset::Image(ImageExportPreset::new(ImageFormat::Jpeg)));
        web.name = "web".to_owned();
        let mut print = DeliveryOutput::new(DeliveryPreset::Image(ImageExportPreset::new(ImageFormat::Png)));
        print.name = "print".to_owned();
        vec![web, print]
    }

    fn set(images: usize) -> DeliverySet {
        let materials: Vec<Material> = (1..=images)
            .map(|index| {
                let mut version = MaterialVersion::new(Uuid::new_v4(), OffsetDateTime::UNIX_EPOCH);
                version.file_url = Some(crate::testing::file_url(format!("/library/img{index}.png")));
                Material::new(
                    Uuid::new_v4(),
                    MaterialKind::Image,
                    OffsetDateTime::UNIX_EPOCH,
                    version,
                )
            })
            .collect();
        DeliverySet::new(
            Uuid::new_v4(),
            "Set",
            OffsetDateTime::UNIX_EPOCH,
            DeliverySet::items_from(&materials),
            outputs(),
        )
    }

    fn folder(path: &str) -> DeliveryFolder {
        DeliveryFolder {
            url: crate::testing::file_url(path),
        }
    }

    #[tokio::test]
    async fn a_failure_is_noted_and_the_run_goes_on() {
        let mut delivery = set(3);
        let tasks = DeliveryPlan::tasks(&delivery, DeliveryScope::All, |_| false);
        let failing = tasks[2].id();
        let missing = tasks[5].id();
        let states: RefCell<HashMap<String, DeliveryTaskState>> = RefCell::default();
        let results = run(
            &tasks,
            |task| {
                let id = task.id();
                let (failing, missing) = (failing.clone(), missing.clone());
                async move {
                    if id == failing {
                        return Err(DeliveryRunError::Other("disk full".to_owned()));
                    }
                    if id == missing {
                        return Err(DeliveryRunError::SourceUnavailable);
                    }
                    Ok(())
                }
            },
            |id, state| {
                states.borrow_mut().insert(id.to_owned(), state);
            },
        )
        .await;
        delivery.record(results.clone(), folder("/out"), OffsetDateTime::now_utc());
        let states = states.into_inner();
        assert_eq!(results.len(), 6);
        assert_eq!(
            states[&failing],
            DeliveryTaskState::Failed("disk full".to_owned())
        );
        assert_eq!(
            states[&missing],
            DeliveryTaskState::Skipped(UNAVAILABLE_MESSAGE.to_owned())
        );
        assert_eq!(
            states
                .values()
                .filter(|state| **state == DeliveryTaskState::Done)
                .count(),
            4
        );
        assert_eq!(
            DeliveryCounts::new(&delivery),
            DeliveryCounts {
                done: 4,
                failed: 1,
                skipped: 1
            }
        );
        assert!(delivery.is_frozen());
    }

    #[tokio::test]
    async fn retry_writes_only_the_failed_files() {
        let mut delivery = set(4);
        let tasks = DeliveryPlan::tasks(&delivery, DeliveryScope::All, |_| false);
        let failing: HashSet<String> = [tasks[1].id(), tasks[6].id()].into();
        let first = run(
            &tasks,
            |task| {
                let fails = failing.contains(&task.id());
                async move {
                    if fails {
                        Err(DeliveryRunError::Other("disk full".to_owned()))
                    } else {
                        Ok(())
                    }
                }
            },
            |_, _| {},
        )
        .await;
        delivery.record(first.clone(), folder("/out"), OffsetDateTime::now_utc());
        // The folder holds what the first run wrote.
        let existing: HashSet<String> = first
            .iter()
            .filter_map(|result| result.file_name.as_ref().map(|name| name.to_lowercase()))
            .collect();
        let written: RefCell<Vec<String>> = RefCell::default();
        let retry = DeliveryPlan::tasks(&delivery, DeliveryScope::Failed, |name| {
            existing.contains(&name.to_lowercase())
        });
        let second = run(
            &retry,
            |task| {
                written.borrow_mut().push(task.id());
                async { Ok(()) }
            },
            |_, _| {},
        )
        .await;
        delivery.record(second, folder("/out"), OffsetDateTime::now_utc());
        assert_eq!(written.into_inner().into_iter().collect::<HashSet<_>>(), failing);
        assert_eq!(
            retry
                .iter()
                .map(|task| task.file_name.clone())
                .collect::<Vec<_>>(),
            vec![tasks[1].file_name.clone(), tasks[6].file_name.clone()]
        );
        assert_eq!(DeliveryCounts::new(&delivery).failed, 0);
        assert_eq!(DeliveryCounts::new(&delivery).done, 8);
        assert_eq!(
            DeliveryPlan::tasks(&delivery, DeliveryScope::Failed, |_| false),
            Vec::<DeliveryTask>::new()
        );
    }

    #[tokio::test]
    async fn a_canceled_run_notes_only_what_finished() {
        let tasks = DeliveryPlan::tasks(&set(3), DeliveryScope::All, |_| false);
        let stop_at = tasks[2].id();
        let results = run(
            &tasks,
            |task| {
                let stop = task.id() == stop_at;
                async move {
                    if stop {
                        Err(DeliveryRunError::Canceled)
                    } else {
                        Ok(())
                    }
                }
            },
            |_, _| {},
        )
        .await;
        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .all(|result| result.status == DeliveryResultStatus::Done)
        );
    }

    #[test]
    fn the_status_says_when_and_where() {
        let mut delivery = set(1);
        assert_eq!(status_text(&delivery, UtcOffset::UTC), "Not exported");
        assert_eq!(DeliveryCounts::new(&delivery).summary(), None);
        let tasks = DeliveryPlan::tasks(&delivery, DeliveryScope::All, |_| false);
        delivery.record(
            vec![
                tasks[0].result(DeliveryResultStatus::Done, None),
                tasks[1].result(DeliveryResultStatus::Failed, Some("x".to_owned())),
            ],
            folder("/Client"),
            OffsetDateTime::now_utc(),
        );
        assert!(status_text(&delivery, UtcOffset::UTC).ends_with("to Client"));
        assert_eq!(
            DeliveryCounts::new(&delivery).summary().as_deref(),
            Some("1 done · 1 failed")
        );
    }

    #[test]
    fn an_output_sums_itself_up() {
        let mut image = ImageExportPreset::new(ImageFormat::Jpeg);
        image.long_edge = Some(2048);
        let mut output = DeliveryOutput::new(DeliveryPreset::Image(image));
        output.aspect = Some(CropAspect::new(4, 5));
        assert_eq!(output_summary(&output), "JPEG · 4:5 · 2048 px · sRGB");
        let mut video = DeliveryOutput::new(DeliveryPreset::Video(ExportPreset::EditingH264));
        video.aspect = Some(CropAspect::new(9, 16));
        video.subtitles = Some(DeliverySubtitles::BurnIn);
        assert_eq!(output_summary(&video), "Editing H.264 · 9:16 · Burn In");
        assert_eq!(
            next_round_name("Delivery 15 Oct 2026"),
            "Delivery 15 Oct 2026 – next round"
        );
        assert_eq!(
            default_name(
                time::Date::from_calendar_date(2026, time::Month::October, 15)
                    .unwrap()
                    .with_hms(10, 41, 0)
                    .unwrap()
                    .assume_utc(),
                UtcOffset::UTC
            ),
            "Delivery 15 Oct 2026"
        );
    }

    #[test]
    fn a_row_changes_its_output() {
        let mut output =
            DeliveryOutput::new(DeliveryPreset::Image(ImageExportPreset::new(ImageFormat::Jpeg)));
        DeliveryOutputChange::Size("2048".to_owned()).apply(&mut output);
        DeliveryOutputChange::Quality(70).apply(&mut output);
        DeliveryOutputChange::Background("#ff0000".to_owned()).apply(&mut output);
        DeliveryOutputChange::Aspect("4x5".to_owned()).apply(&mut output);
        let view = DeliveryOutputView::new(&output);
        assert_eq!(view.size, "2048");
        assert_eq!(view.quality, Some(70));
        assert_eq!(view.background.as_deref(), Some("#ff0000"));
        assert_eq!(view.aspect, "4x5");
        DeliveryOutputChange::Format("png".to_owned()).apply(&mut output);
        let view = DeliveryOutputView::new(&output);
        assert_eq!((view.quality, view.background), (None, None));
        // A frozen set keeps its items.
        let mut delivery = set(2);
        let tasks = DeliveryPlan::tasks(&delivery, DeliveryScope::All, |_| false);
        delivery.record(
            vec![tasks[0].result(DeliveryResultStatus::Done, None)],
            folder("/out"),
            OffsetDateTime::now_utc(),
        );
        let version = delivery.items()[0].reference.version_id.to_string();
        assert_eq!(
            DeliverySetChange::RemoveItem { version_id: version }.apply(&mut delivery),
            Err(DeliverySetError::Frozen)
        );
    }

    #[test]
    fn an_items_badge_shows_what_matters_most() {
        assert_eq!(combined_state(&[]), None);
        assert_eq!(
            combined_state(&[DeliveryTaskState::Done, DeliveryTaskState::Waiting]),
            Some(DeliveryTaskState::Waiting)
        );
        assert_eq!(
            combined_state(&[
                DeliveryTaskState::Skipped("gone".to_owned()),
                DeliveryTaskState::Failed("disk".to_owned()),
                DeliveryTaskState::Done,
            ]),
            Some(DeliveryTaskState::Failed("disk".to_owned()))
        );
        assert_eq!(
            combined_state(&[
                DeliveryTaskState::Failed("disk".to_owned()),
                DeliveryTaskState::Running
            ]),
            Some(DeliveryTaskState::Running)
        );
        // Recorded results after a run, live states during one.
        let mut delivery = set(1);
        let tasks = DeliveryPlan::tasks(&delivery, DeliveryScope::All, |_| false);
        let item = delivery.items()[0].clone();
        assert_eq!(
            item_state(&delivery, &item, None::<&HashMap<String, DeliveryTaskState>>),
            None
        );
        delivery.record(
            vec![tasks[0].result(DeliveryResultStatus::Done, None)],
            folder("/out"),
            OffsetDateTime::now_utc(),
        );
        assert_eq!(
            item_state(&delivery, &item, None::<&HashMap<String, DeliveryTaskState>>),
            Some(DeliveryTaskState::Done)
        );
        let running = std::collections::HashMap::from([(tasks[1].id(), DeliveryTaskState::Running)]);
        assert_eq!(
            item_state(&delivery, &item, Some(&running)),
            Some(DeliveryTaskState::Running)
        );
        assert_eq!(state_hint(&DeliveryTaskState::Done).as_deref(), Some("Exported"));
    }

    #[test]
    fn frames_follow_the_outputs_of_the_items_kind() {
        let mut outputs = outputs();
        outputs[0].aspect = Some(CropAspect::new(4, 5));
        outputs[1].aspect = Some(CropAspect::new(4, 5));
        let mut video = DeliveryOutput::new(DeliveryPreset::Video(ExportPreset::EditingH264));
        video.aspect = Some(CropAspect::new(9, 16));
        outputs.push(video);
        assert_eq!(
            frame_aspects(&outputs, MaterialKind::Image),
            [CropAspect::new(4, 5)]
        );
        assert_eq!(
            frame_aspects(&outputs, MaterialKind::Video),
            [CropAspect::new(9, 16)]
        );
        // A square in the middle of a 16:9 picture.
        let rect = crop_rect(CropFrame::CENTERED, "1x1", 16.0 / 9.0).unwrap();
        assert!((rect.width - 0.5625).abs() < 0.001 && (rect.x - 0.21875).abs() < 0.001);
        assert!((rect.height - 1.0).abs() < 0.001 && rect.y.abs() < 0.001);
        assert_eq!(crop_rect(CropFrame::CENTERED, "nope", 1.0), None);
    }
}
