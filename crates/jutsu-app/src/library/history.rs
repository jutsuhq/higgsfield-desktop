//! The account's history as the Library pages through it (`AccountHistory.swift`): newest first, for one account
//! at a time; and what the Higgsfield origin shows instead of the grid.

use std::collections::{HashMap, HashSet};

use jutsu_core::{HistoryCursor, HistoryImageItem, HistoryItem, HistoryPage, PortError, ProviderKind};
use serde::Serialize;
use uuid::Uuid;

use crate::connection::{ConnectionState, UserMessage};

/// The state of the last load.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HistoryPhase {
    #[default]
    Idle,
    Loading,
    /// The last load failed; the items loaded before stay.
    Failed(UserMessage),
}

/// One load to run: the first page on a refresh, else the page at `cursor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRequest {
    pub owner: Option<String>,
    pub cursor: Option<HistoryCursor>,
    pub is_refresh: bool,
    token: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountHistory {
    owner: Option<String>,
    items: Vec<HistoryItem>,
    images: Vec<HistoryImageItem>,
    phase: HistoryPhase,
    next_cursor: Option<HistoryCursor>,
    has_loaded: bool,
    /// Library IDs by remote ID, so an item keeps its ID across refreshes.
    job_ids: HashMap<String, Uuid>,
    token: u64,
    empty_pages_in_a_row: usize,
}

impl AccountHistory {
    /// Pages in a row that may add nothing before loading stops until the user asks for more.
    pub const MAX_EMPTY_PAGES_IN_A_ROW: usize = 5;

    pub fn new(owner: Option<String>) -> Self {
        Self {
            owner,
            ..Self::default()
        }
    }

    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    pub fn items(&self) -> &[HistoryItem] {
        &self.items
    }

    pub fn images(&self) -> &[HistoryImageItem] {
        &self.images
    }

    pub fn phase(&self) -> &HistoryPhase {
        &self.phase
    }

    pub fn next_cursor(&self) -> Option<&HistoryCursor> {
        self.next_cursor.as_ref()
    }

    pub fn has_loaded(&self) -> bool {
        self.has_loaded
    }

    /// Another account, or none, starts from an empty history; the same account keeps it.
    pub fn switch_account(&mut self, owner: Option<String>) {
        if owner == self.owner {
            return;
        }
        let token = self.token + 1;
        *self = Self::new(owner);
        self.token = token;
    }

    /// There may be pages after the loaded ones.
    pub fn has_more(&self) -> bool {
        !self.has_loaded || self.next_cursor.is_some()
    }

    pub fn is_loading(&self) -> bool {
        self.phase == HistoryPhase::Loading
    }

    /// Loading stopped after too many pages that added nothing; more may follow when the user asks.
    pub fn is_waiting_to_load_more(&self) -> bool {
        self.phase == HistoryPhase::Idle
            && self.next_cursor.is_some()
            && self.empty_pages_in_a_row >= Self::MAX_EMPTY_PAGES_IN_A_ROW
    }

    /// Under the grid after the last page: that Styles made elsewhere aren't listed (`HistoryFooter`). None while
    /// a page failed, more may load, or nothing is listed.
    pub fn footer_note(&self) -> Option<&'static str> {
        let failed = matches!(self.phase, HistoryPhase::Failed(_));
        (!failed && !self.is_waiting_to_load_more() && !self.has_more() && !self.items.is_empty())
            .then_some(STYLES_NOTE)
    }

    /// Starts a load, or `None` when none should start: a page is already loading, or there are no more. A refresh
    /// always starts and wins over a load in flight.
    pub fn begin_load(&mut self, refresh: bool) -> Option<HistoryRequest> {
        if !refresh && self.is_loading() {
            return None;
        }
        let is_refresh = refresh || !self.has_loaded;
        if !is_refresh && self.next_cursor.is_none() {
            return None;
        }
        self.token += 1;
        self.phase = HistoryPhase::Loading;
        // Past the limit only the user starts a load, which counts the empty pages afresh.
        if is_refresh || self.empty_pages_in_a_row >= Self::MAX_EMPTY_PAGES_IN_A_ROW {
            self.empty_pages_in_a_row = 0;
        }
        Some(HistoryRequest {
            owner: self.owner.clone(),
            cursor: if is_refresh {
                None
            } else {
                self.next_cursor.clone()
            },
            is_refresh,
            token: self.token,
        })
    }

    /// Applies the page `request` asked for; answers to a superseded request are dropped. `true` when the page
    /// added nothing but more may follow, so the next one should load right away.
    pub fn receive(&mut self, page: HistoryPage, request: &HistoryRequest) -> bool {
        if request.token != self.token || request.owner != self.owner {
            return false;
        }
        let before = if request.is_refresh {
            0
        } else {
            self.items.len() + self.images.len()
        };
        if request.is_refresh {
            self.items.clear();
            self.images.clear();
        }
        let mut seen: HashSet<String> = self.items.iter().map(|item| item.id.clone()).collect();
        for mut item in page.items {
            if seen.insert(item.id.clone()) {
                item.job_id = *self.job_ids.entry(item.id.clone()).or_insert(item.job_id);
                self.items.push(item);
            }
        }
        let mut seen: HashSet<String> = self.images.iter().map(|image| image.id.clone()).collect();
        for mut image in page.images {
            if seen.insert(image.id.clone()) {
                image.job_id = *self.job_ids.entry(image.id.clone()).or_insert(image.job_id);
                self.images.push(image);
            }
        }
        let added = (self.items.len() + self.images.len()).saturating_sub(before);
        self.next_cursor = page.next_cursor;
        self.has_loaded = true;
        self.phase = HistoryPhase::Idle;
        self.empty_pages_in_a_row = if added == 0 {
            self.empty_pages_in_a_row + 1
        } else {
            0
        };
        added == 0 && self.next_cursor.is_some() && self.empty_pages_in_a_row < Self::MAX_EMPTY_PAGES_IN_A_ROW
    }

    /// Records a like set from the app, so the history agrees with the star until it is loaded again.
    pub fn set_liked(&mut self, id: &str, is_liked: bool) {
        for item in self.items.iter_mut().filter(|item| item.id == id) {
            item.is_liked = is_liked;
        }
        for image in self.images.iter_mut().filter(|image| image.id == id) {
            image.is_liked = is_liked;
        }
    }

    pub fn fail(&mut self, error: &PortError, request: &HistoryRequest) {
        if request.token != self.token || request.owner != self.owner {
            return;
        }
        let message = UserMessage::from_error(error, Some(ProviderKind::Account)).unwrap_or_else(|| {
            UserMessage::new(
                "Can't load your Higgsfield history",
                "Try again in a moment.",
                None,
            )
        });
        self.phase = HistoryPhase::Failed(message);
    }

    /// A load that failed for a reason of this computer, such as the catalog.
    pub fn fail_with(&mut self, message: UserMessage, request: &HistoryRequest) {
        if request.token == self.token && request.owner == self.owner {
            self.phase = HistoryPhase::Failed(message);
        }
    }

    /// A load that was dropped before it answered.
    pub fn cancel(&mut self, request: &HistoryRequest) {
        if request.token == self.token && self.is_loading() {
            self.phase = HistoryPhase::Idle;
        }
    }
}

/// "No videos on Higgsfield yet".
pub const EMPTY_TITLE: &str = "No videos on Higgsfield yet";
/// Neither history tool returns a finished Style job, so only the Styles the app started itself are known.
pub const STYLES_NOTE: &str =
    "Styles made outside Jutsu Studio aren't listed: Higgsfield's history doesn't return them.";

pub fn empty_detail() -> String {
    format!(
        "Finished Motion and Swap videos from Genjutsu, Kling and Seedance on your account appear here. {STYLES_NOTE}"
    )
}

/// Why the active connection can't show the account's history; `None` when it can.
pub fn unavailable_reason(connection: &ConnectionState) -> Option<(&'static str, &'static str)> {
    match connection {
        ConnectionState::Account(_) => None,
        ConnectionState::ApiKey { .. } => Some((
            "History needs a Higgsfield account",
            "An API key can't read past generations. Sign in to your account in Settings to see them.",
        )),
        ConnectionState::None => Some((
            "Sign in to see your Higgsfield history",
            "Videos made on higgsfield.ai appear here once you sign in to your account in Settings.",
        )),
    }
}

/// What the Higgsfield origin shows instead of the grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HistoryPlaceholder {
    /// Why this connection has no history.
    Unavailable { title: String, detail: String },
    /// The first load failed; offers a retry.
    Failed { title: String, message: String },
    /// The first load is running.
    Loading,
    /// The account has no history.
    Empty { title: String, detail: String },
}

impl HistoryPlaceholder {
    /// `None` once there are items to show. `is_empty`: the origin has no items, before filters.
    pub fn new(connection: &ConnectionState, history: &AccountHistory, is_empty: bool) -> Option<Self> {
        if let Some((title, detail)) = unavailable_reason(connection) {
            return Some(Self::Unavailable {
                title: title.to_owned(),
                detail: detail.to_owned(),
            });
        }
        if !is_empty {
            return None;
        }
        Some(match history.phase() {
            HistoryPhase::Failed(message) => Self::Failed {
                title: message.title.clone(),
                message: message.message.clone(),
            },
            _ if !history.has_loaded() || history.is_loading() => Self::Loading,
            _ => Self::Empty {
                title: EMPTY_TITLE.to_owned(),
                detail: empty_detail(),
            },
        })
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
