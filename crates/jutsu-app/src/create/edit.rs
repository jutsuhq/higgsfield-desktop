//! One change the window asks of the form: a pick, a trim drag, a key press. The form's rules decide what it does.

use jutsu_core::{
    AspectRatio, AudioOption, CharacterOrientation, CinemaControl, KlingQuality, Mode, Resolution,
    SeedanceBitrate, VideoModel,
};
use serde::Deserialize;

use super::CreateError;
use super::form::{CreateForm, DraftLength, FrameSlot, TrimPart};

/// An edit of the video form. Values are the raw names the snapshot lists.
#[derive(Debug, Clone, PartialEq, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CreateEdit {
    SetTab {
        mode: String,
    },
    PickTask {
        mode: String,
    },
    SetModel {
        model: String,
    },
    SetResolution {
        resolution: String,
    },
    SetKlingQuality {
        quality: String,
    },
    SetBitrate {
        bitrate: String,
    },
    SetOrientation {
        orientation: String,
    },
    SetKeepsVideoBackground {
        on: bool,
    },
    /// `value` `None` leaves the control to the director.
    SetCinema {
        control: String,
        value: Option<String>,
    },
    SetAudio {
        audio: String,
    },
    SetVariants {
        count: i32,
    },
    /// Extend's added seconds, or a clip made without a source.
    SetLength {
        seconds: f64,
    },
    SetAspectRatio {
        ratio: String,
    },
    SetDraftLength {
        length: DraftLength,
    },
    SetAllowsUpscale {
        allows: bool,
    },
    SetPrompt {
        text: String,
    },
    SetStart {
        time: f64,
    },
    SetEnd {
        time: f64,
    },
    MoveRange {
        start: f64,
    },
    Nudge {
        part: TrimPart,
        steps: i32,
    },
    SelectRange {
        index: i32,
    },
    AddRange,
    RemoveRange {
        index: i32,
    },
    RemoveSource,
    RemoveReference {
        index: i32,
    },
    MoveReference {
        from: i32,
        to: i32,
    },
    /// Moves reference `index` into a frame slot (its role).
    UseAsFrame {
        index: i32,
        slot: FrameSlot,
    },
    ClearFrame {
        slot: FrameSlot,
    },
    /// Restyle: picks the style, or in a batch adds it or takes it out (a full batch leaves it out).
    PickPreset {
        id: String,
    },
    /// The batch switch.
    SetBatch {
        on: bool,
    },
    RemoveCharacterSet {
        index: i32,
    },
    /// Remove on the Speech row.
    RemoveSpeech,
}

fn invalid(what: &str, value: &str) -> CreateError {
    CreateError::Invalid(format!("Unknown {what} “{value}”."))
}

fn index(value: i32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn mode(raw: &str) -> Result<Mode, CreateError> {
    Mode::from_raw(raw).ok_or_else(|| invalid("mode", raw))
}

impl CreateEdit {
    /// Applies the edit to `form`. Fails only for a value the snapshot never lists.
    pub fn apply(self, form: &mut CreateForm) -> Result<(), CreateError> {
        match self {
            Self::SetTab { mode: raw } => form.set_tab(mode(&raw)?),
            Self::PickTask { mode: raw } => form.pick_task(mode(&raw)?),
            Self::SetModel { model } => {
                form.set_model(VideoModel::from_raw(&model).ok_or_else(|| invalid("model", &model))?);
            }
            Self::SetResolution { resolution } => form.set_resolution(
                Resolution::from_raw(&resolution).ok_or_else(|| invalid("resolution", &resolution))?,
            ),
            Self::SetKlingQuality { quality } => form.set_kling_quality(
                KlingQuality::ALL
                    .into_iter()
                    .find(|kling| kling.raw_value() == quality)
                    .ok_or_else(|| invalid("quality", &quality))?,
            ),
            Self::SetBitrate { bitrate } => form.set_bitrate(
                SeedanceBitrate::from_raw(&bitrate).ok_or_else(|| invalid("bitrate", &bitrate))?,
            ),
            Self::SetOrientation { orientation } => form.set_character_orientation(
                CharacterOrientation::from_raw(&orientation)
                    .ok_or_else(|| invalid("orientation", &orientation))?,
            ),
            Self::SetKeepsVideoBackground { on } => form.keeps_video_background = on,
            Self::SetCinema { control, value } => {
                let picked = CinemaControl::from_raw(&control).ok_or_else(|| invalid("control", &control))?;
                form.set_cinema(picked, value.filter(|value| !value.is_empty()));
            }
            Self::SetAudio { audio } => {
                form.audio = AudioOption::ALL
                    .into_iter()
                    .find(|option| super::view::audio_raw(*option) == audio)
                    .ok_or_else(|| invalid("audio", &audio))?;
            }
            Self::SetVariants { count } => form.set_variants(i64::from(count)),
            Self::SetLength { seconds } => {
                if seconds.is_finite() {
                    form.length = seconds;
                }
            }
            Self::SetAspectRatio { ratio } => {
                form.aspect_ratio = AspectRatio::from_raw(&ratio).ok_or_else(|| invalid("frame", &ratio))?;
            }
            Self::SetDraftLength { length } => form.draft_length = length,
            Self::SetAllowsUpscale { allows } => form.allows_upscale = allows,
            Self::SetPrompt { text } => form.prompt = text,
            Self::SetStart { time } => form.set_start(time),
            Self::SetEnd { time } => form.set_end(time),
            Self::MoveRange { start } => form.move_range(start),
            Self::Nudge { part, steps } => form.nudge(part, steps),
            Self::SelectRange { index: at } => form.select_range(index(at)),
            Self::AddRange => form.add_range(),
            Self::RemoveRange { index: at } => form.remove_range(index(at)),
            Self::RemoveSource => form.set_source(None),
            Self::RemoveReference { index: at } => form.remove_reference(index(at)),
            Self::MoveReference { from, to } => form.move_reference(index(from), index(to)),
            Self::UseAsFrame { index: at, slot } => {
                if form.shows_frames()
                    && let Some(image) = form.references().get(index(at)).cloned()
                {
                    form.remove_reference(index(at));
                    form.set_frame(slot, Some(image));
                }
            }
            Self::ClearFrame { slot } => form.set_frame(slot, None),
            Self::PickPreset { id } => {
                let id = uuid::Uuid::parse_str(&id).map_err(|_| invalid("style", &id))?;
                if form.is_batch() {
                    form.toggle_batch_preset(id);
                } else {
                    form.set_preset_id(Some(id));
                }
            }
            Self::SetBatch { on } => form.set_batch(on),
            Self::RemoveCharacterSet { index: at } => form.remove_character_set(index(at)),
            Self::RemoveSpeech => form.set_speech(None),
        }
        Ok(())
    }
}
