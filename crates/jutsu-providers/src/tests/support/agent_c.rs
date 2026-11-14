//! Helpers of the tests agent c ported: Kling 4.0's jobs (`Network.Kling4`, which the Flash, Gemini and Intone
//! suites reuse), the REST body as the Swift tests encode it, and an account provider whose `generate_video` calls
//! they read.

use std::str::FromStr;

use higgsfield::api::ModelEndpoint;
use higgsfield::mcp::account::VideoRequest;
use jutsu_core::{
    AspectRatio, ClosedRange, Cost, JobSpec, LocalImage, Mode, ModelConstraintTable, PixelSize, RemoteAsset,
    RemoteInputs, Resolution, SourceClip, Validation, ValidationIssue, VideoModel,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use url::Url;

use super::account::{calls, full_set};
use super::catalog;
use super::mcp::McpServer;
use super::provider::make_provider;
use super::stub::Stub;
use crate::HiggsfieldMcpProvider;
use crate::account_model;
use crate::api_model::ApiModel;

pub fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

/// A local file, as Swift's `URL(fileURLWithPath:)`.
pub fn file(path: &str) -> Url {
    url(&format!("file://{path}"))
}

/// `count` copies of the local image at `path`.
pub fn images(count: usize, path: &str) -> Vec<LocalImage> {
    vec![LocalImage::new(file(path)); count]
}

/// The first `seconds` of the file at `path`.
pub fn source(path: &str, duration: f64, size: PixelSize, seconds: f64) -> SourceClip {
    SourceClip::new(file(path), duration, size, ClosedRange::new(0.0, seconds))
}

pub fn asset(link: &str, id: &str) -> RemoteAsset {
    RemoteAsset {
        url: url(link),
        id: Some(id.to_owned()),
    }
}

pub fn inputs(video: Option<RemoteAsset>, images: Vec<RemoteAsset>) -> RemoteInputs {
    RemoteInputs {
        video,
        images,
        audio: None,
    }
}

pub fn no_inputs() -> RemoteInputs {
    inputs(None, Vec::new())
}

pub fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

pub fn usd(text: &str, is_estimate: bool) -> Cost {
    Cost::usd(decimal(text), is_estimate)
}

pub fn credits(text: &str, is_estimate: bool) -> Cost {
    Cost::credits(decimal(text), is_estimate)
}

/// Swift's `Validation.check(spec)`: the built-in table.
pub fn check(spec: &JobSpec) -> Vec<ValidationIssue> {
    Validation::check(spec, &ModelConstraintTable::built_in())
}

/// Swift's `AccountModel.estimatedCredits(for:)`.
pub fn estimated(spec: &JobSpec) -> Option<i64> {
    account_model::estimated_credits(spec, &ModelConstraintTable::built_in())
}

/// Swift's `AccountModel.quoteRequest(for:model:)`.
pub fn quote(spec: &JobSpec, model: &str) -> Option<VideoRequest> {
    account_model::quote_request(spec, model, &ModelConstraintTable::built_in())
}

/// The REST body as Swift encodes it: keys sorted, slashes unescaped.
pub fn body(request: &ApiModel) -> String {
    let mut value = serde_json::to_value(&request.body).unwrap();
    value.sort_all_objects();
    serde_json::to_string(&value).unwrap()
}

/// `Network.Kling4.body(of:inputs:)`.
pub fn api_body(spec: &JobSpec, inputs: &RemoteInputs) -> (ModelEndpoint, String) {
    let request = ApiModel::new(spec, inputs).unwrap();
    (request.endpoint, body(&request))
}

/// `items` is empty; on failure the message shows them.
#[track_caller]
pub fn assert_none<T: std::fmt::Debug + PartialEq>(items: &[T]) {
    assert_eq!(items, &[] as &[T]);
}

/// Two JSON values are equal with numbers compared by value, as Swift's `JSONValue` compares them.
#[track_caller]
pub fn assert_json(actual: &Value, expected: &Value) {
    assert!(catalog::same(actual, expected), "{actual} != {expected}");
}

/// `Provider.provider(tools: Provider.fullSet) { _, _ in answer }`.
pub async fn account_provider(answer: Value) -> (HiggsfieldMcpProvider, Stub) {
    let stub = McpServer::new(full_set())
        .call(move |_, _| answer.clone())
        .serve()
        .await;
    (make_provider(&stub, false), stub)
}

/// `{"results": [{"id": id}]}`, a `generate_video` answer.
pub fn submitted(id: &str) -> Value {
    json!({"results": [{"id": id}]})
}

/// The `generate_video` calls the stub got.
pub async fn generate_calls(stub: &Stub) -> Vec<Value> {
    calls(stub)
        .await
        .into_iter()
        .filter(|call| call["name"] == "generate_video")
        .collect()
}

/// Their `params`.
pub async fn generate_params(stub: &Stub) -> Vec<Value> {
    generate_calls(stub)
        .await
        .iter()
        .filter(|call| call["arguments"]["params"].is_object())
        .map(|call| call["arguments"]["params"].clone())
        .collect()
}

/// `params[key]` of each, `null` where missing.
pub fn column(params: &[Value], key: &str) -> Value {
    Value::Array(params.iter().map(|params| params[key].clone()).collect())
}

/// Each `params` keeps to its model's `models_explore get` in the snapshot.
#[track_caller]
pub fn assert_params_keep_to_snapshot(params: &[Value]) {
    for sent in params {
        let found = catalog::violations_of_params(sent);
        assert!(found.is_empty(), "{found:?}");
    }
}

/// Kling 4.0's jobs and inputs (`Network.Kling4`).
pub mod kling4 {
    use super::{
        AspectRatio, JobSpec, Mode, PixelSize, RemoteAsset, Resolution, VideoModel, asset, images, source,
    };

    pub fn kling() -> VideoModel {
        VideoModel::from_raw("kling40").unwrap()
    }

    pub fn video() -> RemoteAsset {
        asset("https://cdn.example.com/in.mp4", "video-1")
    }

    pub fn start() -> RemoteAsset {
        asset("https://cdn.example.com/a.jpg", "image-1")
    }

    pub fn end() -> RemoteAsset {
        asset("https://cdn.example.com/b.jpg", "image-2")
    }

    /// A clip from a prompt alone, or from images when there are some.
    #[derive(Debug, Clone)]
    pub struct Clip {
        pub images: usize,
        pub prompt: &'static str,
        pub seconds: f64,
        pub resolution: Resolution,
        pub aspect_ratio: Option<AspectRatio>,
    }

    impl Default for Clip {
        fn default() -> Self {
            Self {
                images: 0,
                prompt: "a fox in the snow",
                seconds: 12.0,
                resolution: Resolution::P1080,
                aspect_ratio: Some(AspectRatio::Portrait),
            }
        }
    }

    impl Clip {
        pub fn spec(&self) -> JobSpec {
            let mut spec = JobSpec::new(Mode::MotionTransfer, kling(), None, self.resolution);
            spec.references = images(self.images, "/tmp/ref.png");
            self.prompt.clone_into(&mut spec.prompt);
            spec.length = Some(self.seconds);
            spec.aspect_ratio = self.aspect_ratio;
            spec
        }
    }

    pub fn clip() -> JobSpec {
        Clip::default().spec()
    }

    /// A clip from a start frame and, with two images, an end frame.
    pub fn frames(count: usize, prompt: &'static str) -> JobSpec {
        let mut spec = Clip {
            images: count,
            prompt,
            seconds: 5.0,
            resolution: Resolution::P720,
            ..Clip::default()
        }
        .spec();
        spec.references_are_frames = true;
        spec
    }

    /// A job on a source: its references (Motion Transfer) or an Edit of it.
    #[derive(Debug, Clone)]
    pub struct OnSource {
        pub mode: Mode,
        pub seconds: f64,
        pub images: usize,
        pub prompt: &'static str,
    }

    impl Default for OnSource {
        fn default() -> Self {
            Self {
                mode: Mode::MotionTransfer,
                seconds: 10.0,
                images: 1,
                prompt: "make it rain",
            }
        }
    }

    impl OnSource {
        pub fn spec(&self) -> JobSpec {
            let clip = source("/tmp/in.mp4", 60.0, PixelSize::new(1280, 720), self.seconds);
            let mut spec = JobSpec::new(self.mode, kling(), Some(clip), Resolution::P720);
            spec.references = images(self.images, "/tmp/ref.png");
            self.prompt.clone_into(&mut spec.prompt);
            spec
        }
    }

    pub fn on_source() -> JobSpec {
        OnSource::default().spec()
    }

    /// An Edit of the source's first `seconds`.
    pub fn edit(seconds: f64) -> JobSpec {
        OnSource {
            mode: Mode::Edit,
            seconds,
            ..OnSource::default()
        }
        .spec()
    }
}
