//! `ReferenceImageTests.swift`.

use jutsu_core::*;
use time::macros::datetime;
use time::{Duration, OffsetDateTime, UtcOffset};
use uuid::Uuid;

use crate::common::*;

#[test]
fn a_reference_stored_before_prompts_decodes() {
    let image: LocalImage = serde_json::from_str(r#"{"fileURL":"file:///tmp/a.png"}"#).unwrap();
    assert_eq!(image, LocalImage::new(file_url("/tmp/a.png")));
    assert_eq!(image.prompt, None);
}

#[test]
fn a_template_keeps_the_generated_prompt() {
    let mut image = LocalImage::new(file_url("/tmp/a.png"));
    image.prompt = Some("a knight in silver armor".into());
    let mut template = JobTemplate::new(
        Uuid::new_v4(),
        "Knight",
        OffsetDateTime::now_utc(),
        Mode::MotionTransfer,
        VideoModel::genjutsu(),
        Resolution::P720,
    );
    template.references = vec![image];

    let decoded: JobTemplate = serde_json::from_str(&serde_json::to_string(&template).unwrap()).unwrap();

    let prompts: Vec<_> = decoded
        .references
        .iter()
        .map(|image| image.prompt.as_deref())
        .collect();
    assert_eq!(prompts, [Some("a knight in silver armor")]);
}

#[test]
fn the_nearest_frame_is_picked() {
    let model = ImageModel::new(
        "m",
        "M",
        ["1:1", "4:3", "16:9", "9:16"].map(String::from).to_vec(),
    );
    for (width, height, expected) in [
        (1920, 1080, "16:9"),
        (1080, 1920, "9:16"),
        (1000, 1000, "1:1"),
        (1440, 1080, "4:3"),
        (0, 0, "1:1"),
    ] {
        assert_eq!(
            model.nearest_aspect_ratio(width, height).as_deref(),
            Some(expected),
            "{width}x{height}"
        );
    }
}

#[test]
fn image_requests_count_in_their_unit_on_their_day_or_while_running() {
    let day = datetime!(2025-10-12 23:00:00 UTC);
    let charge = |date, cost, model: &str, is_pending| {
        let mut charge = ImageCharge::new(Uuid::new_v4(), date, cost, ProviderKind::ApiKey, model, 1, None);
        charge.is_pending = is_pending;
        charge
    };
    let charges = [
        charge(day, usd("2", false), "a", false),
        charge(day - Duration::days(3), usd("1", true), "b", true),
        charge(day - Duration::days(3), usd("5", false), "c", false),
    ];
    assert_eq!(
        Budget::committed_images(&charges, day, CostUnit::Usd, UtcOffset::UTC),
        dec("3")
    );
    assert_eq!(
        Budget::committed_images(&charges, day, CostUnit::Credits, UtcOffset::UTC),
        dec("0")
    );
}
