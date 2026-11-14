//! `enhance` on this computer (`AgentService+OnDevice.swift`): Smooth Motion and Upscale with engine `mac`, free, run
//! once for its request ID and followed with `jobs` as kind `on_device`.

use jutsu_agent::{
    AgentActed, AgentBudgetUnit, AgentCreated, AgentError, AgentJob, AgentJobAction, AgentSheet,
    EnhanceCommand, ErrorCode, OnDevice,
};
use jutsu_core::OnDeviceEngine;
use time::OffsetDateTime;
use uuid::Uuid;

use super::images::NoRaise;
use super::{AgentService, refusals};
use crate::agents::app_id;
use crate::agents::budget::unit_for;
use crate::on_device::agent::forgotten;

impl AgentService {
    /// The unit the free price is given in: the connection's, else cents.
    fn on_device_unit(&self) -> AgentBudgetUnit {
        self.environment
            .connection
            .state()
            .kind()
            .map_or(AgentBudgetUnit::UsdCents, unit_for)
    }

    pub(super) async fn enhance_on_device(
        &self,
        fields: &EnhanceCommand,
        client: Option<String>,
    ) -> Result<AgentCreated, AgentError> {
        let me = self.strong()?;
        let queued = fields.clone();
        let request_id = &fields.request_id;
        let unit = self.on_device_unit();
        self.requests
            .run(
                &fields.request_id,
                &fields.fingerprint,
                None,
                refusals("enhance", "its run was never started"),
                || {},
                |ids| async move {
                    let agents = self.environment.on_device.agents();
                    let jobs: Vec<AgentJob> = ids
                        .iter()
                        .map(|id| agents.job(*id, unit).unwrap_or_else(|| forgotten(*id)))
                        .collect();
                    Ok((!jobs.is_empty()).then(|| AgentCreated::new(request_id, true, jobs)))
                },
                None::<NoRaise>,
                move || async move {
                    let on_device = &me.environment.on_device;
                    let target = on_device
                        .agent_target(app_id(queued.item))
                        .await
                        .map_err(|_| no_item_here(queued.item))?;
                    let job = on_device.agents().start(&queued, client, &target, unit, |ids| {
                        me.record_ids(&queued.request_id, &queued.fingerprint, None, ids.to_vec())
                    })?;
                    Ok(AgentCreated::new(&queued.request_id, false, vec![job]))
                },
            )
            .await
    }

    /// What `capabilities` says of this computer's engines.
    pub(super) fn on_device_capabilities(&self) -> OnDevice {
        self.environment.on_device.agents().capabilities()
    }

    /// The runs agents started, for `jobs` without IDs.
    pub(super) fn on_device_entries(&self) -> Vec<(OffsetDateTime, Uuid)> {
        self.environment.on_device.agents().started()
    }

    pub(super) fn on_device_job(&self, id: Uuid) -> Option<AgentJob> {
        self.environment.on_device.agents().job(id, self.on_device_unit())
    }

    pub(super) fn on_device_is_settled(&self, id: Uuid) -> bool {
        self.environment.on_device.agents().is_settled(id)
    }

    /// `job_action` on a run here: only cancel; `None` when `id` isn't one.
    pub(super) async fn act_on_device(
        &self,
        action: AgentJobAction,
        id: Uuid,
    ) -> Result<Option<AgentActed>, AgentError> {
        let agents = self.environment.on_device.agents();
        if self.environment.on_device.runs().run(id).is_none() {
            return Ok(None);
        }
        if action != AgentJobAction::Cancel {
            return Err(only_cancel());
        }
        let job = agents.cancel(id, self.on_device_unit()).await?;
        Ok(Some(AgentActed { action, job }))
    }

    /// Why `show` can't open the sheet of `item` (`showSheet`): no video or image here, then for Upscale on This
    /// Computer a computer without the engine; the Subtitles sheet takes a video only.
    pub(super) async fn sheet_refusal(
        &self,
        sheet: AgentSheet,
        item: jutsu_agent::Uuid,
    ) -> Option<AgentError> {
        if sheet == AgentSheet::Subtitles {
            return self.subtitles_sheet_refusal(app_id(item)).await;
        }
        let on_device = &self.environment.on_device;
        if !on_device.has_item(app_id(item)).await {
            return Some(no_item_here(item));
        }
        on_device
            .agents()
            .sheet_refusal(on_device.unavailable_reason(OnDeviceEngine::VideoSuperResolution))
    }
}

/// `show` doesn't put `sheet` over the one the window shows (`showSheet`, `showSubtitles`): Upscale on This Computer
/// replaces only its own sheet, the Subtitles sheet that one or its own.
pub(super) fn sheet_open(sheet: AgentSheet, open: Option<super::WindowSheet>) -> Option<AgentError> {
    let replaces = match open? {
        super::WindowSheet::OnDevice => true,
        super::WindowSheet::Subtitles => sheet == AgentSheet::Subtitles,
        super::WindowSheet::Other => false,
    };
    (!replaces).then(|| {
        AgentError::new(
            ErrorCode::NEEDS_PERSON,
            "The person has a sheet open in Jutsu Studio, so the sheet isn't shown over it.",
            true,
            Some("Ask the person to finish or close it, then call show again."),
        )
    })
}

/// `job_action` other than cancel on a run on this computer.
pub(super) fn only_cancel() -> AgentError {
    AgentError::new(
        ErrorCode::NOT_APPLICABLE,
        "A run on this computer can only be canceled.",
        false,
        Some("Send the request again with a new request_id for another run."),
    )
}

/// No video or image of the Library with its file here (`onDeviceItem`).
pub(super) fn no_item_here(item: jutsu_agent::Uuid) -> AgentError {
    AgentError::new(
        ErrorCode::NOT_FOUND,
        format!("There's no video or image {item} with a file on this computer."),
        false,
        Some("Call library_search for the item IDs; library_download brings one from Higgsfield."),
    )
}
