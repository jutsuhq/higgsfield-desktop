//! A list the provider hands out page by page (`PagedFeed.swift`): the first page loads on first view, the next
//! ones on demand. Used by the Motion Library and References › From Higgsfield.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::future::BoxFuture;
use jutsu_core::{HistoryCursor, PortError, ProviderKind};

use crate::connection::UserMessage;

/// An item a feed keeps once: two pages naming the same ID show it once.
pub trait FeedItem: Clone + Send + Sync + 'static {
    fn feed_id(&self) -> String;
}

impl FeedItem for jutsu_core::MotionPreset {
    fn feed_id(&self) -> String {
        self.id.clone()
    }
}

impl FeedItem for jutsu_core::AccountElement {
    fn feed_id(&self) -> String {
        self.id.clone()
    }
}

impl FeedItem for jutsu_core::UploadedFile {
    fn feed_id(&self) -> String {
        self.id.clone()
    }
}

impl FeedItem for jutsu_core::CharacterPreset {
    fn feed_id(&self) -> String {
        self.id.clone()
    }
}

impl FeedItem for jutsu_core::CharacterSheet {
    fn feed_id(&self) -> String {
        self.id.clone()
    }
}

/// One page: its items and the cursor of the next, `None` on the last.
pub type Page<T> = (Vec<T>, Option<HistoryCursor>);
/// Fetches the page after a cursor; `None` for the first.
pub type Fetch<T> =
    Arc<dyn Fn(Option<HistoryCursor>) -> BoxFuture<'static, Result<Page<T>, PortError>> + Send + Sync>;

#[derive(Debug, Clone)]
struct State<T> {
    items: Vec<T>,
    is_loading: bool,
    failure: Option<UserMessage>,
    has_loaded: bool,
    cursor: Option<HistoryCursor>,
    finished: bool,
    generation: u64,
}

/// What a feed shows now.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedView<T> {
    pub items: Vec<T>,
    pub is_loading: bool,
    /// The last failure; cleared by the next page that arrives.
    pub failure: Option<UserMessage>,
    pub has_more: bool,
    /// The first page has arrived.
    pub has_loaded: bool,
}

/// A paged list. Clones share it.
#[derive(Clone)]
pub struct PagedFeed<T: FeedItem> {
    state: Arc<Mutex<State<T>>>,
    /// Held while a page loads; a second load waits for it and returns, so it joins the first.
    loading: Arc<tokio::sync::Mutex<()>>,
    fetch: Fetch<T>,
    failure_title: String,
}

impl<T: FeedItem> std::fmt::Debug for PagedFeed<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("PagedFeed").finish_non_exhaustive()
    }
}

impl<T: FeedItem> PagedFeed<T> {
    /// `failure_title` is what a failed page says when the error has no message of its own.
    pub fn new(failure_title: &str, fetch: Fetch<T>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                items: Vec::new(),
                is_loading: false,
                failure: None,
                has_loaded: false,
                cursor: None,
                finished: false,
                generation: 0,
            })),
            loading: Arc::new(tokio::sync::Mutex::new(())),
            fetch,
            failure_title: failure_title.to_owned(),
        }
    }

    fn locked(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn view(&self) -> FeedView<T> {
        let state = self.locked();
        FeedView {
            items: state.items.clone(),
            is_loading: state.is_loading,
            failure: state.failure.clone(),
            has_more: !state.finished,
            has_loaded: state.has_loaded,
        }
    }

    pub async fn load_if_needed(&self) {
        if self.locked().has_loaded {
            return;
        }
        self.load_more().await;
    }

    /// The next page; ignored after the last one, and joins the load already running.
    pub async fn load_more(&self) {
        let Ok(_guard) = self.loading.try_lock() else {
            let _joined = self.loading.lock().await;
            return;
        };
        let (cursor, generation) = {
            let mut state = self.locked();
            if state.finished {
                return;
            }
            state.is_loading = true;
            (state.cursor.clone(), state.generation)
        };
        let fetched = (self.fetch)(cursor).await;
        let mut state = self.locked();
        if state.generation != generation {
            return;
        }
        state.is_loading = false;
        match fetched {
            Ok((items, next)) => {
                let known: std::collections::HashSet<String> = if state.has_loaded {
                    state.items.iter().map(FeedItem::feed_id).collect()
                } else {
                    state.items.clear();
                    std::collections::HashSet::new()
                };
                let fresh: Vec<T> = items
                    .into_iter()
                    .filter(|item| !known.contains(&item.feed_id()))
                    .collect();
                let is_empty = fresh.is_empty();
                state.items.extend(fresh);
                state.has_loaded = true;
                // A next page that repeats what is shown is the end too.
                match next {
                    Some(next) if !is_empty => state.cursor = Some(next),
                    _ => state.finished = true,
                }
                state.failure = None;
            }
            Err(PortError::Cancelled) => {}
            Err(error) => {
                state.failure = UserMessage::from_error(&error, Some(ProviderKind::Account))
                    .or_else(|| Some(UserMessage::new(&self.failure_title, "Try again.", None)));
            }
        }
    }

    /// Starts over from the first page; a page still loading is dropped.
    pub async fn refresh(&self) {
        {
            let mut state = self.locked();
            state.generation += 1;
            state.cursor = None;
            state.finished = false;
            state.has_loaded = false;
            state.failure = None;
            state.is_loading = false;
        }
        self.load_more().await;
        let mut state = self.locked();
        if !state.has_loaded {
            state.items.clear();
        }
    }

    /// Puts `item` first, as the newest, in place of an item with its ID. Ignored before the first page arrives,
    /// which brings it from the provider.
    pub fn insert(&self, item: T) {
        let mut state = self.locked();
        if !state.has_loaded {
            return;
        }
        let id = item.feed_id();
        state.items.retain(|old| old.feed_id() != id);
        state.items.insert(0, item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt as _;
    use jutsu_core::ProviderError;

    #[derive(Debug, Clone, PartialEq)]
    struct Item(String);

    impl FeedItem for Item {
        fn feed_id(&self) -> String {
            self.0.clone()
        }
    }

    /// The cursors a feed asked for, in order.
    type Asked = Arc<Mutex<Vec<Option<HistoryCursor>>>>;

    fn cursor(text: &str) -> HistoryCursor {
        HistoryCursor::new(text)
    }

    fn feed(pages: Vec<Result<Page<Item>, PortError>>) -> (PagedFeed<Item>, Asked) {
        let pages = Arc::new(Mutex::new(pages.into_iter()));
        let asked = Arc::new(Mutex::new(Vec::new()));
        let seen = asked.clone();
        let fetch: Fetch<Item> = Arc::new(move |after| {
            seen.lock().unwrap().push(after);
            let page = pages
                .lock()
                .unwrap()
                .next()
                .unwrap_or(Err(PortError::Other("no more pages".to_owned())));
            async move { page }.boxed()
        });
        (PagedFeed::new("Can't load the library", fetch), asked)
    }

    fn ids(feed: &PagedFeed<Item>) -> Vec<String> {
        feed.view().items.into_iter().map(|item| item.0).collect()
    }

    #[tokio::test]
    async fn pages_follow_the_cursor_and_skip_repeats_until_the_last() {
        let (feed, asked) = feed(vec![
            Ok((vec![Item("a".into()), Item("b".into())], Some(cursor("1")))),
            Ok((vec![Item("b".into()), Item("c".into())], Some(cursor("2")))),
            Ok((vec![Item("c".into())], Some(cursor("3")))),
        ]);
        feed.load_if_needed().await;
        feed.load_if_needed().await;
        assert_eq!(ids(&feed), ["a", "b"]);
        feed.load_more().await;
        assert_eq!(ids(&feed), ["a", "b", "c"]);
        assert!(feed.view().has_more);
        // A page that repeats what is shown is the end.
        feed.load_more().await;
        assert!(!feed.view().has_more);
        feed.load_more().await;
        assert_eq!(
            *asked.lock().unwrap(),
            [None, Some(cursor("1")), Some(cursor("2"))]
        );
    }

    #[tokio::test]
    async fn a_failure_is_shown_until_a_page_arrives() {
        let (feed, _) = feed(vec![
            Err(PortError::Provider(ProviderError::Network)),
            Ok((vec![Item("a".into())], None)),
        ]);
        feed.load_if_needed().await;
        let view = feed.view();
        assert_eq!(
            view.failure.map(|failure| failure.title).as_deref(),
            Some("Can't reach Higgsfield")
        );
        assert!(view.has_more);
        assert!(!view.has_loaded);
        feed.load_if_needed().await;
        assert_eq!(feed.view().failure, None);
        assert_eq!(ids(&feed), ["a"]);
    }

    #[tokio::test]
    async fn a_failure_without_a_message_uses_the_title() {
        let (feed, _) = feed(vec![Err(PortError::Cancelled)]);
        feed.load_more().await;
        assert_eq!(feed.view().failure, None);
    }

    #[tokio::test]
    async fn refresh_starts_over_and_insert_puts_the_item_first() {
        let (feed, _) = feed(vec![
            Ok((vec![Item("a".into()), Item("b".into())], None)),
            Ok((vec![Item("c".into())], None)),
        ]);
        feed.insert(Item("x".into()));
        assert_eq!(ids(&feed).len(), 0);
        feed.load_more().await;
        feed.insert(Item("b".into()));
        assert_eq!(ids(&feed), ["b", "a"]);
        feed.refresh().await;
        assert_eq!(ids(&feed), ["c"]);
    }
}
