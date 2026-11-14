//! Port of `KlingO3ProviderTests.swift`: Kling O3's video reference, a catalog entry and the `klingVideoReference`
//! request shape, API key only.

use higgsfield::api::ModelEndpoint;
use jutsu_core::{
    ClosedRange, JobSpec, LocalImage, Mode, PixelSize, RemoteAsset, RemoteInputs, Resolution, SourceClip,
    VideoModel,
};
use serde_json::{Value, json};

use super::support::agent_a::url;
use crate::account_model;
use crate::api_model::ApiModel;

fn model() -> VideoModel {
    VideoModel::from_raw("klingO3VideoReference").unwrap()
}

fn spec(seconds: f64, resolution: Resolution, references: usize) -> JobSpec {
    let source = SourceClip::new(
        url("file:///tmp/in.mp4"),
        30.0,
        PixelSize {
            width: 1280,
            height: 720,
        },
        ClosedRange::new(2.0, 2.0 + seconds),
    );
    let mut spec = JobSpec::new(Mode::MotionTransfer, model(), Some(source), resolution);
    spec.references = (0..references)
        .map(|index| LocalImage::new(url(&format!("file:///tmp/ref{index}.png"))))
        .collect();
    spec.prompt = "she spins".to_owned();
    spec
}

fn body(request: &ApiModel) -> Value {
    serde_json::to_value(&request.body).unwrap()
}

/// The source is the reference video, the selection's seconds the clip's (inside 3–10 s), the quality the mode.
#[test]
fn the_request_carries_the_video_the_images_and_the_quality() {
    let inputs = RemoteInputs {
        video: Some(RemoteAsset::new(url("https://cdn.example.com/in.mp4"))),
        images: vec![RemoteAsset::new(url("https://cdn.example.com/a.jpg"))],
        audio: None,
    };

    let standard = ApiModel::new(&spec(5.5, Resolution::P720, 1), &inputs).unwrap();
    assert_eq!(standard.endpoint, ModelEndpoint::KlingO3VideoReference);
    assert_eq!(
        body(&standard),
        json!({
            "duration": 6, "image_urls": ["https://cdn.example.com/a.jpg"], "mode": "std", "prompt": "she spins",
            "video_urls": ["https://cdn.example.com/in.mp4"],
        })
    );
    let pro = ApiModel::new(&spec(14.0, Resolution::P1080, 1), &inputs).unwrap();
    assert_eq!(body(&pro)["duration"], json!(10));
    assert_eq!(body(&pro)["mode"], json!("pro"));
}

/// The account's model ID for it isn't known yet (issue #37): the account neither offers nor sends it.
#[test]
fn the_account_doesnt_run_it() {
    assert!(!account_model::runs(&model()));
    assert_eq!(account_model::model(&spec(5.0, Resolution::P720, 1)), None);
}
