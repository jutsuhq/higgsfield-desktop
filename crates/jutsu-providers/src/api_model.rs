//! The REST request of each job.

use higgsfield::api::{
    GenerationRequest, Kling4Request, KlingMotionRequest, KlingQuality as ApiKlingQuality,
    KlingVideoEditRequest, KlingVideoReferenceRequest, ModelEndpoint, Orientation, OriginalSound,
    ReferenceToVideoRequest, SeedanceFramesRequest, SeedanceRequest, SeedanceVideoRequest,
};
use jutsu_core::{
    AspectRatio, CharacterOrientation, JobSpec, KlingQuality, Mode, ModelConstraintTable, ModelParameter,
    PortError, RemoteInputs, RequestShape, VideoControl,
};
use serde::Serialize;
use std::collections::BTreeSet;

use crate::account_model;
use crate::mapping::validation;

/// The body of one endpoint.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum ApiBody {
    Generation(GenerationRequest),
    Seedance(SeedanceRequest),
    SeedanceFrames(SeedanceFramesRequest),
    SeedanceVideo(SeedanceVideoRequest),
    KlingMotion(KlingMotionRequest),
    KlingVideoReference(KlingVideoReferenceRequest),
    KlingVideoEdit(KlingVideoEditRequest),
    ReferenceToVideo(ReferenceToVideoRequest),
    Kling4(Kling4Request),
}

/// The REST request of each job: the endpoint (`JobSpec::endpoint_id`, from the catalog) and that endpoint's body,
/// by the model's request shape. Mirrors `account_model::request` for the account, except that Seedance's mode and
/// Kling's quality are the endpoint, and Kling takes no duration (the output follows the source) and has no
/// background parameter.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ApiModel {
    pub(crate) endpoint: ModelEndpoint,
    pub(crate) body: ApiBody,
}

/// The model parameters the REST API sends: all but Kling's background, which it has no parameter for.
pub(crate) fn parameters() -> BTreeSet<ModelParameter> {
    ModelParameter::ALL
        .iter()
        .copied()
        .filter(|parameter| *parameter != ModelParameter::KlingVideoBackground)
        .collect()
}

/// Restyle can't go without a preset, and no Genjutsu mode without a video. Upscale and Remove Background are tools
/// of the account's MCP server; the public REST docs have neither.
pub(crate) fn needs_account() -> PortError {
    validation(None, "Needs a Higgsfield account")
}

impl ApiModel {
    pub(crate) fn new(spec: &JobSpec, inputs: &RemoteInputs) -> Result<Self, PortError> {
        if spec.operation.is_some() {
            return Err(needs_account());
        }
        let Some(endpoint) = ModelEndpoint::from_raw_value(&spec.endpoint_id()) else {
            return Err(validation(
                None,
                format!("{} can't run with an API key.", spec.model.title()),
            ));
        };
        let body = match spec.model.descriptor().shape {
            RequestShape::Genjutsu => ApiBody::Generation(genjutsu(spec, inputs)?),
            RequestShape::Seedance if spec.mode.is_video_operation() => {
                ApiBody::SeedanceVideo(seedance_video(spec, inputs)?)
            }
            RequestShape::Seedance if spec.uses_frames() => {
                ApiBody::SeedanceFrames(seedance_frames(spec, inputs)?)
            }
            RequestShape::Seedance => ApiBody::Seedance(seedance(spec, inputs)),
            RequestShape::KlingMotionControl => ApiBody::KlingMotion(kling(spec, inputs)?),
            RequestShape::KlingVideoReference => {
                ApiBody::KlingVideoReference(kling_video_reference(spec, inputs)?)
            }
            RequestShape::KlingVideoEdit => ApiBody::KlingVideoEdit(kling_video_edit(spec, inputs)?),
            RequestShape::WanReference | RequestShape::CinemaStudio => {
                ApiBody::ReferenceToVideo(reference_to_video(spec, inputs)?)
            }
            RequestShape::Kling4 => ApiBody::Kling4(kling4(spec, inputs)?),
            // No REST endpoint (`models.json`), so `endpoint_id` refused it above.
            RequestShape::GeminiOmni | RequestShape::Intone => return Err(needs_account()),
        };
        Ok(Self { endpoint, body })
    }
}

fn duration(spec: &JobSpec) -> i64 {
    account_model::duration_seconds(spec, &ModelConstraintTable::built_in())
}

fn image_urls(inputs: &RemoteInputs) -> Vec<url::Url> {
    inputs.images.iter().map(|image| image.url.clone()).collect()
}

fn genjutsu(spec: &JobSpec, inputs: &RemoteInputs) -> Result<GenerationRequest, PortError> {
    let Some(video) = &inputs.video else {
        return Err(validation(Some("video"), "Genjutsu needs a source video."));
    };
    let mut preset_id = None;
    if spec.mode == Mode::Restyle {
        let Some(preset) = spec.preset_id else {
            return Err(validation(Some("preset_id"), "Restyle needs a preset."));
        };
        preset_id = Some(preset);
    }
    Ok(GenerationRequest {
        video_url: video.url.clone(),
        image_urls: image_urls(inputs),
        prompt: spec.prompt.clone(),
        resolution: spec.resolution.raw_value().to_owned(),
        preset_id,
    })
}

/// The source clip goes first in `video_urls`. The soundtrack is generated as on the account
/// (`account_model::generates_audio`).
fn seedance(spec: &JobSpec, inputs: &RemoteInputs) -> SeedanceRequest {
    SeedanceRequest {
        prompt: spec.prompt.clone(),
        image_urls: image_urls(inputs),
        video_urls: inputs.video.iter().map(|video| video.url.clone()).collect(),
        duration: duration(spec),
        resolution: spec.resolution.raw_value().to_owned(),
        aspect_ratio: aspect_ratio(spec, &AspectRatio::ALL).map(|ratio| ratio.raw_value().to_owned()),
        generate_audio: account_model::generates_audio(spec),
        bitrate_mode: spec.bitrate.map(|bitrate| bitrate.raw_value().to_owned()),
    }
}

/// The first image is the start frame, the second, when there is one, the end frame.
fn seedance_frames(spec: &JobSpec, inputs: &RemoteInputs) -> Result<SeedanceFramesRequest, PortError> {
    let Some(start) = inputs.images.first() else {
        return Err(validation(Some("image_url"), "Seedance needs the start frame."));
    };
    Ok(SeedanceFramesRequest {
        prompt: spec.prompt.clone(),
        image_url: start.url.clone(),
        end_image_url: inputs.images.get(1).map(|end| end.url.clone()),
        duration: duration(spec),
        resolution: spec.resolution.raw_value().to_owned(),
        generate_audio: account_model::generates_audio(spec),
        bitrate_mode: spec.bitrate.map(|bitrate| bitrate.raw_value().to_owned()),
    })
}

/// Edit and Extend: the source is the clip to change or continue. Edit's output follows the source, so it takes no
/// length; Extend's `duration` is the part it adds.
fn seedance_video(spec: &JobSpec, inputs: &RemoteInputs) -> Result<SeedanceVideoRequest, PortError> {
    let Some(video) = &inputs.video else {
        return Err(validation(
            Some("video_url"),
            "Seedance needs the clip to edit or extend.",
        ));
    };
    Ok(SeedanceVideoRequest {
        prompt: spec.prompt.clone(),
        video_url: video.url.clone(),
        image_urls: image_urls(inputs),
        duration: (spec.mode == Mode::Extend).then(|| duration(spec)),
        resolution: spec.resolution.raw_value().to_owned(),
        generate_audio: account_model::generates_audio(spec),
        bitrate_mode: spec.bitrate.map(|bitrate| bitrate.raw_value().to_owned()),
    })
}

/// The REST schema has no "auto" and defaults to 16:9, so a clip from a source asks for the ratio nearest the
/// source's frame among `candidates`; without a source, the one asked for, or the server's default for Auto.
pub(crate) fn aspect_ratio(spec: &JobSpec, candidates: &[AspectRatio]) -> Option<AspectRatio> {
    let Some(source) = &spec.source else {
        return spec.aspect_ratio.filter(|ratio| *ratio != AspectRatio::Auto);
    };
    AspectRatio::nearest(source.pixel_size, candidates)
}

fn quality(spec: &JobSpec) -> ApiKlingQuality {
    if KlingQuality::from_resolution(spec.resolution) == KlingQuality::Pro {
        ApiKlingQuality::Pro
    } else {
        ApiKlingQuality::Standard
    }
}

/// Kling O3 takes the source as its one reference video and the images in order, and makes a clip as long as the
/// selection inside its 3–10 s, at its quality (`KlingQuality`).
fn kling_video_reference(
    spec: &JobSpec,
    inputs: &RemoteInputs,
) -> Result<KlingVideoReferenceRequest, PortError> {
    let Some(video) = &inputs.video else {
        return Err(validation(
            Some("video_urls"),
            format!("{} needs a source video.", spec.model.title()),
        ));
    };
    Ok(KlingVideoReferenceRequest {
        prompt: spec.prompt.clone(),
        video_url: video.url.clone(),
        image_urls: image_urls(inputs),
        quality: quality(spec),
        duration: duration(spec),
    })
}

/// Kling O3's Edit takes the source as the clip to change and the images in order, at its quality; the output
/// follows the source, so there is no length.
fn kling_video_edit(spec: &JobSpec, inputs: &RemoteInputs) -> Result<KlingVideoEditRequest, PortError> {
    let Some(video) = &inputs.video else {
        return Err(validation(
            Some("video_urls"),
            format!("{} needs a source video.", spec.model.title()),
        ));
    };
    Ok(KlingVideoEditRequest {
        prompt: spec.prompt.clone(),
        video_url: video.url.clone(),
        image_urls: image_urls(inputs),
        quality: quality(spec),
    })
}

/// Wan, Cinema Studio and MiniMax H3 take the source as their reference video and the images in order, and make a
/// clip as long as the selection inside the model's limits at one of the model's resolutions. The frame is the one
/// of the model's `aspect_ratios` nearest the source's, else the server's default. A model with a soundtrack
/// (`GeneratedAudio`) makes one as Seedance does, and Cinema Studio sends the creative controls the job sets.
fn reference_to_video(spec: &JobSpec, inputs: &RemoteInputs) -> Result<ReferenceToVideoRequest, PortError> {
    let Some(video) = &inputs.video else {
        return Err(validation(
            Some("video_urls"),
            format!("{} needs a source video.", spec.model.title()),
        ));
    };
    let frames = spec.model.descriptor().aspect_ratios.clone();
    Ok(ReferenceToVideoRequest {
        prompt: spec.prompt.clone(),
        video_url: video.url.clone(),
        image_urls: image_urls(inputs),
        duration: duration(spec),
        resolution: spec.resolution.raw_value().to_owned(),
        aspect_ratio: if frames.is_empty() {
            None
        } else {
            aspect_ratio(spec, &frames).map(|ratio| ratio.raw_value().to_owned())
        },
        generate_audio: spec
            .model
            .declares(VideoControl::GeneratedAudio)
            .then(|| account_model::generates_audio(spec)),
        controls: if spec.model.declares(VideoControl::CinemaControls) {
            spec.cinema_controls.fields()
        } else {
            std::collections::BTreeMap::new()
        },
    })
}

fn kling4_body(prompt: String, duration: i64, resolution: String) -> Kling4Request {
    Kling4Request {
        prompt,
        image_url: None,
        end_image_url: None,
        video_url: None,
        image_urls: Vec::new(),
        video_urls: Vec::new(),
        duration,
        resolution,
        aspect_ratio: None,
    }
}

/// Kling 4.0 and 4.0 Flash on the endpoint the job picks (`JobSpec::endpoint_id`): Edit sends the source as the clip
/// to edit (`video_url`) with the images as references; frames the start and end frame (Flash the start alone); a
/// clip with a source or images sends them as references (the source is `video_urls`' one item); a prompt alone
/// neither. References and Edit get the prompt's tags in the schemas' form (`kling4_prompt`). The frame follows
/// `account_model::kling4_frame`, with REST's `adaptive` for Auto where the endpoint has it; `image-to-video` takes
/// none.
fn kling4(spec: &JobSpec, inputs: &RemoteInputs) -> Result<Kling4Request, PortError> {
    let duration = duration(spec);
    let resolution = spec.resolution.raw_value().to_owned();
    let images = image_urls(inputs);
    let videos = usize::from(inputs.video.is_some());
    let tagged = kling4_prompt(&spec.prompt, images.len(), videos);
    if spec.mode == Mode::Edit {
        let Some(video) = &inputs.video else {
            return Err(validation(
                Some("video_url"),
                format!("{} needs the clip to edit.", spec.model.title()),
            ));
        };
        let mut request = kling4_body(tagged, duration, resolution);
        request.video_url = Some(video.url.clone());
        request.image_urls = images;
        request.aspect_ratio = account_model::kling4_frame(spec, Some(Kling4Request::ADAPTIVE));
        return Ok(request);
    }
    if spec.uses_frames() {
        let Some(start) = images.first() else {
            return Err(validation(
                Some("image_url"),
                format!("{} needs the start frame.", spec.model.title()),
            ));
        };
        let mut request = kling4_body(spec.prompt.clone(), duration, resolution);
        request.image_url = Some(start.clone());
        request.end_image_url = if spec.model.takes_end_frame() {
            images.get(1).cloned()
        } else {
            None
        };
        return Ok(request);
    }
    if spec.source.is_none() && spec.references.is_empty() {
        let mut request = kling4_body(spec.prompt.clone(), duration, resolution);
        request.aspect_ratio = account_model::kling4_frame(spec, None);
        return Ok(request);
    }
    let mut request = kling4_body(tagged, duration, resolution);
    request.image_urls = images;
    request.video_urls = inputs.video.iter().map(|video| video.url.clone()).collect();
    request.aspect_ratio = account_model::kling4_frame(spec, Some(Kling4Request::ADAPTIVE));
    Ok(request)
}

/// The prompt with the app's tags (`@Image1`, `@Video1`, any case) as Kling 4.0's reference and edit schemas name
/// the media: `@image_N` for the N-th image sent (the job's references, in order) and `@video_N` for the N-th video,
/// the source. A tag past what is sent, or already in the schema's form, is left as written; the whole number is
/// read, so `@Image10` never becomes `@image_1` + "0".
pub(crate) fn kling4_prompt(prompt: &str, images: usize, videos: usize) -> String {
    let mut result = String::new();
    let mut rest = prompt;
    while let Some(at) = rest.find('@') {
        result.push_str(&rest[..at]);
        rest = &rest[at..];
        if let Some((tag, length)) = kling4_tag(rest, images, videos) {
            result.push_str(&tag);
            rest = &rest[length..];
        } else {
            result.push('@');
            rest = &rest[1..];
        }
    }
    result + rest
}

/// The schema's tag for the app's tag `text` starts with, and how many bytes that tag has.
fn kling4_tag(text: &str, images: usize, videos: usize) -> Option<(String, usize)> {
    for (word, kind, count) in [("@image", "image", images), ("@video", "video", videos)] {
        let Some(head) = text.get(..word.len()) else {
            continue;
        };
        if head.to_lowercase() != word {
            continue;
        }
        let digits: String = text[word.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let number: usize = digits.parse().ok()?;
        if number < 1 || number > count {
            return None;
        }
        return Some((format!("@{kind}_{number}"), word.len() + digits.len()));
    }
    None
}

/// Kling takes exactly one character image and the source as the motion.
fn kling(spec: &JobSpec, inputs: &RemoteInputs) -> Result<KlingMotionRequest, PortError> {
    let Some(video) = &inputs.video else {
        return Err(validation(Some("video_url"), "Kling needs a source video."));
    };
    let Some(image) = inputs.images.first() else {
        return Err(validation(Some("image_url"), "Kling needs a reference image."));
    };
    Ok(KlingMotionRequest {
        image_url: image.url.clone(),
        video_url: video.url.clone(),
        prompt: spec.prompt.clone(),
        keep_original_sound: Some(if account_model::keeps_original_sound(spec) {
            OriginalSound::Keep
        } else {
            OriginalSound::Drop
        }),
        character_orientation: spec.character_orientation.map(|orientation| match orientation {
            CharacterOrientation::Image => Orientation::Image,
            CharacterOrientation::Video => Orientation::Video,
        }),
    })
}
