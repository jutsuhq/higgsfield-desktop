//! The `generate_video` request of each job, and its price.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

use higgsfield::mcp::account::{
    MediaRole, ModelLimits, UnlimitedAllowance, VideoEditInput, VideoEditOperation, VideoEditProvider,
    VideoMedia, VideoRequest,
};
use jutsu_core::{
    AspectRatio, AudioOption, JobSpec, KlingQuality, Mode, ModelConstraintEntry, ModelConstraintTable,
    ModelConstraints, ModelParameter, PixelSize, PostAction, Pricing, ReframeAspect, RemoteInputs,
    RequestShape, Resolution, UpscaleEngine, UpscaleFrameRate, UpscaleOptions, UpscaleTarget, VideoControl,
    VideoModel,
};
use rust_decimal::Decimal;

use crate::api_model;

/// The modes `generate_video` runs; Restyle runs as a Genjutsu Style (`HiggsfieldMcpProvider`).
pub(crate) const GENJUTSU_MODES: [Mode; 2] = [Mode::MotionTransfer, Mode::ObjectSwap];

/// `GENJUTSU_MODES` with Edit and Extend.
pub(crate) fn generate_modes() -> BTreeSet<Mode> {
    let mut modes: BTreeSet<Mode> = GENJUTSU_MODES.into_iter().collect();
    modes.extend([Mode::Edit, Mode::Extend]);
    modes
}

/// What an operation works on: the uploaded source when there is one, else the parent's account job ID; `None`
/// with neither.
pub(crate) fn edit_input(spec: &JobSpec, inputs: &RemoteInputs) -> Option<VideoEditInput> {
    if let Some(media) = inputs.video.as_ref().and_then(|video| video.id.clone()) {
        return Some(VideoEditInput::Media(media));
    }
    spec.operation
        .as_ref()
        .and_then(|operation| operation.source_job_id.clone())
        .map(VideoEditInput::Job)
}

/// The tool call of an operation spec; `None` for a generation, and for one the tool can't take: Topaz at 2K, the
/// standard upscaler without the source's size, a reframe of a result longer than a minute or of unknown length.
pub(crate) fn edit_operation(spec: &JobSpec) -> Option<VideoEditOperation> {
    let operation = spec.operation.as_ref()?;
    match &operation.action {
        PostAction::Upscale(options) => {
            upscale(*options, spec.source.as_ref().map(|source| source.pixel_size))
        }
        PostAction::RemoveBackground => Some(VideoEditOperation::RemoveBackground),
        PostAction::Reframe(aspect) => {
            let length = source_length(spec);
            if !(length > 0.0 && length <= ReframeAspect::MAX_SOURCE_LENGTH) {
                return None;
            }
            // `reframe` takes 480p, 720p or 1080p: a MiniMax result's 768p or 2K, or a Gemini one's 360p or 4K, goes
            // as the nearest.
            Some(VideoEditOperation::Reframe {
                aspect_ratio: aspect.raw_value().to_owned(),
                duration_seconds: length,
                resolution: spec.resolution.standardized().raw_value().to_owned(),
            })
        }
        PostAction::Dub(language) => Some(VideoEditOperation::Dub {
            language: language.raw_value().to_owned(),
        }),
        PostAction::ChangeVoice(voice) => Some(VideoEditOperation::ChangeVoice {
            voice_id: voice.id.clone(),
            voice_type: voice.kind.clone(),
        }),
    }
}

/// The length an operation works on: the source's range, else the parent result's length.
pub(crate) fn source_length(spec: &JobSpec) -> f64 {
    spec.source.as_ref().map_or_else(
        || spec.duration(),
        |source| source.range.upper() - source.range.lower(),
    )
}

/// `upscale_video` per its schema: `bytedance` takes "1080p", "2k" or "4k", needs the source's size and takes a
/// frame rate (left out for the server's 24 fps) and a preset; `topaz` takes "1080p" or "2160p" and none of them.
fn upscale(options: UpscaleOptions, size: Option<PixelSize>) -> Option<VideoEditOperation> {
    match options.engine {
        UpscaleEngine::Standard => {
            let size = size.filter(|size| size.width > 0 && size.height > 0)?;
            Some(VideoEditOperation::Upscale {
                resolution: options.target.raw_value().to_owned(),
                fps: (options.frame_rate != UpscaleFrameRate::Keep).then(|| options.frame_rate.raw_value()),
                provider: VideoEditProvider::Bytedance,
                width: Some(size.width),
                height: Some(size.height),
                preset: Some(options.preset.raw_value().to_owned()),
            })
        }
        UpscaleEngine::Topaz => {
            let resolution = match options.target {
                UpscaleTarget::P1080 => Some("1080p"),
                UpscaleTarget::K4 => Some("2160p"),
                UpscaleTarget::K2 => None,
            };
            resolution.map(|resolution| VideoEditOperation::Upscale {
                resolution: resolution.to_owned(),
                fps: None,
                provider: VideoEditProvider::Topaz,
                width: None,
                height: None,
                preset: None,
            })
        }
    }
}

/// The model parameters the account sends.
pub(crate) fn parameters() -> BTreeSet<ModelParameter> {
    BTreeSet::from([
        ModelParameter::SeedanceBitrate,
        ModelParameter::SeedanceFrames,
        ModelParameter::KlingVideoBackground,
    ])
}

/// `None` for a job the account connection can't run: the catalog names no `generate_video` model for its model
/// and mode (Genjutsu's Restyle runs as a Style), or one this client doesn't know.
pub(crate) fn model(spec: &JobSpec) -> Option<String> {
    if !spec.model.serves(spec.mode) {
        return None;
    }
    spec.model.account_model_id(spec.mode)
}

/// Whether `generate_video` runs `model` in any mode; the others are API key only.
pub(crate) fn runs(model: &VideoModel) -> bool {
    model
        .modes()
        .into_iter()
        .any(|mode| model.account_model_id(mode).is_some())
}

/// Every `generate_video` model the catalog names, once each, in its order: what `models_explore` is asked about
/// for limits and the unlimited allowance.
pub(crate) static ACCOUNT_MODELS: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut seen = HashSet::new();
    VideoModel::all()
        .iter()
        .flat_map(|model| {
            model
                .modes()
                .into_iter()
                .filter_map(|mode| model.account_model_id(mode))
                .collect::<Vec<_>>()
        })
        .filter(|id| seen.insert(id.clone()))
        .collect()
});

/// The modes of each model, among `modes`, whose `generate_video` model `allowance` covers.
pub(crate) fn unlimited(
    allowance: &UnlimitedAllowance,
    modes: &BTreeSet<Mode>,
) -> BTreeMap<VideoModel, BTreeSet<Mode>> {
    let mut unlimited = BTreeMap::new();
    for video_model in VideoModel::all() {
        let covered: BTreeSet<Mode> = modes
            .iter()
            .copied()
            .filter(|mode| {
                let spec = JobSpec::new(*mode, video_model.clone(), None, Resolution::P720);
                model(&spec).is_some_and(|id| allowance.covers(&id))
            })
            .collect();
        if !covered.is_empty() {
            unlimited.insert(video_model, covered);
        }
    }
    unlimited
}

/// Seedance's `aspect_ratio` outside frames, edit and extend: without a source the one asked for, or `auto`; with
/// one, the source's own shape, the one the API key connection asks for (`api_model::aspect_ratio`).
pub(crate) fn seedance_frame(spec: &JobSpec) -> Option<String> {
    if spec.source.is_none() {
        return Some(
            spec.aspect_ratio
                .unwrap_or(AspectRatio::Auto)
                .raw_value()
                .to_owned(),
        );
    }
    api_model::aspect_ratio(spec, &AspectRatio::ALL).map(|ratio| ratio.raw_value().to_owned())
}

/// The request of a job. Seedance and Kling make a clip as long as the source selection. Without a source,
/// Seedance makes one as long and in the frame the job asks for, from the prompt alone (`t2v`) or with the images
/// as references; it then makes the soundtrack too, unless the job is muted. With a source it makes one only when
/// asked to (`AudioOption::Generated`); otherwise the source's sound is put back after download. It keeps the
/// source's frame (`seedance_frame`).
#[allow(
    clippy::cast_precision_loss,
    reason = "billed seconds are small whole numbers"
)]
pub(crate) fn request(
    spec: &JobSpec,
    model: &str,
    media: Vec<VideoMedia>,
    parameters: &BTreeSet<ModelParameter>,
    constraints: &ModelConstraintTable,
) -> VideoRequest {
    if let Some(draft_job_id) = &spec.draft_job_id
        && model == VideoRequest::SEEDANCE_25
    {
        return VideoRequest::finalize(draft_job_id.clone());
    }
    let has_media = !media.is_empty();
    let mut request = VideoRequest::new(model, media);
    request.prompt.clone_from(&spec.prompt);
    let seconds = || Some(duration_seconds(spec, constraints) as f64);
    match spec.model.descriptor().shape {
        RequestShape::Genjutsu => request.resolution = Some(spec.resolution.raw_value().to_owned()),
        RequestShape::Seedance => {
            request.resolution = Some(spec.resolution.raw_value().to_owned());
            request.generate_audio = Some(generates_audio(spec));
            if parameters.contains(&ModelParameter::SeedanceBitrate) {
                request.bitrate_mode = spec.bitrate.map(|bitrate| bitrate.raw_value().to_owned());
            }
            match spec.mode {
                // The output follows the source, as on REST: no length.
                Mode::Edit => request.mode = Some(VideoRequest::SEEDANCE_EDIT.to_owned()),
                Mode::Extend => {
                    // The output's frame follows the clip.
                    request.mode = Some(VideoRequest::SEEDANCE_EXTEND.to_owned());
                    request.extension_mode = Some(VideoRequest::EXTEND_FORWARD.to_owned());
                    request.duration = seconds();
                }
                _ => {
                    request.duration = seconds();
                    if spec.uses_frames() {
                        // The server doesn't take the frame from the start image: without `aspect_ratio` a 9:16
                        // start frame made a 16:9 clip (live, 2026-10-15). The engine sets the start frame's shape
                        // when the job is queued.
                        request.mode = Some(VideoRequest::OMNI_REFERENCE.to_owned());
                        request.aspect_ratio = spec.aspect_ratio.map(|ratio| ratio.raw_value().to_owned());
                        request.media = as_frames(&request.media);
                    } else {
                        request.mode = Some(
                            if has_media {
                                VideoRequest::OMNI_REFERENCE
                            } else {
                                VideoRequest::TEXT_TO_VIDEO
                            }
                            .to_owned(),
                        );
                        request.aspect_ratio = seedance_frame(spec);
                    }
                    request.draft = sends_native_draft(spec, model);
                }
            }
        }
        RequestShape::KlingMotionControl => {
            request.mode = Some(kling_mode(spec.resolution).to_owned());
            request.duration = seconds();
            if spec.keeps_video_background {
                request.background_source = Some(VideoRequest::BACKGROUND_FROM_VIDEO.to_owned());
            }
        }
        RequestShape::KlingVideoReference => {
            request.mode = Some(kling_mode(spec.resolution).to_owned());
            request.duration = seconds();
        }
        // The output follows the source, as on REST: no length.
        RequestShape::KlingVideoEdit => request.mode = Some(kling_mode(spec.resolution).to_owned()),
        RequestShape::WanReference => {
            // No `mode`: Wan and MiniMax read the job from the media roles.
            request.resolution = Some(spec.resolution.raw_value().to_owned());
            request.duration = seconds();
            if spec.model.declares(VideoControl::GeneratedAudio) {
                request.generate_audio = Some(generates_audio(spec));
            }
        }
        RequestShape::CinemaStudio => {
            // The output's frame follows the reference video. The creative controls aren't sent: the account takes
            // them as `*_id`s its catalog doesn't list (`ModelParameter::CinemaControls`).
            request.mode = Some(VideoRequest::OMNI_REFERENCE.to_owned());
            request.resolution = Some(spec.resolution.raw_value().to_owned());
            request.duration = seconds();
            request.generate_audio = Some(generates_audio(spec));
        }
        RequestShape::Kling4 => kling4(&mut request, spec, constraints),
        RequestShape::GeminiOmni => gemini_omni(&mut request, spec, constraints),
        RequestShape::Intone => intone(&mut request, spec),
    }
    request
}

/// The first two media as a start and an end frame.
pub(crate) fn as_frames(media: &[VideoMedia]) -> Vec<VideoMedia> {
    media
        .iter()
        .zip([MediaRole::StartFrame, MediaRole::EndFrame])
        .map(|(item, role)| VideoMedia::new(role, item.id.clone()))
        .collect()
}

/// The settings of Kling 4.0 (`kling4_0`) and 4.0 Flash (`kling_4_0_flash`, the same fields at 720p and 3–20 s,
/// with no `end_image` role): the resolution, the length, the frame and the `mode`, with frames as `start_image`
/// and `end_image`; Flash's one frame is its `start_image`.
#[allow(
    clippy::cast_precision_loss,
    reason = "billed seconds are small whole numbers"
)]
fn kling4(request: &mut VideoRequest, spec: &JobSpec, constraints: &ModelConstraintTable) {
    request.resolution = Some(spec.resolution.raw_value().to_owned());
    request.duration = Some(duration_seconds(spec, constraints) as f64);
    request.aspect_ratio = kling4_frame(spec, Some(AspectRatio::Auto.raw_value()));
    request.mode = Some(kling4_mode(spec).to_owned());
    if spec.uses_frames() {
        request.media = as_frames(&request.media);
    }
}

/// Kling 4.0's `mode` (`kling4_0`, catalog snapshot of 2026-10-28), as REST's endpoint is picked
/// (`VideoModel::endpoint_id`): Edit, frames, references (a source or images), else the prompt alone.
pub(crate) fn kling4_mode(spec: &JobSpec) -> &'static str {
    if spec.mode == Mode::Edit {
        return VideoRequest::KLING4_VIDEO_EDIT;
    }
    if spec.uses_frames() {
        return VideoRequest::KLING4_IMAGE_TO_VIDEO;
    }
    if spec.source.is_none() && spec.references.is_empty() {
        VideoRequest::KLING4_TEXT_TO_VIDEO
    } else {
        VideoRequest::KLING4_REFERENCE_TO_VIDEO
    }
}

/// Gemini Omni Flash 1.2's settings (`gemini_omni_flash_1_2`, catalog snapshot of 2026-11-10): the resolution, the
/// `mode` (`gemini_omni_mode`), the length but for Edit, whose output follows the source, and frames as
/// `start_image` and `end_image`. Its `aspect_ratios` are 16:9 and 9:16 with no `auto`, so a clip without a source
/// is sent the nearest of them, and Auto or a job with a source none, to follow the inputs. It makes its soundtrack
/// on its own (no `generate_audio`); a muted job has it removed after download (`AudioPlan`).
#[allow(
    clippy::cast_precision_loss,
    reason = "billed seconds are small whole numbers"
)]
fn gemini_omni(request: &mut VideoRequest, spec: &JobSpec, constraints: &ModelConstraintTable) {
    request.resolution = Some(spec.resolution.raw_value().to_owned());
    request.mode = Some(gemini_omni_mode(spec).to_owned());
    if spec.mode != Mode::Edit {
        request.duration = Some(duration_seconds(spec, constraints) as f64);
    }
    request.aspect_ratio = kling4_frame(spec, None);
    if spec.uses_frames() {
        request.media = as_frames(&request.media);
    }
}

/// Higgsfield Intone's settings (`intone`, catalog snapshot of 2026-11-10): the images as the speakers
/// (`character_images`), the speech as `audio_references`, the resolution and the frame. It takes no `duration`
/// (the speech sets it), `mode` or `generate_audio`; `bitrate_mode` is left at the server's default. Its
/// `aspect_ratios` have no `auto`, so Auto sends none, and a frame asked for the nearest of them.
fn intone(request: &mut VideoRequest, spec: &JobSpec) {
    request.resolution = Some(spec.resolution.raw_value().to_owned());
    request.aspect_ratio = kling4_frame(spec, None);
    for media in &mut request.media {
        if media.role == MediaRole::Image {
            media.role = MediaRole::Character;
        }
    }
}

/// Gemini Omni Flash 1.2's `mode`: its own for Edit and Extend, else Kling 4.0's (`kling4_mode`), which it names
/// alike.
pub(crate) fn gemini_omni_mode(spec: &JobSpec) -> &'static str {
    match spec.mode {
        Mode::Edit => VideoRequest::GEMINI_EDIT,
        Mode::Extend => VideoRequest::GEMINI_EXTEND,
        _ => kling4_mode(spec),
    }
}

/// Kling 4.0's `aspect_ratio` on either connection, and Gemini Omni Flash 1.2's: `auto` (REST's `adaptive`, the
/// account's `auto`), which follows the references, for a job with a source and for Auto; otherwise the model's
/// fixed frame (`models.json` `aspectRatios`) nearest the one asked for, as no Kling 4.0 schema has 4:3 or 3:4.
/// `None` for `auto` leaves the field out.
#[allow(
    clippy::cast_possible_truncation,
    reason = "a ratio times 10 000 is far inside i64, as Swift's Int(_:) of it"
)]
pub(crate) fn kling4_frame(spec: &JobSpec, auto: Option<&str>) -> Option<String> {
    let auto = auto.map(str::to_owned);
    if spec.source.is_some() {
        return auto;
    }
    let Some(shape) = spec.aspect_ratio.and_then(AspectRatio::width_over_height) else {
        return auto;
    };
    let size = PixelSize::new((shape * 10_000.0).round() as i64, 10_000);
    AspectRatio::nearest(size, &spec.model.descriptor().aspect_ratios)
        .map(|ratio| ratio.raw_value().to_owned())
        .or(auto)
}

/// Whether Draft asks for Seedance 2.5's native draft (`draft: true`): a 480p draft of a generation, which the
/// account can finalize at 1080p within seven days (`VideoRequest::finalize`). Edit and Extend, and a draft at
/// another resolution, stay plain renders.
pub(crate) fn sends_native_draft(spec: &JobSpec, model: &str) -> bool {
    spec.is_draft
        && spec.draft_job_id.is_none()
        && model == VideoRequest::SEEDANCE_25
        && spec.resolution == Resolution::P480
        && !spec.mode.is_video_operation()
        && spec.operation.is_none()
}

/// Whether the model makes the soundtrack (Seedance, Wan 3.0, Cinema Studio), on either connection: always without
/// a source and for the part Extend adds, which no source sound belongs to, unless the job is muted; with a source
/// only when asked to.
pub(crate) fn generates_audio(spec: &JobSpec) -> bool {
    if spec.audio == AudioOption::Mute {
        return false;
    }
    spec.source.is_none() || spec.mode == Mode::Extend || spec.audio == AudioOption::Generated
}

/// Whether Kling keeps the motion video's sound on the server, with an API key. Only for `original`: a muted job
/// wants none, and Fade out comes back silent so the faded source sound is put on it after download.
pub(crate) fn keeps_original_sound(spec: &JobSpec) -> bool {
    spec.audio == AudioOption::Original
}

/// The billed seconds of the selection, or of the length asked for (for Extend, the part it adds), inside the
/// model's duration limits (by `constraints` where they say).
#[allow(
    clippy::cast_possible_truncation,
    reason = "limits are seconds, far inside i64"
)]
pub(crate) fn duration_seconds(spec: &JobSpec, constraints: &ModelConstraintTable) -> i64 {
    let limits = spec.limits(constraints).duration;
    let seconds = Pricing::billed_seconds(spec.duration());
    seconds
        .max(limits.lower().ceil() as i64)
        .min(limits.upper().floor() as i64)
}

/// Kling has a quality mode instead of a resolution (`KlingQuality`).
pub(crate) fn kling_mode(resolution: Resolution) -> &'static str {
    if KlingQuality::from_resolution(resolution) == KlingQuality::Pro {
        VideoRequest::KLING_PRO
    } else {
        VideoRequest::KLING_STANDARD
    }
}

/// Whether the job is billed by its source rather than by a length it sends: Genjutsu's Motion Transfer and Object
/// Swap, Seedance's Edit and Kling O3 Edit. `get_cost` prices these only with the source in `medias`, so they are
/// quoted once it is uploaded.
pub(crate) fn bills_source(spec: &JobSpec) -> bool {
    matches!(
        spec.model.descriptor().shape,
        RequestShape::Genjutsu | RequestShape::KlingVideoEdit
    ) || spec.mode == Mode::Edit
}

/// Whether `get_cost` can price the job only with its media: one billed by its source (`bills_source`), a Kling 4.0
/// or Gemini Omni Flash 1.2 job with a source, frames or images, and a model that voices a speech.
pub(crate) fn quotes_after_upload(spec: &JobSpec) -> bool {
    bills_source(spec)
        || (matches!(
            spec.model.descriptor().shape,
            RequestShape::Kling4 | RequestShape::GeminiOmni
        ) && (spec.source.is_some() || !spec.references.is_empty()))
        || spec.model.declares(VideoControl::Speech)
}

/// The `get_cost` question for a job before anything is uploaded: its request with no media, prompt or variants,
/// paid in credits. `None` for a job quoted only with its media (`quotes_after_upload`).
pub(crate) fn quote_request(
    spec: &JobSpec,
    model: &str,
    constraints: &ModelConstraintTable,
) -> Option<VideoRequest> {
    if quotes_after_upload(spec) {
        return None;
    }
    let mut request = request(spec, model, Vec::new(), &parameters(), constraints);
    request.prompt = String::new();
    request.count = 1;
    request.uses_unlimited = false;
    // A finalize names its draft and sends no length: the draft's own is billed.
    (request.duration.is_some() || request.draft_job_id.is_some()).then_some(request)
}

/// The fallback price, by the catalog's `credits`, from `get_cost` quotes of 2026-10-15: shown, as an estimate,
/// only until Higgsfield quotes the job, or when the quote fails. A job billed by its source bills the selection as
/// it is; the others bill the length they are sent (`duration_seconds`). `None` for a model without credits at the
/// job's resolution. The finalize of a draft (`JobSpec::draft_job_id`) is a third of the 1080p price of its length
/// (finalizing a 15 s draft quoted 80 credits, the same 15 s at 1080p 240).
pub(crate) fn estimated_credits(spec: &JobSpec, constraints: &ModelConstraintTable) -> Option<i64> {
    let credits = spec
        .model
        .descriptor()
        .credits
        .at_source(spec.mode, spec.source.is_some());
    let resolution = if spec.draft_job_id.is_none() {
        spec.resolution
    } else {
        Resolution::P1080
    };
    let rate = *credits.per_second.get(&resolution)?;
    let seconds = if bills_source(spec) {
        Pricing::billed_seconds(spec.duration())
    } else {
        duration_seconds(spec, constraints)
    };
    let mut price = credits.base + rate * Decimal::from(seconds);
    if spec.draft_job_id.is_some() {
        price /= Decimal::from(3);
    }
    i64::try_from(price.ceil()).ok()
}

/// A Genjutsu Style's price before upload, fitted to `get_presets` quotes: 1.5 credits plus 3, 7 or 11 a second at
/// 480p, 720p and 1080p (12 s: 37.5, 85.5 and 133.5 on 2026-10-15; 8 s: 25.5, 57.5 and 89.5 on 2026-10-14).
pub(crate) fn style_credits(spec: &JobSpec) -> i64 {
    let rate = match spec.resolution {
        Resolution::P480 => 3,
        Resolution::P720 => 7,
        Resolution::P1080 => 11,
        _ => 0,
    };
    let price = Decimal::new(15, 1) + Decimal::from(rate * Pricing::billed_seconds(spec.duration()));
    i64::try_from(price.ceil()).unwrap_or(0)
}

/// The Jutsu model and mode each `generate_video` model runs: the catalog's first model and mode naming it.
pub(crate) static CONSTRAINED_JOBS: LazyLock<HashMap<String, (VideoModel, Mode)>> = LazyLock::new(|| {
    let mut jobs = HashMap::new();
    for model in VideoModel::all() {
        for mode in model.modes() {
            let Some(id) = model.account_model_id(mode) else {
                continue;
            };
            jobs.entry(id).or_insert_with(|| (model.clone(), mode));
        }
    }
    jobs
});

/// `models_explore`'s limits as the table Create reads, in a fixed order.
pub(crate) fn constraints(limits: &HashMap<String, ModelLimits>) -> ModelConstraintTable {
    ModelConstraintTable::new(
        ACCOUNT_MODELS
            .iter()
            .filter_map(|model| {
                let read = limits.get(model)?;
                let (model, mode) = CONSTRAINED_JOBS.get(model)?.clone();
                Some(ModelConstraintEntry {
                    model,
                    mode,
                    constraints: ModelConstraints::new(
                        read.min_duration,
                        read.max_duration,
                        read.min_images,
                        read.max_images,
                    ),
                })
            })
            .collect(),
    )
}
