//! A video an agent asks for, with its files already in the app: what the app makes of a `quote` or `create_video`
//! (`AgentVideoOrder.swift`, `AgentVideoOrder+Variants.swift`). It becomes a job through a `CreateForm` of its own,
//! never the Create screen's draft, so it is built and checked exactly as the person's own job would be (`spec`), and
//! refused where the form would quietly fit it: a length outside the model's window, a reference too many, a
//! resolution or draft the model doesn't have.

use jutsu_core::{
    AspectRatio, AudioOption, Capabilities, JobSpec, LocalAudio, LocalImage, Mode, ModelConstraintTable,
    PixelSize, Resolution, Validation, ValidationIssue, VideoModel,
};
use url::Url;
use uuid::Uuid;

use super::job::{mode_name, mode_named};
use crate::create::context::CreateContext;
use crate::create::form::{CreateForm, SourceVideo, time_range};

/// The source clip, probed (`SourceVideo`); for a quote, a stand-in as long as the range's end.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderSource {
    pub video: SourceVideo,
    pub start: Option<f64>,
    pub end: Option<f64>,
}

/// One `quote` or `create_video`, its files read.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentVideoOrder {
    /// `motion_transfer`, `restyle`, `object_swap`, `edit`, `extend`.
    pub mode: String,
    pub model: Option<String>,
    pub prompt: String,
    pub source: Option<OrderSource>,
    pub images: Vec<LocalImage>,
    pub style: Option<Uuid>,
    pub resolution: Option<String>,
    pub is_draft: bool,
    pub length: Option<f64>,
    pub aspect_ratio: Option<String>,
    pub audio: Option<String>,
    pub variants: Option<i64>,
    /// The job's name in lists, as the agent sent it; the job keeps it trimmed and cut (`JobSpec::title_from`).
    pub title: Option<String>,
    /// The recording the characters say, read as Create reads a dropped one; for a quote, a stand-in as long as
    /// `speech_length`.
    pub speech: Option<LocalAudio>,
}

/// How many videos one job of a model and mode makes on this connection, and why only one: what `capabilities` and
/// `quote` say before `create_video` refuses `variants`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentVariants {
    /// The most `variants` takes.
    pub maximum: i64,
    /// Why it is one; `None` when more are offered.
    pub reason: Option<String>,
}

/// The same rules as Create's variants stepper (`CreateForm::variants_unavailable_reason`), in English for agents.
/// `capabilities` are the connection's as last read, `None` while not known; `unlimited_opt_in` is the person's Use
/// unlimited generations.
pub fn variants(
    model: &VideoModel,
    mode: Mode,
    capabilities: Option<&Capabilities>,
    unlimited_opt_in: bool,
) -> AgentVariants {
    let one = |reason: &str| AgentVariants {
        maximum: 1,
        reason: Some(reason.to_owned()),
    };
    let Some(capabilities) = capabilities else {
        return one("The connection's limits aren't read yet; quote a video first, then ask again.");
    };
    if capabilities.max_variants <= 1 {
        return one("This connection makes one video per job.");
    }
    if mode == Mode::Restyle {
        return one("A style makes one video per run.");
    }
    if unlimited_opt_in
        && capabilities
            .unlimited
            .get(model)
            .is_some_and(|modes| modes.contains(&mode))
    {
        // The engine pays from the allowance whenever the person opted in and it covers the job; nothing sends one
        // job with credits instead.
        return one(
            "The plan's unlimited generations pay for this model and mode, and they make one video per job. \
             Variants paid with credits need the person to turn off Use unlimited generations in Settings; agents \
             can't choose credits for one job.",
        );
    }
    AgentVariants {
        maximum: capabilities.max_variants,
        reason: None,
    }
}

/// The agent's names of the audio options.
const AUDIO_OPTIONS: [(&str, AudioOption); 4] = [
    ("original", AudioOption::Original),
    ("mute", AudioOption::Mute),
    ("fade_out", AudioOption::FadeOut),
    ("generated", AudioOption::Generated),
];

/// The agent's name of an audio option (`AgentVideoOrder.audioOptions`).
pub fn audio_name(option: AudioOption) -> Option<&'static str> {
    AUDIO_OPTIONS
        .iter()
        .find(|(_, value)| *value == option)
        .map(|(name, _)| *name)
}

const MODES: [Mode; 5] = [
    Mode::MotionTransfer,
    Mode::Restyle,
    Mode::ObjectSwap,
    Mode::Edit,
    Mode::Extend,
];

/// A stand-in file of a quote; only its length and frame matter to a price.
fn quote_url(name: &str) -> Url {
    match Url::parse(&format!("file:///quote/{name}")) {
        Ok(url) => url,
        Err(error) => unreachable!("a constant file URL parses: {error}"),
    }
}

impl AgentVideoOrder {
    /// An order in `mode` with nothing else set.
    pub fn new(mode: &str) -> Self {
        Self {
            mode: mode.to_owned(),
            model: None,
            prompt: String::new(),
            source: None,
            images: Vec::new(),
            style: None,
            resolution: None,
            is_draft: false,
            length: None,
            aspect_ratio: None,
            audio: None,
            variants: None,
            title: None,
            speech: None,
        }
    }

    /// A quote's stand-in for a source of `end` seconds: only its length and frame matter to a price.
    pub fn quote_source(start: Option<f64>, end: f64) -> OrderSource {
        OrderSource {
            video: SourceVideo {
                file_url: quote_url("source.mp4"),
                duration: end,
                pixel_size: PixelSize::new(1280, 720),
                codec: "avc1".to_owned(),
                file_size: 0,
                remote_origin: None,
            },
            start,
            end: Some(end),
        }
    }

    /// A quote's stand-ins for `count` reference images.
    pub fn quote_images(count: usize) -> Vec<LocalImage> {
        (0..count)
            .map(|index| LocalImage::new(quote_url(&format!("image-{index}.png"))))
            .collect()
    }

    /// A quote's stand-in for a speech recording of `seconds`: only its length matters to a price.
    pub fn quote_speech(seconds: f64) -> LocalAudio {
        LocalAudio {
            file_url: quote_url("speech.m4a"),
            duration: seconds,
        }
    }

    /// The frame `spec`'s result gets, as `quote` says it: `source` when the job has one (its result keeps the
    /// source's orientation, Seedance asking for the source's nearest ratio), else the ratio the job asks for.
    pub fn frame(spec: &JobSpec) -> Option<String> {
        if spec.source.is_none() {
            return spec.aspect_ratio.map(|ratio| ratio.raw_value().to_owned());
        }
        Some(
            if chooses_own_frame(&spec.model) {
                "model"
            } else {
                "source"
            }
            .to_owned(),
        )
    }

    /// What `quote` says about the frame of a model that picks it itself; `None` for the others.
    pub fn frame_note(spec: &JobSpec) -> Option<String> {
        if spec.source.is_none() || !chooses_own_frame(&spec.model) {
            return None;
        }
        Some(format!(
            "{} takes no aspect ratio and picks the result's frame itself: it usually follows the source, but a 3:4 \
             result from a 9:16 source has been seen. jobs gives the result's width and height once it is done, \
             with frameNote when they aren't the source's shape; check them before scaling it.",
            spec.model.title()
        ))
    }

    /// The one job this order makes, or why not, in English for the agent.
    /// - `base`: a fresh form with the person's defaults, never the Create draft.
    /// - `context`: the connection, its capabilities and prices, as Create sees them.
    /// - `constraints`: the server's model limits the app holds; `None` keeps the built-in ones.
    pub fn spec(
        &self,
        base: CreateForm,
        context: &CreateContext,
        constraints: Option<ModelConstraintTable>,
    ) -> Result<JobSpec, String> {
        let Some(mode) = mode_named(&self.mode) else {
            return Err(format!("mode is one of {}.", sorted_names(MODES.map(mode_name))));
        };
        if !context.is_connected() {
            return Err(
                "Jutsu Studio isn't connected to Higgsfield; the person connects it in Settings.".to_owned(),
            );
        }
        let model = self.pick_model(mode)?;
        check_offered(&model, mode, context.capabilities.as_ref())?;
        if let Some(reason) = context.unavailable_reason(mode) {
            return Err(reason);
        }
        let table = constraints.clone().unwrap_or_else(ModelConstraintTable::built_in);
        let mut form = base;
        form.set_constraints(constraints);
        form.set_batch(false);
        form.set_mode(mode);
        form.set_model(model.clone());
        if form.mode() != mode || form.selected_model() != model {
            return Err(format!("{} can't run {}.", model.title(), self.mode));
        }
        if self.is_draft && !model.offers_draft() {
            return Err(format!("{} has no drafts; leave draft out.", model.title()));
        }
        if let Some(resolution) = self.pick_resolution(&model, mode)? {
            if self.is_draft {
                form.set_draft_resolution(resolution);
            } else {
                form.set_resolution(resolution);
            }
        }
        self.place(&mut form)?;
        // Set as given, never cut to the limits: Validation names what is over.
        form.set_references_unclamped(self.images.clone());
        form.prompt.clone_from(&self.prompt);
        form.set_preset_id(self.style);
        self.set_length(&mut form)?;
        self.set_aspect_ratio(&mut form)?;
        self.set_audio(&mut form)?;
        self.set_speech(&mut form)?;
        self.set_variants(&mut form, context)?;
        let specs = form.specs_in(self.is_draft, context);
        let Ok([mut spec]) = <[JobSpec; 1]>::try_from(specs) else {
            return Err(if form.source().is_none() {
                format!("{} needs a source clip in {}.", model.title(), self.mode)
            } else {
                "The request makes no job.".to_owned()
            });
        };
        if let Some(issue) = Validation::check(&spec, &table).first() {
            return Err(issue_text(issue, &spec, &table));
        }
        spec.title = JobSpec::title_from(self.title.as_deref());
        Ok(spec)
    }

    fn pick_model(&self, mode: Mode) -> Result<VideoModel, String> {
        let Some(id) = &self.model else {
            return Ok(mode.default_model());
        };
        let Some(model) = VideoModel::from_raw(id) else {
            return Err(format!(
                "There's no model {id}; capabilities with models true lists them."
            ));
        };
        if !model.serves(mode) {
            let served: Vec<&str> = model.modes().into_iter().map(mode_name).collect();
            return Err(format!(
                "{} doesn't do {}; it does {}.",
                model.title(),
                self.mode,
                served.join(", ")
            ));
        }
        Ok(model)
    }

    /// The resolution asked for, if the model renders at it in `mode` (`VideoModel::resolutions_for`).
    fn pick_resolution(&self, model: &VideoModel, mode: Mode) -> Result<Option<Resolution>, String> {
        let Some(name) = &self.resolution else {
            return Ok(None);
        };
        let offered = model.resolutions_for(mode);
        match Resolution::from_raw(name) {
            Some(resolution) if offered.contains(&resolution) => Ok(Some(resolution)),
            _ => {
                let names: Vec<&str> = offered.iter().map(|resolution| resolution.raw_value()).collect();
                let place = if offered == model.resolutions() {
                    String::new()
                } else {
                    format!(" in {}", self.mode)
                };
                Err(format!(
                    "{} renders at {}{place}, not {name}.",
                    model.title(),
                    names.join(", ")
                ))
            }
        }
    }

    /// The source and its one range, as given: a range past the clip or outside the model's window is refused by
    /// Validation rather than moved.
    fn place(&self, form: &mut CreateForm) -> Result<(), String> {
        let Some(source) = &self.source else {
            return Ok(());
        };
        let video = &source.video;
        if !(video.duration.is_finite() && video.duration > 0.0) {
            return Err("The source clip has no length.".to_owned());
        }
        form.set_source(Some(video.clone()));
        let start = source.start.unwrap_or(0.0);
        let end = source
            .end
            .unwrap_or_else(|| video.duration.min(start + form.limits().duration.upper()));
        if start >= end {
            return Err("source.end comes after source.start.".to_owned());
        }
        if end > video.duration + 0.05 {
            return Err(format!(
                "source.end is past the clip, which runs {} seconds.",
                seconds(video.duration)
            ));
        }
        form.set_ranges_unclamped(vec![time_range(start, end.min(video.duration))]);
        // The API takes no smaller frame for a swap; the agent asked for one, so the frame is upscaled as the person
        // would allow it.
        if form.is_frame_too_small() {
            form.allows_upscale = true;
        }
        Ok(())
    }

    fn set_aspect_ratio(&self, form: &mut CreateForm) -> Result<(), String> {
        let Some(name) = &self.aspect_ratio else {
            return Ok(());
        };
        let Some(ratio) = AspectRatio::from_raw(name) else {
            let names: Vec<&str> = AspectRatio::ALL.iter().map(|ratio| ratio.raw_value()).collect();
            return Err(format!("aspect_ratio is one of {}.", names.join(", ")));
        };
        if !form.is_without_source() {
            return Err("aspect_ratio is only for a clip without a source.".to_owned());
        }
        form.aspect_ratio = ratio;
        Ok(())
    }

    fn set_variants(&self, form: &mut CreateForm, context: &CreateContext) -> Result<(), String> {
        let Some(variants) = self.variants.filter(|variants| *variants != 1) else {
            return Ok(());
        };
        if !CreateForm::VARIANT_CHOICES.contains(variants) {
            return Err(format!(
                "variants is 1 to {}.",
                CreateForm::VARIANT_CHOICES.upper()
            ));
        }
        form.set_variants(variants);
        if form.variants_in(context) != variants {
            let most = context
                .capabilities
                .as_ref()
                .map_or(1, |capabilities| capabilities.max_variants);
            return Err(form.variants_unavailable_reason(context).map_or_else(
                || format!("This connection makes at most {most} videos per job."),
                |reason| format!("{reason}."),
            ));
        }
        Ok(())
    }
}

// The length and the sound: the clip's length, the audio option, and the speech of a model whose characters say a
// recording, which sets the length.
impl AgentVideoOrder {
    fn set_length(&self, form: &mut CreateForm) -> Result<(), String> {
        let Some(length) = self.length else {
            return Ok(());
        };
        if form.offers_speech() {
            return Err(format!(
                "{} runs as long as the speech; leave length out.",
                form.selected_model().title()
            ));
        }
        form.length = length;
        let window = form.limits().duration;
        if !(form.is_without_source() || form.mode() == Mode::Extend) {
            return Err(
                "length is only for a clip without a source, or extend; the source range sets it.".to_owned(),
            );
        }
        if !window.contains(length) {
            return Err(format!(
                "length is {} to {} seconds on {}.",
                seconds(window.lower()),
                seconds(window.upper()),
                form.selected_model().title()
            ));
        }
        Ok(())
    }

    fn set_audio(&self, form: &mut CreateForm) -> Result<(), String> {
        let Some(name) = &self.audio else {
            return Ok(());
        };
        let Some(option) = AUDIO_OPTIONS
            .iter()
            .find(|(option_name, _)| option_name == name)
            .map(|(_, option)| *option)
        else {
            return Err(format!(
                "audio is one of {}.",
                sorted_names(AUDIO_OPTIONS.map(|(name, _)| name))
            ));
        };
        if option == AudioOption::Generated && !form.offers_generated_audio() {
            return Err(format!(
                "{} can't make the sound here; leave audio out or keep original.",
                form.selected_model().title()
            ));
        }
        form.audio = option;
        Ok(())
    }

    fn set_speech(&self, form: &mut CreateForm) -> Result<(), String> {
        let Some(speech) = &self.speech else {
            return Ok(());
        };
        if !form.offers_speech() {
            return Err(format!(
                "{} takes no speech; leave speech out.",
                form.selected_model().title()
            ));
        }
        form.set_speech(Some(speech.clone()));
        Ok(())
    }
}

/// Refuses a model the connection doesn't offer, or doesn't run in `mode` (Genjutsu Lite's Object Swap with an API
/// key); `None` capabilities, still loading, refuse nothing.
fn check_offered(model: &VideoModel, mode: Mode, capabilities: Option<&Capabilities>) -> Result<(), String> {
    let Some(capabilities) = capabilities else {
        return Ok(());
    };
    if !capabilities.models.contains(model) {
        return Err(format!("{} isn't available on this connection.", model.title()));
    }
    if !capabilities.runs(model, mode) {
        let runs: Vec<&str> = model
            .modes()
            .into_iter()
            .filter(|mode| capabilities.runs(model, *mode))
            .map(mode_name)
            .collect();
        return Err(format!(
            "{} doesn't do {} on this connection; it does {}.",
            model.title(),
            mode_name(mode),
            runs.join(", ")
        ));
    }
    Ok(())
}

/// Models with a source that Higgsfield sends no aspect ratio to (their catalog lists none) and that were seen to
/// answer in another frame than the source's: Genjutsu Lite (issue #38).
fn chooses_own_frame(model: &VideoModel) -> bool {
    model.raw_value() == "genjutsuLite"
}

fn sorted_names<const N: usize>(mut names: [&str; N]) -> String {
    names.sort_unstable();
    names.join(", ")
}

/// The issue as the agent reads it, with the model's limits.
fn issue_text(issue: &ValidationIssue, spec: &JobSpec, constraints: &ModelConstraintTable) -> String {
    let limits = spec.limits(constraints);
    let model = spec.model.title();
    let window = |duration: f64, verdict: &str| {
        format!(
            "{} seconds is too {verdict}: {model} takes {} to {} seconds.",
            seconds(duration),
            seconds(limits.duration.lower()),
            seconds(limits.duration.upper())
        )
    };
    match issue {
        ValidationIssue::SelectionOutsideSource => "The source range is outside the clip.".to_owned(),
        ValidationIssue::SourceRequired => {
            format!("{model} needs a source clip for {}.", mode_name(spec.mode))
        }
        ValidationIssue::PromptRequired
            if spec.mode.is_video_operation() || spec.model.descriptor().requires_prompt =>
        {
            format!("{model} needs a prompt here.")
        }
        ValidationIssue::PromptRequired => "A clip without a source needs a prompt or an image.".to_owned(),
        ValidationIssue::InvalidFrameSize(_) => "The source clip's frame size can't be read.".to_owned(),
        ValidationIssue::DurationTooShort(duration) => window(*duration, "short"),
        ValidationIssue::DurationTooLong(duration) => window(*duration, "long"),
        ValidationIssue::TooFewReferences { minimum } => {
            format!("{model} needs at least {minimum} image(s) here.")
        }
        ValidationIssue::TooManyReferences { maximum } => {
            format!("{model} takes at most {maximum} image(s) here.")
        }
        ValidationIssue::PresetRequired => "restyle needs style, a style ID.".to_owned(),
        ValidationIssue::FrameTooSmall(_) => "The source frame is below 640×640 for object_swap.".to_owned(),
        ValidationIssue::PromptTooLong { .. } => "The prompt is over 10 000 characters.".to_owned(),
        ValidationIssue::SpeechRequired => {
            format!("{model} needs speech, the recording its characters say.")
        }
        ValidationIssue::SpeechTooShort(speech) | ValidationIssue::SpeechTooLong(speech) => {
            let window = spec
                .model
                .descriptor()
                .speech_seconds
                .unwrap_or_else(|| time_range(0.0, 0.0));
            format!(
                "The speech runs {} seconds: {model} takes {} to {} seconds.",
                seconds(*speech),
                seconds(window.lower()),
                seconds(window.upper())
            )
        }
        ValidationIssue::SourceNotTaken => {
            format!("{model} makes the clip from images and speech; leave source out.")
        }
    }
}

/// "4", "12.5": seconds as an agent reads them, never in the person's locale.
fn seconds(value: f64) -> String {
    if !value.is_finite() {
        return "?".to_owned();
    }
    // Display writes a whole number without ".0".
    format!("{}", (value * 10.0).round() / 10.0)
}

#[cfg(test)]
#[path = "video_order_tests.rs"]
mod tests;
