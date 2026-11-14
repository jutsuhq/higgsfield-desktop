//! `AgentActivity.swift`: what the tray panel shows of agents besides their jobs: the clients that sent requests
//! lately, and the requests that wait for the person to agree to a model download on this computer. In memory only, so
//! a relaunch forgets both.

use std::collections::HashMap;

use jutsu_agent::AgentSheet;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// A request an agent couldn't run until the person agrees in an item's sheet (`needs_person`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub item: Uuid,
    pub sheet: AgentSheet,
    /// The item's title, as the Library shows it.
    pub title: String,
    pub asked_at: OffsetDateTime,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentActivity {
    /// Each client's last noted request.
    last_request: HashMap<String, OffsetDateTime>,
    /// Newest first.
    asks: Vec<Ask>,
}

impl AgentActivity {
    /// A client counts as connected for this long after its last request: the helper holds no connection open, it
    /// sends each request on its own.
    pub const CONNECTED_FOR: Duration = Duration::minutes(10);
    /// An ask nobody answered within this is dropped: the agent has moved on.
    pub const ASK_LIFETIME: Duration = Duration::hours(1);
    /// How often a client's last request is noted: often enough for `CONNECTED_FOR`, rarely enough not to redraw the
    /// panel for every `jobs` call.
    pub const NOTE_INTERVAL: Duration = Duration::minutes(1);
    pub const UNNAMED_CLIENT: &str = "agent";
    pub const KEPT_ASKS: usize = 10;

    pub fn asks(&self) -> &[Ask] {
        &self.asks
    }

    /// Notes a request from `client`; changes nothing while its last one was noted less than `NOTE_INTERVAL` ago.
    pub fn note_request(&mut self, client: Option<&str>, at: OffsetDateTime) {
        let name = client
            .filter(|name| !name.is_empty())
            .unwrap_or(Self::UNNAMED_CLIENT);
        if self
            .last_request
            .get(name)
            .is_some_and(|last| at - *last < Self::NOTE_INTERVAL)
        {
            return;
        }
        self.last_request.insert(name.to_owned(), at);
    }

    /// The clients whose last request was less than `CONNECTED_FOR` ago, by name.
    pub fn connected_clients(&self, now: OffsetDateTime) -> Vec<String> {
        let mut names: Vec<String> = self
            .last_request
            .iter()
            .filter(|(_, last)| now - **last < Self::CONNECTED_FOR)
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
    }

    /// Notes an ask, replacing an older one for the same item and sheet, and drops the asks past `ASK_LIFETIME`.
    pub fn note_ask(&mut self, ask: Ask) {
        self.asks.retain(|old| {
            !((old.item == ask.item && old.sheet == ask.sheet)
                || ask.asked_at - old.asked_at >= Self::ASK_LIFETIME)
        });
        self.asks.insert(0, ask);
        self.asks.truncate(Self::KEPT_ASKS);
    }

    /// The person opened the item's sheet, or the agent's request went through.
    pub fn clear_ask(&mut self, item: Uuid, sheet: AgentSheet) {
        self.asks.retain(|ask| !(ask.item == item && ask.sheet == sheet));
    }

    /// The asks still waiting at `now`.
    pub fn open_asks(&self, now: OffsetDateTime) -> Vec<Ask> {
        self.asks
            .iter()
            .filter(|ask| now - ask.asked_at < Self::ASK_LIFETIME)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_000_000).unwrap()
    }

    fn ask(item: Uuid, sheet: AgentSheet, minutes: i64) -> Ask {
        Ask {
            item,
            sheet,
            title: "clip".to_owned(),
            asked_at: start() + Duration::minutes(minutes),
        }
    }

    #[test]
    fn a_client_is_connected_for_ten_minutes_after_its_last_request() {
        let mut activity = AgentActivity::default();
        activity.note_request(Some("claude-code"), start());
        activity.note_request(None, start());
        activity.note_request(Some(""), start());
        assert_eq!(activity.connected_clients(start()), ["agent", "claude-code"]);
        assert_eq!(
            activity.connected_clients(start() + Duration::minutes(10)),
            Vec::<String>::new()
        );
    }

    /// Requests within the note interval change nothing, so the panel isn't redrawn for each.
    #[test]
    fn a_request_soon_after_the_last_is_not_noted() {
        let mut activity = AgentActivity::default();
        activity.note_request(Some("codex"), start());
        let noted = activity.clone();
        activity.note_request(Some("codex"), start() + Duration::seconds(30));
        assert_eq!(activity, noted);
        activity.note_request(Some("codex"), start() + Duration::seconds(60));
        assert_ne!(activity, noted);
    }

    #[test]
    fn an_ask_replaces_the_older_one_for_its_item_and_sheet_and_expires() {
        let item = Uuid::new_v4();
        let mut activity = AgentActivity::default();
        activity.note_ask(ask(item, AgentSheet::UpscaleOnMac, 0));
        activity.note_ask(ask(item, AgentSheet::Subtitles, 1));
        activity.note_ask(ask(item, AgentSheet::UpscaleOnMac, 2));
        assert_eq!(activity.asks().len(), 2);
        assert_eq!(activity.asks()[0].asked_at, start() + Duration::minutes(2));
        assert_eq!(activity.open_asks(start() + Duration::minutes(61)).len(), 1);
        // An ask an hour newer than the subtitles one drops it.
        activity.note_ask(ask(Uuid::new_v4(), AgentSheet::Subtitles, 61));
        assert_eq!(activity.asks().len(), 2);
        activity.clear_ask(item, AgentSheet::UpscaleOnMac);
        assert_eq!(activity.asks().len(), 1);
    }

    #[test]
    fn only_the_newest_asks_are_kept() {
        let mut activity = AgentActivity::default();
        for minute in 0..12 {
            activity.note_ask(ask(Uuid::new_v4(), AgentSheet::Subtitles, minute));
        }
        assert_eq!(activity.asks().len(), AgentActivity::KEPT_ASKS);
        assert_eq!(activity.asks()[0].asked_at, start() + Duration::minutes(11));
    }
}
