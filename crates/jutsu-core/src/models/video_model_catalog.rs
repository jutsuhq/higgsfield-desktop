//! What each video model is, as `models.json` describes it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;

use crate::models::{AspectRatio, Mode, Resolution, decimal_shortest_of};
use crate::resources::MODELS_JSON;
use crate::support::Seconds;
use crate::support::range::ClosedRange;
use crate::support::string_enum;

string_enum! {
    /// A Create control a model shows; any other model hides it.
    pub enum VideoControl {
        /// Draft at a lower resolution.
        Draft = "draft",
        /// A clip from the prompt and images alone, with its length and frame.
        ClipWithoutSource = "clipWithoutSource",
        /// A soundtrack made by the model for a clip with a source, instead of the source's.
        GeneratedAudio = "generatedAudio",
        /// Seedance's `bitrate_mode`.
        Bitrate = "bitrate",
        /// A start and an end frame instead of references, without a source.
        Frames = "frames",
        /// Standard and Pro instead of a resolution (`KlingQuality`).
        Quality = "quality",
        /// Which way the character faces (`CharacterOrientation`).
        CharacterOrientation = "characterOrientation",
        /// Keeps the source video's background.
        VideoBackground = "videoBackground",
        /// Cinema Studio 4.0's camera, lens, genre, era and the rest (`CinemaControls`).
        CinemaControls = "cinemaControls",
        /// A speech recording the characters say (`JobSpec::speech`), which Create and the agents take only here.
        Speech = "speech",
    }
}

string_enum! {
    /// The code that encodes a job's body for the REST endpoint and its `generate_video` arguments. A model whose
    /// request has the same fields as one of these reuses it, so most models are a JSON entry only.
    pub enum RequestShape {
        /// Genjutsu.
        Genjutsu = "genjutsu",
        /// Seedance.
        Seedance = "seedance",
        /// Kling 3.0 Motion Control.
        KlingMotionControl = "klingMotionControl",
        /// Kling O3 from a reference video: the source, the images and the prompt, with the quality as a field.
        KlingVideoReference = "klingVideoReference",
        /// Kling O3's Edit: the source changed by the prompt, with the images and the quality as a field; no
        /// length, the output follows the source.
        KlingVideoEdit = "klingVideoEdit",
        /// Wan's reference-to-video: the source, the images and the prompt, with the length and the resolution,
        /// and the soundtrack for a model that makes one (`GeneratedAudio`). MiniMax H3 sends the same.
        WanReference = "wanReference",
        /// Cinema Studio 4.0 from a source: the source and the images as references and the prompt, with the
        /// length, the resolution and the soundtrack, and the creative controls the job sets; the director picks
        /// the rest.
        CinemaStudio = "cinemaStudio",
        /// Kling 4.0 and 4.0 Flash: from the prompt alone, from a start and an optional end frame (Flash a start
        /// alone), from references (the source and the images), or the source edited as the prompt says, each
        /// with the length and the resolution.
        Kling4 = "kling4",
        /// Gemini Omni Flash 1.2, on the account only: Kling 4.0's clips from the prompt, frames or references,
        /// with its own `mode`s for an Edit of the source and an Extend of it, and no `aspect_ratio` that follows
        /// the source.
        GeminiOmni = "geminiOmni",
        /// Higgsfield Intone, on the account only: one or two characters (the images) say the speech, in the frame
        /// asked for, with the resolution alone; the speech sets the length.
        Intone = "intone",
    }
}

/// The account's estimate of a job's credits, for when Higgsfield can't quote it yet (a model billed by its source
/// before the source is uploaded) or the quote failed: `base` plus `per_second` at the job's resolution for each
/// billed second, rounded up. A mode priced apart from the model's others has its own in `modes`, and a job with a
/// source video the rates of `with_source` where they are set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct VideoModelCredits {
    /// Credits every job costs.
    pub base: Decimal,
    /// Credits per second, by resolution.
    pub per_second: BTreeMap<Resolution, Decimal>,
    /// A mode's own rates.
    pub modes: BTreeMap<Mode, VideoModelCredits>,
    /// `per_second` for a job with a source video, for a model that bills the source's seconds too; empty to
    /// keep `per_second`.
    pub with_source: BTreeMap<Resolution, Decimal>,
}

impl VideoModelCredits {
    /// The rates of `mode`: its own when it has them, else the model's.
    pub fn at(&self, mode: Mode) -> &Self {
        self.modes.get(&mode).unwrap_or(self)
    }

    /// The rates of `mode` for a job with (`has_source`) or without a source video.
    pub fn at_source(&self, mode: Mode, has_source: bool) -> Self {
        let mut credits = self.at(mode).clone();
        if has_source && !credits.with_source.is_empty() {
            credits.per_second = credits.with_source.clone();
        }
        credits
    }
}

/// What one video model is, as `models.json` describes it: the modes it serves, its limits, the Create controls it
/// declares and how its jobs reach the server. `VideoModel` reads it by ID.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoModelDescriptor {
    /// Its ID.
    pub id: String,
    /// The model picker groups by it.
    pub family: String,
    /// Its title.
    pub title: String,
    /// One line under the picker.
    pub detail: String,
    /// In the order `models.json` lists them.
    pub modes: Vec<Mode>,
    /// The resolutions it renders at, in `Resolution`'s order: 480p, 720p and 1080p (`Resolution::STANDARD`) unless
    /// `models.json` lists its own. The picker offers these, and `Pricing` needs a rate for each.
    pub resolutions: Vec<Resolution>,
    /// A mode's own resolutions, where it renders at fewer of `resolutions`; a mode left out has them all.
    pub mode_resolutions: BTreeMap<Mode, Vec<Resolution>>,
    /// The frames a REST body may name, for a model whose schema takes `aspect_ratio` but not the source's own
    /// frame: the body names the one nearest the source. Empty to leave the field out.
    pub aspect_ratios: Vec<AspectRatio>,
    /// Over the mode's own (`Mode::limits`); `None` keeps it.
    pub duration: Option<ClosedRange<Seconds>>,
    /// Over the mode's own; `None` keeps it.
    pub reference_images: Option<ClosedRange<usize>>,
    /// The images a clip from frames takes (`Frames`): a start frame and an optional end frame, or the start alone
    /// (Kling 4.0 Flash, whose `image-to-video` has no end frame).
    pub frame_images: ClosedRange<usize>,
    /// The shorter length window of a job with references, a source or an end frame, over `duration`; Edit keeps
    /// `duration` (`VideoModel::duration`). `None` where the model has one window.
    pub duration_with_references: Option<ClosedRange<Seconds>>,
    /// How long the speech of a model that takes one (`Speech`) may run.
    pub speech_seconds: Option<ClosedRange<Seconds>>,
    /// What the clip adds to the speech: Intone runs the voices and 2.5 s more (`JobSpec::duration`); 0 for every
    /// other model.
    pub speech_tail: Seconds,
    /// The Create controls it shows.
    pub controls: BTreeSet<VideoControl>,
    /// A job needs a prompt even with a source.
    pub requires_prompt: bool,
    /// The list price bills the source video's seconds too (`Pricing::tariff_id`).
    pub bills_source_video: bool,
    /// The list price is what the job costs, rather than an estimate.
    pub list_price_is_exact: bool,
    /// How its requests are encoded.
    pub shape: RequestShape,
    /// REST endpoints by the keys `VideoModel::endpoint_id` tries; empty for a model that runs on the account only
    /// (`VideoModel::runs_with_api_key`).
    pub endpoints: HashMap<String, String>,
    /// The account's `generate_video` model ID by mode; empty for a model that runs with an API key only.
    pub account_models: BTreeMap<Mode, String>,
    /// The account's estimate.
    pub credits: VideoModelCredits,
}

impl VideoModelDescriptor {
    /// A model `models.json` doesn't describe: serves nothing, so nothing runs on it.
    pub fn unknown(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            family: id.to_owned(),
            title: id.to_owned(),
            detail: String::new(),
            modes: Vec::new(),
            resolutions: Resolution::STANDARD.to_vec(),
            mode_resolutions: BTreeMap::new(),
            aspect_ratios: Vec::new(),
            duration: None,
            reference_images: None,
            frame_images: ClosedRange::new(1, 2),
            duration_with_references: None,
            speech_seconds: None,
            speech_tail: 0.0,
            controls: BTreeSet::new(),
            requires_prompt: false,
            bills_source_video: false,
            list_price_is_exact: false,
            shape: RequestShape::Genjutsu,
            endpoints: HashMap::new(),
            account_models: BTreeMap::new(),
            credits: VideoModelCredits::default(),
        }
    }

    /// The images a clip from frames takes.
    pub fn frame_images(&self) -> ClosedRange<usize> {
        self.frame_images
    }

    /// Reads one `models.json` entry. Fails on anything this version can't run exactly as described: an unknown
    /// mode, control, request shape, resolution or frame, an empty list of resolutions or a mode's list outside
    /// them, or a limit that isn't a range.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let file: DescriptorFile =
            serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
        let resolutions = match file.resolutions {
            Some(listed) => {
                let listed = listed
                    .iter()
                    .map(|raw| parse(raw, Resolution::from_raw))
                    .collect::<Result<Vec<_>, _>>()?;
                if listed.is_empty() {
                    return Err("No resolution".to_owned());
                }
                Resolution::ALL
                    .iter()
                    .copied()
                    .filter(|resolution| listed.contains(resolution))
                    .collect()
            }
            None => Resolution::STANDARD.to_vec(),
        };
        let mut mode_resolutions = BTreeMap::new();
        for (key, values) in file.mode_resolutions.unwrap_or_default() {
            let mode = parse(&key, Mode::from_raw)?;
            let listed = values
                .iter()
                .map(|raw| parse(raw, Resolution::from_raw))
                .collect::<Result<Vec<_>, _>>()?;
            if listed.is_empty() || !listed.iter().all(|resolution| resolutions.contains(resolution)) {
                return Err(format!("{key} needs some of the model's resolutions"));
            }
            mode_resolutions.insert(
                mode,
                Resolution::ALL
                    .iter()
                    .copied()
                    .filter(|resolution| listed.contains(resolution))
                    .collect(),
            );
        }
        let aspect_ratios = file
            .aspect_ratios
            .unwrap_or_default()
            .iter()
            .map(|raw| parse(raw, AspectRatio::from_raw))
            .collect::<Result<Vec<_>, _>>()?;
        let limits = file.limits.unwrap_or_default();
        let controls = file
            .controls
            .unwrap_or_default()
            .iter()
            .map(|raw| parse(raw, VideoControl::from_raw))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let mut account_models = BTreeMap::new();
        for (key, id) in file.request.account.unwrap_or_default() {
            account_models.insert(parse(&key, Mode::from_raw)?, id);
        }
        Ok(Self {
            id: file.id,
            family: file.family,
            title: file.title,
            detail: file.detail.unwrap_or_default(),
            modes: file
                .modes
                .iter()
                .map(|raw| parse(raw, Mode::from_raw))
                .collect::<Result<_, _>>()?,
            resolutions,
            mode_resolutions,
            aspect_ratios,
            duration: limits.duration.map(|bounds| range(&bounds)).transpose()?,
            reference_images: limits.reference_images.map(|bounds| range(&bounds)).transpose()?,
            frame_images: limits
                .frames
                .map(|bounds| range(&bounds))
                .transpose()?
                .unwrap_or(ClosedRange::new(1, 2)),
            duration_with_references: limits
                .duration_with_references
                .map(|bounds| range(&bounds))
                .transpose()?,
            speech_seconds: limits.speech.map(|bounds| range(&bounds)).transpose()?,
            speech_tail: limits.speech_tail.unwrap_or(0.0),
            controls,
            requires_prompt: file.requires_prompt.unwrap_or(false),
            bills_source_video: file.bills_source_video.unwrap_or(false),
            list_price_is_exact: file.list_price_is_exact.unwrap_or(false),
            shape: parse(&file.request.shape, RequestShape::from_raw)?,
            endpoints: file.request.endpoints,
            account_models,
            credits: file
                .credits
                .map(CreditsFile::read)
                .transpose()?
                .unwrap_or_default(),
        })
    }
}

fn parse<T>(raw: &str, read: impl Fn(&str) -> Option<T>) -> Result<T, String> {
    read(raw).ok_or_else(|| format!("Unknown \"{raw}\""))
}

fn range<T: Copy + PartialOrd>(bounds: &[T]) -> Result<ClosedRange<T>, String> {
    match bounds {
        [lower, upper] if lower <= upper => Ok(ClosedRange::new(*lower, *upper)),
        _ => Err("Not a range".to_owned()),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescriptorFile {
    id: String,
    family: String,
    title: String,
    detail: Option<String>,
    modes: Vec<String>,
    resolutions: Option<Vec<String>>,
    mode_resolutions: Option<BTreeMap<String, Vec<String>>>,
    aspect_ratios: Option<Vec<String>>,
    limits: Option<LimitsFile>,
    controls: Option<Vec<String>>,
    requires_prompt: Option<bool>,
    bills_source_video: Option<bool>,
    list_price_is_exact: Option<bool>,
    request: RequestFile,
    credits: Option<CreditsFile>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LimitsFile {
    duration: Option<Vec<f64>>,
    reference_images: Option<Vec<usize>>,
    frames: Option<Vec<usize>>,
    duration_with_references: Option<Vec<f64>>,
    speech: Option<Vec<f64>>,
    speech_tail: Option<f64>,
}

#[derive(Deserialize)]
struct RequestFile {
    shape: String,
    endpoints: HashMap<String, String>,
    account: Option<BTreeMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreditsFile {
    base: Option<f64>,
    per_second: BTreeMap<String, f64>,
    modes: Option<BTreeMap<String, CreditsFile>>,
    with_source: Option<BTreeMap<String, f64>>,
}

impl CreditsFile {
    fn read(self) -> Result<VideoModelCredits, String> {
        let rates = |map: BTreeMap<String, f64>| -> Result<BTreeMap<Resolution, Decimal>, String> {
            map.into_iter()
                .map(|(key, rate)| Ok((parse(&key, Resolution::from_raw)?, amount(rate)?)))
                .collect()
        };
        let mut modes = BTreeMap::new();
        for (key, credits) in self.modes.unwrap_or_default() {
            modes.insert(parse(&key, Mode::from_raw)?, credits.read()?);
        }
        Ok(VideoModelCredits {
            base: amount(self.base.unwrap_or(0.0))?,
            per_second: rates(self.per_second)?,
            modes,
            with_source: rates(self.with_source.unwrap_or_default())?,
        })
    }
}

fn amount(value: f64) -> Result<Decimal, String> {
    decimal_shortest_of(value).ok_or_else(|| format!("Not a finite amount: {value}"))
}

/// The video models the app knows, from `models.json`, in the order the picker lists them.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoModelCatalog {
    descriptors: Vec<VideoModelDescriptor>,
    entry_count: usize,
}

static BUNDLED: LazyLock<VideoModelCatalog> = LazyLock::new(|| {
    VideoModelCatalog::from_json(MODELS_JSON).unwrap_or_else(|_| VideoModelCatalog::empty())
});

impl VideoModelCatalog {
    /// Decodes the `models.json` format. An entry this version can't read is left out, so a newer file still loads
    /// the models it can run; so is a second entry with an ID already listed.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        struct File {
            models: Vec<Value>,
        }
        let file: File = serde_json::from_str(json)?;
        let mut seen = BTreeSet::new();
        let descriptors = file
            .models
            .iter()
            .filter_map(|entry| VideoModelDescriptor::from_json(entry).ok())
            .filter(|descriptor| seen.insert(descriptor.id.clone()))
            .collect();
        Ok(Self {
            descriptors,
            entry_count: file.models.len(),
        })
    }

    /// The bundled `models.json`; empty if it can't be read, which the catalog tests rule out.
    pub fn bundled() -> &'static Self {
        &BUNDLED
    }

    fn empty() -> Self {
        Self {
            descriptors: Vec::new(),
            entry_count: 0,
        }
    }

    /// The models, in the file's order.
    pub fn descriptors(&self) -> &[VideoModelDescriptor] {
        &self.descriptors
    }

    /// How many entries the file had, including the ones this version can't read.
    pub fn entry_count(&self) -> usize {
        self.entry_count
    }

    /// The model of ID `id`.
    pub fn get(&self, id: &str) -> Option<&VideoModelDescriptor> {
        self.descriptors.iter().find(|descriptor| descriptor.id == id)
    }
}
