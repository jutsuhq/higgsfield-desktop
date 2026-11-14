//! The form under the account's cached limits (the form half of `ModelConstraintStoreTests.swift`).

use jutsu_core::{ClosedRange, LocalImage, Mode, ModeLimits, PixelSize, Resolution, VideoModel};
use time::OffsetDateTime;
use url::Url;

use super::{ModelConstraintCache, ModelConstraintStore};
use crate::create::form::{CreateForm, SourceVideo, time_range};

fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_760_515_200).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// The cache file as the app writes it, with `entries` for `account|user_1`.
fn write_fixture(cache: &ModelConstraintCache, entries: &str) {
    let json = format!(
        r#"{{"version": 1, "accounts": {{"account|user_1": {{"fetchedAt": "2026-10-15T08:00:00Z", "table": {{"entries": {entries}}}}}}}}}"#
    );
    std::fs::write(cache.file(), json).unwrap();
}

fn video(duration: f64) -> SourceVideo {
    SourceVideo {
        file_url: Url::from_file_path("/tmp/clip.mp4").unwrap(),
        duration,
        pixel_size: PixelSize::new(1920, 1080),
        codec: "avc1".to_owned(),
        file_size: 38_000_000,
        remote_origin: None,
    }
}

fn image(index: usize) -> LocalImage {
    LocalImage::new(Url::from_file_path(format!("/tmp/ref{index}.png")).unwrap())
}

fn seedance_form(seconds: f64) -> CreateForm {
    let mut form = CreateForm::new(Mode::MotionTransfer, Resolution::P720, Resolution::P480);
    form.set_model(VideoModel::SEEDANCE25);
    form.set_source(Some(video(seconds)));
    form.add_references(vec![image(1)]);
    form
}

#[tokio::test]
async fn a_limit_changed_in_the_cached_json_changes_what_create_allows() {
    let folder = tempfile::tempdir().unwrap();
    let cache = ModelConstraintCache::new(folder.path());
    let store = ModelConstraintStore::new(cache.clone(), now);
    write_fixture(
        &cache,
        r#"[{"model": "seedance25", "mode": "motionTransfer", "constraints": {"maxDuration": 45}}]"#,
    );
    store.switch_account(Some("account|user_1")).await;

    let mut form = seedance_form(40.0);
    // The constants, before the app hands the table over.
    assert_eq!(form.range(), time_range(0.0, 30.0));
    form.set_constraints(store.table());
    assert_eq!(form.limits().duration, ClosedRange::new(4.0, 45.0));
    form.set_source(Some(video(40.0)));
    assert_eq!(form.range(), time_range(0.0, 40.0));
    assert_eq!(form.validation_blocker(&form.make_specs(false)), None);

    write_fixture(
        &cache,
        r#"[{"model": "seedance25", "mode": "motionTransfer", "constraints": {"maxDuration": 20}}]"#,
    );
    store.switch_account(Some("account|user_1")).await;
    form.set_constraints(store.table());
    assert_eq!(form.limits().duration, ClosedRange::new(4.0, 20.0));
    // Brought inside the new limit.
    assert_eq!(form.range(), time_range(0.0, 20.0));
}

#[tokio::test]
async fn fields_missing_from_the_cache_fall_back_to_the_constants() {
    let folder = tempfile::tempdir().unwrap();
    let cache = ModelConstraintCache::new(folder.path());
    let store = ModelConstraintStore::new(cache.clone(), now);
    write_fixture(
        &cache,
        r#"[{"model": "seedance25", "mode": "motionTransfer", "constraints": {"maxReferenceImages": 3}}]"#,
    );
    store.switch_account(Some("account|user_1")).await;
    let mut form = seedance_form(10.0);
    form.set_constraints(store.table());
    let mut expected = ModeLimits::new(ClosedRange::new(1, 3), false);
    expected.duration = ClosedRange::new(4.0, 30.0);
    assert_eq!(form.limits(), expected);
}

#[tokio::test]
async fn an_unreadable_cache_is_ignored() {
    for contents in [
        "not json",
        "{}",
        r#"{"version": 2, "accounts": {}}"#,
        r#"{"version": 1, "accounts": []}"#,
        r#"{"version": 1, "accounts": {"account|user_1": {"fetchedAt": "yesterday", "table": {"entries": []}}}}"#,
        r#"{"version": 1, "accounts": {"account|user_1": {"fetchedAt": "2026-10-15T08:00:00Z", "table": {"entries": 7}}}}"#,
    ] {
        let folder = tempfile::tempdir().unwrap();
        let cache = ModelConstraintCache::new(folder.path());
        std::fs::write(cache.file(), contents).unwrap();
        assert_eq!(cache.read("account|user_1"), None, "{contents}");
        let store = ModelConstraintStore::new(cache, now);
        store.switch_account(Some("account|user_1")).await;
        assert_eq!(store.table(), None, "{contents}");
        assert!(store.is_stale(), "{contents}");
    }
}

#[test]
fn a_tighter_table_brings_the_form_inside() {
    let mut form = seedance_form(28.0);
    form.add_references(vec![image(2), image(3)]);
    let table = serde_json::from_str(
        r#"{"entries": [{"model": "seedance25", "mode": "motionTransfer", "constraints": {"maxDuration": 12, "maxReferenceImages": 2}}]}"#,
    )
    .unwrap();
    form.set_constraints(Some(table));
    assert_eq!(form.range(), time_range(0.0, 12.0));
    assert_eq!(form.references().len(), 2);
    form.set_constraints(None);
    assert_eq!(form.limits().duration, ClosedRange::new(4.0, 30.0));
}

#[test]
fn a_draft_saved_without_constraints_still_decodes() {
    let mut form = seedance_form(10.0);
    form.set_constraints(Some(
        serde_json::from_str(
            r#"{"entries": [{"model": "seedance25", "mode": "motionTransfer", "constraints": {"maxDuration": 45}}]}"#,
        )
        .unwrap(),
    ));
    let mut object = serde_json::to_value(&form).unwrap();
    assert!(object.get("constraints").is_none());
    if let Some(fields) = object.as_object_mut() {
        fields.remove("constraints");
    }
    let decoded: CreateForm = serde_json::from_value(object).unwrap();
    assert_eq!(decoded.limits().duration, ClosedRange::new(4.0, 30.0));
}
