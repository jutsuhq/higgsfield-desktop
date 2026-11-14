//! `export` (`AgentService+Export.swift`) and `delivery_set` (`AgentService+Library.swift`): Export for Editing's and
//! the delivery sheet's own writers, run in the app as operations agents follow with `jobs` (`AgentOperations`). They
//! answer with the operation at once, or once it finished within `wait_seconds`, and never write over a file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use jutsu_agent::{
    AgentDelivery, AgentDeliveryAction, AgentDeliverySet, AgentError, AgentExportFile, AgentExportPreset,
    AgentExportStatus, AgentExported, AgentOperationKind, AgentOperationState, DeliverySetCommand, ErrorCode,
    ExportCommand, Timestamp,
};
use jutsu_core::{DeliveryResultStatus, DeliverySet, DeliveryTask, ExportPreset, OutputSubfolder};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{AgentService, app_id, wire_id};
use crate::agents::job::agent_wait;
use crate::library::delivery::{self, DeliveryTaskState, output_summary};
use crate::library::item::LibraryItem;
use crate::library::media::{MediaRow, material_row};
use crate::library::service::agent_exports::{
    self, AgentTaskReport, delivery_folders, delivery_tasks, editing_names, known_delivery_folder,
};

const NOT_A_VIDEO: &str = "Not a video with a file on this computer.";

fn preset(preset: AgentExportPreset) -> ExportPreset {
    match preset {
        AgentExportPreset::Premiere => ExportPreset::Premiere,
        AgentExportPreset::DavinciResolve => ExportPreset::DavinciResolve,
        AgentExportPreset::FinalCutPro => ExportPreset::FinalCutPro,
        AgentExportPreset::EditingH264 => ExportPreset::EditingH264,
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn make_folder(folder: &Path) -> Result<(), AgentError> {
    std::fs::create_dir_all(folder)
        .map_err(|error| AgentError::new(ErrorCode::FAILED, error.to_string(), true, None))
}

fn skipped(item: Uuid) -> AgentExportFile {
    AgentExportFile {
        item: wire_id(item),
        status: AgentExportStatus::Skipped,
        file: None,
        sidecar: None,
        message: Some(NOT_A_VIDEO.to_owned()),
        output: None,
    }
}

/// A delivery file as `jobs` lists it, once its task finished; `None` while it waits or runs.
fn task_file(task: &DeliveryTask, state: &DeliveryTaskState, folder: &Path) -> Option<AgentExportFile> {
    let (status, message) = match state {
        DeliveryTaskState::Waiting | DeliveryTaskState::Running => return None,
        DeliveryTaskState::Done => (AgentExportStatus::Done, None),
        DeliveryTaskState::Failed(message) => (AgentExportStatus::Failed, Some(message.clone())),
        DeliveryTaskState::Skipped(message) => (AgentExportStatus::Skipped, Some(message.clone())),
    };
    Some(file_of(task, status, message, folder))
}

fn file_of(
    task: &DeliveryTask,
    status: AgentExportStatus,
    message: Option<String>,
    folder: &Path,
) -> AgentExportFile {
    AgentExportFile {
        item: wire_id(task.item.reference.material_id),
        status,
        file: (status == AgentExportStatus::Done).then(|| path_text(&folder.join(&task.file_name))),
        sidecar: None,
        message,
        output: Some(output_summary(&task.output)),
    }
}

fn delivery_answer(action: AgentDeliveryAction) -> AgentDelivery {
    AgentDelivery {
        action,
        set: None,
        sets: None,
        folders: None,
        job: None,
        timed_out: None,
    }
}

fn agent_set(set: &DeliverySet) -> AgentDeliverySet {
    AgentDeliverySet {
        id: wire_id(set.id),
        name: set.name.clone(),
        created_at: Timestamp(set.created_at),
        item_count: i64::try_from(set.items().len()).unwrap_or(i64::MAX),
        frozen: set.is_frozen(),
        exported_at: set.exported_at().map(Timestamp),
        folder: set
            .folder()
            .and_then(|folder| folder.url.to_file_path().ok())
            .map(|path| path_text(&path)),
        outputs: set.outputs.iter().map(output_summary).collect(),
    }
}

impl AgentService {
    // MARK: export

    /// Starts Export for Editing of the videos asked for, into the output folder's Exports folder under names no file
    /// has.
    pub(super) async fn export_for_editing(
        &self,
        fields: &ExportCommand,
    ) -> Result<AgentExported, AgentError> {
        let started_at = Instant::now();
        let preset = preset(fields.preset);
        let folder = OutputSubfolder::Exports.path(&self.output_folder());
        make_folder(&folder)?;
        let materials = self.materials().await?;
        let by_id: HashMap<Uuid, _> = materials.iter().map(|material| (material.id, material)).collect();
        let mut videos: Vec<LibraryItem> = Vec::new();
        let mut skips = Vec::new();
        for id in fields.items.iter().copied().map(app_id) {
            match by_id.get(&id).and_then(|material| material_row(material)) {
                Some(MediaRow::Video(item))
                    if item.history_item.is_none()
                        && item.output_file().is_some_and(|file| file.exists()) =>
                {
                    videos.push(item);
                }
                _ => skips.push(skipped(id)),
            }
        }
        let names = editing_names(&videos, preset, &agent_exports::existing_names(&folder));
        let order: HashMap<_, usize> = fields
            .items
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .rev()
            .collect();
        let sidecar = fields.sidecar.unwrap_or(true);
        let library = Arc::clone(&self.environment.library);
        let into = folder.clone();
        let total = i64::try_from(fields.items.len()).unwrap_or(i64::MAX);
        let started = self.operations.start(
            AgentOperationKind::Export,
            &path_text(&folder),
            total,
            None,
            move |report| async move {
                let mut files = skips.clone();
                for file in skips {
                    report(file);
                }
                for (item, name) in videos.iter().zip(&names) {
                    let file = match library
                        .write_for_editing(item, name, preset, &into, sidecar)
                        .await
                    {
                        Ok(written) => AgentExportFile {
                            item: wire_id(item.id()),
                            status: AgentExportStatus::Done,
                            file: Some(path_text(&written.video)),
                            sidecar: written.sidecar.as_deref().map(path_text),
                            message: written.problem,
                            output: None,
                        },
                        Err(message) => {
                            tracing::error!("an agent's export failed");
                            AgentExportFile {
                                item: wire_id(item.id()),
                                status: AgentExportStatus::Failed,
                                file: None,
                                sidecar: None,
                                message: Some(message),
                                output: None,
                            }
                        }
                    };
                    report(file.clone());
                    files.push(file);
                }
                // In the order asked.
                files.sort_by_key(|file| order.get(&file.item).copied().unwrap_or(0));
                Ok(files)
            },
        );
        let seconds = jutsu_agent::wait_seconds(fields.wait_seconds.unwrap_or(0), started_at.elapsed());
        let timed_out = self.wait_for_operation(started.id, seconds).await;
        Ok(AgentExported {
            job: self.operations.operation(started.id).unwrap_or(started).job(),
            timed_out,
        })
    }

    /// Waits up to `seconds` for an export to finish: `true` when the time ran out first. It runs on either way.
    async fn wait_for_operation(&self, id: jutsu_agent::Uuid, seconds: i64) -> bool {
        let running = || {
            self.operations
                .operation(id)
                .is_some_and(|operation| operation.state == AgentOperationState::Running)
        };
        if seconds <= 0 {
            return running();
        }
        agent_wait(seconds, || async { !running() }).await
    }

    // MARK: delivery_set

    pub(super) async fn delivery_set(
        &self,
        fields: &DeliverySetCommand,
    ) -> Result<AgentDelivery, AgentError> {
        let mut answer = delivery_answer(fields.action);
        match fields.action {
            AgentDeliveryAction::Create => {
                let materials = self.materials().await?;
                let wanted: Vec<Uuid> = fields.items.iter().copied().map(app_id).collect();
                let local = |id: &Uuid| {
                    materials
                        .iter()
                        .find(|material| material.id == *id)
                        .and_then(material_row)
                        .is_some_and(|row| {
                            matches!(row, MediaRow::LocalImage(_))
                                || matches!(row, MediaRow::Video(item) if item.history_item.is_none())
                        })
                };
                if let Some(missing) = wanted.iter().find(|id| !local(id)) {
                    return Err(AgentError::new(
                        ErrorCode::NOT_FOUND,
                        format!(
                            "There's no Library item {} with a file on this computer.",
                            wire_id(*missing)
                        ),
                        false,
                        Some("Call library_search for the item IDs."),
                    ));
                }
                let picked: Vec<_> = wanted
                    .iter()
                    .filter_map(|id| materials.iter().find(|material| material.id == *id).cloned())
                    .collect();
                let now = OffsetDateTime::now_utc();
                let set = DeliverySet::new(
                    Uuid::new_v4(),
                    fields
                        .name
                        .clone()
                        .unwrap_or_else(|| delivery::default_name(now, self.environment.utc_offset)),
                    now,
                    DeliverySet::items_from(&picked),
                    DeliverySet::default_outputs(),
                );
                self.environment
                    .store
                    .save_delivery_set(&set)
                    .await
                    .map_err(|error| AgentError::new(ErrorCode::FAILED, error.to_string(), true, None))?;
                answer.set = Some(agent_set(&set));
            }
            AgentDeliveryAction::List => {
                let mut sets = self.delivery_sets().await?;
                sets.sort_by_key(|set| std::cmp::Reverse(set.created_at));
                answer.folders = Some(
                    delivery_folders(&sets)
                        .iter()
                        .map(|path| path_text(path))
                        .collect(),
                );
                answer.sets = Some(sets.iter().map(agent_set).collect());
            }
            AgentDeliveryAction::Export => return self.export_delivery(fields).await,
        }
        Ok(answer)
    }

    /// Starts writing what is still to write of the set, never over a file, into a folder the person exported a set
    /// to, or the output folder's Exports folder, recording it on the set as the sheet does once done.
    async fn export_delivery(&self, fields: &DeliverySetCommand) -> Result<AgentDelivery, AgentError> {
        let started_at = Instant::now();
        let sets = self.delivery_sets().await?;
        let Some(set) = fields
            .set
            .map(app_id)
            .and_then(|id| sets.iter().find(|set| set.id == id))
        else {
            return Err(AgentError::new(
                ErrorCode::NOT_FOUND,
                format!(
                    "There's no delivery set {}.",
                    fields.set.map(|id| id.to_string()).unwrap_or_default()
                ),
                false,
                Some("Call delivery_set with action list for the set IDs."),
            ));
        };
        let folder = self.delivery_folder(fields.folder.as_deref(), &sets)?;
        make_folder(&folder)?;
        let tasks = delivery_tasks(set, &agent_exports::existing_names(&folder));
        let library = Arc::clone(&self.environment.library);
        let into = folder.clone();
        let set_id = set.id;
        let total = i64::try_from(tasks.len()).unwrap_or(i64::MAX);
        let started = self.operations.start(
            AgentOperationKind::DeliveryExport,
            &path_text(&folder),
            total,
            Some(wire_id(set.id)),
            move |report| async move {
                let reported = into.clone();
                let report: AgentTaskReport = Arc::new(move |task, state| {
                    if let Some(file) = task_file(task, &state, &reported) {
                        report(file);
                    }
                });
                let written = library
                    .write_delivery_for_agent(set_id, tasks, &into, report)
                    .await
                    .map_err(|message| AgentError::new(ErrorCode::FAILED, message, true, None))?;
                Ok(written
                    .iter()
                    .map(|(task, result)| {
                        let status = match result.status {
                            DeliveryResultStatus::Done => AgentExportStatus::Done,
                            DeliveryResultStatus::Failed => AgentExportStatus::Failed,
                            DeliveryResultStatus::Skipped => AgentExportStatus::Skipped,
                        };
                        file_of(task, status, result.message.clone(), &into)
                    })
                    .collect())
            },
        );
        let seconds = jutsu_agent::wait_seconds(fields.wait_seconds.unwrap_or(0), started_at.elapsed());
        let timed_out = self.wait_for_operation(started.id, seconds).await;
        let mut answer = delivery_answer(AgentDeliveryAction::Export);
        answer.set = Some(agent_set(set));
        answer.job = Some(self.operations.operation(started.id).unwrap_or(started).job());
        answer.timed_out = Some(timed_out);
        Ok(answer)
    }

    /// A folder the person exported a set to, else the output folder's Exports.
    fn delivery_folder(&self, path: Option<&str>, sets: &[DeliverySet]) -> Result<PathBuf, AgentError> {
        let Some(path) = path else {
            return Ok(OutputSubfolder::Exports.path(&self.output_folder()));
        };
        known_delivery_folder(path, sets).ok_or_else(|| {
            AgentError::new(
                ErrorCode::FILE_REFUSED,
                format!("{path} isn't a folder the person exported a delivery set to."),
                false,
                Some("Leave out folder for the output folder's Exports, or pick one of list's folders."),
            )
        })
    }

    async fn delivery_sets(&self) -> Result<Vec<DeliverySet>, AgentError> {
        self.environment.store.delivery_sets().await.map_err(|_| {
            AgentError::new(
                ErrorCode::FAILED,
                "Jutsu Studio can't read its delivery sets right now.",
                true,
                Some("Try again in a moment."),
            )
        })
    }

    // MARK: jobs

    /// The exports agents started, for `jobs` without IDs.
    pub(super) fn operation_entries(&self) -> Vec<(OffsetDateTime, Uuid)> {
        self.operations
            .all()
            .iter()
            .map(|operation| (operation.created_at.0, app_id(operation.id)))
            .collect()
    }

    pub(super) fn operation_job(&self, id: Uuid) -> Option<jutsu_agent::AgentJob> {
        self.operations
            .operation(wire_id(id))
            .map(|operation| operation.job())
    }

    pub(super) fn operation_is_settled(&self, id: Uuid) -> bool {
        self.operations
            .operation(wire_id(id))
            .is_none_or(|operation| operation.state != AgentOperationState::Running)
    }
}
