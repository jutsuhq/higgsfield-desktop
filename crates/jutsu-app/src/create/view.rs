//! What the Create screen shows, built in one pass from the form and its context (`CreatePanel`, `CreateStage`,
//! `SubmitBar`): the window renders it and sends edits back; no rule lives in TypeScript.

use std::collections::HashMap;
use std::path::PathBuf;

use jutsu_core::{
    AspectRatio, AudioOption, CharacterOrientation, CinemaControl, KlingQuality, LocalAudio, Mode,
    ModelParameter, SeedanceBitrate,
};
use serde::Serialize;
use url::Url;

use super::context::{CostLine, CreateContext, ModelMenuRow};
use super::form::{
    CreateForm, DraftLength, FrameSlot, TimeRange, TrimPart, audio_title, bitrate_title,
    mode_references_hint, mode_references_title, mode_short_title, mode_tagline, orientation_title,
    seconds_text,
};

/// One entry of a picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub value: String,
    pub title: String,
    /// Listed but not pickable, with why.
    pub unavailable_reason: Option<String>,
}

impl Choice {
    fn new(value: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            title: title.into(),
            unavailable_reason: None,
        }
    }
}

/// A picker: its options, the one picked and what it adds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PickerView {
    pub title: String,
    pub options: Vec<Choice>,
    pub selected: String,
    /// "$0.681 / s", "From the source". A cinema control has its raw value here instead, which `SetCinema` takes.
    pub detail: Option<String>,
    /// The connection can't send it; the control is disabled with this.
    pub unavailable_reason: Option<String>,
    /// The short form of `unavailable_reason`, for the tooltip.
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RangeView {
    pub start: f64,
    pub end: f64,
    /// "00:06.0 – 00:18.5".
    pub text: String,
}

/// How the stage plays the source: the file itself, or a proxy being made.
#[derive(Debug, Clone, Default, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackView {
    /// A file under the app's caches the media server serves; `None` until ready.
    pub path: Option<String>,
    /// While the proxy is made: 0…1.
    pub progress: Option<f64>,
    /// Why it can't be shown.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub name: String,
    /// "186 MB · HEVC".
    pub file_text: String,
    /// "1920×1080".
    pub size_text: String,
    /// "00:06.0 – 00:18.5 · 12.5 s".
    pub summary: String,
    pub duration: f64,
    pub pills: Vec<String>,
    pub playback: PlaybackView,
    /// The trim strip's frames, in order.
    pub thumbnails: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrimView {
    pub ranges: Vec<RangeView>,
    pub selected: i32,
    /// The model's window for one range.
    pub min_length: f64,
    pub max_length: f64,
    /// Parts no range covers.
    pub uncovered: Vec<RangeView>,
    /// "12.5 s selected · billed 13 s".
    pub summary: String,
    pub can_add_range: bool,
    /// The source is on the account: the whole clip is used.
    pub is_locked: bool,
    /// What the handles read aloud: "00:06.0" (`accessibilityValue(of:)`).
    pub start_value: String,
    pub end_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageView {
    pub path: String,
    pub name: String,
    /// A small copy under the caches; `None` until made, or when the file can't be read.
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ReferencesView {
    pub title: String,
    pub hint: String,
    pub items: Vec<ImageView>,
    /// "2 / 8".
    pub count_text: String,
    pub can_add: bool,
    /// Frames are used, so references wait.
    pub disabled_reason: Option<String>,
    /// The frame slots a reference can be moved to (roles), where the model has them.
    pub roles: Vec<FrameSlot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FrameView {
    pub slot: FrameSlot,
    pub title: String,
    pub image: Option<ImageView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FramesView {
    pub slots: Vec<FrameView>,
    pub unavailable_reason: Option<String>,
    /// The section's help: why the connection can't send frames, else "First frame" or "First and last frame".
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptView {
    /// "Prompt" or "Prompt · optional".
    pub title: String,
    pub hint: String,
    /// "120 / 10 000".
    pub counter: String,
    pub text: String,
    pub is_too_long: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LengthView {
    pub title: String,
    /// The length picked: "12 s".
    pub detail: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ToggleView {
    pub title: String,
    pub is_on: bool,
    pub unavailable_reason: Option<String>,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpscaleView {
    pub title: String,
    pub detail: String,
    pub allows: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CinemaView {
    pub controls: Vec<PickerView>,
    /// "2 set", or "Auto" with none set (the director picks them).
    pub summary: String,
    /// One or more controls are set: the summary is lit.
    pub is_set: bool,
    pub unavailable_reason: Option<String>,
    /// The Direction row's help: why the connection can't send them, else what they are.
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VariantsView {
    pub value: i32,
    pub min: i32,
    pub max: i32,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ButtonView {
    /// "Generate", "Draft 480p".
    pub title: String,
    /// "$8.85"; the last known one while the form is quoted again.
    pub price: Option<String>,
    /// Why it can't be pressed.
    pub blocker: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubmitView {
    /// The first thing to fix: Draft's blocker, else Generate's. Shown instead of the cost line.
    pub blocker: Option<String>,
    /// `None` while a blocker shows.
    pub cost_line: Option<CostLine>,
    pub generate: ButtonView,
    /// `None` on a model without drafts.
    pub draft: Option<ButtonView>,
    /// A batch runs as drafts by default: Draft is the main button, which the Generate shortcut presses.
    pub primary_is_draft: bool,
    /// A submit is being queued: both buttons wait.
    pub is_submitting: bool,
}

/// Speech: the recording the characters say, by name and length.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeechView {
    /// "12.5 s"; `None` without a recording.
    pub detail: Option<String>,
    pub name: Option<String>,
}

/// The batch switch and its hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchSwitchView {
    /// "One range × up to 12 styles, side by side".
    pub hint: String,
    pub is_on: bool,
}

/// Where a batch character stands with "Save to Higgsfield…".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SaveState {
    /// No account connection that publishes elements.
    Hidden,
    Available,
    Saving,
    Saved,
}

/// One batch character: its images and whether it is saved on the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSetView {
    /// "Character 2".
    pub title: String,
    /// "3 images".
    pub detail: String,
    /// The first four.
    pub images: Vec<ImageView>,
    pub image_count: i32,
    pub save: SaveState,
    /// What "Save to Higgsfield…" does, for its dialog.
    pub save_explanation: String,
}

/// A motion transfer or object swap batch: one row per character, shown instead of the frames and references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSetsView {
    /// "3 / 12".
    pub detail: String,
    pub sets: Vec<CharacterSetView>,
    /// The batch is full: shown instead of "Add a character".
    pub notice: Option<String>,
}

/// Everything the Create screen's video side shows.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateSnapshot {
    /// Goes up with every change.
    pub revision: i32,
    pub tagline: String,
    pub tabs: Vec<Choice>,
    pub tab: String,
    /// The Motion tab's tasks for the picked model; shown when there are two or more.
    pub tasks: Vec<Choice>,
    pub mode: String,
    pub models: Vec<ModelMenuRow>,
    pub model: String,
    pub source: Option<SourceView>,
    pub trim: Option<TrimView>,
    pub references: ReferencesView,
    pub frames: Option<FramesView>,
    pub prompt: PromptView,
    pub resolution: Option<PickerView>,
    pub kling_quality: Option<PickerView>,
    pub bitrate: Option<PickerView>,
    pub orientation: Option<PickerView>,
    pub video_background: Option<ToggleView>,
    pub cinema: Option<CinemaView>,
    /// From the source, or Generate / Silent where the model makes the sound.
    pub audio: PickerView,
    /// Extend: "Add 4–30 s".
    pub added_length: Option<LengthView>,
    /// A clip made without a source: its length, unless the model runs as long as the speech.
    pub clip_length: Option<LengthView>,
    /// A clip made without a source and without frames: its frame.
    pub aspect_ratio: Option<PickerView>,
    pub draft_length: Option<PickerView>,
    pub upscale: Option<UpscaleView>,
    pub variants: Option<VariantsView>,
    /// Speech, on a model that voices a recording (Intone).
    pub speech: Option<SpeechView>,
    /// Every validation issue of Generate, as actions.
    pub issues: Vec<String>,
    pub submit: SubmitView,
    /// Restyle's style gallery; `None` in the other modes.
    pub presets: Option<super::presets::PresetsView>,
    /// The batch switch, where the mode offers a batch.
    pub batch: Option<BatchSwitchView>,
    /// A motion transfer or swap batch's characters.
    pub character_sets: Option<CharacterSetsView>,
    /// The empty stage shows the connection's Motion Library.
    pub shows_motion_library: bool,
    /// References offers From Higgsfield: the account's uploads and elements.
    pub offers_higgsfield_references: bool,
    /// A Paste URL… download, shown in the section it goes to.
    pub link_download: Option<super::links::LinkDownloadView>,
    /// Under Source video: "4–30 s", or "Optional · 4–30 s" for a model that also makes clips without one.
    pub source_detail: String,
    /// Under Add a video without one: "Or describe the clip in the prompt", or "Any format · trimmed to 4–30 s".
    pub source_hint: String,
}

/// What the service holds besides the form.
#[derive(Debug, Clone, Default)]
pub struct MediaState {
    pub playback: PlaybackView,
    pub thumbnails: Vec<PathBuf>,
    /// Reference and frame thumbnails by file URL.
    pub images: HashMap<Url, PathBuf>,
    pub is_submitting: bool,
    pub revision: u64,
}

fn path_text(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

fn range_view(range: TimeRange) -> RangeView {
    RangeView {
        start: range.lower(),
        end: range.upper(),
        text: format!(
            "{} – {}",
            crate::queue::format::timecode(range.lower()),
            crate::queue::format::timecode(range.upper())
        ),
    }
}

fn image_view(image: &jutsu_core::LocalImage, media: &MediaState) -> ImageView {
    let path = image
        .file_url
        .to_file_path()
        .map_or_else(|()| image.file_url.to_string(), |path| path_text(&path));
    let name = image
        .file_url
        .path_segments()
        .and_then(|mut segments| segments.next_back().map(str::to_owned))
        .map(|name| {
            percent_encoding::percent_decode_str(&name)
                .decode_utf8_lossy()
                .into_owned()
        })
        .unwrap_or_default();
    ImageView {
        path,
        name,
        thumbnail: media.images.get(&image.file_url).map(|path| path_text(path)),
    }
}

fn picker(title: &str, options: Vec<Choice>, selected: impl Into<String>) -> PickerView {
    PickerView {
        title: title.to_owned(),
        options,
        selected: selected.into(),
        detail: None,
        unavailable_reason: None,
        hint: None,
    }
}

fn to_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// The snapshot of `form` in `context`.
#[allow(clippy::too_many_lines, reason = "one field per section of the screen")]
pub fn snapshot(form: &CreateForm, context: &CreateContext, media: &MediaState) -> CreateSnapshot {
    let mode = form.mode();
    let limits = form.limits();
    let tabs = CreateForm::MODE_CHOICES
        .iter()
        .map(|tab| {
            // Motion runs whichever of its tasks the connection has.
            let reason = if *tab == Mode::MotionTransfer && form.tab() == Mode::MotionTransfer {
                context.mode_reason(mode)
            } else {
                context.mode_reason(*tab)
            };
            Choice {
                value: tab.raw_value().to_owned(),
                title: mode_short_title(*tab).to_owned(),
                unavailable_reason: reason,
            }
        })
        .collect();
    let task_modes = form.task_choices(context);
    let tasks = if task_modes.len() > 1 {
        task_modes
            .into_iter()
            .map(|task| Choice::new(task.raw_value(), super::form::mode_title(task)))
            .collect()
    } else {
        Vec::new()
    };

    let source = form.source().map(|source| SourceView {
        name: source.name(),
        file_text: source.file_text(),
        size_text: source.size_text(),
        summary: form.source_summary(),
        duration: source.duration,
        pills: form.stage_pills(),
        playback: media.playback.clone(),
        thumbnails: media.thumbnails.iter().map(|path| path_text(path)).collect(),
    });
    let trim = form.source().map(|_| TrimView {
        ranges: form.ranges().iter().map(|range| range_view(*range)).collect(),
        selected: i32::try_from(form.selected_range_index()).unwrap_or(0),
        min_length: limits.duration.lower(),
        max_length: limits.duration.upper(),
        uncovered: form.uncovered_ranges().into_iter().map(range_view).collect(),
        summary: form.trim_summary(context.connection),
        can_add_range: form.can_add_range(),
        is_locked: form.is_source_remote(),
        start_value: form.accessibility_value(TrimPart::Start),
        end_value: form.accessibility_value(TrimPart::End),
    });

    let references = ReferencesView {
        title: mode_references_title(mode).to_owned(),
        hint: mode_references_hint(mode).to_owned(),
        items: form
            .references()
            .iter()
            .map(|image| image_view(image, media))
            .collect(),
        count_text: form.reference_count_text(),
        can_add: form.can_add_references() && !form.uses_frames(),
        disabled_reason: form.references_disabled_reason(),
        roles: if form.shows_frames() {
            form.frame_slots()
        } else {
            Vec::new()
        },
    };
    let frames = form.shows_frames().then(|| FramesView {
        slots: form
            .frame_slots()
            .into_iter()
            .map(|slot| FrameView {
                slot,
                title: slot.title().to_owned(),
                image: form.frame(slot).map(|image| image_view(image, media)),
            })
            .collect(),
        unavailable_reason: context.parameter_reason(ModelParameter::SeedanceFrames),
        hint: context
            .parameter_hint(ModelParameter::SeedanceFrames)
            .unwrap_or_else(|| {
                if form.frame_slots().len() > 1 {
                    "First and last frame"
                } else {
                    "First frame"
                }
                .to_owned()
            }),
    });
    let prompt = PromptView {
        title: form.prompt_title().to_owned(),
        hint: form.prompt_hint().to_owned(),
        counter: form.prompt_counter_text(),
        text: form.prompt.clone(),
        is_too_long: form.prompt_length() > jutsu_core::Validation::MAXIMUM_PROMPT_LENGTH,
    };

    let model = form.selected_model();
    let resolution = (!form.shows_kling_quality()).then(|| {
        let mut view = picker(
            "Resolution",
            form.resolution_options()
                .into_iter()
                .map(|resolution| Choice::new(resolution.raw_value(), resolution.raw_value()))
                .collect(),
            form.picked_resolution().raw_value(),
        );
        view.detail = form.rate_line(context);
        view
    });
    let kling_quality = form.shows_kling_quality().then(|| {
        let mut view = picker(
            "Quality",
            KlingQuality::ALL
                .iter()
                .map(|quality| {
                    Choice::new(
                        quality.raw_value(),
                        crate::queue::format::resolution_title(quality.resolution(), &model),
                    )
                })
                .collect(),
            form.kling_quality().raw_value(),
        );
        view.detail = form.rate_line(context);
        view
    });
    let bitrate = form.shows_bitrate().then(|| {
        let mut view = picker(
            "Quality",
            SeedanceBitrate::ALL
                .iter()
                .map(|bitrate| Choice::new(bitrate.raw_value(), bitrate_title(*bitrate)))
                .collect(),
            form.bitrate(context.connection).raw_value(),
        );
        view.unavailable_reason = context.parameter_reason(ModelParameter::SeedanceBitrate);
        view.hint = Some(
            context
                .parameter_hint(ModelParameter::SeedanceBitrate)
                .unwrap_or_else(|| "Video bitrate".to_owned()),
        );
        view
    });
    let orientation = form.shows_character_orientation().then(|| {
        let mut view = picker(
            "Character faces",
            CharacterOrientation::ALL
                .iter()
                .map(|orientation| Choice::new(orientation.raw_value(), orientation_title(*orientation)))
                .collect(),
            form.character_orientation().raw_value(),
        );
        view.unavailable_reason = context.parameter_reason(ModelParameter::KlingOrientation);
        view.hint = Some(
            context
                .parameter_hint(ModelParameter::KlingOrientation)
                .unwrap_or_else(|| "Follow the image or video".to_owned()),
        );
        view
    });
    let video_background = form.shows_video_background().then(|| ToggleView {
        title: "Keep the video's background".to_owned(),
        is_on: form.keeps_video_background,
        unavailable_reason: context.parameter_reason(ModelParameter::KlingVideoBackground),
        hint: Some(
            context
                .parameter_hint(ModelParameter::KlingVideoBackground)
                .unwrap_or_else(|| "Use the video's scene".to_owned()),
        ),
    });
    let cinema = form.shows_cinema_controls().then(|| {
        let picked = form.cinema_controls();
        CinemaView {
            controls: CinemaControl::ALL
                .iter()
                .map(|control| {
                    let mut options = vec![Choice::new("", "Auto")];
                    options.extend(
                        control
                            .options()
                            .iter()
                            .map(|option| Choice::new(*option, control.title_of(option))),
                    );
                    let mut view = picker(
                        &control.title(),
                        options,
                        picked.get(*control).unwrap_or_default(),
                    );
                    view.detail = Some(control.raw_value().to_owned());
                    view
                })
                .collect(),
            summary: form
                .cinema_controls_summary()
                .unwrap_or_else(|| "Auto".to_owned()),
            is_set: form.cinema_controls_summary().is_some(),
            unavailable_reason: context.parameter_reason(ModelParameter::CinemaControls),
            hint: context
                .parameter_hint(ModelParameter::CinemaControls)
                .unwrap_or_else(|| "Camera, lens, genre and more".to_owned()),
        }
    });
    // Where the model makes the sound it can be turned off: Generate keeps any choice but Mute.
    let audio = if form.generates_audio() {
        let mut view = picker(
            "Audio",
            vec![
                Choice::new(audio_raw(AudioOption::Original), "Generate"),
                Choice::new(audio_raw(AudioOption::Mute), "Silent"),
            ],
            audio_raw(if form.audio == AudioOption::Mute {
                AudioOption::Mute
            } else {
                AudioOption::Original
            }),
        );
        view.detail = Some(format!("Made by {}", model.title()));
        view
    } else {
        let mut view = picker(
            "Audio",
            form.audio_choices()
                .into_iter()
                .map(|audio| Choice::new(audio_raw(audio), audio_title(audio)))
                .collect(),
            audio_raw(form.audio),
        );
        view.detail = Some(form.audio_detail());
        view
    };
    let added_length = form.shows_added_length().then(|| LengthView {
        title: form.added_length_title(),
        detail: crate::queue::format::limit(form.clip_length()),
        value: form.clip_length(),
        min: limits.duration.lower(),
        max: limits.duration.upper(),
    });
    // A model that voices a speech runs as long as the speech.
    let clip_length = (form.is_without_source() && !form.offers_speech()).then(|| LengthView {
        title: "Length".to_owned(),
        detail: crate::queue::format::limit(form.clip_length()),
        value: form.clip_length(),
        min: limits.duration.lower(),
        max: limits.duration.upper(),
    });
    let aspect_ratio = (form.is_without_source() && !form.uses_frames()).then(|| {
        picker(
            "Frame",
            AspectRatio::ALL
                .iter()
                .map(|ratio| {
                    let title = if *ratio == AspectRatio::Auto {
                        "Auto"
                    } else {
                        ratio.raw_value()
                    };
                    Choice::new(ratio.raw_value(), title)
                })
                .collect(),
            form.aspect_ratio.raw_value(),
        )
    });
    let draft_length = form.shows_draft_length().then(|| {
        let mut view = picker(
            "Draft length",
            DraftLength::ALL
                .iter()
                .map(|length| Choice::new(draft_length_raw(*length), length.title()))
                .collect(),
            draft_length_raw(form.draft_length),
        );
        view.detail = Some(form.draft_length_detail(context));
        view
    });
    let upscale = form.upscale_notice().map(|notice| UpscaleView {
        title: notice.title,
        detail: notice.detail,
        allows: form.allows_upscale,
    });
    let variants = form.shows_variants(context).then(|| VariantsView {
        value: to_i32(form.variants_in(context)),
        min: to_i32(CreateForm::VARIANT_CHOICES.lower()),
        max: to_i32(
            context
                .capabilities
                .as_ref()
                .map_or(1, |capabilities| capabilities.max_variants)
                .min(CreateForm::VARIANT_CHOICES.upper()),
        ),
        unavailable_reason: form.variants_unavailable_reason(context),
    });
    let speech = form.offers_speech().then(|| SpeechView {
        detail: form.speech().map(|audio| seconds_text(audio.duration)),
        name: form.speech().map(LocalAudio::name),
    });

    let generate_specs = form.specs_in(false, context);
    let issues = if form.source().is_some() || form.is_without_source() {
        form.validation_issues(&generate_specs)
    } else {
        Vec::new()
    };
    let generate = ButtonView {
        title: form.generate_button_title(),
        price: form.price_text(false, context),
        blocker: form.blocker(false, context),
    };
    let draft = form.offers_draft().then(|| ButtonView {
        title: form.draft_button_title(),
        price: form.price_text(true, context),
        blocker: form.blocker(true, context),
    });
    let blocker = draft
        .as_ref()
        .and_then(|draft| draft.blocker.clone())
        .or_else(|| generate.blocker.clone());
    let submit = SubmitView {
        cost_line: if blocker.is_none() {
            form.cost_line(context)
        } else {
            None
        },
        blocker,
        generate,
        draft,
        primary_is_draft: form.runs_as_draft(),
        is_submitting: media.is_submitting,
    };

    let batch = form.offers_batch().then(|| BatchSwitchView {
        hint: form.batch_hint(),
        is_on: form.is_batch(),
    });
    let character_sets = (form.is_batch() && mode != Mode::Restyle).then(|| CharacterSetsView {
        detail: form.batch_count_text(),
        sets: form
            .character_sets()
            .iter()
            .enumerate()
            .map(|(index, images)| CharacterSetView {
                title: format!("Character {}", index + 1),
                detail: format!("{} images", images.len()),
                images: images
                    .iter()
                    .take(4)
                    .map(|image| image_view(image, media))
                    .collect(),
                image_count: i32::try_from(images.len()).unwrap_or(i32::MAX),
                save: SaveState::Hidden,
                save_explanation: super::elements::SavedElement::save_explanation(images.len()),
            })
            .collect(),
        notice: form.batch_cap_notice(),
    });

    CreateSnapshot {
        revision: i32::try_from(media.revision % (i32::MAX as u64)).unwrap_or(0),
        tagline: mode_tagline(mode).to_owned(),
        tabs,
        tab: form.tab().raw_value().to_owned(),
        tasks,
        mode: mode.raw_value().to_owned(),
        models: form.model_menu(context),
        model: model.raw_value().to_owned(),
        source,
        trim,
        references,
        frames,
        prompt,
        resolution,
        kling_quality,
        bitrate,
        orientation,
        video_background,
        cinema,
        audio,
        added_length,
        clip_length,
        aspect_ratio,
        draft_length,
        upscale,
        variants,
        speech,
        issues,
        submit,
        presets: None,
        batch,
        character_sets,
        shows_motion_library: false,
        offers_higgsfield_references: false,
        link_download: None,
        source_detail: if model.makes_clips_without_source() {
            "Optional · 4–30 s"
        } else {
            "4–30 s"
        }
        .to_owned(),
        source_hint: if model.makes_clips_without_source() {
            "Or describe the clip in the prompt"
        } else {
            "Any format · trimmed to 4–30 s"
        }
        .to_owned(),
    }
}

pub fn audio_raw(audio: jutsu_core::AudioOption) -> &'static str {
    match audio {
        jutsu_core::AudioOption::Original => "original",
        jutsu_core::AudioOption::Mute => "mute",
        jutsu_core::AudioOption::FadeOut => "fadeOut",
        jutsu_core::AudioOption::Generated => "generated",
    }
}

pub fn draft_length_raw(length: DraftLength) -> &'static str {
    match length {
        DraftLength::WholeRange => "wholeRange",
        DraftLength::FirstFiveSeconds => "firstFiveSeconds",
    }
}
