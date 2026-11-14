//! `AccountHistoryTests` and the history part of `LibraryMediaTests`.

use jutsu_core::{HistoryCursor, HistoryPage, PortError, ProviderError};

use super::{AccountHistory, HistoryPhase, HistoryPlaceholder, STYLES_NOTE};
use crate::connection::{AccountInfo, ConnectionState};
use crate::library::fixture::{history_image, history_item};

fn page(ids: &[&str], next: Option<&str>) -> HistoryPage {
    HistoryPage {
        items: ids.iter().map(|id| history_item(id, false)).collect(),
        images: Vec::new(),
        next_cursor: next.map(HistoryCursor::new),
    }
}

fn item_ids(history: &AccountHistory) -> Vec<&str> {
    history.items().iter().map(|item| item.id.as_str()).collect()
}

#[allow(clippy::unnecessary_wraps, reason = "the owner as the history keeps it")]
fn owner() -> Option<String> {
    Some("user_1".to_owned())
}

fn network() -> PortError {
    PortError::Provider(ProviderError::Network)
}

#[test]
fn pages_append_in_order_until_the_last_one() {
    let mut history = AccountHistory::new(owner());
    assert!(history.has_more());
    let first = history.begin_load(false).unwrap();
    assert!(first.is_refresh && first.cursor.is_none());
    assert_eq!(history.begin_load(false), None);
    history.receive(page(&["a", "b"], Some("c1")), &first);

    let second = history.begin_load(false).unwrap();
    assert_eq!(second.cursor, Some(HistoryCursor::new("c1")));
    assert!(!second.is_refresh);
    history.receive(page(&["b", "c"], None), &second);

    assert_eq!(item_ids(&history), ["a", "b", "c"]);
    assert!(!history.has_more());
    assert_eq!(history.begin_load(false), None);
}

/// `HistoryFooter`: the Styles note shows under the grid once the last page is in, and only then.
#[test]
fn the_styles_note_follows_the_last_page() {
    let mut history = AccountHistory::new(owner());
    assert_eq!(history.footer_note(), None);
    let first = history.begin_load(false).unwrap();
    history.receive(page(&["a"], Some("c1")), &first);
    assert_eq!(history.footer_note(), None);
    let second = history.begin_load(false).unwrap();
    history.receive(page(&["b"], None), &second);
    assert_eq!(history.footer_note(), Some(STYLES_NOTE));

    let mut empty = AccountHistory::new(owner());
    let only = empty.begin_load(false).unwrap();
    empty.receive(page(&[], None), &only);
    assert_eq!(empty.footer_note(), None);
}

#[test]
fn a_load_in_flight_is_not_started_twice_but_a_refresh_wins() {
    let mut history = AccountHistory::new(owner());
    let first = history.begin_load(true).unwrap();
    history.receive(page(&["a"], Some("c1")), &first);
    let more = history.begin_load(false).unwrap();
    assert_eq!(history.begin_load(false), None);
    let refresh = history.begin_load(true).unwrap();
    history.receive(page(&["late"], None), &more);
    history.receive(page(&["new", "a"], Some("c2")), &refresh);
    assert_eq!(item_ids(&history), ["new", "a"]);
    assert_eq!(history.next_cursor(), Some(&HistoryCursor::new("c2")));
}

#[test]
fn items_keep_their_library_id_across_refreshes() {
    let mut history = AccountHistory::new(owner());
    let first = history.begin_load(true).unwrap();
    history.receive(page(&["a"], None), &first);
    let id = history.items()[0].job_id;
    let refresh = history.begin_load(true).unwrap();
    history.receive(page(&["a"], None), &refresh);
    assert_eq!(history.items()[0].job_id, id);
}

#[test]
fn an_empty_page_asks_for_the_next_until_too_many_in_a_row() {
    let mut history = AccountHistory::new(owner());
    let mut continues = Vec::new();
    let mut request = history.begin_load(true).unwrap();
    for _ in 0..AccountHistory::MAX_EMPTY_PAGES_IN_A_ROW {
        continues.push(history.receive(page(&[], Some("more")), &request));
        request = history.begin_load(false).unwrap();
    }
    let mut expected = vec![true; AccountHistory::MAX_EMPTY_PAGES_IN_A_ROW - 1];
    expected.push(false);
    assert_eq!(continues, expected);
    assert!(!history.receive(page(&["a"], Some("more")), &request));
    assert!(history.has_more());
}

#[test]
fn paging_stopped_on_empty_pages_waits_for_the_user_then_goes_on() {
    let mut history = AccountHistory::new(owner());
    let mut request = history.begin_load(true).unwrap();
    for _ in 1..AccountHistory::MAX_EMPTY_PAGES_IN_A_ROW {
        history.receive(page(&[], Some("more")), &request);
        request = history.begin_load(false).unwrap();
    }
    assert!(!history.receive(page(&[], Some("more")), &request));
    assert!(history.is_waiting_to_load_more());
    let asked = history.begin_load(false).unwrap();
    assert!(!history.is_waiting_to_load_more());
    assert!(history.receive(page(&[], Some("more")), &asked));
}

#[test]
fn another_account_starts_empty_and_drops_answers_for_the_old_one() {
    let mut history = AccountHistory::new(owner());
    let first = history.begin_load(true).unwrap();
    history.receive(page(&["a"], None), &first);
    let late = history.begin_load(true).unwrap();
    history.switch_account(Some("user_2".to_owned()));
    history.receive(page(&["b"], None), &late);
    assert!(history.items().is_empty() && !history.has_loaded());
    history.switch_account(None);
    assert_eq!(history.owner(), None);
}

#[test]
fn a_failed_load_keeps_the_items_and_offers_a_retry() {
    let mut history = AccountHistory::new(owner());
    let first = history.begin_load(true).unwrap();
    history.receive(page(&["a"], Some("c1")), &first);
    let more = history.begin_load(false).unwrap();
    history.fail(&network(), &more);
    assert_eq!(item_ids(&history), ["a"]);
    let HistoryPhase::Failed(message) = history.phase() else {
        panic!("expected a failure, got {:?}", history.phase());
    };
    assert_ne!(message.title, "");
    assert_eq!(
        history.begin_load(false).unwrap().cursor,
        Some(HistoryCursor::new("c1"))
    );
}

#[test]
fn history_needs_a_signed_in_account() {
    let account = ConnectionState::Account(AccountInfo::default());
    let empty = AccountHistory::default();
    assert!(!matches!(
        HistoryPlaceholder::new(&account, &empty, false),
        Some(HistoryPlaceholder::Unavailable { .. })
    ));
    let api_key = ConnectionState::ApiKey {
        key_id: "kid".to_owned(),
    };
    // A connection without history says why, even with items.
    assert_eq!(
        HistoryPlaceholder::new(&api_key, &empty, false),
        Some(HistoryPlaceholder::Unavailable {
            title: "History needs a Higgsfield account".to_owned(),
            detail:
                "An API key can't read past generations. Sign in to your account in Settings to see them."
                    .to_owned(),
        })
    );
    assert!(matches!(
        HistoryPlaceholder::new(&ConnectionState::None, &empty, true),
        Some(HistoryPlaceholder::Unavailable { .. })
    ));
}

#[test]
fn the_placeholder_follows_the_first_load() {
    let account = ConnectionState::Account(AccountInfo::default());
    let mut history = AccountHistory::new(owner());
    assert_eq!(
        HistoryPlaceholder::new(&account, &history, true),
        Some(HistoryPlaceholder::Loading)
    );
    let first = history.begin_load(true).unwrap();
    assert_eq!(
        HistoryPlaceholder::new(&account, &history, true),
        Some(HistoryPlaceholder::Loading)
    );
    history.fail(&network(), &first);
    let Some(HistoryPlaceholder::Failed { title, .. }) = HistoryPlaceholder::new(&account, &history, true)
    else {
        panic!("expected a failed first load");
    };
    assert_eq!(title, "Can't reach Higgsfield");
    let retry = history.begin_load(true).unwrap();
    history.receive(page(&[], None), &retry);
    let Some(HistoryPlaceholder::Empty { detail, .. }) = HistoryPlaceholder::new(&account, &history, true)
    else {
        panic!("expected an empty history");
    };
    assert!(
        detail.ends_with(STYLES_NOTE),
        "an empty history says Styles made elsewhere aren't listed"
    );
    assert_eq!(HistoryPlaceholder::new(&account, &history, false), None);
}

#[test]
fn images_merge_across_pages_and_a_like_sets_their_star() {
    let mut history = AccountHistory::new(owner());
    let first = history.begin_load(true).unwrap();
    let images = vec![history_image("img-1", 1.0), history_image("img-2", 2.0)];
    let kept = images[1].job_id;
    history.receive(
        HistoryPage {
            items: Vec::new(),
            images: images.clone(),
            next_cursor: Some(HistoryCursor::new("c1")),
        },
        &first,
    );
    let second = history.begin_load(false).unwrap();
    let mut again = history_image("img-2", 2.0);
    again.job_id = uuid::Uuid::new_v4();
    let added = history.receive(
        HistoryPage {
            items: Vec::new(),
            images: vec![again, history_image("img-3", 3.0)],
            next_cursor: None,
        },
        &second,
    );
    assert!(!added);
    assert_eq!(
        history
            .images()
            .iter()
            .map(|image| image.id.as_str())
            .collect::<Vec<_>>(),
        ["img-1", "img-2", "img-3"]
    );
    assert_eq!(history.images()[1].job_id, kept);
    history.set_liked("img-1", true);
    assert!(history.images()[0].is_liked);
}
