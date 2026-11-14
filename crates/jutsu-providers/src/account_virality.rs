//! Virality scores of the account's results (issues #45, #111), confirmed by a live run on 2026-10-15.

use std::time::Duration;

use async_trait::async_trait;
use higgsfield::mcp::account::{ViralityPrediction, ViralityStatus};
use jutsu_core::{Cost, PortError, ViralityOutcome, ViralityScore, ViralityScoring};
use rust_decimal::Decimal;
use time::OffsetDateTime;

use crate::account_mapping::whole_number;
use crate::mapping::{port, validation};
use crate::mcp_provider::HiggsfieldMcpProvider;

/// A live run on 2026-10-15 left the balance unchanged: scoring a video is free.
pub(crate) const PRICE: Cost = Cost::credits(Decimal::ZERO, false);
/// The pause between polls when the answer names none; the live answer said 1.5 s.
pub(crate) const DEFAULT_POLL_INTERVAL: f64 = 1.5;
/// No pause is longer than this, whatever the answer asks.
pub(crate) const MAXIMUM_POLL_INTERVAL: f64 = 10.0;
/// How many times a job is polled before the app gives up on it: about 5 minutes at the live 1.5 s (the live job
/// took about one).
pub(crate) const MAXIMUM_POLLS: usize = 200;

pub(crate) const FAILED: &str = "Higgsfield couldn't score this video.";
pub(crate) const TIMED_OUT: &str = "Higgsfield is still scoring this video. Try again later.";

pub(crate) fn pause(poll_interval: Option<f64>) -> Duration {
    let seconds = poll_interval
        .unwrap_or(DEFAULT_POLL_INTERVAL)
        .min(MAXIMUM_POLL_INTERVAL);
    Duration::try_from_secs_f64(seconds).unwrap_or(Duration::ZERO)
}

pub(crate) fn outcome(prediction: Option<&ViralityPrediction>, scored_at: OffsetDateTime) -> ViralityOutcome {
    ViralityOutcome {
        score: prediction.map(|prediction| score(prediction, scored_at)),
        charged: Some(PRICE),
    }
}

pub(crate) fn score(prediction: &ViralityPrediction, scored_at: OffsetDateTime) -> ViralityScore {
    ViralityScore {
        score: whole(prediction.overall),
        viral_potential: whole(prediction.viral_potential),
        hook: whole(prediction.hook),
        engagement: whole(prediction.engagement),
        sustain: whole(prediction.sustain),
        peak_second: prediction.peak_second,
        per_frame: (!prediction.per_frame.is_empty()).then(|| prediction.per_frame.clone()),
        disclaimer: prediction.disclaimer.clone(),
        report_url: prediction.dashboard_url.clone(),
        scored_at,
    }
}

fn whole(value: Option<f64>) -> Option<i64> {
    value.and_then(|value| whole_number(value.round()))
}

#[async_trait]
impl ViralityScoring for HiggsfieldMcpProvider {
    /// Free, as the live run showed; nothing is asked.
    async fn virality_estimate(&self, _job_id: &str) -> Result<Option<Cost>, PortError> {
        Ok(Some(PRICE))
    }

    /// Starts the Virality Predictor job and polls it with `job_status` (only that answer carries the readings)
    /// until it ends. The call is free, so an answer lost on the way is an ordinary error the user can retry.
    async fn score_virality(&self, job_id: &str) -> Result<ViralityOutcome, PortError> {
        let job = self.account.predict_virality(job_id).await.map_err(port)?;
        let pause = pause(job.poll_interval);
        for _ in 0..MAXIMUM_POLLS {
            tokio::time::sleep(pause).await;
            match self.account.virality_status(&job.id).await.map_err(port)? {
                ViralityStatus::Running => {}
                ViralityStatus::Completed(prediction) => {
                    return Ok(outcome(prediction.as_ref(), OffsetDateTime::now_utc()));
                }
                ViralityStatus::Failed { message } => {
                    return Err(validation(None, message.unwrap_or_else(|| FAILED.to_owned())));
                }
            }
        }
        Err(validation(None, TIMED_OUT))
    }
}
