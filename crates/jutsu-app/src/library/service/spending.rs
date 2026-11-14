//! Settings › Spending on the Library's store and image charges (`SpendingSettingsView`): the summary of a period,
//! the CSV, the cap and low-credit fields, and the account's credit history.

use std::sync::Arc;

use futures::FutureExt as _;
use jutsu_core::{CostUnit, CreditEntry, ProviderKind};
use time::OffsetDateTime;

use super::LibraryService;
use crate::create::paged_feed::{Fetch, PagedFeed};
use crate::preferences::PreferencesError;
use crate::settings::spending::{
    self, CREDIT_HISTORY_FAILURE, CREDIT_HISTORY_STEP, CreditHistoryView, SpendingPeriod, SpendingSummary,
    SpendingView,
};

/// The credit history of one connection and how many of its entries are shown.
pub(super) struct CreditFeed {
    generation: u64,
    feed: PagedFeed<CreditEntry>,
    shown: usize,
}

impl LibraryService {
    /// USD for an API key, credits for an account; `None` while nothing is connected.
    fn spending_unit(&self) -> Option<CostUnit> {
        match self.connection.state().kind()? {
            ProviderKind::ApiKey => Some(CostUnit::Usd),
            ProviderKind::Account => Some(CostUnit::Credits),
        }
    }

    /// What is committed today in the connection's unit, as the spend cap counts it; `None` while nothing is
    /// connected or when it can't be read. The tray panel's header shows it for an API key.
    pub async fn committed_today(&self) -> Option<rust_decimal::Decimal> {
        let unit = self.spending_unit()?;
        spending::committed_today(
            &self.store,
            &self.image_spend,
            unit,
            OffsetDateTime::now_utc(),
            self.utc_offset,
        )
        .await
        .ok()
    }

    /// Settings › Spending for `period`; `None` while nothing is connected, when the tab isn't listed.
    pub async fn spending(&self, period: SpendingPeriod) -> Result<Option<SpendingView>, String> {
        let Some(unit) = self.spending_unit() else {
            return Ok(None);
        };
        let now = OffsetDateTime::now_utc();
        let jobs = self
            .store
            .finished_jobs_since(SpendingPeriod::Month.start(now, self.utc_offset))
            .await
            .map_err(|error| error.to_string())?;
        let summary = SpendingSummary::new(&jobs, &self.image_spend.charges(), period, now, self.utc_offset);
        let committed =
            spending::committed_today(&self.store, &self.image_spend, unit, now, self.utc_offset).await;
        Ok(Some(spending::spending_view(
            &summary,
            period,
            unit,
            &self.preferences.get(),
            Some(committed),
            now,
            self.utc_offset,
        )))
    }

    /// Export CSV…: every finished job and image charge, oldest first, and the file's suggested name.
    pub async fn spending_csv(&self) -> Result<(String, String), String> {
        let jobs = self
            .store
            .finished_jobs()
            .await
            .map_err(|error| error.to_string())?;
        let csv = spending::spending_csv(&jobs, &self.image_spend.charges(), self.utc_offset);
        Ok((
            csv,
            spending::csv_file_name(OffsetDateTime::now_utc(), self.utc_offset),
        ))
    }

    /// The Daily cap toggle of the connection's unit.
    pub fn set_spend_cap_enabled(&self, is_on: bool) -> Result<(), PreferencesError> {
        let Some(unit) = self.spending_unit() else {
            return Ok(());
        };
        self.preferences
            .update(spending::cap_enabled_patch(unit, is_on))?;
        Ok(())
    }

    /// The Amount field on commit: a valid amount becomes the cap, anything else keeps the last one. Returns what
    /// the field shows next.
    pub fn commit_spend_cap(&self, text: &str) -> Result<String, PreferencesError> {
        let Some(unit) = self.spending_unit() else {
            return Ok(text.to_owned());
        };
        if let Some(patch) = spending::cap_patch(text, unit) {
            self.preferences.update(patch)?;
        }
        Ok(spending::cap_text(&self.preferences.get(), unit))
    }

    /// The Warn below field on commit, like the cap.
    pub fn commit_low_credits(&self, text: &str) -> Result<String, PreferencesError> {
        if let Some(patch) = spending::low_credits_patch(text) {
            self.preferences.update(patch)?;
        }
        Ok(spending::low_credits_text(&self.preferences.get()))
    }

    /// The credit feed of the current connection, made on first use and again for another one; `None` for an API
    /// key, whose history can't be read.
    fn credit_feed(&self) -> Option<PagedFeed<CreditEntry>> {
        let mut state = self.locked();
        let generation = state.account.generation;
        if let Some(current) = state
            .credit_feed
            .as_ref()
            .filter(|feed| feed.generation == generation)
        {
            return Some(current.feed.clone());
        }
        let source = state.account.credits.clone()?;
        let fetch: Fetch<CreditEntry> = Arc::new(move |cursor| {
            let source = source.clone();
            async move {
                let page = source.credit_history(cursor).await?;
                Ok((page.items, page.next_cursor))
            }
            .boxed()
        });
        let feed = PagedFeed::new(CREDIT_HISTORY_FAILURE, fetch);
        state.credit_feed = Some(CreditFeed {
            generation,
            feed: feed.clone(),
            shown: CREDIT_HISTORY_STEP,
        });
        Some(feed)
    }

    fn credit_view(&self, feed: &PagedFeed<CreditEntry>) -> CreditHistoryView {
        let shown = self
            .locked()
            .credit_feed
            .as_ref()
            .map_or(CREDIT_HISTORY_STEP, |current| current.shown);
        CreditHistoryView::new(&feed.view(), shown, self.utc_offset)
    }

    /// Credit history: the first page loads on first view.
    pub async fn credit_history(&self) -> Option<CreditHistoryView> {
        let feed = self.credit_feed()?;
        feed.load_if_needed().await;
        Some(self.credit_view(&feed))
    }

    /// Show More: ten more entries, and the next page once the loaded ones run out.
    pub async fn credit_history_more(&self) -> Option<CreditHistoryView> {
        let feed = self.credit_feed()?;
        let shown = {
            let mut state = self.locked();
            let current = state.credit_feed.as_mut()?;
            current.shown += CREDIT_HISTORY_STEP;
            current.shown
        };
        let view = feed.view();
        if view.items.len() < shown && view.has_more {
            feed.load_more().await;
        }
        Some(self.credit_view(&feed))
    }

    /// Try Again after a failed page.
    pub async fn credit_history_retry(&self) -> Option<CreditHistoryView> {
        let feed = self.credit_feed()?;
        feed.load_more().await;
        Some(self.credit_view(&feed))
    }
}
