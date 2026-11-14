//! `LibraryFixture`, `CatalogFixture` and `HistoryFixture` of the macOS tests.

use std::str::FromStr;

use jutsu_core::{
    ClosedRange, Cost, Culling, HistoryImageItem, HistoryItem, Job, JobSpec, JobState, LocalImage, Material,
    MaterialGeneration, MaterialKind, MaterialVersion, Mode, PixelSize, ProviderKind, RemoteAsset,
    RemoteInputs, RemoteJob, Resolution, SourceClip, VideoModel,
};
use rust_decimal::Decimal;
use time::{Duration, OffsetDateTime};
use url::Url;
use uuid::Uuid;

use super::item::LibraryItem;
use super::media::{MediaRow, material_row};

pub fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_000_000).unwrap()
}

pub fn minutes_ago(minutes: f64) -> OffsetDateTime {
    now() - Duration::seconds_f64(minutes * 60.0)
}

pub fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

pub fn file(path: &str) -> Url {
    Url::from_file_path(path).unwrap()
}

/// The fields `LibraryFixture.item` varies.
pub struct ItemFixture {
    pub mode: Mode,
    pub source: &'static str,
    pub range: (f64, f64),
    pub resolution: Resolution,
    pub is_draft: bool,
    pub prompt: &'static str,
    pub references: usize,
    pub cost: Cost,
    pub via: ProviderKind,
    pub preset_name: Option<&'static str>,
    pub minutes_ago: f64,
    pub queued_minutes_ago: f64,
}

impl Default for ItemFixture {
    fn default() -> Self {
        Self {
            mode: Mode::Restyle,
            source: "rooftop.mov",
            range: (4.0, 10.0),
            resolution: Resolution::P480,
            is_draft: true,
            prompt: "",
            references: 0,
            cost: Cost::usd(dec("1.908"), false),
            via: ProviderKind::ApiKey,
            preset_name: None,
            minutes_ago: 36.0,
            queued_minutes_ago: 60.0,
        }
    }
}

impl ItemFixture {
    pub fn item(self) -> LibraryItem {
        let clip = SourceClip::new(
            file(&format!("/home/me/Movies/{}", self.source)),
            30.0,
            PixelSize::new(1920, 1080),
            ClosedRange::new(self.range.0, self.range.1),
        );
        let mut spec = JobSpec::new(self.mode, VideoModel::GENJUTSU, Some(clip), self.resolution);
        spec.references = (0..self.references)
            .map(|index| LocalImage::new(file(&format!("/tmp/ref{index}.png"))))
            .collect();
        spec.preset_id = self.preset_name.map(|_| Uuid::new_v4());
        spec.prompt = self.prompt.to_owned();
        spec.is_draft = self.is_draft;
        let mut job = Job::new(
            Uuid::new_v4(),
            spec,
            self.via,
            self.cost,
            Uuid::new_v4(),
            minutes_ago(self.queued_minutes_ago),
        );
        job.state = JobState::Done;
        job.finished_at = Some(minutes_ago(self.minutes_ago));
        job.output_file = Some(file("/home/me/Movies/Jutsu Studio/out.mp4"));
        let mut remote = RemoteJob::new("req-123".to_owned());
        remote.correlation_id = Some("corr-9".to_owned());
        job.remote = Some(remote);
        let mut item = LibraryItem::new(job, false);
        item.preset_name = self.preset_name.map(str::to_owned);
        item
    }
}

/// `LibraryFixture.item()` with the defaults.
pub fn item() -> LibraryItem {
    ItemFixture::default().item()
}

/// A history item made 2 h before `now`, Genjutsu Object swap, 8 s at 720p.
pub fn history_item(id: &str, is_liked: bool) -> HistoryItem {
    let mut item = HistoryItem::new(
        id,
        Mode::ObjectSwap,
        VideoModel::GENJUTSU,
        minutes_ago(120.0),
        history_url(&format!("{id}.mp4")),
        RemoteInputs {
            video: Some(RemoteAsset::new(history_url("source.mp4"))),
            images: Vec::new(),
            audio: None,
        },
    );
    item.is_liked = is_liked;
    item.duration = Some(8.0);
    item.resolution = Some(Resolution::P720);
    item.prompt = "spin".to_owned();
    item
}

pub fn history_url(name: &str) -> Url {
    Url::parse(&format!("https://cdn.example.com/{name}")).unwrap()
}

pub fn history_image(id: &str, minutes: f64) -> HistoryImageItem {
    HistoryImageItem {
        id: id.to_owned(),
        job_id: Uuid::new_v4(),
        prompt: "A knight in armor".to_owned(),
        model: "soul-2.0".to_owned(),
        created_at: minutes_ago(minutes),
        is_liked: false,
        output_url: history_url(&format!("{id}.png")),
        pixel_size: Some(PixelSize::new(1024, 1024)),
    }
}

/// `CatalogFixture.video`.
pub fn catalog_video(
    prompt: &'static str,
    group: Option<Uuid>,
    minutes: f64,
    culling: Culling,
    primary: bool,
) -> Material {
    let mut job = ItemFixture {
        prompt,
        minutes_ago: minutes,
        ..ItemFixture::default()
    }
    .item()
    .job;
    job.variant_group_id = group;
    let mut version = MaterialVersion::new(job.id, job.finished_at.unwrap_or(job.created_at));
    version.file_url.clone_from(&job.output_file);
    version.generation = Some(generation(prompt, job.spec.model.raw_value()));
    version.culling = culling;
    let id = job.id;
    let created = job.created_at;
    version.video = Some(Box::new(job));
    let mut material = Material::new(id, MaterialKind::Video, created, version);
    material.variant_group_id = group;
    material.is_variant_primary = primary;
    material
}

pub fn video(minutes: f64) -> Material {
    catalog_video("", None, minutes, Culling::Unreviewed, false)
}

/// `CatalogFixture.image`.
pub fn catalog_image(
    prompt: &str,
    group: Option<Uuid>,
    minutes: f64,
    culling: Culling,
    primary: bool,
) -> Material {
    let id = Uuid::new_v4();
    let mut version = MaterialVersion::new(id, minutes_ago(minutes));
    version.file_url = Some(file(&format!("/tmp/{}.png", Uuid::new_v4())));
    version.generation = Some(generation(prompt, "soul"));
    version.culling = culling;
    let mut material = Material::new(id, MaterialKind::Image, minutes_ago(minutes), version);
    material.variant_group_id = group;
    material.is_variant_primary = primary;
    material
}

pub fn image(minutes: f64) -> Material {
    catalog_image("A knight", None, minutes, Culling::Unreviewed, false)
}

pub fn generation(prompt: &str, model: &str) -> MaterialGeneration {
    MaterialGeneration {
        prompt: prompt.to_owned(),
        model: model.to_owned(),
        parameters: None,
        title: None,
    }
}

pub fn rows(materials: &[Material]) -> Vec<MediaRow> {
    materials.iter().filter_map(material_row).collect()
}

pub fn ids(rows: &[MediaRow]) -> Vec<Uuid> {
    rows.iter().map(MediaRow::id).collect()
}
