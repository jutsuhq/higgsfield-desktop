//! `HistoryItemTests.swift`.

use jutsu_core::*;
use time::OffsetDateTime;
use url::Url;

use crate::common::*;

fn asset(url: &str, id: &str) -> RemoteAsset {
    let mut asset = RemoteAsset::new(Url::parse(url).unwrap());
    asset.id = Some(id.to_owned());
    asset
}

pub fn make_history_item() -> HistoryItem {
    let cdn = "https://cdn.example";
    let inputs = RemoteInputs {
        video: Some(asset(&format!("{cdn}/in.mp4"), "media-v")),
        images: vec![asset(&format!("{cdn}/a.png"), "media-a")],
        audio: None,
    };
    let mut item = HistoryItem::new(
        "job-1",
        Mode::ObjectSwap,
        VideoModel::genjutsu(),
        OffsetDateTime::from_unix_timestamp(1_791_032_606).unwrap(),
        Url::parse(&format!("{cdn}/job-1.mp4")).unwrap(),
        inputs,
    );
    item.duration = Some(8.0);
    item.pixel_size = Some(PixelSize::new(1080, 1920));
    item.resolution = Some(Resolution::P720);
    item.prompt = "swap the cup".into();
    item
}

#[test]
fn imported_history_item_is_a_finished_account_job_with_its_remote_id() {
    let item = make_history_item();
    let now = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();

    let job = Job::importing(&item, Some("user_1".into()), now);

    assert_eq!(job.id, item.job_id);
    assert!(job.state == JobState::Done && (job.progress - 1.0).abs() < f64::EPSILON);
    assert!(job.provider_kind == ProviderKind::Account && job.owner.as_deref() == Some("user_1"));
    assert_eq!(job.remote, Some(RemoteJob::new("job-1")));
    assert_eq!(job.inputs.as_ref(), Some(&item.inputs));
    assert_eq!(job.imported_at, Some(now));
    assert!(job.created_at == item.created_at && job.finished_at == Some(item.created_at));
    assert_eq!(job.output_file, None);
    assert_eq!(*job.price(), credits("0", true));
    assert_eq!(job.spec.remote_inputs.as_ref(), Some(&item.inputs));
    let source = job.spec.source.as_ref().unwrap();
    assert_eq!(
        Some(&source.file_url),
        item.inputs.video.as_ref().map(|video| &video.url)
    );
    assert_eq!(source.range, ClosedRange::new(0.0, 8.0));
    assert_eq!(source.pixel_size, PixelSize::new(1080, 1920));
    assert_eq!(
        job.spec
            .references
            .iter()
            .map(|image| &image.file_url)
            .collect::<Vec<_>>(),
        item.inputs
            .images
            .iter()
            .map(|image| &image.url)
            .collect::<Vec<_>>()
    );
    assert!(
        job.spec.mode == Mode::ObjectSwap
            && job.spec.resolution == Resolution::P720
            && job.spec.prompt == "swap the cup"
    );
}

#[test]
#[allow(clippy::float_cmp, reason = "the probed length is kept as it came")]
fn history_spec_uses_the_probed_source_whole() {
    let spec = make_history_item().spec(Some(14.2), Some(PixelSize::new(1920, 1080)));
    let source = spec.source.unwrap();
    assert_eq!(source.duration, 14.2);
    assert_eq!(source.range, ClosedRange::new(0.0, 14.2));
    assert_eq!(source.pixel_size, PixelSize::new(1920, 1080));
}

#[test]
fn job_stored_before_imports_and_reused_inputs_decodes_without_them() {
    let mut job = Job::importing(
        &make_history_item(),
        Some("user_1".into()),
        OffsetDateTime::from_unix_timestamp(5).unwrap(),
    );
    let mut object = serde_json::to_value(&job).unwrap();
    let map = object.as_object_mut().unwrap();
    assert!(map.remove("importedAt").is_some());
    assert!(
        map["spec"]
            .as_object_mut()
            .unwrap()
            .remove("remoteInputs")
            .is_some()
    );

    let decoded: Job = serde_json::from_value(object).unwrap();

    assert_eq!(decoded.imported_at, None);
    assert_eq!(decoded.spec.remote_inputs, None);
    assert_eq!(decoded.id, job.id);
    job.output_file = Some(file_url("/tmp/out.mp4"));
    assert_eq!(
        serde_json::from_str::<Job>(&serde_json::to_string(&job).unwrap()).unwrap(),
        job
    );
}
