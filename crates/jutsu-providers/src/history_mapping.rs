//! `show_generations` pages in the app's terms: finished videos of the models the app runs, with their inputs.

use std::collections::BTreeSet;

use higgsfield::core::is_https;
use higgsfield::mcp::account::{
    GenerationPage, GenerationType, MediaRole, PastGeneration, PastMedia, VideoRequest,
};
use jutsu_core::{
    HistoryCursor, HistoryImageItem, HistoryItem, HistoryPage, Mode, RemoteAsset, RemoteInputs, RequestShape,
    Resolution, VideoControl, VideoModel,
};
use serde_json::Value;
use uuid::Uuid;

use crate::account_mapping::pixel_size;
use crate::account_model::CONSTRAINED_JOBS;

/// Tags a packed cursor, so a server cursor that happens to be an array never reads as a packed one.
pub(crate) const CURSOR_TAG: &str = "jutsu-history-cursors-v1";

/// The `model` of a Genjutsu Style (`execute_preset`) job, as `job_status` and `jobs_wait` showed it in a live run
/// on 2026-10-15 (issue #21). Neither history tool listed that job, so a Style from history is rare.
pub(crate) const STYLE_MODEL: &str = "restyle";

pub(crate) fn page(page: &GenerationPage) -> HistoryPage {
    log_skipped_models(&page.items);
    HistoryPage {
        items: page.items.iter().filter_map(item).collect(),
        images: page.items.iter().filter_map(image).collect(),
        next_cursor: page.next_cursor.as_ref().and_then(cursor),
    }
}

/// `None` for anything the app can't show or run again: other media types and models, unfinished generations, and
/// inputs without a source video, except for a model that makes clips without one (not in Edit or Extend) and a
/// Style run's, whose input roles are unconfirmed. Kling 4.0, 4.0 Flash and Gemini Omni Flash 1.2 come back in the
/// mode their `params.mode` names (`job_mode`), and a clip from frames with them as frames (`frames`); Intone's
/// speakers (`character_images`) come back as its images, in order. Only https URLs count: the app downloads them
/// and runs them again. A Style job is `type: "image"` until it completes and `"video"` once it has, so only a
/// completed one, a video, is kept, like every other job.
pub(crate) fn item(generation: &PastGeneration) -> Option<HistoryItem> {
    let video = generation.media.iter().find_map(asset(MediaRole::Video));
    if generation.kind != GenerationType::Video.raw_value() || generation.status != "completed" {
        return None;
    }
    let output = generation.output_url.clone().filter(is_https)?;
    let (catalog_mode, video_model) = kind_of_model(&generation.model)?;
    let mode = job_mode(generation, &video_model).unwrap_or(catalog_mode);
    if video.is_none()
        && !(!mode.is_video_operation()
            && (mode == Mode::Restyle || video_model.makes_clips_without_source()))
    {
        return None;
    }
    let frames = if video.is_none() && video_model.declares(VideoControl::Frames) {
        frames(generation)
    } else {
        None
    };
    let references_are_frames = frames.is_some();
    let images = frames.unwrap_or_else(|| {
        generation
            .media
            .iter()
            .filter_map(|media| asset(MediaRole::Image)(media).or_else(|| asset(MediaRole::Character)(media)))
            .collect()
    });
    let inputs = RemoteInputs {
        video,
        images,
        audio: generation.media.iter().find_map(asset(MediaRole::Audio)),
    };
    let mut history = HistoryItem::new(
        generation.id.clone(),
        mode,
        video_model,
        generation.created_at,
        output,
        inputs,
    );
    history.is_liked = generation.is_liked;
    history.duration = generation.duration;
    history.pixel_size = pixel_size(generation.width, generation.height);
    history.resolution = generation.resolution.as_deref().and_then(Resolution::from_raw);
    history.prompt.clone_from(&generation.prompt);
    history.references_are_frames = references_are_frames;
    // Only Seedance 2.5 has a native draft and its finalize (`models_explore get`, 2026-10-15).
    if generation.model == VideoRequest::SEEDANCE_25 {
        history.is_native_draft = generation.is_draft;
        history.draft_job_id.clone_from(&generation.draft_job_id);
    }
    Some(history)
}

/// The job mode a generation's `params.mode` names, for a model whose one server model runs several of the app's
/// (Kling 4.0's and Flash's `video-edit` is Edit, Gemini Omni Flash 1.2's `edit` and `extend` Edit and Extend, their
/// other modes Motion Transfer); `None` keeps the catalog's.
pub(crate) fn job_mode(generation: &PastGeneration, model: &VideoModel) -> Option<Mode> {
    let mode = generation.mode.as_deref()?;
    match model.descriptor().shape {
        RequestShape::Kling4 => Some(if mode == VideoRequest::KLING4_VIDEO_EDIT {
            Mode::Edit
        } else {
            Mode::MotionTransfer
        }),
        RequestShape::GeminiOmni => Some(match mode {
            VideoRequest::GEMINI_EDIT => Mode::Edit,
            VideoRequest::GEMINI_EXTEND => Mode::Extend,
            _ => Mode::MotionTransfer,
        }),
        _ => None,
    }
}

/// The start and, when there is one, end frame of a clip made from frames, in that order: the inputs with those
/// roles, or for Kling 4.0's `image-to-video` its first two images; `None` for a clip made otherwise.
pub(crate) fn frames(generation: &PastGeneration) -> Option<Vec<RemoteAsset>> {
    if let Some(first) = generation.media.iter().find_map(asset(MediaRole::StartFrame)) {
        let mut frames = vec![first];
        frames.extend(
            generation
                .media
                .iter()
                .filter_map(asset(MediaRole::EndFrame))
                .take(1),
        );
        return Some(frames);
    }
    if generation.mode.as_deref() != Some(VideoRequest::KLING4_IMAGE_TO_VIDEO) {
        return None;
    }
    let images: Vec<RemoteAsset> = generation
        .media
        .iter()
        .filter_map(asset(MediaRole::Image))
        .take(2)
        .collect();
    (!images.is_empty()).then_some(images)
}

/// A server model ID in the app's terms, by the catalog (`account_model::CONSTRAINED_JOBS`); `None` for a model the
/// app doesn't run. A Style run goes through `execute_preset`, so its ID names no catalog model.
pub(crate) fn kind_of_model(id: &str) -> Option<(Mode, VideoModel)> {
    if let Some((model, mode)) = CONSTRAINED_JOBS.get(id) {
        return Some((*mode, model.clone()));
    }
    (id == STYLE_MODEL).then_some((Mode::Restyle, VideoModel::GENJUTSU))
}

/// A completed image generation as the library shows it; `None` for anything else. The model ID is kept as
/// reported: the app runs no image model by catalog, so there is nothing to map it to.
pub(crate) fn image(generation: &PastGeneration) -> Option<HistoryImageItem> {
    if generation.kind != GenerationType::Image.raw_value() || generation.status != "completed" {
        return None;
    }
    let output = generation.output_url.clone().filter(is_https)?;
    Some(HistoryImageItem {
        id: generation.id.clone(),
        job_id: Uuid::new_v4(),
        prompt: generation.prompt.clone(),
        model: generation.model.clone(),
        created_at: generation.created_at,
        is_liked: generation.is_liked,
        output_url: output,
        pixel_size: pixel_size(generation.width, generation.height),
    })
}

/// The cursors of the video and image pages as one: each stream pages on its own. `None` past both last pages.
pub(crate) fn joined_cursor(
    video: Option<&HistoryCursor>,
    image: Option<&HistoryCursor>,
) -> Option<HistoryCursor> {
    if video.is_none() && image.is_none() {
        return None;
    }
    let values = vec![
        Value::String(CURSOR_TAG.to_owned()),
        video.and_then(cursor_value).unwrap_or(Value::Null),
        image.and_then(cursor_value).unwrap_or(Value::Null),
    ];
    cursor(&Value::Array(values))
}

/// The two streams' cursors packed by `joined_cursor`, each `None` past its last page. Anything else decodes as a
/// video-only cursor from before images were listed, so old in-flight loads keep working.
pub(crate) fn split_cursor(cursor: Option<&HistoryCursor>) -> (Option<Value>, Option<Value>) {
    let Some(value) = cursor.and_then(cursor_value) else {
        return (None, None);
    };
    match &value {
        Value::Array(values) if values.len() == 3 && values[0] == Value::String(CURSOR_TAG.to_owned()) => {
            let part = |value: &Value| (!value.is_null()).then(|| value.clone());
            (part(&values[1]), part(&values[2]))
        }
        _ => (Some(value), None),
    }
}

/// Each video model ID a page skips, once, so a model the app doesn't know yet shows up in the log. Only the ID is
/// logged, nothing else about the generation.
fn log_skipped_models(generations: &[PastGeneration]) {
    let skipped: BTreeSet<&str> = generations
        .iter()
        .filter(|generation| {
            generation.kind == GenerationType::Video.raw_value() && kind_of_model(&generation.model).is_none()
        })
        .map(|generation| generation.model.as_str())
        .collect();
    for model in skipped {
        tracing::info!(model, "Skipped history videos of the unknown model");
    }
}

/// Inputs without an https URL can't be probed or uploaded again, so they are left out.
fn asset(role: MediaRole) -> impl Fn(&PastMedia) -> Option<RemoteAsset> {
    move |media| {
        if media.role != Some(role) {
            return None;
        }
        let url = media.url.clone().filter(is_https)?;
        Some(RemoteAsset {
            url,
            id: Some(media.id.clone()),
        })
    }
}

/// The cursor as JSON text, so it goes back exactly as it came.
pub(crate) fn cursor(value: &Value) -> Option<HistoryCursor> {
    serde_json::to_string(value).ok().map(HistoryCursor::new)
}

pub(crate) fn cursor_value(cursor: &HistoryCursor) -> Option<Value> {
    serde_json::from_str(&cursor.value).ok()
}
