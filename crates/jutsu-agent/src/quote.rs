//! What a video would cost now, before anything is uploaded or queued.

use crate::AgentAmount;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentQuote {
    /// `None` when no price list or quote says.
    pub price: Option<AgentAmount>,
    /// The provider's price or an exact list price; false for an estimate.
    pub is_exact: bool,
    pub mode: String,
    pub model: String,
    pub resolution: String,
    /// Seconds the result runs.
    pub duration: f64,
    /// What is left of today's agent budget; `None` while nothing is connected.
    pub remaining_today: Option<i64>,
    /// It would be paid from the plan's unlimited generations.
    pub pays_unlimited: bool,
    /// What `create_video` would refuse it for now, in English; `None` when it would queue it.
    pub blocker: Option<String>,
    /// What an estimate means for the job, in English; `None` for an exact price or none.
    pub price_note: Option<String>,
    /// The most `variants` takes for this request; 1 with `variants_unavailable` saying why.
    pub max_variants: i64,
    pub variants_unavailable: Option<String>,
    /// The reference images the job has, and how many its model takes in this mode.
    pub images: i64,
    pub min_images: i64,
    pub max_images: i64,
    /// The result's frame: `source`, `model`, or the ratio asked for; `None` for a model that takes none.
    pub aspect_ratio: Option<String>,
    /// What to check when the model may answer in another frame than the source's, in English.
    pub frame_note: Option<String>,
}

impl AgentQuote {
    /// What `create_video` would pause the job for, as far as a quote can tell before upload.
    pub fn blocker(
        price: Option<i64>,
        remaining: Option<i64>,
        pays_unlimited: bool,
        unlimited_allowed: bool,
    ) -> Option<String> {
        if pays_unlimited {
            return (!unlimited_allowed).then(|| {
                "It would be paid from the plan's unlimited generations, which the person hasn't let agents use \
                 (Settings › Agents); the job would wait for the person."
                    .to_owned()
            });
        }
        let Some(price) = price else {
            return Some(
                "The price isn't known before upload; unless Higgsfield quotes it then, the job waits for the person."
                    .to_owned(),
            );
        };
        if let Some(remaining) = remaining
            && price > remaining
        {
            return Some(format!(
                "It costs more than is left of today's agent budget ({remaining}); the job would wait for the person."
            ));
        }
        None
    }

    /// `price_note` for a price that is an estimate; `None` for an exact one, none, or one the plan's unlimited
    /// generations pay.
    pub fn price_note(is_estimate: bool, has_price: bool, pays_unlimited: bool) -> Option<String> {
        (is_estimate && has_price && !pays_unlimited).then(|| {
            "An estimate from the model's rate per second. Higgsfield quotes the exact price before the job is sent \
             (once its source is uploaded); the job then runs on its own when that price is within max_cost, what is \
             left of today's agent budget and the person's daily cap, and waits for the person only when it is above \
             one of them."
                .to_owned()
        })
    }
}
