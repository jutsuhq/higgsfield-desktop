//! `LibraryFilterTests`, `LibraryStarredFilterTests`, `LibrarySortAndModelTests`, `LibraryKindTests`,
//! `LibraryFilterVisibilityTests`, `LibraryKindCountsTests` and `LibraryFilterOptionsTests`.

use std::collections::BTreeSet;

use jutsu_core::{Cost, Mode, PixelSize, ProviderKind, Resolution, SavedImage, VideoModel};
use time::Duration;
use uuid::Uuid;

use super::{FilterField, FilterOptions, Finish, LibraryFilter, LibrarySort};
use crate::library::fixture::{ItemFixture, dec, file, history_image, history_item, history_url, item, now};
use crate::library::item::LibraryItem;
use crate::library::media::{KindCounts, LibraryKind, MediaRow};

fn filter_items() -> Vec<LibraryItem> {
    vec![
        ItemFixture {
            preset_name: Some("Claymation"),
            ..ItemFixture::default()
        }
        .item(),
        ItemFixture {
            mode: Mode::MotionTransfer,
            source: "dance.mp4",
            resolution: Resolution::P720,
            is_draft: false,
            prompt: "Keep the Camera move",
            ..ItemFixture::default()
        }
        .item(),
        ItemFixture {
            mode: Mode::ObjectSwap,
            source: "sneaker.mov",
            resolution: Resolution::P1080,
            is_draft: false,
            ..ItemFixture::default()
        }
        .item(),
    ]
}

fn ids(items: &[LibraryItem]) -> Vec<Uuid> {
    items.iter().map(LibraryItem::id).collect()
}

fn pick(items: &[LibraryItem], indices: &[usize]) -> Vec<Uuid> {
    indices.iter().map(|index| items[*index].id()).collect()
}

#[test]
fn an_empty_filter_keeps_everything_in_order() {
    let items = filter_items();
    assert_eq!(ids(&LibraryFilter::default().apply_videos(&items)), ids(&items));
    assert!(!LibraryFilter::default().is_active());
}

#[test]
fn filters_by_mode_resolution_and_kind() {
    let items = filter_items();
    let apply = |filter: LibraryFilter| ids(&filter.apply_videos(&items));
    let mode = |mode| LibraryFilter {
        mode: Some(mode),
        ..LibraryFilter::default()
    };
    assert_eq!(apply(mode(Mode::ObjectSwap)), pick(&items, &[2]));
    assert_eq!(
        apply(LibraryFilter {
            resolution: Some(Resolution::P720),
            ..LibraryFilter::default()
        }),
        pick(&items, &[1])
    );
    let finish = |finish| LibraryFilter {
        finish,
        ..LibraryFilter::default()
    };
    assert_eq!(apply(finish(Finish::Drafts)), pick(&items, &[0]));
    assert_eq!(apply(finish(Finish::Finals)), pick(&items, &[1, 2]));
    assert_eq!(
        apply(LibraryFilter {
            finish: Finish::Finals,
            ..mode(Mode::Restyle)
        })
        .len(),
        0
    );
}

#[test]
fn search_covers_prompt_and_title_case_insensitively() {
    let items = filter_items();
    let search = |query: &str| {
        ids(&LibraryFilter {
            query: query.to_owned(),
            ..LibraryFilter::default()
        }
        .apply_videos(&items))
    };
    assert_eq!(search("camera"), pick(&items, &[1]));
    assert_eq!(search("  CLAY "), pick(&items, &[0]));
    assert_eq!(search("sneaker"), pick(&items, &[2]));
    assert_eq!(search("motion"), pick(&items, &[1]));
    assert_eq!(search("nothing like it").len(), 0);
    assert_eq!(search("   "), ids(&items));
}

#[test]
fn the_starred_filter_keeps_only_starred_items_and_combines_with_the_others() {
    let mut starred_draft = item();
    starred_draft.is_starred = true;
    let mut starred_final = ItemFixture {
        resolution: Resolution::P1080,
        is_draft: false,
        ..ItemFixture::default()
    }
    .item();
    starred_final.is_starred = true;
    let plain = ItemFixture {
        is_draft: false,
        ..ItemFixture::default()
    }
    .item();
    let items = vec![starred_draft, plain, starred_final];
    let starred = LibraryFilter {
        is_starred_only: true,
        ..LibraryFilter::default()
    };
    assert_eq!(ids(&starred.apply_videos(&items)), pick(&items, &[0, 2]));
    let finals = LibraryFilter {
        finish: Finish::Finals,
        ..starred.clone()
    };
    assert_eq!(ids(&finals.apply_videos(&items)), pick(&items, &[2]));
    assert!(starred.is_active());
}

fn sort_items() -> Vec<LibraryItem> {
    let mut seedance = ItemFixture {
        mode: Mode::MotionTransfer,
        source: "dance.mp4",
        cost: Cost::usd(dec("4"), false),
        ..ItemFixture::default()
    }
    .item();
    seedance.job.spec.model = VideoModel::SEEDANCE25;
    let credits = ItemFixture {
        source: "credits.mov",
        cost: Cost::credits(dec("90"), false),
        via: ProviderKind::Account,
        ..ItemFixture::default()
    }
    .item();
    let cheap = ItemFixture {
        source: "cheap.mov",
        cost: Cost::usd(dec("1"), false),
        ..ItemFixture::default()
    }
    .item();
    let imported = LibraryItem::from_history(history_item("gen-1", false));
    let same_as_cheap = ItemFixture {
        source: "also-cheap.mov",
        cost: Cost::usd(dec("1"), true),
        ..ItemFixture::default()
    }
    .item();
    vec![seedance, credits, cheap, imported, same_as_cheap]
}

#[test]
fn filters_by_model_and_sorts() {
    let items = sort_items();
    let model = |model| LibraryFilter {
        model: Some(model),
        ..LibraryFilter::default()
    };
    assert_eq!(
        ids(&model(VideoModel::SEEDANCE25).apply_videos(&items)),
        pick(&items, &[0])
    );
    assert_eq!(
        model(VideoModel::KLING_MOTION_CONTROL).apply_videos(&items).len(),
        0
    );
    assert_eq!(
        ids(&model(VideoModel::GENJUTSU).apply_videos(&items)),
        pick(&items, &[1, 2, 3, 4])
    );
    let sort = |sort| LibraryFilter {
        sort,
        ..LibraryFilter::default()
    };
    assert_eq!(ids(&sort(LibrarySort::Newest).apply_videos(&items)), ids(&items));
    assert_eq!(
        ids(&sort(LibrarySort::Oldest).apply_videos(&items)),
        pick(&items, &[4, 3, 2, 1, 0])
    );
    // Dollars before credits, ties in their newest-first order, an unknown price last.
    assert_eq!(
        ids(&sort(LibrarySort::HighestPrice).apply_videos(&items)),
        pick(&items, &[0, 2, 4, 1, 3])
    );
    let oldest_genjutsu = LibraryFilter {
        sort: LibrarySort::Oldest,
        ..model(VideoModel::GENJUTSU)
    };
    assert_eq!(
        ids(&oldest_genjutsu.apply_videos(&items)),
        pick(&items, &[4, 3, 2, 1])
    );
}

#[test]
fn the_order_is_not_a_filter() {
    assert!(!sort_oldest().is_active());
    let filtered = LibraryFilter {
        query: "x".to_owned(),
        model: Some(VideoModel::SEEDANCE25),
        ..sort_oldest()
    };
    assert!(filtered.is_active());
    assert_eq!(filtered.cleared(), sort_oldest());
}

fn sort_oldest() -> LibraryFilter {
    LibraryFilter {
        sort: LibrarySort::Oldest,
        ..LibraryFilter::default()
    }
}

fn saved_image(model: &str, minutes: f64) -> SavedImage {
    SavedImage {
        id: Uuid::new_v4(),
        file_url: file("/home/me/Movies/Jutsu Studio/Images/knight.png"),
        prompt: "A knight in silver armor".to_owned(),
        model: model.to_owned(),
        aspect_ratio: Some("16:9".to_owned()),
        created_at: now() - Duration::seconds_f64(minutes * 60.0),
        remote_url: Some(history_url("knight.png")),
        is_starred: false,
        title: None,
    }
}

/// `LibraryMediaFixture.rows`.
fn media_rows() -> Vec<MediaRow> {
    let mut remote = history_image("img-1", 20.0);
    remote.prompt = "Neon dance".to_owned();
    vec![
        MediaRow::Video(item()),
        MediaRow::Video(
            ItemFixture {
                mode: Mode::MotionTransfer,
                source: "dance.mp4",
                resolution: Resolution::P720,
                is_draft: false,
                prompt: "Keep the camera move",
                minutes_ago: 5.0,
                ..ItemFixture::default()
            }
            .item(),
        ),
        MediaRow::LocalImage(saved_image("Higgsfield Soul 2.0", 10.0)),
        MediaRow::RemoteImage(remote),
    ]
}

fn videos(rows: &[MediaRow]) -> usize {
    rows.iter().filter(|row| row.is_video()).count()
}

#[test]
fn the_kind_picks_its_media_and_each_filter_its_own() {
    let rows = media_rows();
    let all = LibraryFilter::default();
    assert_eq!(all.apply(&rows, LibraryKind::All).len(), 4);
    assert_eq!(videos(&all.apply(&rows, LibraryKind::Video)), 2);
    assert_eq!(all.apply(&rows, LibraryKind::Image).len(), 2);
    assert_eq!(videos(&all.apply(&rows, LibraryKind::Image)), 0);

    let motion = LibraryFilter {
        mode: Some(Mode::MotionTransfer),
        ..LibraryFilter::default()
    };
    let filtered = motion.apply(&rows, LibraryKind::All);
    assert_eq!(filtered.len(), 3);
    assert_eq!(filtered.len() - videos(&filtered), 2);

    let soul = LibraryFilter {
        image_model: Some("Higgsfield Soul 2.0".to_owned()),
        ..LibraryFilter::default()
    };
    let filtered = soul.apply(&rows, LibraryKind::All);
    assert_eq!((videos(&filtered), filtered.len() - videos(&filtered)), (2, 1));
    let portrait = LibraryFilter {
        image_aspect_ratio: Some("9:16".to_owned()),
        ..LibraryFilter::default()
    };
    assert_eq!(videos(&portrait.apply(&rows, LibraryKind::All)), 2);
}

#[test]
fn image_search_matches_every_word_against_prompt_and_model() {
    let rows = media_rows();
    let count = |query: &str| {
        LibraryFilter {
            query: query.to_owned(),
            ..LibraryFilter::default()
        }
        .apply(&rows, LibraryKind::Image)
        .len()
    };
    assert_eq!(count("knight armor"), 1);
    assert_eq!(count("soul knight"), 1);
    assert_eq!(count("knight dance"), 0);
    assert_eq!(count("soul"), 2);
    assert_eq!(count("camera"), 0);
}

#[test]
fn starred_covers_stars_and_likes() {
    let mut rows = media_rows();
    if let MediaRow::Video(item) = &mut rows[0] {
        item.is_starred = true;
    }
    if let MediaRow::LocalImage(image) = &mut rows[2] {
        image.is_starred = true;
    }
    let starred = LibraryFilter {
        is_starred_only: true,
        ..LibraryFilter::default()
    };
    assert_eq!(starred.apply(&rows, LibraryKind::All).len(), 2);
    if let MediaRow::RemoteImage(image) = &mut rows[3] {
        image.is_liked = true;
    }
    assert_eq!(starred.apply(&rows, LibraryKind::All).len(), 3);
}

#[test]
fn all_interleaves_by_date_and_highest_price_puts_images_last() {
    let rows = media_rows();
    let newest = LibraryFilter::default().apply(&rows, LibraryKind::All);
    assert_eq!(
        newest.iter().map(MediaRow::is_video).collect::<Vec<_>>(),
        [true, false, false, true]
    );
    let oldest = sort_oldest().apply(&rows, LibraryKind::All);
    assert!(
        oldest
            .windows(2)
            .all(|pair| pair[0].created_at() <= pair[1].created_at())
    );
    let priced = LibraryFilter {
        sort: LibrarySort::HighestPrice,
        ..LibraryFilter::default()
    }
    .apply(&rows, LibraryKind::All);
    assert!(priced[..2].iter().all(MediaRow::is_video));
    assert!(!priced[2..].iter().any(MediaRow::is_video));
}

#[test]
fn counts_match_the_filtered_grid_and_ignore_only_the_kind() {
    let rows = media_rows();
    let soul = LibraryFilter {
        query: "soul".to_owned(),
        ..LibraryFilter::default()
    };
    for kind in [LibraryKind::All, LibraryKind::Video, LibraryKind::Image] {
        let shown = i32::try_from(soul.apply(&rows, kind).len()).unwrap();
        assert_eq!(shown, soul.counts(&rows).count(kind));
    }
    let restyle = LibraryFilter {
        mode: Some(Mode::Restyle),
        ..LibraryFilter::default()
    };
    assert_eq!(restyle.counts(&rows), KindCounts::new(1, 2));
    let none = LibraryFilter {
        image_model: Some("no such model".to_owned()),
        ..LibraryFilter::default()
    };
    assert_eq!(none.counts(&rows), KindCounts::new(2, 0));
}

#[test]
fn each_kind_shows_only_the_filters_that_can_change_its_grid() {
    use FilterField::{
        Finish, ImageAspectRatio, ImageModel, Mode, Model, Rejected, Resolution, Sort, Starred,
    };
    assert_eq!(
        FilterField::visible(LibraryKind::Video),
        [Mode, Model, Resolution, Finish, Starred, Rejected, Sort]
    );
    assert_eq!(
        FilterField::visible(LibraryKind::Image),
        [ImageModel, ImageAspectRatio, Starred, Rejected, Sort]
    );
    assert_eq!(FilterField::visible(LibraryKind::All).len(), 9);
}

#[test]
fn menus_list_only_the_values_the_rows_have() {
    let mut rows = media_rows();
    rows[3] = MediaRow::RemoteImage(history_image("img-1", 20.0));
    let options = FilterOptions::new(&rows);
    let modes: BTreeSet<&str> = options.modes.iter().map(|choice| choice.value.as_str()).collect();
    assert_eq!(modes, BTreeSet::from(["motionTransfer", "restyle"]));
    assert_eq!(
        options
            .resolutions
            .iter()
            .map(|choice| choice.value.as_str())
            .collect::<Vec<_>>(),
        ["480p", "720p"]
    );
    assert!(options.has_drafts_and_finals);
    assert_eq!(options.image_models, ["Higgsfield Soul 2.0", "soul-2.0"]);

    // A menu with one value is left out unless it holds one.
    let filter = LibraryFilter::default();
    assert_eq!(options.models.len(), 1);
    assert!(!options.shows(FilterField::Model, &filter));
    let picked = LibraryFilter {
        model: Some(VideoModel::GENJUTSU),
        ..LibraryFilter::default()
    };
    assert!(options.shows(FilterField::Model, &picked));
    assert!(options.shows(FilterField::Mode, &filter));
    assert!(options.shows(FilterField::Finish, &filter));
    assert!(options.shows(FilterField::Starred, &filter));
}

#[test]
fn finish_needs_both_drafts_and_finals() {
    let drafts = FilterOptions::new(&[MediaRow::Video(item())]);
    assert!(!drafts.shows(FilterField::Finish, &LibraryFilter::default()));
    let finals = LibraryFilter {
        finish: Finish::Finals,
        ..LibraryFilter::default()
    };
    assert!(drafts.shows(FilterField::Finish, &finals));
}

#[test]
fn the_aspect_ratio_reduces_the_size() {
    assert_eq!(history_image("a", 1.0).aspect_ratio().as_deref(), Some("1:1"));
    let mut wide = history_image("b", 1.0);
    wide.pixel_size = Some(PixelSize::new(1920, 1080));
    assert_eq!(wide.aspect_ratio().as_deref(), Some("16:9"));
    wide.pixel_size = None;
    assert_eq!(wide.aspect_ratio(), None);
}
