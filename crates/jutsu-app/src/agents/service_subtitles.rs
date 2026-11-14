//! `subtitles` (`AgentService+Subtitles.swift`): make, a run followed with `jobs` as kind `on_device`; get and export
//! of the current version's cues. The rules are `AgentSubtitleRuns`'; this reads the item and writes the export.

use std::collections::BTreeSet;
use std::io::Write;

use jutsu_agent::{
    AgentActed, AgentError, AgentJob, AgentJobAction, AgentResponse, AgentSubtitleFormat, AgentSubtitles,
    AgentSubtitlesAction, ErrorCode, SubtitlesCommand,
};
use jutsu_core::OutputSubfolder;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{AgentService, app_id, refusals};
use crate::library::media::{MediaRow, material_row};
use crate::subtitles::agent::{AgentSubtitleRuns, ON_DEVICE_KIND, SubtitleTarget};

impl AgentService {
    pub(super) async fn subtitles(
        &self,
        fields: &SubtitlesCommand,
        client: Option<String>,
    ) -> Result<AgentResponse, AgentError> {
        let mut response = AgentResponse::default();
        match fields.action {
            AgentSubtitlesAction::Make => {
                let me = self.strong()?;
                let recorder = self.strong()?;
                let (request_id, fingerprint) = (
                    fields.request_id.clone().unwrap_or_default(),
                    fields.fingerprint.clone().unwrap_or_default(),
                );
                let item = app_id(fields.item);
                let created = self
                    .environment
                    .subtitles
                    .agent()
                    .make(
                        fields,
                        client,
                        &self.requests,
                        refusals("subtitles", "its run never started"),
                        move |ids| recorder.record_on_device(&request_id, &fingerprint, ids),
                        move || async move { me.subtitle_target(item).await },
                    )
                    .await?;
                response.created = Some(created);
            }
            AgentSubtitlesAction::Get => {
                let target = self.subtitle_target(app_id(fields.item)).await?;
                response.subtitles = Some(AgentSubtitleRuns::answer(
                    target.subtitles.as_ref(),
                    target.source,
                    fields.format,
                    None,
                )?);
            }
            AgentSubtitlesAction::Export => response.subtitles = Some(self.export_subtitles(fields).await?),
        }
        Ok(response)
    }

    /// Records a run on this computer with its ID before it starts.
    fn record_on_device(&self, request_id: &str, fingerprint: &str, ids: &[Uuid]) -> Result<(), AgentError> {
        self.requests
            .record(
                request_id,
                fingerprint,
                None,
                ids.to_vec(),
                OffsetDateTime::now_utc(),
                Some(ON_DEVICE_KIND),
            )
            .map_err(|_| {
                AgentError::new(
                    ErrorCode::FAILED,
                    "Jutsu Studio can't record the request, so nothing is queued or sent.",
                    true,
                    Some("Ask the person to check the computer has room, then send the request again."),
                )
            })
    }

    /// Writes the current version's cues into the output folder's Exports folder, never over a file.
    async fn export_subtitles(&self, fields: &SubtitlesCommand) -> Result<AgentSubtitles, AgentError> {
        let target = self.subtitle_target(app_id(fields.item)).await?;
        let format = fields.format.unwrap_or(AgentSubtitleFormat::Srt);
        let answer = AgentSubtitleRuns::answer(target.subtitles.as_ref(), target.source, None, None)?;
        let Some(subtitles) = &target.subtitles else {
            return Ok(answer);
        };
        let folder = OutputSubfolder::Exports.path(&self.output_folder());
        let existing: BTreeSet<String> = std::fs::read_dir(&folder)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        let name = AgentSubtitleRuns::file_name(&target.file, subtitles.language(), format, &existing);
        let file = folder.join(name);
        let written = std::fs::create_dir_all(&folder).and_then(|()| {
            std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(&file)?
                .write_all(AgentSubtitleRuns::text(subtitles, format).as_bytes())
        });
        if let Err(error) = written {
            return Err(AgentError::new(
                ErrorCode::FAILED,
                format!("The subtitles can't be written: {error}"),
                true,
                Some("Check that the output folder can be written to, then export again."),
            ));
        }
        AgentSubtitleRuns::answer(Some(subtitles), target.source, None, Some(&file))
    }

    /// The video's current version, its file and its subtitles, as the Subtitles sheet would read them. An image is
    /// refused as one; anything else as no item with a file here (`onDeviceItem`).
    async fn subtitle_target(&self, id: Uuid) -> Result<SubtitleTarget, AgentError> {
        self.video_here(id, || {
            AgentError::new(
                ErrorCode::INVALID_REQUEST,
                "Subtitles are made of a video's speech; this item is an image.",
                false,
                Some("Pick a video's Library ID (library_search with kind video)."),
            )
        })
        .await
    }

    /// Why `show` can't open the Subtitles sheet of `id`: it is an image, or no video here (`showSubtitles`).
    pub(super) async fn subtitles_sheet_refusal(&self, id: Uuid) -> Option<AgentError> {
        self.video_here(id, || {
            AgentError::new(
                ErrorCode::INVALID_REQUEST,
                "The Subtitles sheet is for videos; this item is an image.",
                false,
                Some("Pick a video's Library ID."),
            )
        })
        .await
        .err()
    }

    /// The video `id` with its file here; an image is refused with `image`, anything else as no item here.
    async fn video_here(
        &self,
        id: Uuid,
        image: impl FnOnce() -> AgentError,
    ) -> Result<SubtitleTarget, AgentError> {
        if let Some(target) = self.environment.subtitles.target(id).await {
            return Ok(target);
        }
        let material = self.environment.store.material(id).await.ok().flatten();
        if let Some(MediaRow::LocalImage(_)) = material.as_ref().and_then(material_row) {
            return Err(image());
        }
        Err(super::on_device::no_item_here(super::wire_id(id)))
    }

    /// The subtitles runs agents started, for `jobs` without IDs.
    pub(super) fn subtitle_entries(&self) -> Vec<(OffsetDateTime, Uuid)> {
        self.environment.subtitles.agent().started()
    }

    pub(super) fn subtitle_job(&self, id: Uuid) -> Option<AgentJob> {
        self.environment.subtitles.agent().job(id)
    }

    pub(super) fn subtitles_settled(&self, id: Uuid) -> bool {
        self.environment.subtitles.agent().is_settled(id)
    }

    /// `job_action` on a subtitles run: only cancel, and only an agent's; `None` when `id` isn't one.
    pub(super) async fn act_on_subtitles(
        &self,
        action: AgentJobAction,
        id: Uuid,
    ) -> Result<Option<AgentActed>, AgentError> {
        let agent = self.environment.subtitles.agent();
        if !agent.knows(id) {
            return Ok(None);
        }
        if action != AgentJobAction::Cancel {
            return Err(super::on_device::only_cancel());
        }
        let job = agent.cancel(id).await?;
        Ok(Some(AgentActed { action, job }))
    }
}
