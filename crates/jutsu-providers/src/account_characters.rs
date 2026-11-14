//! Maps the account's AI Influencer designs, presets and generated sheets.

use std::collections::BTreeMap;

use async_trait::async_trait;
use higgsfield::core::{GenerationStatus, HiggsfieldError, is_https};
use higgsfield::mcp::account::{
    AccountAsset, HiggsfieldAccount, InfluencerDesign, InfluencerImage, InfluencerImageKind,
    InfluencerPreset, InfluencerQuote, InfluencerTraitGroup,
};
use jutsu_core::{
    CharacterDesign, CharacterPreset, CharacterPresetPage, CharacterPresetSettings, CharacterQuote,
    CharacterSheet, CharacterSheetMaker, CharacterSheetPage, CharacterTier, CharacterTrait,
    CharacterTraitGroup, HistoryCursor, ImageJobStatus, PortError, PreparedFile, ProviderError, RemoteAsset,
    RemoteJob, decimal_shortest_of,
};
use rust_decimal::prelude::ToPrimitive;

use crate::history_mapping;
use crate::mapping::{INSECURE_RESULT, image_status, port, provider, rejection, validation};
use crate::mcp_provider::{HiggsfieldMcpProvider, page_cursor, unexpected_upload, unrepeatable};

pub(crate) const PRESET_PAGE_SIZE: i64 = 30;
pub(crate) const SHEET_PAGE_SIZE: i64 = 30;

pub(crate) fn group(group: &InfluencerTraitGroup) -> CharacterTraitGroup {
    CharacterTraitGroup::new(
        group.id.clone(),
        group.label.clone(),
        group.max,
        group
            .options
            .iter()
            .map(|option| CharacterTrait {
                id: option.id.clone(),
                title: option.label.clone(),
            })
            .collect(),
    )
}

pub(crate) fn preset(preset: &InfluencerPreset) -> CharacterPreset {
    let settings = preset.design.as_ref().and_then(|design| {
        CharacterTier::from_raw(&design.tier).map(|tier| CharacterPresetSettings {
            tier,
            traits: design.selection.clone(),
            brief: design.brief.clone(),
            seed: design.seed,
        })
    });
    let unavailable_reason = settings.is_none().then(|| {
        preset
            .unavailable_reason
            .clone()
            .unwrap_or_else(|| "This preset can't be made again.".to_owned())
    });
    CharacterPreset {
        id: preset.id.clone(),
        name: preset.name.clone(),
        preview_url: preset.preview_url.clone().or_else(|| preset.sheet_url.clone()),
        settings,
        unavailable_reason,
    }
}

pub(crate) fn sheet(asset: &AccountAsset) -> CharacterSheet {
    CharacterSheet {
        id: asset.id.clone(),
        image_url: asset.url.clone(),
        thumbnail_url: asset.thumbnail_url.clone(),
        created_at: Some(asset.created_at),
        file_url: None,
    }
}

/// The design as `ai_influencer_prepare` takes it: uploads as `media_input`. An image without a media ID can't be
/// named, so it is refused before anything is sent.
pub(crate) fn design(design: &CharacterDesign) -> Result<InfluencerDesign, PortError> {
    if !CharacterDesign::SHEET_COUNTS.contains(design.count) {
        return Err(validation(Some("count"), "Make 1 to 4 sheets."));
    }
    if design.styles.len() > CharacterDesign::MAX_STYLES {
        return Err(validation(None, "Use at most 3 style references."));
    }
    let image = |asset: &RemoteAsset| {
        asset
            .id
            .clone()
            .map(|id| InfluencerImage {
                id,
                kind: InfluencerImageKind::MediaInput,
            })
            .ok_or_else(unexpected_upload)
    };
    let mut request = InfluencerDesign::new(design.tier.map(|tier| tier.raw_value().to_owned()));
    request.selection.clone_from(&design.traits);
    request.brief.clone_from(&design.brief);
    request.seed = design.seed;
    request.batch_size = design.count;
    request.identity = design.face.as_ref().map(image).transpose()?;
    request.styles = design.styles.iter().map(image).collect::<Result<_, _>>()?;
    request.randomize = design.randomize;
    Ok(request)
}

pub(crate) fn quote(quote: &InfluencerQuote, count: i64) -> Result<CharacterQuote, PortError> {
    let credits =
        decimal_shortest_of(quote.credits).ok_or(provider(ProviderError::Server { status: 200 }))?;
    let request = serde_json::to_vec(&quote.params).map_err(|error| PortError::Other(error.to_string()))?;
    Ok(CharacterQuote {
        credits,
        count,
        request,
    })
}

pub(crate) fn influencer_quote(quote: &CharacterQuote) -> Result<InfluencerQuote, PortError> {
    let params =
        serde_json::from_slice(&quote.request).map_err(|error| PortError::Other(error.to_string()))?;
    Ok(InfluencerQuote {
        params,
        credits: quote.credits.to_f64().unwrap_or(0.0),
        free_gens_remaining: None,
    })
}

pub(crate) fn status(status: GenerationStatus) -> ImageJobStatus {
    match status {
        GenerationStatus::Queued | GenerationStatus::InProgress => ImageJobStatus::Pending,
        GenerationStatus::Completed { video_url } if is_https(&video_url) => {
            ImageJobStatus::Completed(vec![video_url])
        }
        GenerationStatus::Completed { .. } => ImageJobStatus::Failed {
            message: Some(INSECURE_RESULT.to_owned()),
        },
        GenerationStatus::Failed { message } => ImageJobStatus::Failed { message },
        GenerationStatus::Rejected(moderation) => ImageJobStatus::Rejected(rejection(moderation)),
        GenerationStatus::Canceled => ImageJobStatus::Canceled,
    }
}

#[async_trait]
impl CharacterSheetMaker for HiggsfieldMcpProvider {
    fn owner(&self) -> Option<String> {
        self.owner.clone()
    }

    async fn character_traits(&self, tier: CharacterTier) -> Result<Vec<CharacterTraitGroup>, PortError> {
        let groups = self
            .account
            .influencer_options(Some(tier.raw_value()))
            .await
            .map_err(port)?;
        Ok(groups
            .iter()
            .filter(|group| !group.options.is_empty())
            .map(group)
            .collect())
    }

    async fn character_presets(
        &self,
        after: Option<HistoryCursor>,
    ) -> Result<CharacterPresetPage, PortError> {
        let cursor = after.and_then(|cursor| cursor.value.parse::<i64>().ok());
        let page = self
            .account
            .influencer_presets(cursor, PRESET_PAGE_SIZE)
            .await
            .map_err(port)?;
        Ok(CharacterPresetPage {
            items: page.items.iter().map(preset).collect(),
            next_cursor: page.next_cursor.map(|next| HistoryCursor::new(next.to_string())),
        })
    }

    async fn character_sheets(&self, after: Option<HistoryCursor>) -> Result<CharacterSheetPage, PortError> {
        let cursor = after.as_ref().and_then(history_mapping::cursor_value);
        let page = self
            .account
            .influencer_sheets(cursor.as_ref(), SHEET_PAGE_SIZE)
            .await
            .map_err(port)?;
        Ok(CharacterSheetPage {
            items: page.items.iter().map(sheet).collect(),
            next_cursor: page_cursor(page.items.is_empty(), page.next_cursor.as_ref()),
        })
    }

    async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError> {
        jutsu_core::GenerationProvider::upload(self, file).await
    }

    async fn quote_characters(&self, character: CharacterDesign) -> Result<CharacterQuote, PortError> {
        let request = design(&character)?;
        let prepared = self.account.prepare_influencer(&request).await.map_err(port)?;
        quote(&prepared, character.count)
    }

    /// `max_credits` is the quote's own price, so the server refuses the submit when the price went up since. Sent
    /// once: any failure once the call may have reached the server fails with `OutcomeUnknown`.
    async fn generate_characters(&self, character: CharacterQuote) -> Result<Vec<RemoteJob>, PortError> {
        let request = influencer_quote(&character)?;
        let submission = self
            .account
            .generate_influencer(&request, request.credits)
            .await
            .map_err(unrepeatable)?;
        Ok(submission.job_ids.into_iter().map(RemoteJob::new).collect())
    }

    /// One `jobs_wait` for up to 12 jobs; the jobs it doesn't answer for, or all of them when it can't be used, are
    /// asked with `job_status` one by one. A job whose status can't be read is left out.
    async fn character_sheet_statuses(
        &self,
        jobs: Vec<RemoteJob>,
    ) -> Result<BTreeMap<String, ImageJobStatus>, PortError> {
        let ids: Vec<String> = jobs.into_iter().map(|job| job.request_id).collect();
        let mut statuses = BTreeMap::new();
        if ids.len() <= HiggsfieldAccount::JOBS_WAIT_LIMIT {
            match self.account.wait_for_jobs(&ids).await {
                Ok(Some(answer)) => {
                    statuses.extend(
                        answer
                            .statuses
                            .into_iter()
                            .map(|(id, answered)| (id, status(answered))),
                    );
                }
                Err(HiggsfieldError::Cancelled) => return Err(PortError::Cancelled),
                Ok(None) | Err(_) => {}
            }
        }
        for id in ids {
            if statuses.contains_key(&id) {
                continue;
            }
            match self.account.image_status(&id).await {
                Ok(answered) => {
                    statuses.insert(id, image_status(answered));
                }
                Err(HiggsfieldError::Cancelled) => return Err(PortError::Cancelled),
                Err(_) => {}
            }
        }
        Ok(statuses)
    }
}
