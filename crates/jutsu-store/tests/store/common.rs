//! Fixtures shared by the ported tests.

use std::path::Path;

use jutsu_core::{
    ClosedRange, Cost, Job, JobSpec, JobState, LocalImage, MaterialGeneration, Mode, PixelSize, ProviderKind,
    Resolution, SavedImage, SourceClip, VideoModel,
};
use jutsu_store::Store;
use rust_decimal::Decimal;
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

/// A `file://` URL of `path`, as `URL(fileURLWithPath:)`.
pub fn file_url(path: impl AsRef<Path>) -> Url {
    let path = path.as_ref();
    if path.is_absolute() {
        return Url::from_file_path(path).unwrap();
    }
    // The tests' Unix paths get a drive on Windows.
    Url::from_file_path(format!("C:{}", path.to_string_lossy().replace('/', "\\"))).unwrap()
}

/// `Date(timeIntervalSince1970:)`.
pub fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap()
}

/// A Genjutsu job on a 60 s, 1280×720 source with one reference and the prompt "a cat", at 720p, priced at an
/// estimated 12 credits (`makeJob`).
pub fn make_job(mode: Mode, created_at: OffsetDateTime, state: JobState) -> Job {
    let source = SourceClip::new(
        file_url("/tmp/source.mov"),
        60.0,
        PixelSize::new(1280, 720),
        ClosedRange::new(0.0, 10.0),
    );
    let mut spec = JobSpec::new(mode, VideoModel::genjutsu(), Some(source), Resolution::P720);
    spec.references = vec![LocalImage::new(file_url("/tmp/ref.jpg"))];
    "a cat".clone_into(&mut spec.prompt);
    let mut job = Job::new(
        Uuid::new_v4(),
        spec,
        ProviderKind::ApiKey,
        Cost::credits(Decimal::from(12), true),
        Uuid::new_v4(),
        created_at,
    );
    job.state = state;
    job
}

/// `makeJob(state:)`, queued now.
pub fn job(state: JobState) -> Job {
    make_job(Mode::MotionTransfer, OffsetDateTime::now_utc(), state)
}

pub async fn memory_store() -> Store {
    Store::open_in_memory().await.unwrap()
}

/// An image made with the prompt "known prompt" on Soul at 16:9, at `folder/<name>.png`.
pub fn saved_image(folder: &Path, name: &str) -> SavedImage {
    SavedImage {
        id: Uuid::new_v4(),
        file_url: file_url(folder.join(format!("{name}.png"))),
        prompt: "known prompt".to_owned(),
        model: "soul".to_owned(),
        aspect_ratio: Some("16:9".to_owned()),
        created_at: at(1_000),
        remote_url: None,
        is_starred: false,
        title: None,
    }
}

pub fn generation(prompt: &str, model: &str, parameters: Option<&str>) -> MaterialGeneration {
    MaterialGeneration {
        prompt: prompt.to_owned(),
        model: model.to_owned(),
        parameters: parameters.map(|text| text.as_bytes().to_vec()),
        title: None,
    }
}

/// Writes `contents` at `path`, creating its folder.
pub fn write_file(path: &Path, contents: &str) -> Url {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
    file_url(path)
}
