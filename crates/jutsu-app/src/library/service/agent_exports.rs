//! What an agent's `export` and `delivery_set` write (`AgentExports.swift`, `AgentService+Export.swift`): the Library's
//! own writers, with one rule more. An agent never writes over a file, not even one a delivery set wrote before: such
//! a file gets a new name beside it. Neither shows an alert or takes the window's one export at a time.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jutsu_core::{
    DeliveryFolder, DeliveryPlan, DeliveryResult, DeliveryScope, DeliverySet, DeliveryTask, ExportNames,
    ExportNaming, ExportPreset, ExportSidecar, OutputNamePattern, SubtitleFormat,
};
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use super::LibraryService;
use super::delivery::SAVES;
use super::editing::{taken_names, write_new};
use crate::library::delivery::{self, DeliveryTaskState};
use crate::library::item::LibraryItem;
use crate::queue::MissingFfmpeg;

/// Tells each task's state as it changes.
pub type AgentTaskReport = Arc<dyn Fn(&DeliveryTask, DeliveryTaskState) + Send + Sync>;

/// One take an agent's export wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditingWritten {
    pub video: PathBuf,
    /// The sidecar, when asked for and written.
    pub sidecar: Option<PathBuf>,
    /// Why the sidecar or the subtitles weren't written; the take was.
    pub problem: Option<String>,
}

/// The names already in `folder`, lowercased, as the usual case-insensitive volumes compare them.
pub fn existing_names(folder: &Path) -> HashSet<String> {
    taken_names(folder)
}

/// Each video's take and sidecar names, as Export for Editing names them, none taken in the folder or by an earlier
/// video of the same export.
pub(crate) fn editing_names(
    items: &[LibraryItem],
    preset: ExportPreset,
    existing: &HashSet<String>,
) -> Vec<ExportNames> {
    let mut taken = existing.clone();
    items
        .iter()
        .map(|item| {
            let names = ExportNaming::names(item.spec(), item.preset_name.as_deref(), preset, |name| {
                taken.contains(&name.to_lowercase())
            });
            for name in [&names.video, &names.sidecar, &names.subtitles] {
                taken.insert(name.to_lowercase());
            }
            names
        })
        .collect()
}

/// The files of `set` still to write (`DeliveryScope::Pending`), none of them over an existing file.
pub(crate) fn delivery_tasks(set: &DeliverySet, existing: &HashSet<String>) -> Vec<DeliveryTask> {
    let tasks = DeliveryPlan::tasks(set, DeliveryScope::Pending, |name| {
        existing.contains(&name.to_lowercase())
    });
    let mut taken: HashSet<String> = existing
        .iter()
        .cloned()
        .chain(tasks.iter().map(|task| task.file_name.to_lowercase()))
        .collect();
    tasks
        .into_iter()
        .map(|mut task| {
            if task.replaces_existing {
                task.file_name = OutputNamePattern::available(&task.file_name, |name| {
                    taken.contains(&name.to_lowercase())
                });
                task.replaces_existing = false;
                taken.insert(task.file_name.to_lowercase());
            }
            task
        })
        .collect()
}

/// The folders the person exported delivery sets to, newest export first, each once.
pub fn delivery_folders(sets: &[DeliverySet]) -> Vec<PathBuf> {
    let mut exported: Vec<&DeliverySet> = sets.iter().filter(|set| set.folder().is_some()).collect();
    exported.sort_by_key(|set| std::cmp::Reverse(set.exported_at()));
    let mut seen = HashSet::new();
    exported
        .into_iter()
        .filter_map(|set| set.folder()?.url.to_file_path().ok())
        .map(|path| trimmed(&path))
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// `path` when the person already exported a set there; `None` for any other folder.
pub fn known_delivery_folder(path: &str, sets: &[DeliverySet]) -> Option<PathBuf> {
    let wanted = trimmed(Path::new(path));
    delivery_folders(sets)
        .into_iter()
        .find(|folder| *folder == wanted)
}

fn trimmed(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let text = if text.len() > 1 {
        text.trim_end_matches('/')
    } else {
        &text
    };
    PathBuf::from(text)
}

impl LibraryService {
    /// Writes the take `item` as Export for Editing does, as `names`, with its sidecar when `writes_sidecar` and its
    /// subtitles when the person keeps them on.
    pub async fn write_for_editing(
        &self,
        item: &LibraryItem,
        names: &ExportNames,
        preset: ExportPreset,
        folder: &Path,
        writes_sidecar: bool,
    ) -> Result<EditingWritten, String> {
        let result = item
            .output_file()
            .filter(|path| path.exists())
            .ok_or_else(|| "The result's file isn't on this computer.".to_owned())?;
        let tools = self
            .tools
            .as_ref()
            .ok_or_else(|| MissingFfmpeg::MESSAGE.to_owned())?;
        let output = folder.join(&names.video);
        let source = item
            .spec()
            .source
            .as_ref()
            .filter(|clip| clip.file_url.scheme() == "file");
        jutsu_media::editing::export(
            tools,
            &result,
            source,
            preset,
            None,
            &[],
            &output,
            None,
            &CancellationToken::new(),
        )
        .await
        .map_err(|error| error.to_string())?;
        let mut problem = None;
        let mut sidecar = None;
        if writes_sidecar {
            let path = folder.join(&names.sidecar);
            let written = ExportSidecar::new(
                &item.job,
                item.preset_name.as_deref(),
                preset,
                OffsetDateTime::now_utc(),
            )
            .encoded()
            .map_err(|error| error.to_string())
            .and_then(|json| write_new(&path, &json).map_err(|error| error.to_string()));
            match written {
                Ok(()) => sidecar = Some(path),
                Err(message) => problem = Some(format!("The JSON sidecar wasn't written: {message}")),
            }
        }
        let cues = self.subtitle_cues(item).await;
        if self.preferences.get().writes_subtitles.unwrap_or(true)
            && !cues.is_empty()
            && let Err(error) = write_new(&folder.join(&names.subtitles), &SubtitleFormat::Srt.text(&cues))
        {
            problem = Some(format!("The subtitles weren't written: {error}"));
        }
        Ok(EditingWritten {
            video: output,
            sidecar,
            problem,
        })
    }

    /// Writes `tasks` of the set `set_id` into `folder`, telling `report` each task's state, then records the results
    /// on the set and writes its manifest beside them, never over one. Answers each finished task with its result.
    pub async fn write_delivery_for_agent(
        &self,
        set_id: Uuid,
        tasks: Vec<DeliveryTask>,
        folder: &Path,
        report: AgentTaskReport,
    ) -> Result<Vec<(DeliveryTask, DeliveryResult)>, String> {
        let materials = self.materials().await.map_err(|error| error.to_string())?;
        let cancel = CancellationToken::new();
        let by_id: std::collections::HashMap<String, DeliveryTask> =
            tasks.iter().map(|task| (task.id(), task.clone())).collect();
        let results = delivery::run(
            &tasks,
            |task| {
                let (materials, cancel) = (&materials, &cancel);
                async move { self.export_delivery_task(&task, materials, folder, cancel).await }
            },
            |id, state| {
                if let Some(task) = by_id.get(id) {
                    report(task, state);
                }
            },
        )
        .await;
        if !results.is_empty() {
            let folder_url =
                Url::from_directory_path(folder).map_err(|()| "The folder can't be used.".to_owned())?;
            let _save = SAVES.lock().await;
            let mut set = self
                .delivery_set_model(set_id)
                .await
                .map_err(|error| error.to_string())?;
            set.record(
                results.clone(),
                DeliveryFolder { url: folder_url },
                OffsetDateTime::now_utc(),
            );
            Self::write_manifest(&set, folder, false);
            if let Err(error) = self.store.save_delivery_set(&set).await {
                tracing::error!(%error, "can't record a delivery export");
            }
        }
        Ok(tasks.into_iter().zip(results).collect())
    }
}

#[cfg(test)]
mod tests {
    use jutsu_core::{DeliveryItem, MaterialKind, MaterialReference};

    use super::*;

    fn set(name: &str, folder: Option<&str>, exported_at: i64) -> DeliverySet {
        let reference = MaterialReference::new(Uuid::new_v4(), Uuid::new_v4());
        let item = DeliveryItem::new(reference, MaterialKind::Video, "Take");
        let mut set = DeliverySet::new(
            Uuid::new_v4(),
            name,
            OffsetDateTime::UNIX_EPOCH,
            vec![item],
            DeliverySet::default_outputs(),
        );
        if let Some(folder) = folder {
            let url = Url::from_directory_path(folder).expect("a folder URL");
            let at = OffsetDateTime::from_unix_timestamp(exported_at).expect("a date");
            let output = set.outputs[0].clone();
            let done = DeliveryResult::new(reference, output, jutsu_core::DeliveryResultStatus::Done);
            set.record(vec![done], DeliveryFolder { url }, at);
        }
        set
    }

    // Unix paths: the rule is the same on Windows, with its own separators and drives.
    #[cfg(unix)]
    #[test]
    fn delivery_folders_are_the_ones_the_person_exported_to_newest_first() {
        let sets = [
            set("A", Some("/tmp/client"), 10),
            set("B", Some("/tmp/review/"), 20),
            set("C", None, 0),
            set("D", Some("/tmp/client"), 5),
        ];
        assert_eq!(
            delivery_folders(&sets),
            [PathBuf::from("/tmp/review"), PathBuf::from("/tmp/client")]
        );
        assert_eq!(
            known_delivery_folder("/tmp/review/", &sets),
            Some(PathBuf::from("/tmp/review"))
        );
        assert_eq!(known_delivery_folder("/tmp/elsewhere", &sets), None);
    }

    #[test]
    fn a_delivery_file_already_there_gets_a_new_name_beside_it() {
        let set = set("A", None, 0);
        let first = DeliveryPlan::tasks(&set, DeliveryScope::Pending, |_| false);
        let name = first[0].file_name.to_lowercase();
        let tasks = delivery_tasks(&set, &HashSet::from([name.clone()]));
        assert_eq!(tasks.len(), first.len());
        assert!(tasks.iter().all(|task| !task.replaces_existing));
        assert_ne!(tasks[0].file_name.to_lowercase(), name);
        let names: HashSet<String> = tasks.iter().map(|task| task.file_name.to_lowercase()).collect();
        assert_eq!(names.len(), tasks.len());
    }
}
