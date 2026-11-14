//! Finds what the account was charged for a finished job Higgsfield can't quote beforehand (a Topaz upscale:
//! "job_type 'topaz_video' does not support alpha v2 cost estimation", issue #113). A credit transaction names no
//! job, only what it was for and when (`CreditTransaction`), so the charge is matched by its name and time; anything
//! unclear leaves the price unknown rather than guessing.

use std::collections::HashSet;

use higgsfield::mcp::account::CreditTransaction;
use jutsu_core::{ClosedRange, Cost, Job, PortError};
use serde_json::Value;
use time::{Duration, OffsetDateTime};

use crate::mapping::{port, server_credits};
use crate::mcp_provider::HiggsfieldMcpProvider;

/// How far a charge may be from the job's run: the device's clock and the server's can differ, and the charge may
/// be booked when the job is accepted or when it finishes.
pub(crate) const TOLERANCE: Duration = Duration::seconds(120);
pub(crate) const PAGE_SIZE: i64 = 50;
/// Pages of the history read at most, newest first, before giving up.
pub(crate) const MAX_PAGES: usize = 4;

/// From `TOLERANCE` before the submit to `TOLERANCE` after the job finished (or after the submit, when its end isn't
/// known).
pub(crate) fn window(
    submitted_at: OffsetDateTime,
    finished_at: Option<OffsetDateTime>,
) -> ClosedRange<OffsetDateTime> {
    let end = submitted_at.max(finished_at.unwrap_or(submitted_at));
    ClosedRange::new(submitted_at - TOLERANCE, end + TOLERANCE)
}

/// Whether a transaction's name says it was a Topaz video upscale. Higgsfield's wording isn't documented, and other
/// charges read like "Higgsfield Genjutsu - Object Swap", so only the provider's name is relied on, in any case; a
/// Topaz image upscale is told apart by "image".
pub(crate) fn is_topaz_video(title: &str) -> bool {
    let title = folded(title);
    title.contains("topaz") && !title.contains("image")
}

/// `title` lowercased with the accents of the letters of "topaz" and "image" dropped, as Swift's case- and
/// diacritic-insensitive search reads it.
fn folded(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|character| match character {
            'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
            'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
            'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' => 'i',
            'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
            'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
            'ţ' | 'ť' => 't',
            'ź' | 'ż' | 'ž' => 'z',
            other => other,
        })
        .collect()
}

/// The credits of the one Topaz spend in `window`. `None` when there is none, or when anything else named for Topaz
/// is in the window too: a second spend (two upscales sent together can't be told apart) or a refund.
pub(crate) fn credits(
    transactions: &[CreditTransaction],
    window: ClosedRange<OffsetDateTime>,
) -> Option<f64> {
    // Pages that overlap list the same transaction twice.
    let mut named: Vec<&CreditTransaction> = Vec::new();
    for transaction in transactions {
        if window.contains(transaction.date)
            && is_topaz_video(&transaction.title)
            && !named.contains(&transaction)
        {
            named.push(transaction);
        }
    }
    let [charge] = named.as_slice() else {
        return None;
    };
    if charge.action.to_lowercase() != "spend" || !charge.credits.is_finite() || charge.credits == 0.0 {
        return None;
    }
    // Spent amounts are negative.
    Some(charge.credits.abs())
}

/// Whether a page reaching back to `oldest` may still be followed by transactions in `window`.
pub(crate) fn reads_on(oldest: Option<OffsetDateTime>, window: ClosedRange<OffsetDateTime>) -> bool {
    oldest.is_some_and(|oldest| oldest >= window.lower())
}

impl HiggsfieldMcpProvider {
    /// A Topaz upscale's charge, read from the account's credit history (`transactions`, read-only) once it has
    /// finished, and matched to it by its name and time. `None` for any other job, one without a submit time, and
    /// when no single charge matches.
    pub(crate) async fn topaz_charge(&self, job: &Job) -> Result<Option<Cost>, PortError> {
        let priced_after_run = job
            .spec
            .operation
            .as_ref()
            .is_some_and(|operation| operation.action.is_priced_after_run());
        let (true, Some(submitted_at)) = (priced_after_run, job.submitted_at) else {
            return Ok(None);
        };
        let window = window(submitted_at, job.charged_at);
        let mut transactions = Vec::new();
        let mut cursor: Option<Value> = None;
        let mut seen = HashSet::new();
        for _ in 0..MAX_PAGES {
            let page = self
                .account
                .transactions(cursor.as_ref(), PAGE_SIZE)
                .await
                .map_err(port)?;
            let oldest = page.items.last().map(|transaction| transaction.date);
            transactions.extend(page.items);
            // Newest first: a page that reaches back past the window is the last one needed.
            let Some(next) = page.next_cursor else {
                break;
            };
            if !seen.insert(next.to_string()) || !reads_on(oldest, window) {
                break;
            }
            cursor = Some(next);
        }
        let Some(credits) = credits(&transactions, window) else {
            return Ok(None);
        };
        // Up, as every server price.
        Ok(server_credits(credits, false, 1).ok())
    }
}
