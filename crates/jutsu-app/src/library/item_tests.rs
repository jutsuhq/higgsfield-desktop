//! `LibraryCardTests`, `LibraryDetailTests`, `LibraryTimingTests` and the history item's part of
//! `AccountHistoryTests`.

use jutsu_core::{Cost, Mode, PixelSize, ProviderKind, Resolution};
use time::Duration;

use super::LibraryItem;
use crate::library::fixture::{ItemFixture, dec, file, history_item, history_url, item, now};

#[test]
fn title_uses_the_preset_name_or_the_mode_title() {
    let preset = ItemFixture {
        preset_name: Some("Claymation"),
        ..ItemFixture::default()
    }
    .item();
    assert_eq!(preset.title(), "rooftop · Claymation");
    let motion = ItemFixture {
        mode: Mode::MotionTransfer,
        source: "dance.mp4",
        ..ItemFixture::default()
    }
    .item();
    assert_eq!(motion.title(), "dance · Motion transfer");
}

#[test]
fn subtitle_in_usd_and_credits() {
    assert_eq!(item().subtitle(now()), "480p · $1.91 · 36 min ago");
    let credits = ItemFixture {
        resolution: Resolution::P720,
        cost: Cost::credits(dec("48"), true),
        via: ProviderKind::Account,
        minutes_ago: 1560.0,
        ..ItemFixture::default()
    }
    .item();
    assert_eq!(credits.subtitle(now()), "720p · 48 cr · 1 d ago");
    let final_now = ItemFixture {
        is_draft: false,
        minutes_ago: 0.2,
        ..ItemFixture::default()
    }
    .item();
    assert_eq!(final_now.headline(now()), "just now · Final");
    let draft = ItemFixture {
        minutes_ago: 72.0,
        ..ItemFixture::default()
    }
    .item();
    assert_eq!(draft.headline(now()), "1 h ago · Draft");
}

#[test]
fn tags_and_duration() {
    let swap = ItemFixture {
        mode: Mode::ObjectSwap,
        range: (0.0, 9.6),
        ..ItemFixture::default()
    }
    .item();
    assert_eq!(swap.mode_tag(), "Swap");
    assert_eq!(swap.duration_text(), "10 s");
}

#[test]
fn the_accessibility_label_reads_the_tags_and_badges() {
    let mut item = ItemFixture {
        preset_name: Some("Claymation"),
        ..ItemFixture::default()
    }
    .item();
    assert_eq!(
        item.accessibility_label(now()),
        "rooftop · Claymation, Restyle, Draft, 6 s, 480p · $1.91 · 36 min ago"
    );
    item.is_starred = true;
    item.job.spec.is_draft = false;
    assert_eq!(
        item.accessibility_label(now()),
        "rooftop · Claymation, Restyle, 6 s, 480p · $1.91 · 36 min ago, Starred"
    );

    let remote = LibraryItem::from_history(history_item("gen-1", true));
    let label = remote.accessibility_label(now());
    assert!(
        label.ends_with(", Starred, On Higgsfield, not downloaded yet"),
        "{label}"
    );
    assert!(!label.contains('$'));
}

#[test]
fn facts_list_only_what_was_used() {
    let item = ItemFixture {
        prompt: "Rain on the windows",
        references: 2,
        preset_name: Some("Film Noir"),
        ..ItemFixture::default()
    }
    .item();
    assert_eq!(item.facts(), ["Restyle", "Film Noir", "480p", "6 s", "2 images"]);
    let one = ItemFixture {
        references: 1,
        ..ItemFixture::default()
    }
    .item();
    assert_eq!(one.facts(), ["Restyle", "480p", "6 s", "1 image"]);
    assert_eq!(self::item().range_text().unwrap(), "00:04.0 – 00:10.0");
}

#[test]
fn billing_summary_in_usd_and_credits() {
    assert_eq!(
        item().billing_summary(),
        "$1.91 · Estimate · 6 s × $0.318 · API key"
    );
    let credits = ItemFixture {
        cost: Cost::credits(dec("24"), true),
        via: ProviderKind::Account,
        ..ItemFixture::default()
    }
    .item();
    assert_eq!(
        credits.billing_summary(),
        "24 cr · Estimate · 6 s × 4 cr · Higgsfield account"
    );
    // An account price that isn't an estimate is Higgsfield's quote before upload.
    let quoted = ItemFixture {
        cost: Cost::credits(dec("24"), false),
        via: ProviderKind::Account,
        ..ItemFixture::default()
    }
    .item();
    assert_eq!(
        quoted.billing_summary(),
        "24 cr · Price from Higgsfield · 6 s × 4 cr · Higgsfield account"
    );
    let odd = ItemFixture {
        range: (0.0, 4.0),
        cost: Cost::credits(dec("18"), false),
        via: ProviderKind::Account,
        ..ItemFixture::default()
    }
    .item();
    assert_eq!(odd.billed_text().unwrap(), "4 s × 4.5 cr");
}

#[test]
fn the_aspect_ratio_is_the_sources() {
    let mut item = item();
    assert!((item.aspect_ratio().unwrap() - 1920.0 / 1080.0).abs() < 1e-9);
    if let Some(source) = item.job.spec.source.as_mut() {
        source.pixel_size = PixelSize::new(0, 0);
    }
    assert_eq!(item.aspect_ratio(), None);
}

#[test]
fn only_a_local_file_is_the_output() {
    let mut item = item();
    assert!(item.output_file().is_some());
    item.job.output_file = Some(history_url("out.mp4"));
    assert_eq!(item.output_file(), None);
    item.job.output_file = None;
    assert_eq!(item.output_file(), None);
    item.job.output_file = Some(file("/tmp/out.mp4"));
    assert!(item.output_file().is_some());
}

#[test]
fn shows_the_stage_times_that_were_recorded() {
    let mut item = item();
    assert_eq!(item.timing_summary(), None);
    let created = item.job.created_at;
    item.job.submitted_at = Some(created + Duration::seconds(30));
    item.job.processing_at = Some(created + Duration::seconds(125));
    assert_eq!(item.timing_summary().unwrap(), "Queue wait 2:05");
    item.job.charged_at = Some(created + Duration::seconds(197));
    assert_eq!(
        item.timing_summary().unwrap(),
        "Queue wait 2:05 · Processing 1:12"
    );
}

#[test]
fn a_history_item_reads_like_a_local_one_without_a_price() {
    let item = LibraryItem::from_history(history_item("a", false));
    assert_eq!(item.title(), "Genjutsu · Object swap");
    assert_eq!(item.mode_tag(), "Swap");
    assert_eq!(item.duration_text(), "8 s");
    assert_eq!(item.subtitle(now()), "720p · 2 h ago");
    assert_eq!(item.billing_summary(), "Made on Higgsfield · Higgsfield account");
    assert_eq!(item.output_file(), None);
    assert_eq!(item.remote_url(), Some(&history_url("a.mp4")));
}

#[test]
fn a_like_on_higgsfield_is_the_star() {
    let liked = LibraryItem::from_history(history_item("a", true));
    assert!(liked.is_starred);
    assert_eq!(liked.like_id().as_deref(), Some("a"));
    assert_eq!(item().like_id(), None);
}
