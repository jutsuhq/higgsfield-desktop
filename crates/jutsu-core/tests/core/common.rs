//! Fixtures shared by the ported tests (`Fixtures.swift`).

use std::str::FromStr;

use jutsu_core::*;
use rust_decimal::Decimal;
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

/// A decimal from its text.
pub fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

/// Dollars, exact.
pub fn usd(text: &str, is_estimate: bool) -> Cost {
    Cost::usd(dec(text), is_estimate)
}

/// Credits.
pub fn credits(text: &str, is_estimate: bool) -> Cost {
    Cost::credits(dec(text), is_estimate)
}

/// A `file://` URL of `path`, as `URL(fileURLWithPath:)`.
pub fn file_url(path: &str) -> Url {
    let mut url = Url::parse("file:///").unwrap();
    url.set_path(path);
    url
}

/// `makeSpec`'s arguments.
pub struct SpecArgs {
    pub mode: Mode,
    pub duration: f64,
    pub size: PixelSize,
    pub allows_upscale: bool,
    pub references: usize,
    pub preset_id: Option<Uuid>,
    pub prompt: String,
    pub resolution: Resolution,
}

impl Default for SpecArgs {
    fn default() -> Self {
        Self {
            mode: Mode::MotionTransfer,
            duration: 10.0,
            size: PixelSize::new(1280, 720),
            allows_upscale: false,
            references: 1,
            preset_id: None,
            prompt: String::new(),
            resolution: Resolution::P480,
        }
    }
}

/// A Genjutsu job on a 60 s, 1280×720 source with one reference.
pub fn make_spec() -> JobSpec {
    spec_with(SpecArgs::default())
}

/// `makeSpec` with other arguments.
pub fn spec_with(args: SpecArgs) -> JobSpec {
    let mut source = SourceClip::new(
        file_url("/tmp/source.mov"),
        60.0,
        args.size,
        ClosedRange::new(0.0, args.duration),
    );
    source.allows_upscale = args.allows_upscale;
    let mut spec = JobSpec::new(args.mode, VideoModel::genjutsu(), Some(source), args.resolution);
    spec.references = (0..args.references)
        .map(|i| LocalImage::new(file_url(&format!("/tmp/ref{i}.jpg"))))
        .collect();
    spec.preset_id = args.preset_id;
    spec.prompt = args.prompt;
    spec
}

/// A spec on `model` with no source.
pub fn bare_spec(mode: Mode, model: VideoModel, resolution: Resolution) -> JobSpec {
    JobSpec::new(mode, model, None, resolution)
}

/// `Job(spec:providerKind:cost:)`: new IDs, made now.
pub fn new_job(spec: JobSpec, kind: ProviderKind, cost: Cost) -> Job {
    Job::new(
        Uuid::new_v4(),
        spec,
        kind,
        cost,
        Uuid::new_v4(),
        OffsetDateTime::now_utc(),
    )
}
