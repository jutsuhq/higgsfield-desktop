//! AI Influencer character sheets as the catalog snapshot of 2026-10-15 states them, checked against live answers of
//! the free calls. A character for Genjutsu is not a trained Soul: it is a character sheet, an ordinary image job.
//!
//! - `ai_influencer_get_options {tier?}` (read-only, free) answers `{tiers, groups: [{id, label, max, options: [{id,
//!   label, slot, tiers}]}]}`.
//! - `ai_influencer_list_presets {cursor?, size?}` answers `{total, cursor, items: [{id, name, preview, sheet,
//!   recreate: {params: {tier, selection, brief, seed, medias}} | null, recreate_unavailable_reason}]}`; `cursor` is
//!   the next page's number, pages counted from 1.
//! - `ai_influencer_prepare` (free, not idempotent: a randomized design differs each time) answers `{params,
//!   credits, free_gens_remaining?, submitted: false}`.
//! - `ai_influencer_generate {params, max_credits}` (paid, not idempotent) answers `{submitted: false, credits,
//!   error}` when it refused, else `{submitted: true, job_id, job_ids, credits, results}`.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use url::Url;

use super::{AccountAssetPage, AssetKind, HiggsfieldAccount, ToolRole, failure, no_job_started, object};
use crate::core::HiggsfieldError;
use crate::mcp::json::{self, Object};

/// One option of a trait group.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InfluencerTraitOption {
    pub id: String,
    pub label: String,
    /// The tiers the option is offered for.
    pub tiers: Vec<String>,
}

/// A trait group of the AI Influencer builder: "Hair", "Eyes"…
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InfluencerTraitGroup {
    pub id: String,
    pub label: String,
    /// How many options of the group one design takes.
    pub max: i64,
    pub options: Vec<InfluencerTraitOption>,
}

/// What `recreate.params` holds: the design without images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfluencerPresetDesign {
    pub tier: String,
    pub selection: BTreeMap<String, Vec<String>>,
    pub brief: String,
    pub seed: Option<i64>,
}

/// A character sheet preset, with the settings that recreate it when it has them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfluencerPreset {
    pub id: String,
    pub name: String,
    pub preview_url: Option<Url>,
    pub sheet_url: Option<Url>,
    /// `None` when the preset can't be recreated (`unavailable_reason` says why).
    pub design: Option<InfluencerPresetDesign>,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfluencerPresetPage {
    pub items: Vec<InfluencerPreset>,
    /// The next page's `cursor`; `None` on the last page.
    pub next_cursor: Option<i64>,
}

/// How an image a design names is known to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InfluencerImageKind {
    MediaInput,
    ImageJob,
}

impl InfluencerImageKind {
    #[must_use]
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::MediaInput => "media_input",
            Self::ImageJob => "image_job",
        }
    }
}

/// An image a design names: an upload's `media_id` or a finished image job's ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InfluencerImage {
    pub id: String,
    pub kind: InfluencerImageKind,
}

impl InfluencerImage {
    fn argument(&self) -> Value {
        json!({ "role": "image", "data": { "id": self.id, "type": self.kind.raw_value() } })
    }
}

/// What `ai_influencer_prepare` is asked to design and price.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfluencerDesign {
    /// `None` leaves it to the server: random with `randomize`, "freak" without.
    pub tier: Option<String>,
    /// Option IDs by group ID; at most 4 per group are sent.
    pub selection: BTreeMap<String, Vec<String>>,
    pub brief: String,
    pub seed: Option<i64>,
    /// Sheets, 1–4.
    pub batch_size: i64,
    /// The face to keep, at most one.
    pub identity: Option<InfluencerImage>,
    /// Style references, at most three.
    pub styles: Vec<InfluencerImage>,
    /// Fill the traits left unset at random.
    pub randomize: bool,
}

impl InfluencerDesign {
    pub const MAX_BRIEF: usize = 4000;
    pub const MAX_PER_GROUP: usize = 4;
    pub const MAX_STYLES: usize = 3;
    pub const SEEDS: std::ops::RangeInclusive<i64> = 1..=1_000_000;
    pub const BATCH_SIZES: std::ops::RangeInclusive<i64> = 1..=4;

    #[must_use]
    pub fn new(tier: Option<String>) -> Self {
        Self {
            tier,
            selection: BTreeMap::new(),
            brief: String::new(),
            seed: None,
            batch_size: 1,
            identity: None,
            styles: Vec::new(),
            randomize: true,
        }
    }

    pub(crate) fn arguments(&self) -> Object {
        let selection: Object = self
            .selection
            .iter()
            .filter(|(_, options)| !options.is_empty())
            .map(|(group, options)| {
                (
                    group.clone(),
                    json!(options.iter().take(Self::MAX_PER_GROUP).collect::<Vec<_>>()),
                )
            })
            .collect();
        let mut arguments = object(json!({
            "selection": selection,
            "brief": self.brief.chars().take(Self::MAX_BRIEF).collect::<String>(),
            "batch_size": self.batch_size.clamp(*Self::BATCH_SIZES.start(), *Self::BATCH_SIZES.end()),
            "medias": self.identity.iter().map(InfluencerImage::argument).collect::<Vec<_>>(),
            "item_medias": self.styles.iter().take(Self::MAX_STYLES).map(InfluencerImage::argument).collect::<Vec<_>>(),
            "randomize": self.randomize,
        }));
        if let Some(tier) = &self.tier {
            arguments.insert("tier".into(), tier.clone().into());
        }
        if let Some(seed) = self.seed.filter(|seed| Self::SEEDS.contains(seed)) {
            arguments.insert("seed".into(), seed.into());
        }
        arguments
    }
}

/// `ai_influencer_prepare`'s answer: the exact params to generate, and their price.
#[derive(Debug, Clone, PartialEq)]
pub struct InfluencerQuote {
    /// Sent to `ai_influencer_generate` unchanged.
    pub params: Value,
    /// The price of the whole batch.
    pub credits: f64,
    pub free_gens_remaining: Option<i64>,
}

/// What `ai_influencer_generate` started: one job per sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct InfluencerSubmission {
    /// Every job ID the answer names, the first job's first; never empty.
    pub job_ids: Vec<String>,
    /// What the answer says it charged.
    pub credits: Option<f64>,
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

impl InfluencerTraitGroup {
    fn from_value(value: &Value) -> Option<Self> {
        let id = json::string(value, "id").filter(|id| !id.is_empty())?.to_owned();
        let options = json::array(value, "options")
            .into_iter()
            .flatten()
            .filter_map(|option| {
                let id = json::string(option, "id").filter(|id| !id.is_empty())?.to_owned();
                Some(InfluencerTraitOption {
                    label: json::string(option, "label").unwrap_or(&id).to_owned(),
                    tiers: strings(json::get(option, "tiers")),
                    id,
                })
            })
            .collect();
        Some(Self {
            label: json::string(value, "label").unwrap_or(&id).to_owned(),
            max: json::num(value, "max").and_then(json::count).unwrap_or(1),
            options,
            id,
        })
    }
}

impl InfluencerPreset {
    fn from_value(value: &Value) -> Option<Self> {
        let id = json::string(value, "id").filter(|id| !id.is_empty())?.to_owned();
        let design = json::get(value, "recreate")
            .and_then(|recreate| json::get(recreate, "params"))
            .and_then(|params| {
                let selection = json::get(params, "selection")
                    .and_then(Value::as_object)
                    .map(|selection| {
                        selection
                            .iter()
                            .map(|(group, options)| (group.clone(), strings(Some(options))))
                            .collect()
                    })
                    .unwrap_or_default();
                Some(InfluencerPresetDesign {
                    tier: json::string(params, "tier")?.to_owned(),
                    selection,
                    brief: json::string(params, "brief").unwrap_or("").to_owned(),
                    seed: json::num(params, "seed").and_then(json::count),
                })
            });
        let preview = json::get(value, "preview");
        let preview_url = preview
            .and_then(|preview| {
                json::string(preview, "thumbnail_url").or_else(|| json::string(preview, "url"))
            })
            .and_then(json::server_media);
        Some(Self {
            id,
            name: json::string(value, "name").unwrap_or("").to_owned(),
            preview_url,
            sheet_url: json::get(value, "sheet").and_then(|sheet| json::media_url(sheet, "url")),
            design,
            unavailable_reason: json::string(value, "recreate_unavailable_reason").map(str::to_owned),
        })
    }
}

impl HiggsfieldAccount {
    /// The trait groups, each with the options of `tier` only; every tier's when `tier` is `None`.
    pub async fn influencer_options(
        &self,
        tier: Option<&str>,
    ) -> Result<Vec<InfluencerTraitGroup>, HiggsfieldError> {
        let mut arguments = Object::new();
        if let Some(tier) = tier {
            arguments.insert("tier".into(), tier.into());
        }
        let value = self.call(ToolRole::InfluencerOptions, arguments).await?;
        let groups =
            json::array(&value, "groups").ok_or_else(|| failure(&value, ToolRole::InfluencerOptions))?;
        Ok(groups
            .iter()
            .filter_map(InfluencerTraitGroup::from_value)
            .map(|mut group| {
                if let Some(tier) = tier {
                    group.options.retain(|option| {
                        option.tiers.is_empty() || option.tiers.iter().any(|known| known == tier)
                    });
                }
                group
            })
            .collect())
    }

    /// A page of the character sheet presets, in the server's order. `cursor` is `next_cursor` of the page before;
    /// `size` is 1–100.
    pub async fn influencer_presets(
        &self,
        cursor: Option<i64>,
        size: i64,
    ) -> Result<InfluencerPresetPage, HiggsfieldError> {
        let size = size.clamp(1, 100);
        let mut arguments = object(json!({ "size": size }));
        if let Some(cursor) = cursor.filter(|cursor| *cursor >= 1) {
            arguments.insert("cursor".into(), cursor.into());
        }
        let value = self.call(ToolRole::InfluencerPresets, arguments).await?;
        let items =
            json::array(&value, "items").ok_or_else(|| failure(&value, ToolRole::InfluencerPresets))?;
        let presets = items.iter().filter_map(InfluencerPreset::from_value).collect();
        // A short page is the last; an empty one that still named a next one would never end.
        let cursor = json::num(&value, "cursor")
            .and_then(json::count)
            .filter(|cursor| *cursor > 0);
        let full = i64::try_from(items.len()).unwrap_or(i64::MAX) >= size;
        Ok(InfluencerPresetPage {
            items: presets,
            next_cursor: cursor.filter(|_| full),
        })
    }

    /// The exact params and price of `design`; free and starts nothing. A randomized design differs each time it is
    /// asked for, so what is generated is always the quote's `params`, never the design again.
    pub async fn prepare_influencer(
        &self,
        design: &InfluencerDesign,
    ) -> Result<InfluencerQuote, HiggsfieldError> {
        let value = self.call(ToolRole::InfluencerPrepare, design.arguments()).await?;
        let params = json::get(&value, "params").filter(|params| params.is_object());
        let credits = json::num(&value, "credits").filter(|credits| credits.is_finite() && *credits >= 0.0);
        let (Some(params), Some(credits)) = (params, credits) else {
            return Err(failure(&value, ToolRole::InfluencerPrepare));
        };
        Ok(InfluencerQuote {
            params: params.clone(),
            credits,
            free_gens_remaining: json::num(&value, "free_gens_remaining").and_then(json::count),
        })
    }

    /// Starts the quoted sheets, paid from the account's credits up to `max_credits`; never with free generations.
    /// Not idempotent: sending it again may start and charge another batch. A refusal (`submitted: false` with its
    /// `error`, or `isError`) is `Validation`: nothing started. An answer that names no job and gives no reason is
    /// `Server { status: 200 }`, as it may have started some.
    pub async fn generate_influencer(
        &self,
        quote: &InfluencerQuote,
        max_credits: f64,
    ) -> Result<InfluencerSubmission, HiggsfieldError> {
        let max_credits = json::number(max_credits.max(0.0))
            .ok_or_else(|| HiggsfieldError::validation("The request has a value that can't be sent"))?;
        let value = self
            .call(
                ToolRole::InfluencerGenerate,
                object(json!({ "params": quote.params, "max_credits": max_credits })),
            )
            .await?;
        let mut ids: Vec<String> = Vec::new();
        let named = json::string(&value, "job_id").into_iter().chain(
            json::array(&value, "job_ids")
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
        for id in named {
            if !id.is_empty() && !ids.iter().any(|known| known == id) {
                ids.push(id.to_owned());
            }
        }
        if ids.is_empty() {
            return Err(no_job_started(&value));
        }
        Ok(InfluencerSubmission {
            job_ids: ids,
            credits: json::num(&value, "credits"),
        })
    }

    /// A page of `assets_picker_list` `source: "influencer"`, images only: the account's character sheets, each by
    /// its job ID.
    pub async fn influencer_sheets(
        &self,
        cursor: Option<&Value>,
        size: i64,
    ) -> Result<AccountAssetPage, HiggsfieldError> {
        self.assets("influencer", AssetKind::Image, cursor, size).await
    }
}
