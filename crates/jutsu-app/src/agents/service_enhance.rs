//! `enhance` on Higgsfield (`AgentService+Library.swift`): the Enhance sheet's job on a downloaded video result, queued
//! once for its request ID with the agent's origin, so the engine's spending guard checks it before its submit.

use jutsu_agent::{AgentCreated, AgentEnhancement, AgentError, EnhanceCommand, ErrorCode};
use jutsu_core::{AgentOrigin, JobSpec, PostAction, PostActionKind, ProviderKind};
use jutsu_engine::EnqueueOptions;
use uuid::Uuid;

use super::images::NoRaise;
use super::{AgentService, check_unit, enqueue_error, not_connected, refusals};
use crate::agents::budget::money;
use crate::agents::enhance_order::{AgentEnhanceOrder, AgentEnhanceRefusal, RefusalKind};
use crate::agents::job::agent_job;
use crate::agents::library_search::rows_by_id;
use crate::library::enhance::{EnhanceAvailability, OFFERS_POST_OPERATIONS, ReframeCanvas, operation_spec};
use crate::library::item::LibraryItem;
use crate::library::media::MediaRow;

impl AgentService {
    pub(super) async fn enhance_on_higgsfield(
        &self,
        fields: &EnhanceCommand,
        client: Option<String>,
    ) -> Result<AgentCreated, AgentError> {
        let me = self.strong()?;
        let queued = fields.clone();
        let request_id = &fields.request_id;
        self.requests
            .run(
                &fields.request_id,
                &fields.fingerprint,
                None,
                refusals("enhance", "its job was never queued"),
                || {},
                |ids| async move {
                    let jobs = self.known_jobs(&ids).await?;
                    Ok((!jobs.is_empty())
                        .then(|| AgentCreated::new(request_id, true, jobs.iter().map(agent_job).collect())))
                },
                None::<NoRaise>,
                move || async move { me.queue_enhancement(&queued, client).await },
            )
            .await
    }

    /// Builds the job as the Enhance sheet would, records the request with its ID, then queues it with the agent's
    /// origin: the engine's spending guard checks it before its submit, and holds one whose price isn't known.
    async fn queue_enhancement(
        &self,
        fields: &EnhanceCommand,
        client: Option<String>,
    ) -> Result<AgentCreated, AgentError> {
        let Some(kind) = self.kind() else {
            return Err(not_connected());
        };
        // The command's own check has refused one without it already.
        let Some(max_cost) = fields.max_cost else {
            return Err(AgentError::bad_request("enhance on Higgsfield needs max_cost"));
        };
        check_unit(&max_cost, kind)?;
        let item = self.enhance_target(fields.item.into()).await?;
        let spec = self.enhancement_spec(fields, &item, kind).await?;
        let job_id = Uuid::new_v4();
        self.record_ids(&fields.request_id, &fields.fingerprint, None, vec![job_id])?;
        let origin = AgentOrigin::new(client, fields.request_id.clone(), money(max_cost));
        let options = EnqueueOptions {
            id: Some(job_id),
            parent_id: Some(item.id()),
            agent: Some(origin),
            ..EnqueueOptions::default()
        };
        let job = self
            .environment
            .engine
            .enqueue(spec, options)
            .await
            .map_err(|error| enqueue_error(&error))?;
        tracing::info!(
            operation = fields.operation.as_str(),
            "an agent queued an enhancement"
        );
        Ok(AgentCreated::new(
            &fields.request_id,
            false,
            vec![agent_job(&job)],
        ))
    }

    /// A video result with its file on this computer.
    async fn enhance_target(&self, id: Uuid) -> Result<LibraryItem, AgentError> {
        let materials = self.materials().await?;
        match rows_by_id(&materials, |_| true).remove(&id) {
            Some(MediaRow::Video(item)) if item.history_item.is_none() && item.output_file().is_some() => {
                Ok(item)
            }
            _ => Err(AgentError::new(
                ErrorCode::NOT_FOUND,
                format!(
                    "There's no video {} with a file on this computer.",
                    super::wire_id(id)
                ),
                false,
                Some("Call library_search with kind video for the item IDs."),
            )),
        }
    }

    /// The job the Enhance sheet would queue for these fields on `item`, refused as the sheet would refuse it.
    async fn enhancement_spec(
        &self,
        fields: &EnhanceCommand,
        item: &LibraryItem,
        connection: ProviderKind,
    ) -> Result<JobSpec, AgentError> {
        let Some(kind) = post_kind(fields.operation) else {
            return Err(AgentError::bad_request(format!(
                "enhance: {} runs on this computer only",
                fields.operation.as_str()
            )));
        };
        let order = AgentEnhanceOrder {
            kind,
            target: fields.target.clone(),
            frame_rate: fields.frame_rate,
            preset: fields.preset.clone(),
            engine: fields.engine.clone(),
            aspect_ratio: fields.aspect_ratio.clone(),
            language: fields.language.clone(),
            voice: fields.voice.clone(),
        };
        let voices = if kind == PostActionKind::ChangeVoice {
            self.environment
                .library
                .voices()
                .await
                .ok()
                .map(|rows| rows.into_iter().map(|row| row.choice).collect::<Vec<_>>())
        } else {
            None
        };
        let action = order
            .action(voices.as_deref())
            .map_err(|refusal| error(&refusal))?;
        let availability = EnhanceAvailability::of(item, Some(connection), OFFERS_POST_OPERATIONS);
        if let Some(refusal) =
            AgentEnhanceOrder::refusal(&action, item, &availability, item.job.output_duration())
        {
            return Err(error(&refusal));
        }
        if let PostAction::Reframe(aspect) = &action
            && let Some(size) = item.job.output_pixel_size()
            && ReframeCanvas::new(size, *aspect).is_some_and(|canvas| canvas.is_unchanged())
        {
            return Err(AgentError::invalid_request(format!(
                "The video is {} already, so Reframe has nothing to fill in.",
                aspect.raw_value()
            )));
        }
        let owner = self
            .environment
            .provider
            .current()
            .and_then(|provider| provider.owner());
        operation_spec(&item.job, action, owner.as_deref(), None, None)
            .ok_or_else(|| AgentError::invalid_request("The video's file isn't on this computer."))
    }
}

fn post_kind(operation: AgentEnhancement) -> Option<PostActionKind> {
    match operation {
        AgentEnhancement::Upscale => Some(PostActionKind::Upscale),
        AgentEnhancement::Reframe => Some(PostActionKind::Reframe),
        AgentEnhancement::RemoveBackground => Some(PostActionKind::RemoveBackground),
        AgentEnhancement::Dub => Some(PostActionKind::Dub),
        AgentEnhancement::ChangeVoice => Some(PostActionKind::ChangeVoice),
        AgentEnhancement::SmoothMotion => None,
    }
}

fn error(refusal: &AgentEnhanceRefusal) -> AgentError {
    match refusal.kind {
        RefusalKind::Invalid => AgentError::bad_request(format!("enhance: {}", refusal.message)),
        RefusalKind::PricedAfterRun => AgentError::new(
            ErrorCode::PRICED_AFTER_RUN,
            refusal.message.clone(),
            false,
            Some("Use the standard engine, or ask the person to run a Topaz upscale themselves."),
        ),
        RefusalKind::Unavailable => AgentError::new(
            ErrorCode::NOT_APPLICABLE,
            refusal.message.clone(),
            true,
            Some("Ask the person to check."),
        ),
    }
}
