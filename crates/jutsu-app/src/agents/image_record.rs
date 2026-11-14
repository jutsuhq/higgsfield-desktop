//! An agent's `create_image` request (`AgentImageRecord.swift`, `AgentImageQueue+Texts.swift`): the fields checked
//! against the connection's image models with the Image side of Create's rules, the record `agent-images.json` keeps,
//! and how `jobs` sees it.

use std::path::PathBuf;

use jutsu_agent::{AgentJob, Timestamp};
use jutsu_core::{
    AgentHold, AgentMoney, AgentOrigin, Cost, CostUnit, ImageModel, ImageModelInput, JobSpec, LocalImage,
    ProviderKind,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::OffsetDateTime;
use uuid::Uuid;

use super::budget;
use super::job::hold_name;
use super::wire_id;
use crate::images::form::ImageRequestDraft;

/// An agent's `create_image` fields, never read from or written to the visible form.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentImageOrder {
    /// `None` takes the connection's first model, as the form does.
    pub model: Option<String>,
    pub prompt: String,
    /// `None` takes 1:1 when the model makes it, else its first frame.
    pub aspect_ratio: Option<String>,
    /// `None` makes one image.
    pub count: Option<i64>,
    /// The images to edit, in order.
    pub inputs: Vec<LocalImage>,
    /// The agent's name for the images, as it sent it; kept trimmed and cut (`JobSpec::title_from`).
    pub title: Option<String>,
}

/// Why an agent's image request isn't taken: the request itself, or a log that can't be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct AgentImageRefusal {
    pub message: String,
    /// The request is fine, but Jutsu Studio can't record it, so nothing is sent.
    pub is_not_saved: bool,
}

impl AgentImageRefusal {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            is_not_saved: false,
        }
    }
}

impl AgentImageOrder {
    /// The request the form would send, or why it can't be sent, in English for the agent.
    pub fn draft(
        &self,
        models: &[ImageModel],
        models_are_saved: bool,
    ) -> Result<ImageRequestDraft, AgentImageRefusal> {
        if models.is_empty() {
            return Err(AgentImageRefusal::new(
                "This connection offers no image models right now.",
            ));
        }
        if models_are_saved {
            return Err(AgentImageRefusal::new(
                "Jutsu Studio is still reading the image models; try again shortly.",
            ));
        }
        let chosen = match &self.model {
            Some(id) => models.iter().find(|model| &model.id == id),
            None => models.first(),
        };
        let Some(chosen) = chosen else {
            let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
            return Err(AgentImageRefusal::new(format!(
                "There's no image model {}. This connection offers {}.",
                self.model.as_deref().unwrap_or_default(),
                ids.join(", ")
            )));
        };
        let prompt = self.prompt.trim();
        if prompt.is_empty() {
            return Err(AgentImageRefusal::new(
                "create_image needs prompt, the image to make.",
            ));
        }
        match chosen.input {
            ImageModelInput::Required if self.inputs.is_empty() => {
                return Err(AgentImageRefusal::new(format!(
                    "{} edits an image: send inputs, the images to edit.",
                    chosen.id
                )));
            }
            ImageModelInput::None if !self.inputs.is_empty() => {
                return Err(AgentImageRefusal::new(format!(
                    "{} makes images from text only; leave out inputs.",
                    chosen.id
                )));
            }
            _ => {}
        }
        let given = i64::try_from(self.inputs.len()).unwrap_or(i64::MAX);
        if given > chosen.input_limit() {
            return Err(AgentImageRefusal::new(format!(
                "{} takes at most {} input images, not {given}.",
                chosen.id,
                chosen.input_limit()
            )));
        }
        let ratio = self.aspect_ratio.clone().or_else(|| {
            if chosen.aspect_ratios.iter().any(|ratio| ratio == "1:1") {
                Some("1:1".to_owned())
            } else {
                chosen.aspect_ratios.first().cloned()
            }
        });
        let Some(ratio) = ratio.filter(|ratio| chosen.aspect_ratios.contains(ratio)) else {
            return Err(AgentImageRefusal::new(format!(
                "{} makes {}, not {}.",
                chosen.id,
                chosen.aspect_ratios.join(", "),
                self.aspect_ratio.as_deref().unwrap_or("any frame")
            )));
        };
        let count = self
            .count
            .unwrap_or_else(|| chosen.variant_counts.first().copied().unwrap_or(1));
        if !chosen.variant_counts.contains(&count) {
            let counts: Vec<String> = chosen.variant_counts.iter().map(ToString::to_string).collect();
            return Err(AgentImageRefusal::new(format!(
                "{} makes {} images per request, not {count}.",
                chosen.id,
                counts.join(", ")
            )));
        }
        Ok(ImageRequestDraft {
            model: chosen.clone(),
            prompt: prompt.to_owned(),
            aspect_ratio: ratio,
            count,
            inputs: self.inputs.clone(),
            title: JobSpec::title_from(self.title.as_deref()),
        })
    }
}

/// Where a request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentImageState {
    /// Taken, its price being read; nothing was sent.
    Pricing,
    /// The guard stopped it; only the person sends it.
    Held,
    /// Reserved and being sent: it may have reached the provider.
    Sending,
    /// Accepted, its images being made.
    Following,
    Finished,
    Failed,
    Canceled,
}

/// Why it is held: one of the agent spending guard's holds, as a video job's, or the person's own daily cap. Stored
/// as the case's name, as `AgentHold`'s are: "overMaxCost", "priceAboveCap"…
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentImageHold {
    Agent(AgentHold),
    /// It takes today past the person's own daily cap.
    PriceAboveCap,
}

const PRICE_ABOVE_CAP: &str = "priceAboveCap";

impl Serialize for AgentImageHold {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Agent(hold) => hold.serialize(serializer),
            Self::PriceAboveCap => serializer.serialize_str(PRICE_ABOVE_CAP),
        }
    }
}

impl<'de> Deserialize<'de> for AgentImageHold {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        if name == PRICE_ABOVE_CAP {
            return Ok(Self::PriceAboveCap);
        }
        serde_json::from_value::<AgentHold>(serde_json::Value::String(name.clone()))
            .map(Self::Agent)
            .map_err(|_| serde::de::Error::custom(format!("No hold {name}")))
    }
}

/// An agent's image request, as `agent-images.json` keeps it from the moment it is taken until 30 days after it
/// ended: what was asked, what the guard saw and whether it may have been sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentImageRecord {
    /// Also the image run's ID: what `jobs` and `job_action` take.
    pub id: Uuid,
    pub origin: AgentOrigin,
    pub provider_kind: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub model: ImageModel,
    pub prompt: String,
    pub aspect_ratio: String,
    pub count: i64,
    /// The first image to edit. A request of one image keeps only this, which is all a 0.2.0 record holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<LocalImage>,
    /// Every image to edit, in order, when there are several; `None` for one or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub several_inputs: Option<Vec<LocalImage>>,
    pub state: AgentImageState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<AgentImageHold>,
    /// The price the guard last saw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<Cost>,
    pub price_is_known: bool,
    /// The price the person agreed to with Send on a held request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_price: Option<Cost>,
    /// Saved with the reservation, right before the submit: from then on it may have been sent.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "jutsu_core::support::date::rfc3339_option"
    )]
    pub sent_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(with = "jutsu_core::support::date::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "jutsu_core::support::date::rfc3339_option"
    )]
    pub finished_at: Option<OffsetDateTime>,
    /// The agent's name for the images; `None` without one, and in records from before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl AgentImageRecord {
    /// A request just taken, before its price is read.
    pub fn taken(
        id: Uuid,
        origin: AgentOrigin,
        kind: ProviderKind,
        owner: Option<String>,
        draft: ImageRequestDraft,
        now: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            origin,
            provider_kind: kind,
            owner,
            model: draft.model,
            prompt: draft.prompt,
            aspect_ratio: draft.aspect_ratio,
            count: draft.count,
            input: draft.inputs.first().cloned(),
            several_inputs: (draft.inputs.len() > 1).then_some(draft.inputs),
            state: AgentImageState::Pricing,
            hold: None,
            price: None,
            price_is_known: false,
            approved_price: None,
            sent_at: None,
            error: None,
            created_at: now,
            finished_at: None,
            title: draft.title,
        }
    }

    /// The images to edit, in order.
    pub fn inputs(&self) -> Vec<LocalImage> {
        self.several_inputs
            .clone()
            .or_else(|| self.input.clone().map(|input| vec![input]))
            .unwrap_or_default()
    }

    pub fn draft(&self) -> ImageRequestDraft {
        ImageRequestDraft {
            model: self.model.clone(),
            prompt: self.prompt.clone(),
            aspect_ratio: self.aspect_ratio.clone(),
            count: self.count,
            inputs: self.inputs(),
            title: self.title.clone(),
        }
    }

    /// It may have reached the provider, so its price counts.
    pub fn may_have_been_sent(&self) -> bool {
        self.sent_at.is_some()
    }

    pub fn is_over(&self) -> bool {
        matches!(
            self.state,
            AgentImageState::Finished | AgentImageState::Failed | AgentImageState::Canceled
        )
    }

    /// The model's list price for the request, an estimate.
    pub fn list_price(&self) -> Option<Cost> {
        let price = self.model.list_price.as_ref()?;
        let total = price.amount() * Decimal::from(self.count);
        Some(match price.unit() {
            CostUnit::Usd => Cost::usd(total, true),
            CostUnit::Credits => Cost::credits(total, true),
        })
    }
}

/// Why a held request waits, for its row in the Queue and its tile in Create: the reason in the words a held video
/// job's row gives it, then what an image request's row offers, Review… and then Send, where a video job's row has
/// Confirm.
pub fn hold_text(hold: AgentImageHold) -> &'static str {
    match hold {
        AgentImageHold::Agent(AgentHold::UnknownPrice) => {
            "An agent's job at a price not known yet · review to send it"
        }
        AgentImageHold::Agent(AgentHold::OverMaxCost) => {
            "Costs more than the agent was allowed · review to send it"
        }
        AgentImageHold::Agent(AgentHold::OverDailyBudget) => "Over today's agent budget · review to send it",
        AgentImageHold::Agent(AgentHold::LedgerUnavailable) => {
            "Agent spending couldn't be read · nothing was sent, review to send it"
        }
        AgentImageHold::Agent(AgentHold::UnlimitedNotAllowed) => {
            "Would use your plan's unlimited generations, which agents may not use · review to send it"
        }
        AgentImageHold::Agent(AgentHold::AboveApprovedPrice) => {
            "Costs more now than the price you approved · review to send it"
        }
        AgentImageHold::PriceAboveCap => {
            "Price from Higgsfield goes over today's spending cap · review to send it"
        }
    }
}

pub const NOT_RECORDED: &str = "Jutsu Studio lost the record of this request.";
pub const LOST_ANSWER: &str =
    "Jutsu Studio quit while sending this request, so it may have been sent. Its price still counts.";

/// An agent's image request as `jobs` sees it; `files` are its images saved in Library › Images.
pub fn image_job(record: &AgentImageRecord, files: &[PathBuf]) -> AgentJob {
    let state = match record.state {
        AgentImageState::Pricing => "preparing",
        AgentImageState::Held => "paused",
        AgentImageState::Sending | AgentImageState::Following => "processing",
        AgentImageState::Finished if files.is_empty() => "downloading",
        AgentImageState::Finished => "done",
        AgentImageState::Failed => "failed",
        AgentImageState::Canceled => "canceled",
    };
    let mut job = AgentJob::new(wire_id(record.id), state, Some(Timestamp(record.created_at)));
    "image".clone_into(&mut job.kind);
    job.request_id = Some(record.origin.request_id.clone());
    job.client.clone_from(&record.origin.client);
    job.mode = Some("image".to_owned());
    job.model = Some(record.model.id.clone());
    job.finished_at = record.finished_at.map(Timestamp);
    if record.state == AgentImageState::Held {
        job.pause_reason = Some(
            if record.hold == Some(AgentImageHold::PriceAboveCap) {
                "price_above_cap"
            } else {
                "agent_limit"
            }
            .to_owned(),
        );
        job.hold = match record.hold {
            Some(AgentImageHold::Agent(hold)) => Some(hold_name(hold).to_owned()),
            _ => None,
        };
        job.needs_person = true;
    }
    job.price = record
        .price
        .as_ref()
        .and_then(AgentMoney::from_cost)
        .map(budget::amount);
    job.is_price_exact = record.price_is_known;
    job.charged = record.may_have_been_sent();
    job.error = (record.state == AgentImageState::Failed)
        .then(|| record.error.clone())
        .flatten();
    let paths: Vec<String> = files.iter().map(|file| file.display().to_string()).collect();
    job.result = paths.first().cloned();
    job.results = (!paths.is_empty()).then_some(paths);
    job
}
