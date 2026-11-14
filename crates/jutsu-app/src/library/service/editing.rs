//! Export for Editing (`EditingExportButton`): one export at a time into a folder the person picked, named so it never
//! writes over a file there, with the JSON sidecar and the version's subtitles beside it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use jutsu_core::{ExportNaming, ExportPreset, ExportSidecar, Progress, SubtitleCue, SubtitleFormat};
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{LibraryError, LibraryService};
use crate::library::editing_export::{
    EditingExportMenu, FAILURE_TITLE, SIDECAR_FAILURE_TITLE, SUBTITLES_FAILURE_TITLE, preset_items,
};
use crate::library::item::LibraryItem;
use crate::queue::MissingFfmpeg;

/// The names in `folder`, lowercased: a file system that ignores case takes `Take_v1.mov` for `take_v1.mov`.
pub(crate) fn taken_names(folder: &Path) -> HashSet<String> {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

/// Writes `text` to a new file at `path`; an existing file is never replaced.
pub(crate) fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::File::options().write(true).create_new(true).open(path)?;
    file.write_all(text.as_bytes())
}

impl LibraryService {
    /// The cues of the version `item` made, when it has subtitles.
    pub(super) async fn subtitle_cues(&self, item: &LibraryItem) -> Vec<SubtitleCue> {
        let Ok(materials) = self.materials().await else {
            return Vec::new();
        };
        materials
            .iter()
            .flat_map(|material| material.versions())
            .find(|version| version.video.as_ref().is_some_and(|job| job.id == item.id()))
            .and_then(|version| version.subtitles.as_ref())
            .map(|subtitles| subtitles.cues.clone())
            .unwrap_or_default()
    }

    /// The Export for Editing menu of the result `id`.
    pub async fn editing_export_menu(&self, id: Uuid) -> Result<EditingExportMenu, LibraryError> {
        let item = self.video_item(id).await?;
        let preferences = self.preferences.get();
        Ok(EditingExportMenu {
            presets: preset_items(),
            is_enabled: item.output_file().is_some_and(|path| path.exists()),
            writes_sidecar: preferences.writes_sidecar.unwrap_or(true),
            has_subtitles: !self.subtitle_cues(&item).await.is_empty(),
            writes_subtitles: preferences.writes_subtitles.unwrap_or(true),
            is_exporting: self.locked().editing_export.is_some(),
        })
    }

    /// Exports the result `id` with `preset` into `folder`, then the sidecar and the subtitles when they are on.
    /// Answers the video's path, `None` when canceled or when it failed; a failure shows as an alert.
    pub async fn export_for_editing(
        &self,
        id: Uuid,
        preset: ExportPreset,
        folder: &Path,
        progress: Progress<f64>,
    ) -> Option<PathBuf> {
        let cancel = CancellationToken::new();
        {
            let mut state = self.locked();
            if state.editing_export.is_some() {
                return None;
            }
            state.editing_export = Some(cancel.clone());
        }
        self.notify();
        let result = self
            .run_editing_export(id, preset, folder, progress, &cancel)
            .await;
        self.locked().editing_export = None;
        self.notify();
        match result {
            Ok(path) => Some(path),
            Err(None) => None,
            Err(Some(message)) => {
                self.alert(FAILURE_TITLE, &message);
                None
            }
        }
    }

    /// Cancel Export.
    pub fn cancel_editing_export(&self) {
        if let Some(cancel) = self.locked().editing_export.as_ref() {
            cancel.cancel();
        }
    }

    /// `Err(None)` when canceled.
    async fn run_editing_export(
        &self,
        id: Uuid,
        preset: ExportPreset,
        folder: &Path,
        progress: Progress<f64>,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, Option<String>> {
        let item = self
            .video_item(id)
            .await
            .map_err(|error| Some(error.to_string()))?;
        let result = item
            .output_file()
            .filter(|path| path.exists())
            .ok_or_else(|| Some("The result's file isn't on this computer.".to_owned()))?;
        let tools = self
            .tools
            .as_ref()
            .ok_or_else(|| Some(MissingFfmpeg::MESSAGE.to_owned()))?;
        let cues = self.subtitle_cues(&item).await;
        let preferences = self.preferences.get();
        let writes_sidecar = preferences.writes_sidecar.unwrap_or(true);
        let writes_subtitles = preferences.writes_subtitles.unwrap_or(true) && !cues.is_empty();
        let taken = taken_names(folder);
        let names = ExportNaming::names(item.spec(), item.preset_name.as_deref(), preset, |name| {
            taken.contains(&name.to_lowercase())
        });
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
            Some(progress),
            cancel,
        )
        .await
        .map_err(|error| match error {
            jutsu_media::MediaError::Cancelled => None,
            error => Some(error.to_string()),
        })?;
        if writes_sidecar {
            let written = ExportSidecar::new(
                &item.job,
                item.preset_name.as_deref(),
                preset,
                OffsetDateTime::now_utc(),
            )
            .encoded()
            .map_err(|error| error.to_string())
            .and_then(|json| {
                write_new(&folder.join(&names.sidecar), &json).map_err(|error| error.to_string())
            });
            if let Err(message) = written {
                self.alert(SIDECAR_FAILURE_TITLE, &message);
            }
        }
        if writes_subtitles
            && let Err(error) = write_new(&folder.join(&names.subtitles), &SubtitleFormat::Srt.text(&cues))
        {
            self.alert(SUBTITLES_FAILURE_TITLE, &error.to_string());
        }
        Ok(output)
    }
}
