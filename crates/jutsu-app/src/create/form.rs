//! The Create screen's video form (`CreateForm.swift`, `+Ranges`, `+Keyboard`, `+ModelOptions`, `+Variants`,
//! `CreateDraft.swift`, `SourceVideo.swift`): one draft job per range of the source, all with the same inputs. Edits
//! go through the methods, which keep every range inside the selected model's duration window and apart from the
//! others, and the references inside its limits (`limits`).

use std::collections::{BTreeMap, BTreeSet};

use jutsu_core::{
    AspectRatio, AudioOption, CharacterOrientation, CinemaControl, CinemaControls, ClosedRange, JobSpec,
    KlingQuality, LocalAudio, LocalImage, Mode, ModeLimits, ModelConstraintTable, PixelSize, Pricing,
    ProviderKind, RemoteAsset, RemoteInputs, Resolution, SeedanceBitrate, SourceClip, Validation,
    VideoControl, VideoModel,
};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::queue::format;

/// Seconds from the start of the file.
pub type TimeRange = ClosedRange<f64>;

pub(crate) fn time_range(lower: f64, upper: f64) -> TimeRange {
    ClosedRange::new(lower, upper)
}

pub(crate) fn span(range: TimeRange) -> f64 {
    range.upper() - range.lower()
}

/// "12.5 s": one decimal.
pub fn seconds_text(seconds: f64) -> String {
    format!("{seconds:.1} s")
}

/// A probed source video as the Create screen shows it (`SourceVideo.swift`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceVideo {
    pub file_url: Url,
    /// Length of the whole file, in seconds.
    pub duration: f64,
    pub pixel_size: PixelSize,
    /// `FourCC` of the video codec, e.g. `avc1`.
    pub codec: String,
    pub file_size: i64,
    /// The `https:` link the file was downloaded from, carried into the job's `SourceClip`.
    #[serde(default)]
    pub remote_origin: Option<Url>,
}

impl SourceVideo {
    pub fn is_file(&self) -> bool {
        self.file_url.scheme() == "file"
    }

    pub fn name(&self) -> String {
        if !self.is_file() {
            return "Source on Higgsfield".to_owned();
        }
        self.file_url
            .to_file_path()
            .ok()
            .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
            .unwrap_or_default()
    }

    fn extension(&self) -> String {
        self.file_url
            .path()
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_lowercase())
            .unwrap_or_default()
    }

    /// Anything but H.264 in MP4 is transcoded before upload, which takes noticeably longer.
    pub fn needs_conversion(&self) -> bool {
        self.codec != "avc1" || self.extension() != "mp4"
    }

    /// The webview plays it as it is: H.264 in MP4 or MOV. `ProRes`, HEVC and the rest get a proxy.
    pub fn plays_in_webview(&self) -> bool {
        matches!(self.codec.as_str(), "avc1" | "avc3")
            && matches!(self.extension().as_str(), "mp4" | "m4v" | "mov")
    }

    /// "1920×1080".
    pub fn size_text(&self) -> String {
        format!("{}×{}", self.pixel_size.width, self.pixel_size.height)
    }

    /// "186 MB · HEVC".
    pub fn file_text(&self) -> String {
        if !self.is_file() {
            return self.codec_name();
        }
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            reason = "megabytes of a file"
        )]
        let megabytes = ((self.file_size as f64 / 1_000_000.0).round() as i64).max(1);
        format!("{} MB · {}", grouped(megabytes), self.codec_name())
    }

    pub fn codec_name(&self) -> String {
        match self.codec.as_str() {
            "avc1" | "avc3" => "H.264".to_owned(),
            "hvc1" | "hev1" => "HEVC".to_owned(),
            "ap4h" | "ap4x" | "apch" | "apcn" | "apcs" | "apco" => "ProRes".to_owned(),
            other => other.trim().to_uppercase(),
        }
    }
}

/// "1,234".
fn grouped(value: i64) -> String {
    let digits = value.abs().to_string();
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    if value < 0 { format!("-{text}") } else { text }
}

/// How much of the range a draft renders.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum DraftLength {
    #[default]
    WholeRange,
    FirstFiveSeconds,
}

impl DraftLength {
    pub const SHORT_SECONDS: f64 = 5.0;
    pub const ALL: [Self; 2] = [Self::WholeRange, Self::FirstFiveSeconds];

    pub fn title(self) -> &'static str {
        match self {
            Self::WholeRange => "Whole range",
            Self::FirstFiveSeconds => "First 5 s",
        }
    }
}

/// One of Seedance's frame slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum FrameSlot {
    Start,
    End,
}

impl FrameSlot {
    pub fn title(self) -> &'static str {
        match self {
            Self::Start => "Start frame",
            Self::End => "End frame",
        }
    }
}

/// A part of the trim strip the arrow keys move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum TrimPart {
    Start,
    End,
    Selection,
}

/// The submit bar's main button, which the Generate shortcut presses: Generate, or Draft in a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimarySubmit {
    pub title: String,
    pub is_draft: bool,
    /// `false` while a blocker stands or a submit is running.
    pub is_enabled: bool,
}

/// Options only some models take, each `None` until picked (`ModelPicks`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelPicks {
    #[serde(rename = "seedanceBitrate", default)]
    pub bitrate: Option<SeedanceBitrate>,
    #[serde(rename = "klingOrientation", default)]
    pub orientation: Option<CharacterOrientation>,
    #[serde(rename = "cinemaPicks", default)]
    pub cinema: Option<CinemaControls>,
    #[serde(rename = "variantCount", default)]
    pub variants: Option<i64>,
}

/// Seedance's start and end frames, for a clip made without a source (`FramePair`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FramePair {
    #[serde(rename = "startFrame", default)]
    pub start: Option<LocalImage>,
    #[serde(rename = "endFrame", default)]
    pub end: Option<LocalImage>,
}

impl FramePair {
    /// The first two of `images`, as a job made from frames lists them.
    pub fn from_images(images: &[LocalImage]) -> Self {
        Self {
            start: images.first().cloned(),
            end: images.get(1).cloned(),
        }
    }

    pub fn get(&self, slot: FrameSlot) -> Option<&LocalImage> {
        match slot {
            FrameSlot::Start => self.start.as_ref(),
            FrameSlot::End => self.end.as_ref(),
        }
    }

    pub fn set(&mut self, slot: FrameSlot, image: Option<LocalImage>) {
        match slot {
            FrameSlot::Start => self.start = image,
            FrameSlot::End => self.end = image,
        }
    }

    pub fn images(&self) -> Vec<LocalImage> {
        self.start.iter().chain(self.end.iter()).cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.start.is_none() && self.end.is_none()
    }
}

/// One range rendered once per style (restyle) or per character set (motion transfer, object swap): the batch's
/// entries and the character sets already published as elements (`CreateBatch`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateBatch {
    #[serde(rename = "isBatch", default)]
    pub is_on: bool,
    /// Restyle: the styles in the order picked.
    #[serde(rename = "batchPresetIDs", default)]
    pub preset_ids: Vec<Uuid>,
    /// Motion transfer and object swap: one set of reference images per character.
    #[serde(rename = "characterSets", default)]
    pub character_sets: Vec<Vec<LocalImage>>,
    /// Character sets published as elements ("Save to Higgsfield…"), by their images' file URLs (`key`), so a set
    /// isn't published twice. Optional so a draft saved before it still decodes.
    #[serde(
        rename = "savedCharacterSets",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub saved_sets: Option<Vec<SavedSet>>,
}

/// A character set published as an element, known by its files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSet {
    pub files: Vec<Url>,
    pub element: super::elements::SavedElement,
}

impl CreateBatch {
    /// The jobs the batch makes in `mode`: one per style for restyle, one per character set otherwise.
    pub fn count(&self, mode: Mode) -> usize {
        if mode == Mode::Restyle {
            self.preset_ids.len()
        } else {
            self.character_sets.len()
        }
    }

    /// A set is known by its files, which stay the same while it is in the form.
    pub fn key(images: &[LocalImage]) -> Vec<Url> {
        images.iter().map(|image| image.file_url.clone()).collect()
    }

    fn saved(&self, key: &[Url]) -> Option<&super::elements::SavedElement> {
        self.saved_sets
            .as_ref()?
            .iter()
            .find(|saved| saved.files == key)
            .map(|saved| &saved.element)
    }

    /// Removes a set; it stays published only while an equal set is still in the batch.
    pub fn remove_character_set(&mut self, index: usize) {
        if index >= self.character_sets.len() {
            return;
        }
        let removed = Self::key(&self.character_sets.remove(index));
        if !self.character_sets.iter().any(|set| Self::key(set) == removed)
            && let Some(saved) = &mut self.saved_sets
        {
            saved.retain(|saved| saved.files != removed);
        }
    }

    /// The element the set at `index` was published as on the account `owner`; `None` when it wasn't.
    pub fn saved_element(&self, index: usize, owner: Option<&str>) -> Option<&super::elements::SavedElement> {
        let set = self.character_sets.get(index)?;
        self.saved(&Self::key(set))
            .filter(|saved| saved.owner.as_deref() == owner)
    }

    /// Records that the set of `images` was published as `element`.
    pub fn set_saved_element(&mut self, element: super::elements::SavedElement, images: &[LocalImage]) {
        let key = Self::key(images);
        let saved = self.saved_sets.get_or_insert_with(Vec::new);
        saved.retain(|saved| saved.files != key);
        saved.push(SavedSet { files: key, element });
    }

    /// The character sets with their files kept by `keeps`. A set that lost every file is dropped; one that lost some
    /// is no longer the one published.
    pub fn reopening_sets(&self, keeps: &dyn Fn(&Url) -> bool) -> Self {
        let mut batch = self.clone();
        let mut saved: Vec<SavedSet> = Vec::new();
        batch.character_sets = self
            .character_sets
            .iter()
            .filter_map(|images| {
                let reopened: Vec<LocalImage> = images
                    .iter()
                    .filter(|image| keeps(&image.file_url))
                    .cloned()
                    .collect();
                if reopened.is_empty() {
                    return None;
                }
                if reopened.len() == images.len()
                    && let Some(element) = self.saved(&Self::key(images))
                {
                    saved.push(SavedSet {
                        files: Self::key(&reopened),
                        element: element.clone(),
                    });
                }
                Some(reopened)
            })
            .collect();
        batch.saved_sets = self.saved_sets.as_ref().map(|_| saved);
        batch
    }
}

/// A title and one line of detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub title: String,
    pub detail: String,
}

/// The Create screen's video draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateForm {
    mode: Mode,
    /// Kept across mode switches; a mode the model doesn't serve runs on its default model (`selected_model`).
    model: VideoModel,
    source: Option<SourceVideo>,
    /// Sorted and not overlapping; ranges may touch.
    ranges: Vec<TimeRange>,
    /// The range the trim handles move.
    selected_range_index: usize,
    references: Vec<LocalImage>,
    /// Kept across model switches, sent only on a model that takes one.
    #[serde(default)]
    speech: Option<LocalAudio>,
    /// Only restyle sends it; the style gallery isn't ported, so nothing sets it but a loaded job.
    #[serde(default)]
    preset_id: Option<Uuid>,
    pub prompt: String,
    resolution: Resolution,
    /// What Draft renders at; a setting.
    draft_resolution: Resolution,
    #[serde(default)]
    pub draft_length: DraftLength,
    /// Object swap only: the user accepted upscaling a frame below the minimum.
    #[serde(default)]
    pub allows_upscale: bool,
    pub audio: AudioOption,
    /// Kling only: keep the source video's background rather than the reference image's.
    #[serde(default)]
    pub keeps_video_background: bool,
    /// Seconds a clip made without a source runs, or Extend adds.
    pub length: f64,
    pub aspect_ratio: AspectRatio,
    #[serde(flatten)]
    picks: ModelPicks,
    #[serde(flatten)]
    frames: FramePair,
    /// The batch's styles or character sets (`is_batch`).
    #[serde(flatten)]
    batch: CreateBatch,
    /// The library item the inputs were loaded from; a new source clears it.
    #[serde(default)]
    parent_id: Option<Uuid>,
    /// Inputs the account already holds, from a generation loaded for editing; a new source clears them.
    #[serde(default)]
    remote_inputs: Option<RemoteInputs>,
    /// The server's limits of each model; `None` keeps the constants. The app sets it, never an edit.
    #[serde(skip)]
    constraints: Option<ModelConstraintTable>,
    /// The connection's `Capabilities::unserved_modes`. The app sets it, never an edit.
    #[serde(skip)]
    unserved_modes: BTreeMap<VideoModel, BTreeSet<Mode>>,
}

impl Default for CreateForm {
    fn default() -> Self {
        Self::new(Mode::MotionTransfer, Resolution::P720, Resolution::P480)
    }
}

/// Motion's tasks: a model that changes or continues a clip does that from the Motion tab.
pub const MOTION_TASKS: [Mode; 3] = [Mode::MotionTransfer, Mode::Edit, Mode::Extend];

impl CreateForm {
    /// Seedance's own default length.
    pub const DEFAULT_LENGTH: f64 = 5.0;
    /// One arrow-key press moves a trim handle by a tenth of a second.
    pub const TRIM_STEPS_PER_SECOND: f64 = 10.0;
    /// The Variants stepper's range.
    pub const VARIANT_CHOICES: ClosedRange<i64> = ClosedRange::new(1, JobSpec::MAXIMUM_VARIANTS);

    pub fn new(mode: Mode, resolution: Resolution, draft_resolution: Resolution) -> Self {
        Self {
            mode,
            model: VideoModel::GENJUTSU,
            source: None,
            ranges: vec![time_range(0.0, 0.0)],
            selected_range_index: 0,
            references: Vec::new(),
            speech: None,
            preset_id: None,
            prompt: String::new(),
            resolution,
            draft_resolution,
            draft_length: DraftLength::WholeRange,
            allows_upscale: false,
            audio: AudioOption::Original,
            keeps_video_background: false,
            length: Self::DEFAULT_LENGTH,
            aspect_ratio: AspectRatio::Auto,
            picks: ModelPicks::default(),
            frames: FramePair::default(),
            batch: CreateBatch::default(),
            parent_id: None,
            remote_inputs: None,
            constraints: None,
            unserved_modes: BTreeMap::new(),
        }
    }

    // MARK: Plain reads

    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn model(&self) -> &VideoModel {
        &self.model
    }
    pub fn source(&self) -> Option<&SourceVideo> {
        self.source.as_ref()
    }
    pub fn ranges(&self) -> &[TimeRange] {
        &self.ranges
    }
    pub fn selected_range_index(&self) -> usize {
        self.selected_range_index
    }
    pub fn references(&self) -> &[LocalImage] {
        &self.references
    }
    pub fn resolution(&self) -> Resolution {
        self.resolution
    }
    pub fn draft_resolution(&self) -> Resolution {
        self.draft_resolution
    }
    pub fn parent_id(&self) -> Option<Uuid> {
        self.parent_id
    }
    pub fn preset_id(&self) -> Option<Uuid> {
        self.preset_id
    }
    /// The style restyle sends; the gallery that picks it isn't ported, so only a loaded job and tests set it.
    pub fn set_preset_id(&mut self, preset: Option<Uuid>) {
        self.preset_id = preset;
    }
    pub fn constraints(&self) -> ModelConstraintTable {
        self.constraints
            .clone()
            .unwrap_or_else(ModelConstraintTable::built_in)
    }

    /// What Draft renders at, from Settings.
    pub fn set_draft_resolution(&mut self, resolution: Resolution) {
        self.draft_resolution = resolution;
    }

    // MARK: Mode and model

    /// A mode change brings the inputs inside the new limits and forgets the upscale.
    pub fn set_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        self.clamp();
        self.allows_upscale = false;
        if !self.offers_batch() {
            self.set_batch(false);
        }
    }

    /// Within Motion, a model runs one of its own tasks.
    pub fn set_model(&mut self, model: VideoModel) {
        if model == self.model {
            return;
        }
        self.model = model;
        if self.tab() == Mode::MotionTransfer
            && !self.model.serves(self.mode)
            && let Some(task) = MOTION_TASKS.iter().copied().find(|task| self.model.serves(*task))
        {
            self.set_mode(task);
        }
        self.clamp();
    }

    pub fn set_constraints(&mut self, constraints: Option<ModelConstraintTable>) {
        if constraints == self.constraints {
            return;
        }
        self.constraints = constraints;
        self.clamp();
    }

    pub fn set_unserved_modes(&mut self, unserved: BTreeMap<VideoModel, BTreeSet<Mode>>) {
        if unserved == self.unserved_modes {
            return;
        }
        self.unserved_modes = unserved;
        self.clamp();
    }

    /// The model the jobs run on: the picked one when it serves the mode on this connection, else the mode's
    /// default.
    pub fn selected_model(&self) -> VideoModel {
        let unserved = self
            .unserved_modes
            .get(&self.model)
            .is_some_and(|modes| modes.contains(&self.mode));
        if self.model.serves(self.mode) && !unserved {
            self.model.clone()
        } else {
            self.mode.default_model()
        }
    }

    /// What one job takes on the selected model; the ranges and references stay inside these.
    pub fn limits(&self) -> ModeLimits {
        let model = self.selected_model();
        let has_source = self.source.is_some() || !model.makes_clips_without_source();
        let mut limits = model.limits(self.mode, has_source, &self.constraints());
        let with_references = self.source.is_some()
            || if self.uses_frames() {
                self.frames.end.is_some()
            } else {
                !self.references.is_empty()
            };
        limits.duration = model.duration(limits.duration, self.mode, with_references);
        limits
    }

    /// The tab the mode belongs to: Edit and Extend are Motion's.
    pub fn tab(&self) -> Mode {
        if self.mode.is_video_operation() {
            Mode::MotionTransfer
        } else {
            self.mode
        }
    }

    /// Picking Motion runs the model's first task.
    pub fn set_tab(&mut self, tab: Mode) {
        if tab == self.tab() {
            return;
        }
        let mode = if tab == Mode::MotionTransfer {
            MOTION_TASKS
                .iter()
                .copied()
                .find(|task| self.model.serves(*task))
                .unwrap_or(Mode::MotionTransfer)
        } else {
            tab
        };
        self.set_mode(mode);
    }

    /// Picks one of the task choices, keeping the model the picker shows.
    pub fn pick_task(&mut self, task: Mode) {
        let model = self.selected_model();
        self.set_model(model);
        self.set_mode(task);
    }

    pub const MODE_CHOICES: [Mode; 3] = [Mode::MotionTransfer, Mode::Restyle, Mode::ObjectSwap];

    pub fn shows_added_length(&self) -> bool {
        self.mode == Mode::Extend
    }

    /// "Add 4–30 s".
    pub fn added_length_title(&self) -> String {
        let window = self.limits().duration;
        format!("Add {}–{} s", trunc(window.lower()), trunc(window.upper()))
    }

    /// Moves a model the connection doesn't offer, or doesn't run in the form's mode, to Genjutsu. Returns what to
    /// tell the user when the model changed.
    pub fn fit_model(&mut self, offered: Option<&BTreeSet<VideoModel>>) -> Option<Notice> {
        let offered = offered?;
        let unserved = self
            .unserved_modes
            .get(&self.model)
            .is_some_and(|modes| modes.contains(&self.mode));
        if offered.contains(&self.model) && !unserved {
            return None;
        }
        let unavailable = self.model.title();
        self.set_model(VideoModel::GENJUTSU);
        Some(Notice {
            title: "Running on Genjutsu".to_owned(),
            detail: format!("These inputs use {unavailable}, which this connection doesn't offer."),
        })
    }

    // MARK: Source

    /// Replaces the source; a clip up to the longest a job takes is selected whole, a longer one from the start.
    pub fn set_source(&mut self, video: Option<SourceVideo>) {
        let duration = video.as_ref().map_or(0.0, |video| video.duration.max(0.0));
        self.source = video;
        self.parent_id = None;
        self.remote_inputs = None;
        self.allows_upscale = false;
        self.draft_length = DraftLength::WholeRange;
        self.selected_range_index = 0;
        self.ranges = vec![time_range(0.0, duration.min(self.limits().duration.upper()))];
    }

    /// Cuts the references to the limits and brings every range's length inside them.
    fn clamp(&mut self) {
        self.clamp_to(self.limits());
    }

    /// `clamp` to `limits`: a range grows only into the room before the next range or the end of the clip.
    pub(crate) fn clamp_to(&mut self, limits: ModeLimits) {
        self.references.truncate(limits.reference_images.upper());
        let Some(total) = self.source.as_ref().map(|source| source.duration) else {
            return;
        };
        let window = limits.duration;
        for index in 0..self.ranges.len() {
            let start = self.ranges[index].lower();
            let room = self
                .ranges
                .get(index + 1)
                .map_or(total, jutsu_core::ClosedRange::lower);
            let end = room.min(
                self.ranges[index]
                    .upper()
                    .min(start + window.upper())
                    .max(start + window.lower()),
            );
            self.ranges[index] = time_range(start, end.max(start));
        }
    }

    pub fn is_source_remote(&self) -> bool {
        self.source.as_ref().is_some_and(|source| !source.is_file())
    }

    /// The range being edited on the timeline.
    pub fn range(&self) -> TimeRange {
        self.ranges
            .get(self.selected_range_index)
            .copied()
            .unwrap_or_else(|| time_range(0.0, 0.0))
    }

    fn set_range(&mut self, range: TimeRange) {
        if let Some(slot) = self.ranges.get_mut(self.selected_range_index) {
            *slot = range;
        }
    }

    pub fn selected_duration(&self) -> f64 {
        span(self.range())
    }

    fn can_trim(&self) -> bool {
        self.source.is_some() && !self.is_source_remote()
    }

    /// Moves the start handle; the end stays and the length stays inside the window where the room allows.
    pub fn set_start(&mut self, time: f64) {
        if !self.can_trim() {
            return;
        }
        let end = self.range().upper();
        let window = self.limits().duration;
        let start = self
            .room()
            .lower()
            .max(time.max(end - window.upper()).min(end - window.lower()));
        self.set_range(time_range(start.min(end), end));
    }

    /// Moves the end handle; the start stays.
    pub fn set_end(&mut self, time: f64) {
        if !self.can_trim() {
            return;
        }
        let start = self.range().lower();
        let window = self.limits().duration;
        let end = self
            .room()
            .upper()
            .min(time.min(start + window.upper()).max(start + window.lower()));
        self.set_range(time_range(start, end.max(start)));
    }

    /// Slides the whole selection so it starts at `time`, keeping its length between its neighbours.
    pub fn move_range(&mut self, time: f64) {
        if !self.can_trim() {
            return;
        }
        let length = self.selected_duration();
        let room = self.room();
        let start = time
            .max(room.lower())
            .min(room.lower().max(room.upper() - length));
        self.set_range(time_range(start, start + length));
    }

    /// Moves `part` by `steps` tenths of a second, clamped as a drag is; the moved edge lands on the grid.
    pub fn nudge(&mut self, part: TrimPart, steps: i32) {
        let target = |time: f64| {
            (time * Self::TRIM_STEPS_PER_SECOND + f64::from(steps)).round() / Self::TRIM_STEPS_PER_SECOND
        };
        let range = self.range();
        match part {
            TrimPart::Start => self.set_start(target(range.lower())),
            TrimPart::End => self.set_end(target(range.upper())),
            TrimPart::Selection => self.move_range(target(range.lower())),
        }
    }

    /// The frame a part shows in the preview: its own edge, or the start of the selection.
    pub fn time_of(&self, part: TrimPart) -> f64 {
        let range = self.range();
        if part == TrimPart::End {
            range.upper()
        } else {
            range.lower()
        }
    }

    /// "00:06.0" for a handle, "00:06.0 – 00:18.5" for the selection.
    pub fn accessibility_value(&self, part: TrimPart) -> String {
        let range = self.range();
        match part {
            TrimPart::Start => format::timecode(range.lower()),
            TrimPart::End => format::timecode(range.upper()),
            TrimPart::Selection => self.range_text(self.selected_range_index()),
        }
    }

    /// What the Generate shortcut submits now.
    pub fn primary_submit(
        &self,
        context: &super::context::CreateContext,
        is_submitting: bool,
    ) -> PrimarySubmit {
        let is_draft = self.runs_as_draft();
        PrimarySubmit {
            title: if is_draft {
                self.draft_button_title()
            } else {
                self.generate_button_title()
            },
            is_draft,
            is_enabled: !is_submitting && self.blocker(is_draft, context).is_none(),
        }
    }

    // MARK: Ranges

    pub fn has_several_ranges(&self) -> bool {
        self.ranges.len() > 1
    }

    /// "00:06.0 – 00:18.5"; empty for an index outside the list.
    pub fn range_text(&self, index: usize) -> String {
        self.ranges.get(index).map_or_else(String::new, |range| {
            format!(
                "{} – {}",
                format::timecode(range.lower()),
                format::timecode(range.upper())
            )
        })
    }

    /// Whole seconds billed over all ranges on `connection`; each job is rounded up on its own.
    pub fn billed_seconds(&self, connection: Option<ProviderKind>) -> i64 {
        self.ranges
            .iter()
            .map(|range| {
                let selected = span(*range);
                let seconds = match (self.mode, connection) {
                    (Mode::Extend, Some(ProviderKind::Account)) => self.clip_length(),
                    (Mode::Extend, _) => selected + self.clip_length(),
                    _ => selected,
                };
                Pricing::billed_seconds(seconds)
            })
            .sum()
    }

    /// "12.5 s selected · billed 13 s"; with several ranges "12.5 s selected · 3 ranges · billed 31 s".
    pub fn trim_summary(&self, connection: Option<ProviderKind>) -> String {
        let selected = seconds_text(self.selected_duration());
        let billed = self.billed_seconds(connection);
        if self.has_several_ranges() {
            format!(
                "{selected} selected · {} ranges · billed {billed} s",
                self.ranges.len()
            )
        } else {
            format!("{selected} selected · billed {billed} s")
        }
    }

    /// Under the source name: "00:06.0 – 00:18.5 · 12.5 s", or "3 ranges · 30.0 s".
    pub fn source_summary(&self) -> String {
        if !self.has_several_ranges() {
            return format!(
                "{} · {}",
                self.range_text(self.selected_range_index),
                seconds_text(self.selected_duration())
            );
        }
        let total: f64 = self.ranges.iter().map(|range| span(*range)).sum();
        format!("{} ranges · {}", self.ranges.len(), seconds_text(total))
    }

    /// Before, between and after the ranges, in order; empty without a source.
    fn gaps(&self) -> Vec<TimeRange> {
        let Some(total) = self.source.as_ref().map(|source| source.duration) else {
            return Vec::new();
        };
        let mut gaps = Vec::new();
        let mut start = 0.0_f64;
        for range in &self.ranges {
            gaps.push(time_range(start, start.max(range.lower())));
            start = range.upper();
        }
        gaps.push(time_range(start, start.max(total)));
        gaps
    }

    /// Parts of the source no range covers, for dimming on the timeline.
    pub fn uncovered_ranges(&self) -> Vec<TimeRange> {
        self.gaps()
            .into_iter()
            .filter(|gap| gap.upper() > gap.lower())
            .collect()
    }

    fn next_range(&self) -> Option<TimeRange> {
        let window = self.limits().duration;
        let length = self.selected_duration().max(window.lower()).min(window.upper());
        let gap = self.gaps().into_iter().find(|gap| span(*gap) >= window.lower())?;
        Some(time_range(gap.lower(), gap.upper().min(gap.lower() + length)))
    }

    pub fn can_add_range(&self) -> bool {
        !self.is_source_remote() && self.next_range().is_some()
    }

    /// Adds a range in the first gap that fits the window, as long as the edited one where it fits, and edits it.
    pub fn add_range(&mut self) {
        if !self.can_add_range() {
            return;
        }
        let Some(next) = self.next_range() else {
            return;
        };
        let index = self
            .ranges
            .iter()
            .position(|range| range.lower() >= next.upper())
            .unwrap_or(self.ranges.len());
        self.ranges.insert(index, next);
        self.selected_range_index = index;
    }

    /// The last range can't be removed; removing the edited one edits the next, or the previous at the end.
    pub fn remove_range(&mut self, index: usize) {
        if !self.has_several_ranges() || index >= self.ranges.len() {
            return;
        }
        self.ranges.remove(index);
        if index < self.selected_range_index || self.selected_range_index == self.ranges.len() {
            self.selected_range_index = self.selected_range_index.saturating_sub(1);
        }
    }

    pub fn select_range(&mut self, index: usize) {
        if index < self.ranges.len() {
            self.selected_range_index = index;
        }
    }

    /// Where the edited range may go: from the end of the previous range to the start of the next.
    fn room(&self) -> TimeRange {
        let index = self.selected_range_index;
        let lower = index
            .checked_sub(1)
            .and_then(|previous| self.ranges.get(previous))
            .map_or(0.0, jutsu_core::ClosedRange::upper);
        let upper = self.ranges.get(index + 1).map_or_else(
            || self.source.as_ref().map_or(0.0, |source| source.duration),
            jutsu_core::ClosedRange::lower,
        );
        time_range(lower, lower.max(upper))
    }

    // MARK: References

    pub fn can_add_references(&self) -> bool {
        self.references.len() < self.limits().reference_images.upper()
    }

    /// How many more references the form takes.
    pub fn reference_room(&self) -> usize {
        self.limits()
            .reference_images
            .upper()
            .saturating_sub(self.references.len())
    }

    /// "2 / 8".
    pub fn reference_count_text(&self) -> String {
        format!(
            "{} / {}",
            self.references.len(),
            self.limits().reference_images.upper()
        )
    }

    /// Appends images in order, skipping files already added and any beyond the limit. Returns how many were added.
    pub fn add_references(&mut self, images: Vec<LocalImage>) -> usize {
        let mut added = 0;
        for image in images {
            if let Some(index) = self
                .references
                .iter()
                .position(|old| old.file_url == image.file_url)
            {
                if let Some(material) = image.material {
                    self.references[index].material = Some(material);
                }
            } else if self.can_add_references() {
                self.references.push(image);
                added += 1;
            }
        }
        added
    }

    /// Adds opened images as references (`addOpenedReferences`). The alert says how many didn't fit the mode's
    /// limit; images already added are skipped silently.
    pub fn add_opened_references(
        &mut self,
        images: Vec<LocalImage>,
    ) -> Option<crate::connection::UserMessage> {
        let count = images.len();
        let left = count.saturating_sub(self.add_references(images));
        if left == 0 || self.can_add_references() {
            return None;
        }
        let limit = self.limits().reference_images.upper();
        let images = if limit == 1 {
            "1 reference image".to_owned()
        } else {
            format!("{limit} reference images")
        };
        let title = if left == 1 {
            "1 image wasn't added".to_owned()
        } else {
            format!("{left} images weren't added")
        };
        Some(crate::connection::UserMessage::new(
            &title,
            format!(
                "{} takes up to {images}.",
                crate::connection::mode_title(self.mode())
            ),
            None,
        ))
    }

    pub fn remove_reference(&mut self, index: usize) {
        if index < self.references.len() {
            self.references.remove(index);
        }
    }

    /// Moves one image to `new_index`, clamped to the list.
    pub fn move_reference(&mut self, index: usize, new_index: usize) {
        if index >= self.references.len() {
            return;
        }
        let image = self.references.remove(index);
        let at = new_index.min(self.references.len());
        self.references.insert(at, image);
    }

    // MARK: Speech

    /// The selected model voices a recording (Intone).
    pub fn offers_speech(&self) -> bool {
        self.selected_model().declares(VideoControl::Speech)
    }

    /// The recording the characters say; kept across model switches, and sent only on a model that takes one.
    pub fn speech(&self) -> Option<&LocalAudio> {
        self.speech.as_ref()
    }

    /// Replaces the recording; `None` removes it.
    pub fn set_speech(&mut self, audio: Option<LocalAudio>) {
        self.speech = audio;
    }

    fn job_speech(&self) -> Option<LocalAudio> {
        if self.offers_speech() {
            self.speech.clone()
        } else {
            None
        }
    }

    /// How long the selected model's speech may run; a model without a window takes any length.
    pub fn speech_seconds(&self) -> TimeRange {
        self.selected_model()
            .descriptor()
            .speech_seconds
            .unwrap_or_else(|| time_range(0.0, f64::INFINITY))
    }

    // MARK: Prompt

    /// Counted in Unicode scalars, as the API does.
    pub fn prompt_length(&self) -> usize {
        self.prompt.chars().count()
    }

    /// "120 / 10 000".
    pub fn prompt_counter_text(&self) -> String {
        format!("{} / 10 000", self.prompt_length())
    }

    // MARK: Object swap upscale

    pub fn is_frame_too_small(&self) -> bool {
        self.mode == Mode::ObjectSwap
            && self.source.as_ref().is_some_and(|source| {
                source.pixel_size.pixel_count() < Validation::OBJECT_SWAP_MINIMUM_PIXELS
            })
    }

    /// The size the preparer renders a too-small swap frame at.
    pub fn upscale_target(&self) -> Option<PixelSize> {
        if !self.is_frame_too_small() {
            return None;
        }
        let size = self.source.as_ref()?.pixel_size;
        Some(Validation::upscaled_size(size.width, size.height))
    }

    /// Shown next to the upscale switch while the swap frame is below the minimum.
    pub fn upscale_notice(&self) -> Option<Notice> {
        let source = self.source.as_ref()?;
        let target = self.upscale_target()?;
        Some(Notice {
            title: format!(
                "{} is below the 640×640 minimum for object swap.",
                source.size_text()
            ),
            detail: format!(
                "Upscale to {}×{} before upload? Detail won't improve, but the API will accept it.",
                target.width, target.height
            ),
        })
    }

    /// Short facts over the preview: conversion, upscale, a clip longer than the API takes.
    pub fn stage_pills(&self) -> Vec<String> {
        let Some(source) = &self.source else {
            return Vec::new();
        };
        let mut pills = Vec::new();
        if source.needs_conversion() {
            pills.push("Will convert to MP4 · H.264".to_owned());
        }
        if self.allows_upscale
            && let Some(target) = self.upscale_target()
        {
            pills.push(format!("Will upscale to {}×{}", target.width, target.height));
        }
        if self.is_source_remote() {
            pills.push("On Higgsfield · the whole clip is used".to_owned());
        }
        let most = self.limits().duration.upper();
        if source.duration > most && !self.is_source_remote() {
            pills.push(format!(
                "{} clip — pick up to {} below",
                seconds_text(source.duration),
                format::limit(most)
            ));
        }
        pills
    }

    // MARK: Draft

    /// "Draft 480p": the draft resolution, or the nearest the mode renders at.
    pub fn draft_title(&self) -> String {
        format!(
            "Draft {}",
            self.selected_model()
                .fitted(self.draft_resolution, self.mode)
                .raw_value()
        )
    }

    pub fn offers_draft(&self) -> bool {
        self.selected_model().offers_draft()
    }

    /// The draft length picker only matters when a range is longer than the short one; Extend never cuts.
    pub fn shows_draft_length(&self) -> bool {
        self.offers_draft()
            && self.source.is_some()
            && !self.is_source_remote()
            && self.mode != Mode::Extend
            && self
                .ranges
                .iter()
                .any(|range| span(*range) > DraftLength::SHORT_SECONDS)
    }

    /// Seconds a draft renders, over all ranges; a clip made without a source renders whole.
    pub fn draft_duration(&self) -> f64 {
        if self.is_without_source() {
            return self.clip_length();
        }
        self.ranges.iter().map(|range| self.draft_length_of(*range)).sum()
    }

    fn draft_length_of(&self, range: TimeRange) -> f64 {
        let length = span(range);
        if self.is_source_remote() {
            return length;
        }
        match self.draft_length {
            DraftLength::WholeRange => length,
            DraftLength::FirstFiveSeconds => length.min(DraftLength::SHORT_SECONDS),
        }
    }

    // MARK: Specs

    /// The jobs to queue, one per range in timeline order; without a source, one clip of `length` when the model
    /// makes one, else none. A draft renders at `draft_resolution` and may be cut to its first seconds; on a model
    /// without drafts it is the same as Generate. Each makes one variant; `CreateContext` sets the count. A batch
    /// queues the edited range once per style or character set instead (`batch_specs`).
    pub fn make_specs(&self, is_draft: bool) -> Vec<JobSpec> {
        let specs = self.range_specs(is_draft);
        if !(self.is_batch() && self.offers_batch()) {
            return specs;
        }
        specs
            .get(self.selected_range_index)
            .map_or_else(Vec::new, |base| self.batch_specs(base))
    }

    fn range_specs(&self, is_draft: bool) -> Vec<JobSpec> {
        let is_draft = is_draft && self.offers_draft();
        let model = self.selected_model();
        let Some(source) = &self.source else {
            if !self.is_without_source() {
                return Vec::new();
            }
            let uses_frames = self.uses_frames();
            let images = if uses_frames {
                self.frames.images()
            } else {
                self.references.clone()
            };
            let mut spec = JobSpec::new(self.mode, model, None, self.job_resolution(is_draft));
            spec.remote_inputs = self.reusable_inputs_for(&images);
            spec.references = images;
            spec.prompt.clone_from(&self.prompt);
            spec.is_draft = is_draft;
            spec.audio = self.job_audio();
            spec.length = Some(self.clip_length());
            spec.aspect_ratio = (!uses_frames).then_some(self.aspect_ratio);
            spec.bitrate = self.job_bitrate();
            spec.references_are_frames = uses_frames;
            spec.speech = self.job_speech();
            return vec![spec];
        };
        self.ranges
            .iter()
            .map(|range| {
                let length = if is_draft && self.mode != Mode::Extend {
                    self.draft_length_of(*range)
                } else {
                    span(*range)
                };
                let mut clip = SourceClip::new(
                    source.file_url.clone(),
                    source.duration,
                    source.pixel_size,
                    time_range(range.lower(), range.lower() + length),
                );
                clip.allows_upscale = self.is_frame_too_small() && self.allows_upscale;
                clip.remote_origin.clone_from(&source.remote_origin);
                let mut spec = JobSpec::new(
                    self.mode,
                    model.clone(),
                    Some(clip),
                    self.job_resolution(is_draft),
                );
                spec.references.clone_from(&self.references);
                spec.preset_id = if self.mode.limits().requires_preset {
                    self.preset_id
                } else {
                    None
                };
                spec.prompt.clone_from(&self.prompt);
                spec.is_draft = is_draft;
                spec.audio = self.job_audio();
                spec.remote_inputs = self.reusable_inputs_for(&self.references);
                spec.keeps_video_background = self.shows_video_background() && self.keeps_video_background;
                spec.length = (self.mode == Mode::Extend).then(|| self.clip_length());
                spec.bitrate = self.job_bitrate();
                spec.character_orientation = self.job_character_orientation();
                spec.cinema_controls = self.job_cinema_controls();
                spec.speech = self.job_speech();
                spec
            })
            .collect()
    }

    /// The held media IDs while the inputs are still the ones they belong to; `None` when any isn't.
    fn reusable_inputs_for(&self, references: &[LocalImage]) -> Option<RemoteInputs> {
        let held = self.remote_inputs.as_ref()?;
        let video_url = held.video.as_ref().map(|video| &video.url);
        if self.source.as_ref().map(|source| &source.file_url) != video_url {
            return None;
        }
        let speech = self.job_speech();
        if let Some(speech) = &speech
            && Some(&speech.file_url) != held.audio.as_ref().map(|audio| &audio.url)
        {
            return None;
        }
        let mut images: Vec<RemoteAsset> = Vec::new();
        for reference in references {
            images.push(
                held.images
                    .iter()
                    .find(|image| image.url == reference.file_url)?
                    .clone(),
            );
        }
        Some(RemoteInputs {
            video: held.video.clone(),
            images,
            audio: if speech.is_none() {
                None
            } else {
                held.audio.clone()
            },
        })
    }

    // MARK: Loading

    /// Opens a queued or finished job's inputs for editing. `video` is the clip probed again; `None` when it can't
    /// be read any more, so the form has no source.
    pub fn load(&mut self, spec: &JobSpec, video: Option<&SourceVideo>, parent_id: Option<Uuid>) {
        self.set_mode(spec.mode);
        self.set_model(spec.model.clone());
        self.set_source(video.cloned());
        if let Some(source) = &mut self.source
            && source.remote_origin.is_none()
        {
            source.remote_origin = spec.source.as_ref().and_then(|clip| clip.remote_origin.clone());
        }
        if let Some(video) = video
            && !video.is_file()
        {
            self.set_range(time_range(0.0, video.duration.max(0.0)));
            self.remote_inputs.clone_from(&spec.remote_inputs);
        } else if let (Some(video), Some(clip)) = (video, &spec.source) {
            let extends_to_end = spec.mode == Mode::Extend && clip.range.upper() >= clip.duration;
            let end = if extends_to_end {
                video.duration
            } else {
                clip.range.upper().min(video.duration)
            };
            let start = clip.range.lower().max(0.0).min(end);
            let lower = if extends_to_end {
                start.max(end - self.limits().duration.upper())
            } else {
                start
            };
            self.set_range(time_range(lower, end));
        } else if spec.source.is_none() {
            self.length = spec.length.unwrap_or(Self::DEFAULT_LENGTH);
            self.aspect_ratio = spec.aspect_ratio.unwrap_or(AspectRatio::Auto);
            self.remote_inputs.clone_from(&spec.remote_inputs);
        }
        if spec.mode == Mode::Extend {
            self.length = spec.length.unwrap_or(Self::DEFAULT_LENGTH);
        }
        if spec.uses_frames() {
            self.references = Vec::new();
            self.frames = FramePair::from_images(&spec.references);
        } else {
            let most = self.limits().reference_images.upper();
            self.references = spec.references.iter().take(most).cloned().collect();
            self.frames = FramePair::default();
        }
        self.preset_id = spec.preset_id;
        self.speech.clone_from(&spec.speech);
        self.prompt.clone_from(&spec.prompt);
        self.resolution = spec.resolution;
        self.allows_upscale = spec.source.as_ref().is_some_and(|clip| clip.allows_upscale);
        self.audio = spec.audio;
        self.keeps_video_background = spec.keeps_video_background;
        if let Some(bitrate) = spec.bitrate {
            self.picks.bitrate = Some(bitrate);
        }
        if let Some(orientation) = spec.character_orientation {
            self.picks.orientation = Some(orientation);
        }
        if spec.model.declares(VideoControl::CinemaControls) {
            self.set_cinema_controls(spec.cinema_controls.clone());
        }
        self.picks.variants = (spec.variants > 1).then_some(spec.variants);
        self.parent_id = parent_id;
        self.set_batch(false);
    }

    // MARK: Kling quality and resolutions

    pub fn shows_kling_quality(&self) -> bool {
        self.selected_model().declares(VideoControl::Quality)
    }

    pub fn shows_video_background(&self) -> bool {
        self.selected_model().declares(VideoControl::VideoBackground)
    }

    pub fn kling_quality(&self) -> KlingQuality {
        KlingQuality::from_resolution(self.resolution)
    }

    pub fn set_kling_quality(&mut self, quality: KlingQuality) {
        self.resolution = quality.resolution();
    }

    /// The resolution the jobs of Draft or Generate are made at.
    pub fn job_resolution(&self, is_draft: bool) -> Resolution {
        if self.shows_kling_quality() {
            return self.kling_quality().resolution();
        }
        let wanted = if is_draft {
            self.draft_resolution
        } else {
            self.resolution
        };
        self.selected_model().fitted(wanted, self.mode)
    }

    pub fn resolution_options(&self) -> Vec<Resolution> {
        self.selected_model().resolutions_for(self.mode)
    }

    pub fn picked_resolution(&self) -> Resolution {
        self.selected_model().fitted(self.resolution, self.mode)
    }

    pub fn set_resolution(&mut self, resolution: Resolution) {
        self.resolution = resolution;
    }

    // MARK: Without a source

    /// No video is picked and the selected model makes a clip from the prompt and images alone.
    pub fn is_without_source(&self) -> bool {
        self.source.is_none()
            && self.selected_model().makes_clips_without_source()
            && !self.mode.is_video_operation()
    }

    /// `length` inside the model's limits.
    pub fn clip_length(&self) -> f64 {
        let window = self.limits().duration;
        self.length.max(window.lower()).min(window.upper())
    }

    pub fn prompt_title(&self) -> &'static str {
        let required = (self.is_without_source() && self.references.is_empty() && !self.uses_frames())
            || self.mode.is_video_operation()
            || (self.selected_model().descriptor().requires_prompt && !self.uses_frames());
        if required { "Prompt" } else { "Prompt · optional" }
    }

    /// The model makes the sound: a clip made without a source, or the part Extend adds.
    pub fn generates_audio(&self) -> bool {
        self.is_without_source() || self.mode == Mode::Extend
    }

    pub fn prompt_hint(&self) -> &'static str {
        if self.is_without_source() {
            "Describe the clip, e.g. “A fox runs through fresh snow at dawn, slow motion”"
        } else {
            mode_prompt_hint(self.mode)
        }
    }

    // MARK: Audio

    pub fn offers_generated_audio(&self) -> bool {
        self.selected_model().declares(VideoControl::GeneratedAudio)
            && self.source.is_some()
            && self.mode != Mode::Extend
    }

    pub fn audio_choices(&self) -> Vec<AudioOption> {
        let mut choices = vec![AudioOption::Original, AudioOption::Mute, AudioOption::FadeOut];
        if self.offers_generated_audio() {
            choices.push(AudioOption::Generated);
        }
        choices
    }

    fn job_audio(&self) -> AudioOption {
        if self.audio == AudioOption::Generated && !self.offers_generated_audio() {
            AudioOption::Original
        } else {
            self.audio
        }
    }

    /// "From the source", or the model when it makes the sound.
    pub fn audio_detail(&self) -> String {
        if self.job_audio() == AudioOption::Generated {
            format!("Made by {}", self.selected_model().title())
        } else {
            "From the source".to_owned()
        }
    }

    // MARK: Seedance bitrate, Kling orientation, Cinema Studio

    pub fn shows_bitrate(&self) -> bool {
        self.selected_model().declares(VideoControl::Bitrate)
    }

    /// The bitrate picked, or what the connection's server uses when none is sent.
    pub fn bitrate(&self, connection: Option<ProviderKind>) -> SeedanceBitrate {
        self.picks
            .bitrate
            .unwrap_or(if connection == Some(ProviderKind::Account) {
                SeedanceBitrate::Standard
            } else {
                SeedanceBitrate::High
            })
    }

    pub fn set_bitrate(&mut self, bitrate: SeedanceBitrate) {
        self.picks.bitrate = Some(bitrate);
    }

    fn job_bitrate(&self) -> Option<SeedanceBitrate> {
        if self.shows_bitrate() {
            self.picks.bitrate
        } else {
            None
        }
    }

    pub fn shows_character_orientation(&self) -> bool {
        self.selected_model().declares(VideoControl::CharacterOrientation)
    }

    pub fn character_orientation(&self) -> CharacterOrientation {
        self.picks.orientation.unwrap_or(CharacterOrientation::Video)
    }

    pub fn set_character_orientation(&mut self, orientation: CharacterOrientation) {
        self.picks.orientation = Some(orientation);
    }

    fn job_character_orientation(&self) -> Option<CharacterOrientation> {
        self.shows_character_orientation()
            .then(|| self.character_orientation())
    }

    pub fn shows_cinema_controls(&self) -> bool {
        self.selected_model().declares(VideoControl::CinemaControls)
    }

    pub fn cinema_controls(&self) -> CinemaControls {
        self.picks.cinema.clone().unwrap_or_default()
    }

    pub fn set_cinema_controls(&mut self, controls: CinemaControls) {
        self.picks.cinema = if controls.is_empty() { None } else { Some(controls) };
    }

    /// One control's value; `None` leaves it to the director.
    pub fn set_cinema(&mut self, control: CinemaControl, value: Option<String>) {
        let mut controls = self.cinema_controls();
        controls.set(control, value);
        self.set_cinema_controls(controls);
    }

    /// "2 set", or `None` with none.
    pub fn cinema_controls_summary(&self) -> Option<String> {
        let count = self.cinema_controls().count();
        (count > 0).then(|| format!("{count} set"))
    }

    fn job_cinema_controls(&self) -> CinemaControls {
        if self.shows_cinema_controls() {
            self.cinema_controls()
        } else {
            CinemaControls::default()
        }
    }

    // MARK: Frames

    /// A model with frames can start and end on given images without a source.
    pub fn shows_frames(&self) -> bool {
        self.selected_model().declares(VideoControl::Frames) && self.is_without_source() && !self.is_batch()
    }

    /// A frame is filled, so the jobs run from the frames and the references wait.
    pub fn uses_frames(&self) -> bool {
        self.shows_frames() && !self.frames.is_empty()
    }

    pub fn frame_slots(&self) -> Vec<FrameSlot> {
        if self.selected_model().takes_end_frame() {
            vec![FrameSlot::Start, FrameSlot::End]
        } else {
            vec![FrameSlot::Start]
        }
    }

    pub fn frame(&self, slot: FrameSlot) -> Option<&LocalImage> {
        self.frames.get(slot)
    }

    pub fn set_frame(&mut self, slot: FrameSlot, image: Option<LocalImage>) {
        self.frames.set(slot, image);
    }

    /// Why the references are off while frames are used.
    pub fn references_disabled_reason(&self) -> Option<String> {
        self.uses_frames().then(|| {
            format!(
                "{} starts from the frames, so references aren't used. Clear both frames to use them.",
                self.selected_model().title()
            )
        })
    }

    /// The frames need a start; an end alone can't run.
    pub fn frames_blocker(&self) -> Option<String> {
        (self.uses_frames() && self.frames.start.is_none())
            .then(|| "Add a start frame, or clear the end frame".to_owned())
    }

    // MARK: Variants

    pub fn variants(&self) -> i64 {
        self.picks.variants.unwrap_or(1)
    }

    pub fn set_variants(&mut self, count: i64) {
        let choices = Self::VARIANT_CHOICES;
        self.picks.variants = Some(count.max(choices.lower()).min(choices.upper()));
    }

    // MARK: Draft persistence

    /// Something the user put into the form: a source, images, a prompt, a style or frames.
    pub fn has_inputs(&self) -> bool {
        self.source.is_some()
            || !self.references.is_empty()
            || !self.prompt.trim().is_empty()
            || self.preset_id.is_some()
            || !self.frames.is_empty()
    }

    /// Whether this form, replacing `previous`, loses an input `previous` had.
    pub fn drops_inputs(&self, previous: &Self) -> bool {
        let had_prompt = !previous.prompt.trim().is_empty();
        (previous.source.is_some() && (self.source != previous.source || self.ranges != previous.ranges))
            || (!previous.references.is_empty() && self.references != previous.references)
            || (had_prompt && self.prompt != previous.prompt)
            || (previous.preset_id.is_some() && self.preset_id != previous.preset_id)
            || (previous.frames.start.is_some() && self.frames.start != previous.frames.start)
            || (previous.frames.end.is_some() && self.frames.end != previous.frames.end)
    }

    /// The local files the form points at.
    pub fn local_files(&self) -> Vec<Url> {
        let mut files: Vec<Url> = self.source.iter().map(|source| source.file_url.clone()).collect();
        files.extend(self.references.iter().map(|image| image.file_url.clone()));
        files.extend(self.frames.images().into_iter().map(|image| image.file_url));
        files.retain(|url| url.scheme() == "file");
        files
    }

    /// The form with its local files that can't be read any more dropped; a dropped source takes its ranges with it.
    /// Files on the account are kept.
    pub fn reopening_files(&self, readable: &dyn Fn(&Url) -> bool) -> Self {
        let keeps = |url: &Url| url.scheme() != "file" || readable(url);
        let mut form = self.clone();
        if let Some(source) = &self.source
            && !keeps(&source.file_url)
        {
            form.set_source(None);
        }
        form.references.retain(|image| keeps(&image.file_url));
        form.frames.start = self.frames.start.clone().filter(|image| keeps(&image.file_url));
        form.frames.end = self.frames.end.clone().filter(|image| keeps(&image.file_url));
        form.batch = self.batch.reopening_sets(&keeps);
        form
    }
}

fn trunc(value: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation, reason = "a few seconds")]
    let whole = value.trunc() as i64;
    whole
}

/// The mode switch label.
pub fn mode_short_title(mode: Mode) -> &'static str {
    match mode {
        Mode::MotionTransfer => "Motion",
        Mode::Restyle => "Restyle",
        Mode::ObjectSwap => "Swap",
        Mode::Edit => "Edit",
        Mode::Extend => "Extend",
    }
}

pub fn mode_title(mode: Mode) -> &'static str {
    match mode {
        Mode::MotionTransfer => "Motion transfer",
        Mode::Restyle => "Restyle",
        Mode::ObjectSwap => "Object swap",
        Mode::Edit => "Edit",
        Mode::Extend => "Extend",
    }
}

pub fn mode_tagline(mode: Mode) -> &'static str {
    match mode {
        Mode::MotionTransfer => "Move your characters, products or clothes with motion from any clip.",
        Mode::Restyle => "Redraw the whole clip in one of the catalog styles.",
        Mode::ObjectSwap => "Replace an object in the clip with your own item.",
        Mode::Edit => "Change a clip as the prompt says, with Seedance 2.5.",
        Mode::Extend => "Continue a clip past its last frame, with Seedance 2.5.",
    }
}

pub fn mode_references_title(mode: Mode) -> &'static str {
    match mode {
        Mode::MotionTransfer => "Characters",
        Mode::Restyle => "Character refs",
        Mode::ObjectSwap => "Replacement objects",
        Mode::Edit | Mode::Extend => "References",
    }
}

pub fn mode_references_hint(mode: Mode) -> &'static str {
    match mode {
        Mode::MotionTransfer => "Characters, products or clothes",
        Mode::Restyle => "Optional, keeps faces consistent",
        Mode::ObjectSwap => "The item to put in the clip",
        Mode::Edit | Mode::Extend => "Optional, for faces, products or a look",
    }
}

pub fn mode_prompt_hint(mode: Mode) -> &'static str {
    match mode {
        Mode::MotionTransfer => "Say who moves, e.g. “Image 1 dances on a rooftop at night”",
        Mode::Restyle => "Refine the look, e.g. “warm sunset light, film grain”",
        Mode::ObjectSwap => {
            "Say what changes, e.g. “Replace the car with Image 1, keep the rest of the shot”"
        }
        Mode::Edit => "What should change? e.g. “Make it night, with rain on the street”",
        Mode::Extend => "What happens next? e.g. “She turns around and walks to the door”",
    }
}

pub fn audio_title(audio: AudioOption) -> &'static str {
    match audio {
        AudioOption::Original => "Original",
        AudioOption::Mute => "Mute",
        AudioOption::FadeOut => "Fade out",
        AudioOption::Generated => "Generated",
    }
}

pub fn bitrate_title(bitrate: SeedanceBitrate) -> &'static str {
    match bitrate {
        SeedanceBitrate::Standard => "Standard",
        SeedanceBitrate::High => "High bitrate",
    }
}

pub fn orientation_title(orientation: CharacterOrientation) -> &'static str {
    match orientation {
        CharacterOrientation::Image => "As in image",
        CharacterOrientation::Video => "As in video",
    }
}

// MARK: Templates (`JobTemplates.swift`)

impl CreateForm {
    /// The form's inputs as a template; the source and ranges are left out.
    pub fn make_template(
        &self,
        name: &str,
        id: Uuid,
        created_at: time::OffsetDateTime,
    ) -> jutsu_core::JobTemplate {
        let uses_frames = self.uses_frames();
        let mut template = jutsu_core::JobTemplate::new(
            id,
            name,
            created_at,
            self.mode,
            self.selected_model(),
            self.resolution,
        );
        template.references = if uses_frames {
            self.frames.images()
        } else {
            self.references.clone()
        };
        template.preset_id = self.preset_id;
        template.prompt.clone_from(&self.prompt);
        template.audio = self.job_audio();
        template.bitrate = self.job_bitrate();
        template.character_orientation = self.job_character_orientation();
        template.cinema_controls = self.job_cinema_controls();
        template.references_are_frames = uses_frames;
        template.normalize();
        template
    }

    /// Fills the inputs from `template`. A loaded source and its ranges stay, so only the video is left to add, cut
    /// to the model's limits. A model outside `offered` runs on Genjutsu, and the notice says so.
    pub fn apply_template(
        &mut self,
        template: &jutsu_core::JobTemplate,
        offered: &BTreeSet<VideoModel>,
    ) -> Option<Notice> {
        self.set_mode(template.mode);
        let runs_on_model = offered.contains(&template.model)
            && !self
                .unserved_modes
                .get(&template.model)
                .is_some_and(|modes| modes.contains(&template.mode));
        self.set_model(if runs_on_model {
            template.model.clone()
        } else {
            VideoModel::GENJUTSU
        });
        if template.references_are_frames && self.model.declares(VideoControl::Frames) {
            self.references = Vec::new();
            self.frames = FramePair::from_images(&template.references);
        } else {
            let most = self.limits().reference_images.upper();
            self.references = template.references.iter().take(most).cloned().collect();
            self.frames = FramePair::default();
        }
        if template.mode.limits().requires_preset {
            self.preset_id = template.preset_id;
        }
        self.prompt.clone_from(&template.prompt);
        self.resolution = template.resolution;
        self.audio = template.audio;
        if let Some(bitrate) = template.bitrate {
            self.picks.bitrate = Some(bitrate);
        }
        if let Some(orientation) = template.character_orientation {
            self.picks.orientation = Some(orientation);
        }
        if self.model.declares(VideoControl::CinemaControls) {
            self.set_cinema_controls(template.cinema_controls.clone());
        }
        if runs_on_model {
            return None;
        }
        Some(Notice {
            title: "Running on Genjutsu".to_owned(),
            detail: format!(
                "\"{}\" uses {}, which this connection doesn't offer.",
                template.name,
                template.model.title()
            ),
        })
    }
}

// MARK: Batch (`CreateForm+Batch.swift`)

impl CreateForm {
    /// A batch compares at least two results and runs at most twelve jobs, so one click can't queue an unbounded
    /// bill.
    pub const BATCH_SIZES: ClosedRange<usize> = ClosedRange::new(2, 12);

    /// A batch compares styles or characters; Edit and Extend have neither.
    pub fn offers_batch(&self) -> bool {
        !self.mode.is_video_operation()
    }

    pub fn is_batch(&self) -> bool {
        self.batch.is_on
    }

    /// Turning the batch on starts the list from the single preset or image set the form has.
    pub fn set_batch(&mut self, is_on: bool) {
        let was_on = self.batch.is_on;
        self.batch.is_on = is_on;
        if !is_on || was_on {
            return;
        }
        if self.batch.preset_ids.is_empty()
            && let Some(preset) = self.preset_id
        {
            self.batch.preset_ids = vec![preset];
        }
        if self.batch.character_sets.is_empty() && self.mode != Mode::Restyle && !self.references.is_empty() {
            self.batch.character_sets = vec![self.references.clone()];
        }
    }

    /// A batch runs as drafts by default, on a model that has them.
    pub fn runs_as_draft(&self) -> bool {
        self.is_batch() && self.offers_draft()
    }

    /// Restyle batch: the styles in the order picked.
    pub fn batch_preset_ids(&self) -> &[Uuid] {
        &self.batch.preset_ids
    }

    /// Motion transfer and object swap batch: one set of reference images per character.
    pub fn character_sets(&self) -> &[Vec<LocalImage>] {
        &self.batch.character_sets
    }

    /// How many jobs the batch makes: one per style for restyle, one per character set otherwise.
    pub fn batch_count(&self) -> usize {
        self.batch.count(self.mode)
    }

    /// "3 / 12".
    pub fn batch_count_text(&self) -> String {
        format!("{} / {}", self.batch_count(), Self::BATCH_SIZES.upper())
    }

    pub fn can_add_batch_variant(&self) -> bool {
        self.batch_count() < Self::BATCH_SIZES.upper()
    }

    /// Next to the batch switch.
    pub fn batch_hint(&self) -> String {
        let most = Self::BATCH_SIZES.upper();
        if self.mode == Mode::Restyle {
            format!("One range × up to {most} styles, side by side")
        } else {
            format!("One range × up to {most} characters, side by side")
        }
    }

    /// "A batch takes up to 12 styles" or "… characters".
    fn batch_full_text(&self) -> String {
        let most = Self::BATCH_SIZES.upper();
        if self.mode == Mode::Restyle {
            format!("A batch takes up to {most} styles")
        } else {
            format!("A batch takes up to {most} characters")
        }
    }

    /// Shown once the batch is full, so a style or set that doesn't get added isn't a mystery.
    pub fn batch_cap_notice(&self) -> Option<String> {
        if !self.is_batch() || self.can_add_batch_variant() {
            return None;
        }
        Some(format!("{}. Remove one to add another.", self.batch_full_text()))
    }

    /// "Draft 480p × 4" in a batch, else "Draft 480p".
    pub fn draft_button_title(&self) -> String {
        if self.is_batch() {
            format!("{} × {}", self.draft_title(), self.batch_count())
        } else {
            self.draft_title()
        }
    }

    /// "Generate × 4" in a batch, else "Generate".
    pub fn generate_button_title(&self) -> String {
        if self.is_batch() {
            format!("Generate × {}", self.batch_count())
        } else {
            "Generate".to_owned()
        }
    }

    /// Adds the style to a restyle batch, or takes it out. Returns `false` when the batch is full and the style
    /// isn't in it, which leaves the batch as it was.
    pub fn toggle_batch_preset(&mut self, id: Uuid) -> bool {
        if let Some(index) = self.batch.preset_ids.iter().position(|preset| *preset == id) {
            self.batch.preset_ids.remove(index);
            return true;
        }
        if self.batch.preset_ids.len() >= Self::BATCH_SIZES.upper() {
            return false;
        }
        self.batch.preset_ids.push(id);
        true
    }

    /// Adds one character from `images`, in order and without repeats, up to the mode's image limit. Returns what
    /// the user has to know: the batch is full, or some images didn't fit the set.
    pub fn add_character_set(&mut self, images: &[LocalImage]) -> Option<Notice> {
        let mut distinct: Vec<LocalImage> = Vec::new();
        for image in images {
            if !distinct.iter().any(|kept| kept.file_url == image.file_url) {
                distinct.push(image.clone());
            }
        }
        if distinct.is_empty() {
            return None;
        }
        if !self.can_add_batch_variant() {
            return Some(Notice {
                title: self.batch_full_text(),
                detail: "Remove a character to add another.".to_owned(),
            });
        }
        let limit = self.limits().reference_images.upper();
        let left = distinct.len().saturating_sub(limit);
        distinct.truncate(limit);
        self.batch.character_sets.push(distinct);
        if left == 0 {
            return None;
        }
        Some(Notice {
            title: if left == 1 {
                "1 image wasn't added".to_owned()
            } else {
                format!("{left} images weren't added")
            },
            detail: if limit == 1 {
                "Each character takes one reference image.".to_owned()
            } else {
                format!("Each character takes up to {limit} reference images.")
            },
        })
    }

    /// Removes a set; it stays published only while an equal set is still in the batch.
    pub fn remove_character_set(&mut self, index: usize) {
        self.batch.remove_character_set(index);
    }

    /// The element the set at `index` was published as on the account `owner`; `None` when it wasn't.
    pub fn saved_element(&self, index: usize, owner: Option<&str>) -> Option<&super::elements::SavedElement> {
        self.batch.saved_element(index, owner)
    }

    /// Records that the set of `images` was published as `element`.
    pub fn set_saved_element(&mut self, element: super::elements::SavedElement, images: &[LocalImage]) {
        self.batch.set_saved_element(element, images);
    }

    /// `base` once per style or character set, in the order they were added.
    fn batch_specs(&self, base: &JobSpec) -> Vec<JobSpec> {
        if self.mode == Mode::Restyle {
            return self
                .batch
                .preset_ids
                .iter()
                .map(|id| {
                    let mut spec = base.clone();
                    spec.preset_id = Some(*id);
                    spec
                })
                .collect();
        }
        self.batch
            .character_sets
            .iter()
            .map(|images| {
                let mut spec = base.clone();
                spec.references.clone_from(images);
                // The media IDs held for an account source belong to its own images, not to another character's.
                spec.remote_inputs = self.reusable_inputs_for(images);
                spec
            })
            .collect()
    }

    /// Why the batch can't be queued, before each job's own checks; `None` when it can or isn't a batch.
    pub fn batch_blocker(&self) -> Option<String> {
        if !self.is_batch() {
            return None;
        }
        if self.has_several_ranges() {
            return Some("A batch uses one range, remove the others".to_owned());
        }
        let least = Self::BATCH_SIZES.lower();
        if self.batch_count() < least {
            return Some(if self.mode == Mode::Restyle {
                format!("Pick at least {least} styles")
            } else {
                format!("Add at least {least} characters")
            });
        }
        if self.batch_count() > Self::BATCH_SIZES.upper() {
            return Some(self.batch_full_text());
        }
        None
    }
}

// Inputs set as given, never cut to the limits: an agent's order is refused by Validation where the form would fit it
// (`AgentVideoOrder.swift`).
impl CreateForm {
    /// Replaces the ranges as given and selects the first; nothing moves them inside the model's window or the clip.
    pub fn set_ranges_unclamped(&mut self, ranges: Vec<TimeRange>) {
        self.ranges = ranges;
        self.selected_range_index = 0;
    }

    /// Replaces the references as given; nothing drops those over the model's limit.
    pub fn set_references_unclamped(&mut self, images: Vec<LocalImage>) {
        self.references = images;
    }
}
