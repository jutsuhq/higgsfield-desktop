//! The price of a request that isn't a job.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::models::Cost;
use crate::provider::ProviderKind;
use crate::support::string_enum;

string_enum! {
    /// What an image charge paid for.
    pub enum ImageChargePurpose {
        /// Reference images.
        ReferenceImages = "referenceImages",
        /// A virality score.
        ViralityScore = "viralityScore",
        /// Character sheets.
        CharacterSheets = "characterSheets",
    }
}

/// The price of one reference image request (`ImageProvider`), of one character sheet request, or of scoring one
/// result's virality (`ViralityScoring`). Neither is a `Job`, so the daily cap and Spending count these beside the
/// jobs. A request counts from its submit: while it runs, like a job in flight, and once it finished with images, on
/// the day it was sent. One that failed, was refused or was cancelled while queued isn't charged and is dropped.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ImageCharge {
    /// Its identity.
    pub id: Uuid,
    /// When the request was submitted.
    #[serde(with = "crate::support::date::rfc3339")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub date: OffsetDateTime,
    /// The price.
    pub cost: Cost,
    /// The connection kind.
    pub provider_kind: ProviderKind,
    /// The model's title, for Spending.
    pub model: String,
    /// Images or sheets asked for, or results scored.
    pub images: i64,
    /// Not finished yet. Still set after a quit, since the request may have finished and been charged since: it
    /// keeps counting then, on the day it was sent.
    pub is_pending: bool,
    /// What was paid for; `None` in charges stored before virality scores existed, which are reference images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<ImageChargePurpose>,
}

impl ImageCharge {
    /// A pending charge.
    pub fn new(
        id: Uuid,
        date: OffsetDateTime,
        cost: Cost,
        provider_kind: ProviderKind,
        model: impl Into<String>,
        images: i64,
        purpose: Option<ImageChargePurpose>,
    ) -> Self {
        Self {
            id,
            date,
            cost,
            provider_kind,
            model: model.into(),
            images,
            is_pending: true,
            purpose,
        }
    }

    /// `purpose`, reading a charge stored without one as reference images.
    pub fn paid_for(&self) -> ImageChargePurpose {
        self.purpose.unwrap_or(ImageChargePurpose::ReferenceImages)
    }
}
