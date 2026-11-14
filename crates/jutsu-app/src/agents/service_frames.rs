//! `frames` (`AgentService+Frames.swift`): still frames and a contact sheet into the output folder's Exports folder
//! (`jutsu_media::frames::write`). Free.

use std::collections::HashMap;
use std::path::PathBuf;

use jutsu_agent::{AgentError, AgentFrame, AgentFrames, ErrorCode, FramesCommand};
use jutsu_core::{Material, OutputSubfolder};
use jutsu_media::FfmpegTools;
use jutsu_media::frames::{self, FrameClip, FrameGrabError, FrameGrabReason, FrameRole};
use uuid::Uuid;

use super::{AgentService, app_id, wire_id};
use crate::library::media::{MediaRow, material_row};

impl AgentService {
    pub(super) async fn frames(&self, fields: &FramesCommand) -> Result<AgentFrames, AgentError> {
        let files: Vec<String> = fields.files().into_iter().map(str::to_owned).collect();
        let written = self.write_frames(fields).await;
        self.discard(&files);
        written
    }

    async fn write_frames(&self, fields: &FramesCommand) -> Result<AgentFrames, AgentError> {
        let by_id: HashMap<Uuid, Material> = if fields.item.is_none() && fields.compare_item.is_none() {
            HashMap::new()
        } else {
            let mut by_id = HashMap::new();
            for material in self.materials().await? {
                by_id.entry(material.id).or_insert(material);
            }
            by_id
        };
        let item = fields.item.map(app_id);
        let mut clips = vec![self.clip(item, fields.file.as_deref(), &by_id)?];
        if let Some(id) = fields.compare_item {
            clips.push(self.clip(Some(app_id(id)), None, &by_id)?);
        } else if let Some(path) = &fields.compare_file {
            clips.push(self.clip(None, Some(path), &by_id)?);
        } else if fields.compare_source
            && let Some(id) = item
        {
            let source = match by_id.get(&id).and_then(material_row) {
                Some(MediaRow::Video(item)) => item.spec().source.clone(),
                _ => None,
            };
            let Some(source) = source else {
                return Err(AgentError::bad_request(
                    "frames: compare_source is a video's source clip; this item has none",
                ));
            };
            // A source only on Higgsfield (a history item's) is read from its https link, as the Library plays it.
            let offset = source.range.lower();
            let clip = match source.file_url.to_file_path() {
                Ok(path) => FrameClip::source(path, offset),
                Err(()) => FrameClip::remote_source(&source.file_url, offset).ok_or_else(|| {
                    AgentError::bad_request(
                        "frames: this item's source clip can't be read from this computer",
                    )
                })?,
            };
            clips.push(clip);
        }
        let Some(tools) = FfmpegTools::from_environment() else {
            return Err(AgentError::new(
                ErrorCode::FAILED,
                "The frames couldn't be written: ffmpeg isn't installed.",
                false,
                Some("Ask the person to install ffmpeg, then call frames again."),
            ));
        };
        let folder = OutputSubfolder::Exports.path(&self.output_folder());
        std::fs::create_dir_all(&folder)
            .map_err(|error| AgentError::new(ErrorCode::FAILED, error.to_string(), true, None))?;
        let written = frames::write(&tools, &clips, &fields.times, &folder)
            .await
            .map_err(|error| frames_error(&error))?;
        Ok(AgentFrames {
            folder: path_text(&folder),
            frames: written
                .frames
                .iter()
                .map(|frame| AgentFrame {
                    clip: if frame.clip == 0 { "main" } else { "compare" }.to_owned(),
                    time: frame.time,
                    file: path_text(&frame.file),
                })
                .collect(),
            sheet: written.sheet.as_deref().map(path_text),
        })
    }

    /// A Library result with a file on this computer, a video or an image, or a file the helper sent.
    fn clip(
        &self,
        id: Option<Uuid>,
        path: Option<&str>,
        by_id: &HashMap<Uuid, Material>,
    ) -> Result<FrameClip, AgentError> {
        if let Some(path) = path {
            return Ok(FrameClip::new(self.inbox_file(path)?));
        }
        let file = match id.and_then(|id| by_id.get(&id)).and_then(material_row) {
            Some(MediaRow::Video(item)) if item.history_item.is_none() => item.output_file(),
            Some(MediaRow::LocalImage(image)) => image.file_url.to_file_path().ok(),
            _ => None,
        };
        match file.filter(|file| file.exists()) {
            Some(file) => Ok(FrameClip::new(file)),
            None => Err(AgentError::new(
                ErrorCode::NOT_FOUND,
                format!(
                    "There's no Library item {} with a file on this computer.",
                    id.map(|id| wire_id(id).to_string()).unwrap_or_default()
                ),
                false,
                Some("Call library_search for the item IDs."),
            )),
        }
    }

    /// The chosen output folder, or the default one.
    pub(super) fn output_folder(&self) -> PathBuf {
        self.environment.preferences.get().output_folder.map_or_else(
            || self.environment.layout.default_output_folder.clone(),
            PathBuf::from,
        )
    }
}

fn frames_error(error: &FrameGrabError) -> AgentError {
    let clip = &error.clip;
    match &error.reason {
        FrameGrabReason::PastEnd { time, duration } => AgentError::bad_request(format!(
            "frames: {} s is past the end of {}, which runs {} s",
            frames::format_seconds(*time),
            clip.name(),
            frames::format_seconds(*duration)
        )),
        FrameGrabReason::Unreadable(reason) => {
            let role = match clip.role {
                FrameRole::Video => "video",
                FrameRole::Image => "image",
                FrameRole::SourceVideo => "source video",
            };
            AgentError::new(
                ErrorCode::FAILED,
                format!("frames: can't read the {role} {}: {reason}", clip.name()),
                false,
                Some(if clip.role == FrameRole::SourceVideo {
                    "Check in the Library that the source still plays; if not, frames of the result alone still work."
                } else {
                    "Check that the file is a video or an image the app can read."
                }),
            )
        }
        FrameGrabReason::NotWritten(reason) => AgentError::new(
            ErrorCode::FAILED,
            format!("The frames couldn't be written: {reason}"),
            false,
            Some("Check that the file is a video or an image the app can read."),
        ),
    }
}

fn path_text(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}
