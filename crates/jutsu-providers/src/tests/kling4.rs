//! Port of `Kling4Tests.swift`: Kling 4.0 on both connections, REST's four endpoints from their documented schemas
//! (open.higgsfield.ai, 2026-10-28) and `kling4_0`'s `mode`s against its `models_explore get` in the catalog
//! snapshot.

use higgsfield::api::ModelEndpoint;
use higgsfield::mcp::account::{MediaRole, VideoMedia, VideoRequest};
use jutsu_core::{
    AspectRatio, GenerationProvider, Mode, ModelConstraintTable, PortError, Pricing, Resolution,
    ValidationIssue,
};
use serde_json::json;
use uuid::Uuid;

use super::support::account::calls;
use super::support::agent_c::kling4::{Clip, OnSource, clip, edit, end, frames, on_source, start, video};
use super::support::agent_c::{
    account_provider, api_body, assert_json, assert_none, assert_params_keep_to_snapshot, check, column,
    credits, estimated, generate_params, inputs, no_inputs, quote, submitted, usd,
};
use crate::account_model;
use crate::api_model::{ApiModel, kling4_prompt};

// MARK: REST

/// A prompt alone: `text-to-video`, the frame asked for (it has no `adaptive`, so Auto leaves it out for the
/// server's 16:9) and the length inside 3–30 s.
#[test]
fn a_prompt_alone_is_text_to_video() {
    let none = no_inputs();
    let (endpoint, body) = api_body(&clip(), &none);
    assert_eq!(endpoint, ModelEndpoint::Kling4TextToVideo);
    assert_eq!(
        body,
        r#"{"aspect_ratio":"9:16","duration":12,"prompt":"a fox in the snow","resolution":"1080p"}"#
    );
    let with = |clip: Clip| api_body(&clip.spec(), &none).1;
    assert!(
        with(Clip {
            aspect_ratio: Some(AspectRatio::Cinema),
            ..Clip::default()
        })
        .contains(r#""aspect_ratio":"21:9""#)
    );
    assert!(
        !with(Clip {
            aspect_ratio: Some(AspectRatio::Auto),
            ..Clip::default()
        })
        .contains("aspect_ratio")
    );
    assert!(
        with(Clip {
            seconds: 1.0,
            ..Clip::default()
        })
        .contains(r#""duration":3,"#)
    );
    assert!(
        with(Clip {
            seconds: 45.0,
            ..Clip::default()
        })
        .contains(r#""duration":30,"#)
    );
}

/// Frames: `image-to-video` with the start and end frame, no prompt needed and no frame (the images set it).
#[test]
fn frames_are_image_to_video() {
    let (endpoint, body) = api_body(&frames(2, ""), &inputs(None, vec![start(), end()]));
    assert_eq!(endpoint, ModelEndpoint::Kling4ImageToVideo);
    assert_eq!(
        body,
        r#"{"duration":5,"end_image_url":"https://cdn.example.com/b.jpg","image_url":"https://cdn.example.com/a.jpg","resolution":"720p"}"#
    );
    let start_only = api_body(&frames(1, ""), &inputs(None, vec![start()]));
    assert!(!start_only.1.contains("end_image_url"));
    assert!(matches!(
        ApiModel::new(&frames(0, ""), &no_inputs()),
        Err(PortError::Provider(_))
    ));
}

/// References: `reference-to-video` with the images (`@image_1`…) and the source as the one reference video
/// (`@video_1`); Auto with a source is `adaptive`, without one the frame asked for.
#[test]
fn references_are_reference_to_video() {
    let (endpoint, body) = api_body(&on_source(), &inputs(Some(video()), vec![start()]));
    assert_eq!(endpoint, ModelEndpoint::Kling4ReferenceToVideo);
    assert_eq!(
        body,
        r#"{"aspect_ratio":"adaptive","duration":10,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"make it rain","resolution":"720p","video_urls":["https://cdn.example.com/in.mp4"]}"#
    );
    let images = api_body(
        &Clip {
            images: 2,
            aspect_ratio: Some(AspectRatio::Landscape),
            ..Clip::default()
        }
        .spec(),
        &inputs(None, vec![start(), end()]),
    );
    assert_eq!(images.0, ModelEndpoint::Kling4ReferenceToVideo);
    assert_eq!(
        images.1,
        r#"{"aspect_ratio":"16:9","duration":12,"image_urls":["https://cdn.example.com/a.jpg","https://cdn.example.com/b.jpg"],"prompt":"a fox in the snow","resolution":"1080p"}"#
    );
    let auto = api_body(
        &Clip {
            images: 1,
            aspect_ratio: Some(AspectRatio::Auto),
            ..Clip::default()
        }
        .spec(),
        &inputs(None, vec![start()]),
    );
    assert!(auto.1.contains(r#""aspect_ratio":"adaptive""#));
}

/// Edit: `video-edit` with the source as `video_url` and the images as references; its `duration` is the output's,
/// the selection's inside 3–30 s.
#[test]
fn an_edit_is_video_edit() {
    let with_video = inputs(Some(video()), vec![start()]);
    let (endpoint, body) = api_body(&edit(12.0), &with_video);
    assert_eq!(endpoint, ModelEndpoint::Kling4VideoEdit);
    assert_eq!(
        body,
        r#"{"aspect_ratio":"adaptive","duration":12,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"make it rain","resolution":"720p","video_url":"https://cdn.example.com/in.mp4"}"#
    );
    assert!(api_body(&edit(2.0), &with_video).1.contains(r#""duration":3,"#));
    assert!(api_body(&edit(40.0), &with_video).1.contains(r#""duration":30,"#));
    assert!(matches!(
        ApiModel::new(&edit(10.0), &inputs(None, vec![start()])),
        Err(PortError::Provider(_))
    ));
}

/// The app's tags become the schemas': `@ImageN` the N-th image sent, `@Video1` the source; the whole number is
/// read, and a tag past what is sent, or already `@image_N`, stays as written.
#[test]
fn the_tags_are_the_schemas() {
    assert_eq!(
        kling4_prompt("@Image1 and @image10 by @Video1, @Image11", 10, 1),
        "@image_1 and @image_10 by @video_1, @Image11"
    );
    assert_eq!(
        kling4_prompt("@Image1 meets @Image12", 12, 0),
        "@image_1 meets @image_12"
    );
    assert_eq!(kling4_prompt("@Video1 @Image1", 0, 0), "@Video1 @Image1");
    assert_eq!(
        kling4_prompt("@image_2 @ @Image mail@Image2x", 2, 0),
        "@image_2 @ @Image mail@image_2x"
    );
    assert_eq!(kling4_prompt("no tags", 3, 1), "no tags");

    let mut spec = on_source();
    spec.prompt = "@Image1 dances in @Video1".to_owned();
    let reference = api_body(&spec, &inputs(Some(video()), vec![start()]));
    assert!(reference.1.contains(r#""prompt":"@image_1 dances in @video_1""#));
    spec.mode = Mode::Edit;
    let edit = api_body(&spec, &inputs(Some(video()), vec![start()]));
    assert!(edit.1.contains(r#""prompt":"@image_1 dances in @video_1""#));
    // The account gets the prompt as written, as for every account model.
    let account = account_model::request(
        &spec,
        "kling4_0",
        vec![VideoMedia::new(MediaRole::Video, "video-1")],
        &account_model::parameters(),
        &ModelConstraintTable::built_in(),
    );
    assert_eq!(account.prompt, "@Image1 dances in @Video1");
}

/// A blank prompt is left out, as Validation reads it: only `image-to-video` takes none.
#[test]
fn a_blank_prompt_is_left_out() {
    let (_, body) = api_body(&frames(2, "  \n"), &inputs(None, vec![start()]));
    assert!(!body.contains("prompt"));
}

// MARK: Account

/// `kling4_0`'s four `mode`s, each keeping to its `models_explore get`: frames as `start_image` and `end_image`,
/// the source and images as `video_references` and `image_references`, Auto as `auto`.
#[tokio::test]
async fn the_account_sends_each_mode() {
    let (provider, stub) = account_provider(submitted("job-1")).await;
    let jobs = [
        (clip(), no_inputs()),
        (frames(2, ""), inputs(None, vec![start(), end()])),
        (on_source(), inputs(Some(video()), vec![start()])),
        (edit(40.0), inputs(Some(video()), vec![start()])),
        (
            Clip {
                images: 1,
                seconds: 1.0,
                aspect_ratio: Some(AspectRatio::Auto),
                ..Clip::default()
            }
            .spec(),
            inputs(None, vec![start()]),
        ),
    ];
    for (spec, inputs) in &jobs {
        provider.submit(spec, inputs, Uuid::new_v4()).await.unwrap();
    }
    let params = generate_params(&stub).await;
    assert_eq!(params.len(), jobs.len());
    assert_params_keep_to_snapshot(&params);
    assert_json(
        &column(&params, "mode"),
        &json!([
            "text-to-video",
            "image-to-video",
            "reference-to-video",
            "video-edit",
            "reference-to-video"
        ]),
    );
    assert!(params.iter().all(|params| params["model"] == "kling4_0"));

    assert!(params[0]["aspect_ratio"] == "9:16" && params[0]["resolution"] == "1080p");
    assert_json(&params[0]["duration"], &json!(12));
    assert_eq!(params[0]["prompt"], "a fox in the snow");
    assert!(params[0]["medias"].as_array().is_none_or(Vec::is_empty));
    assert_json(
        &params[1]["medias"],
        &json!([{"role": "start_image", "value": "image-1"}, {"role": "end_image", "value": "image-2"}]),
    );
    assert_eq!(params[1]["aspect_ratio"], "9:16");
    assert_json(&params[1]["duration"], &json!(5));
    assert_json(
        &params[2],
        &json!({
            "model": "kling4_0", "mode": "reference-to-video",
            "medias": [
                {"role": "video_references", "value": "video-1"},
                {"role": "image_references", "value": "image-1"},
            ],
            "prompt": "make it rain", "resolution": "720p", "duration": 10, "aspect_ratio": "auto",
            "use_unlim": false,
        }),
    );
    assert_json(&params[3]["duration"], &json!(30));
    assert_eq!(params[3]["aspect_ratio"], "auto");
    assert_eq!(params[3]["medias"][0]["role"], "video_references");
    assert_json(&params[4]["duration"], &json!(3));
    assert_eq!(params[4]["aspect_ratio"], "auto");
}

// MARK: Validation

/// Up to 10 images, more are refused; a prompt is needed except from frames; Edit needs its clip.
#[test]
fn validation_keeps_to_the_schemas() {
    let too_many = |maximum| ValidationIssue::TooManyReferences { maximum };
    let images = |images| Clip {
        images,
        ..Clip::default()
    };
    assert_none(&check(&clip()));
    assert_none(&check(&images(10).spec()));
    assert!(check(&images(11).spec()).contains(&too_many(10)));
    let sourced = |images| OnSource {
        images,
        ..OnSource::default()
    };
    assert_none(&check(&sourced(10).spec()));
    assert!(check(&sourced(11).spec()).contains(&too_many(10)));
    // `reference-to-video` needs a prompt and an image or the video; without them it is text-to-video, which needs
    // the prompt alone.
    let unprompted = Clip {
        images: 2,
        prompt: "",
        ..Clip::default()
    };
    assert_eq!(check(&unprompted.spec()), [ValidationIssue::PromptRequired]);
    let blank = Clip {
        prompt: " ",
        ..Clip::default()
    };
    assert_eq!(check(&blank.spec()), [ValidationIssue::PromptRequired]);
    let silent = OnSource {
        prompt: "",
        ..OnSource::default()
    };
    assert_eq!(check(&silent.spec()), [ValidationIssue::PromptRequired]);
    // `image-to-video` takes no prompt, but needs its start frame.
    assert_none(&check(&frames(2, "")));
    assert!(check(&frames(0, "")).contains(&ValidationIssue::TooFewReferences { minimum: 1 }));
    assert!(check(&frames(3, "")).contains(&too_many(2)));
    // `video-edit` needs the clip and a prompt.
    let mut edit_spec = edit(10.0);
    assert_none(&check(&edit_spec));
    edit_spec.source = None;
    assert!(check(&edit_spec).contains(&ValidationIssue::SourceRequired));
    let unprompted_edit = OnSource {
        mode: Mode::Edit,
        prompt: "",
        ..OnSource::default()
    };
    assert!(check(&unprompted_edit.spec()).contains(&ValidationIssue::PromptRequired));
    assert!(check(&edit(31.0)).contains(&ValidationIssue::DurationTooLong(31.0)));
    let short = OnSource {
        seconds: 2.0,
        ..OnSource::default()
    };
    assert!(check(&short.spec()).contains(&ValidationIssue::DurationTooShort(2.0)));
}

// MARK: Prices

/// The REST list prices: $0.157 and $0.235 a second without an input video; with one, $0.094 and $0.141 on the
/// output's seconds plus as many of the input's, which are the selection's.
#[test]
fn rest_list_prices() {
    let pricing = Pricing::bundled().unwrap();
    for (resolution, with_video, price) in [
        (Resolution::P720, false, "1.57"),
        (Resolution::P1080, false, "2.35"),
        (Resolution::P720, true, "1.88"),
        (Resolution::P1080, true, "2.82"),
    ] {
        let expected = usd(price, true);
        let mut spec = if with_video {
            on_source()
        } else {
            Clip {
                images: 1,
                seconds: 10.0,
                ..Clip::default()
            }
            .spec()
        };
        spec.resolution = resolution;
        assert_eq!(
            pricing.cost(&spec).unwrap(),
            expected,
            "{resolution:?} {with_video}"
        );
        if with_video {
            let mut edit_spec = edit(10.0);
            edit_spec.resolution = resolution;
            assert_eq!(pricing.cost(&edit_spec).unwrap(), expected);
        } else {
            spec.references = Vec::new();
            assert_eq!(pricing.cost(&spec).unwrap(), expected);
            let mut framed = frames(2, "");
            framed.resolution = resolution;
            framed.length = Some(10.0);
            assert_eq!(pricing.cost(&framed).unwrap(), expected);
        }
    }
}

/// On the account: 5 credits a second at 720p and 7.5 at 1080p, Higgsfield's announced price, until a quote; with a
/// source video (references or Edit) 1.2 times that, as REST bills the input video too.
#[test]
#[allow(clippy::cast_precision_loss, reason = "a few dozen credits")]
fn the_account_estimates_from_the_announced_price() {
    let mut spec = Clip {
        seconds: 10.0,
        ..Clip::default()
    }
    .spec();
    assert_eq!(estimated(&spec), Some(75));
    spec.length = Some(45.0);
    assert_eq!(estimated(&spec), Some(225));
    let mut images = Clip {
        images: 2,
        seconds: 10.0,
        ..Clip::default()
    }
    .spec();
    images.resolution = Resolution::P720;
    assert_eq!(estimated(&images), Some(50));
    assert_eq!(estimated(&frames(2, "")), Some(25));

    // Never below the announced rate times REST's ratio with an input video to without (10 s here).
    let mut reference = on_source();
    let at720 = estimated(&reference).unwrap();
    assert!(at720 == 60 && at720 as f64 >= 5.0 * 10.0 * 0.188 / 0.157);
    reference.resolution = Resolution::P1080;
    let at1080 = estimated(&reference).unwrap();
    assert!(at1080 == 90 && at1080 as f64 >= (7.5 * 10.0 * 0.282 / 0.235_f64).round());
    let mut edit_spec = edit(12.0);
    assert_eq!(estimated(&edit_spec), Some(72));
    edit_spec.resolution = Resolution::P1080;
    assert_eq!(estimated(&edit_spec), Some(108));
}

/// A prompt alone is quoted before anything is uploaded, with its `mode`; a job with a source, frames or images
/// only once they are, since the media name the `mode` and may change the price.
#[tokio::test]
async fn only_a_prompt_alone_is_quoted_before_upload() {
    let model = "kling4_0";
    let request = quote(&clip(), model).unwrap();
    assert_eq!(request.mode.as_deref(), Some(VideoRequest::KLING4_TEXT_TO_VIDEO));
    assert!(request.resolution.as_deref() == Some("1080p") && request.duration == Some(12.0));
    let with_images = Clip {
        images: 1,
        ..Clip::default()
    };
    for spec in [frames(2, ""), with_images.spec(), on_source(), edit(10.0)] {
        assert!(quote(&spec, model).is_none());
    }
    let (provider, stub) = account_provider(json!({})).await;
    assert_eq!(
        provider.estimate(&on_source()).await.unwrap(),
        credits("60", true)
    );
    assert_none(&calls(&stub).await);
}
