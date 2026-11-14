//! `CullingHistoryTests`, `CullingFilterTests` and `CullingToggleTests`.

use std::collections::HashMap;

use jutsu_core::{Culling, Material};
use uuid::Uuid;

use super::{CatalogIndex, CullingAction, CullingHistory, advanced_cursor, cursor_after};
use crate::library::filter::LibraryFilter;
use crate::library::fixture::{catalog_image, catalog_video, ids, image, rows, video};
use crate::library::media::{KindCounts, LibraryKind};

fn culled_image(culling: Culling) -> Material {
    catalog_image("A knight", None, 10.0, culling, false)
}

/// How many steps Undo can take back.
fn undo_depth(mut history: CullingHistory) -> usize {
    std::iter::from_fn(|| history.undo()).count()
}

#[test]
fn applying_writes_only_the_versions_that_change() {
    let materials = [image(10.0), culled_image(Culling::Kept), video(10.0)];
    let index = CatalogIndex::new(&materials);
    let all: Vec<Uuid> = materials.iter().map(|material| material.id).collect();
    let mut history = CullingHistory::default();
    let writes = history.apply(CullingAction::Keep, &all, &index).unwrap();
    assert_eq!(
        writes,
        HashMap::from([
            (materials[0].reference(), Culling::Kept),
            (materials[2].reference(), Culling::Kept),
        ])
    );
    // Keeping what is kept already changes nothing and takes no Undo of its own.
    assert_eq!(history.apply(CullingAction::Keep, &[all[1]], &index), None);
    assert_eq!(
        history.apply(CullingAction::Keep, &[Uuid::new_v4()], &index),
        None
    );
    assert_eq!(undo_depth(history), 1);
}

#[test]
fn undo_restores_the_last_change_first() {
    let materials = [image(10.0), culled_image(Culling::Kept)];
    let index = CatalogIndex::new(&materials);
    let mut history = CullingHistory::default();
    history.apply(CullingAction::Reject, &[materials[0].id], &index);
    history.apply(CullingAction::Clear, &[materials[1].id], &index);
    assert_eq!(
        history.undo(),
        Some(HashMap::from([(materials[1].reference(), Culling::Kept)]))
    );
    assert_eq!(
        history.undo(),
        Some(HashMap::from([(materials[0].reference(), Culling::Unreviewed)]))
    );
    assert_eq!(history.undo(), None);
    assert!(!history.can_undo());
}

#[test]
fn the_stack_keeps_the_newest_steps() {
    let mut history = CullingHistory::default();
    for step in 0..CullingHistory::LIMIT + 5 {
        let (action, before) = if step % 2 == 0 {
            (CullingAction::Reject, Culling::Kept)
        } else {
            (CullingAction::Keep, Culling::Rejected)
        };
        let material = culled_image(before);
        history.apply(action, &[material.id], &CatalogIndex::new(&[material]));
    }
    assert_eq!(undo_depth(history), CullingHistory::LIMIT);
}

fn culling_materials() -> Vec<Material> {
    vec![
        catalog_video("rejected dance", None, 5.0, Culling::Rejected, false),
        catalog_video("kept dance", None, 6.0, Culling::Kept, false),
        catalog_image("rejected knight", None, 7.0, Culling::Rejected, false),
        catalog_image("knight", None, 8.0, Culling::Unreviewed, false),
    ]
}

fn shown(materials: &[Material], filter: &LibraryFilter, kind: LibraryKind) -> Vec<Uuid> {
    let culled = filter.culled(rows(materials), &CatalogIndex::new(materials));
    ids(&filter.apply(&culled, kind))
}

#[test]
fn rejected_results_stay_out_of_every_tab_and_every_search() {
    let materials = culling_materials();
    let filter = LibraryFilter::default();
    assert_eq!(
        shown(&materials, &filter, LibraryKind::All),
        [materials[1].id, materials[3].id]
    );
    assert_eq!(shown(&materials, &filter, LibraryKind::Video), [materials[1].id]);
    assert_eq!(shown(&materials, &filter, LibraryKind::Image), [materials[3].id]);
    let culled = filter.culled(rows(&materials), &CatalogIndex::new(&materials));
    assert_eq!(filter.counts(&culled), KindCounts::new(1, 1));

    let search = LibraryFilter {
        query: "rejected".to_owned(),
        ..LibraryFilter::default()
    };
    for kind in [LibraryKind::All, LibraryKind::Video, LibraryKind::Image] {
        assert_eq!(shown(&materials, &search, kind).len(), 0);
    }
}

#[test]
fn show_rejected_brings_them_back_with_the_other_filters() {
    let materials = culling_materials();
    let filter = LibraryFilter {
        query: "rejected".to_owned(),
        shows_rejected: true,
        ..LibraryFilter::default()
    };
    assert_eq!(
        shown(&materials, &filter, LibraryKind::All),
        [materials[0].id, materials[2].id]
    );
    assert_eq!(shown(&materials, &filter, LibraryKind::Image), [materials[2].id]);
    let culled = filter.culled(rows(&materials), &CatalogIndex::new(&materials));
    assert_eq!(filter.counts(&culled), KindCounts::new(1, 1));
    let only = LibraryFilter {
        shows_rejected: true,
        ..LibraryFilter::default()
    };
    assert!(!only.is_active());
    assert!(!only.cleared().shows_rejected);
}

/// The lit verdict pressed again takes it back; another verdict or a mixed selection is set.
#[test]
fn a_verdict_toggles_only_when_every_one_has_it() {
    assert_eq!(
        CullingAction::Reject.toggled(&[Culling::Rejected]),
        CullingAction::Clear
    );
    assert_eq!(
        CullingAction::Keep.toggled(&[Culling::Kept, Culling::Kept]),
        CullingAction::Clear
    );
    assert_eq!(
        CullingAction::Reject.toggled(&[Culling::Kept]),
        CullingAction::Reject
    );
    assert_eq!(
        CullingAction::Reject.toggled(&[Culling::Rejected, Culling::Unreviewed]),
        CullingAction::Reject
    );
    assert_eq!(CullingAction::Keep.toggled(&[]), CullingAction::Keep);
    assert_eq!(
        CullingAction::Clear.toggled(&[Culling::Unreviewed]),
        CullingAction::Clear
    );
}

/// `LibraryCursor.advanced`: a verdict moves on to the next card; at the end it stays, or goes back one when the
/// card leaves the grid.
#[test]
fn a_verdict_moves_the_cursor_to_the_next_card() {
    let ids = ["a", "b", "c"];
    assert_eq!(advanced_cursor(&"a", &ids, false), Some("b"));
    assert_eq!(advanced_cursor(&"c", &ids, false), Some("c"));
    assert_eq!(advanced_cursor(&"c", &ids, true), Some("b"));
    assert_eq!(advanced_cursor(&"a", &["a"], true), None);
    assert_eq!(advanced_cursor(&"x", &ids, true), Some("x"));
}

/// `LibraryView.cull`: Clear stays on the card; a reject leaves the grid only while rejected results are hidden.
#[test]
fn clear_stays_and_a_hidden_reject_steps_back() {
    let order: Vec<String> = ["a", "b"].map(str::to_owned).to_vec();
    assert_eq!(
        cursor_after(CullingAction::Clear, "a", &order, false).as_deref(),
        Some("a")
    );
    assert_eq!(
        cursor_after(CullingAction::Keep, "a", &order, false).as_deref(),
        Some("b")
    );
    assert_eq!(
        cursor_after(CullingAction::Reject, "b", &order, false).as_deref(),
        Some("a")
    );
    assert_eq!(
        cursor_after(CullingAction::Reject, "b", &order, true).as_deref(),
        Some("b")
    );
}
