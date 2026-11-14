//! `delivery_set` and `export`: the Library's delivery sets and Export for Editing, as an agent asks for them; and
//! `frames`, still frames and a contact sheet for checking a result without a video player.

use crate::{
    AgentJob, JobsCommand, Timestamp, Uuid, commands, names,
    values::wire_enum,
    video::{AgentSource, AgentVideoRequest},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

wire_enum! {
    pub enum AgentDeliveryAction { Create = "create", List = "list", Export = "export" }
}

/// Delivery sets as Library › Delivery Sets keeps them: a named set of exact versions, exported with its outputs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliverySetCommand {
    pub action: AgentDeliveryAction,
    /// create: the set's name; the date's when left out.
    pub name: Option<String>,
    /// create: Library item IDs, at their current versions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Uuid>,
    /// export: the set's ID.
    pub set: Option<Uuid>,
    /// export: a folder the person already exported a set to.
    pub folder: Option<String>,
    /// export: how long to wait for it to finish before answering.
    pub wait_seconds: Option<i64>,
}

impl DeliverySetCommand {
    pub const MAX_ITEMS: usize = 200;
    /// In Unicode scalars.
    pub const MAX_NAME_LENGTH: usize = 120;

    pub fn new(action: AgentDeliveryAction) -> Self {
        Self {
            action,
            name: None,
            items: Vec::new(),
            set: None,
            folder: None,
            wait_seconds: None,
        }
    }

    pub fn problem(&self) -> Option<String> {
        if self.name.as_deref().is_some_and(|name| {
            names::trim_whitespace_and_newlines(name).is_empty()
                || names::scalar_count(name) > Self::MAX_NAME_LENGTH
        }) {
            return Some(format!("name is 1 to {} characters", Self::MAX_NAME_LENGTH));
        }
        if self
            .folder
            .as_deref()
            .is_some_and(|folder| !AgentVideoRequest::is_valid_path(folder))
        {
            return Some("folder is a folder path from list".to_owned());
        }
        if let Some(wait) = self.wait_seconds {
            if self.action != AgentDeliveryAction::Export {
                return Some("wait_seconds is for export".to_owned());
            }
            if let Some(problem) = JobsCommand::wait_problem(wait) {
                return Some(problem);
            }
        }
        match self.action {
            AgentDeliveryAction::Create => {
                if !(1..=Self::MAX_ITEMS).contains(&self.items.len()) {
                    return Some(format!(
                        "create needs items, 1 to {} Library item IDs",
                        Self::MAX_ITEMS
                    ));
                }
                if self.set.is_some() || self.folder.is_some() {
                    return Some("create takes name and items".to_owned());
                }
            }
            AgentDeliveryAction::List => {
                if self.name.is_some()
                    || !self.items.is_empty()
                    || self.set.is_some()
                    || self.folder.is_some()
                {
                    return Some("list takes no other fields".to_owned());
                }
            }
            AgentDeliveryAction::Export => {
                if self.set.is_none() {
                    return Some("export needs set, a set ID".to_owned());
                }
                if self.name.is_some() || !self.items.is_empty() {
                    return Some("export takes set and folder".to_owned());
                }
            }
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDeliverySet {
    pub id: Uuid,
    pub name: String,
    pub created_at: Timestamp,
    pub item_count: i64,
    /// Exported at least once: its versions no longer change.
    pub frozen: bool,
    pub exported_at: Option<Timestamp>,
    pub folder: Option<String>,
    /// "JPEG · sRGB", "Editing H.264"…
    pub outputs: Vec<String>,
}

wire_enum! {
    pub enum AgentExportStatus { Done = "done", Failed = "failed", Skipped = "skipped" }
}

/// How one file of an export went.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentExportFile {
    pub item: Uuid,
    pub status: AgentExportStatus,
    pub file: Option<String>,
    pub sidecar: Option<String>,
    pub message: Option<String>,
    pub output: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDelivery {
    pub action: AgentDeliveryAction,
    pub set: Option<AgentDeliverySet>,
    pub sets: Option<Vec<AgentDeliverySet>>,
    pub folders: Option<Vec<String>>,
    /// export: the export, running in the app; follow it with `jobs` by its ID.
    pub job: Option<AgentJob>,
    pub timed_out: Option<bool>,
}

wire_enum! {
    /// The presets of Export for Editing.
    pub enum AgentExportPreset {
        Premiere = "premiere",
        DavinciResolve = "davinci_resolve",
        FinalCutPro = "final_cut_pro",
        EditingH264 = "editing_h264",
    }
}

/// Exports video results for editing into the output folder's Exports folder. Never replaces a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportCommand {
    pub items: Vec<Uuid>,
    pub preset: AgentExportPreset,
    /// Also writes the JSON sidecar; true when left out.
    pub sidecar: Option<bool>,
    pub wait_seconds: Option<i64>,
}

impl ExportCommand {
    pub const MAX_ITEMS: usize = 20;

    pub fn problem(&self) -> Option<String> {
        if !(1..=Self::MAX_ITEMS).contains(&self.items.len()) {
            return Some(format!("items takes 1 to {} video IDs", Self::MAX_ITEMS));
        }
        if self.items.iter().collect::<HashSet<_>>().len() != self.items.len() {
            return Some("items names each video once".to_owned());
        }
        self.wait_seconds.and_then(JobsCommand::wait_problem)
    }
}

/// The answer to `export`: the export, running in the app or finished, to follow with `jobs` by its ID.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExported {
    pub job: AgentJob,
    pub timed_out: bool,
}

/// PNG frames of a Library result or a file at the times asked, and with a comparison, the same times of a second
/// clip and a contact sheet of both.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FramesCommand {
    pub item: Option<Uuid>,
    /// Else a file, a path in the app's inbox from an upload.
    pub file: Option<String>,
    /// Seconds from the start of the clip.
    pub times: Vec<f64>,
    pub compare_item: Option<Uuid>,
    pub compare_file: Option<String>,
    #[serde(default, skip_serializing_if = "commands::is_false")]
    pub compare_source: bool,
}

impl FramesCommand {
    pub const MAX_TIMES: usize = 12;

    /// The files it sends, in order.
    pub fn files(&self) -> Vec<&str> {
        [self.file.as_deref(), self.compare_file.as_deref()]
            .into_iter()
            .flatten()
            .collect()
    }

    pub fn problem(&self) -> Option<String> {
        if self.item.is_some() == self.file.is_some() {
            return Some("frames takes exactly one of item or file".to_owned());
        }
        let compared = [
            self.compare_item.is_some(),
            self.compare_file.is_some(),
            self.compare_source,
        ]
        .iter()
        .filter(|flag| **flag)
        .count();
        if compared > 1 {
            return Some(
                "frames compares with one of compare_item, compare_file or compare_source".to_owned(),
            );
        }
        if self.compare_source && self.item.is_none() {
            return Some("compare_source is the item's own source clip; it needs item".to_owned());
        }
        if !(1..=Self::MAX_TIMES).contains(&self.times.len()) {
            return Some(format!("times takes 1 to {} seconds", Self::MAX_TIMES));
        }
        if self
            .times
            .iter()
            .any(|time| !(time.is_finite() && (0.0..=AgentSource::MAX_SECONDS).contains(time)))
        {
            return Some("times are seconds from 0 to 86400".to_owned());
        }
        if let Some(path) = self
            .files()
            .into_iter()
            .find(|path| !AgentVideoRequest::is_valid_path(path))
        {
            return Some(format!("{} isn't a file path", names::prefix(path, 80)));
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentFrame {
    /// "main", the item or file asked for, or "compare".
    pub clip: String,
    pub time: f64,
    pub file: String,
}

/// What `frames` wrote.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentFrames {
    pub folder: String,
    pub frames: Vec<AgentFrame>,
    pub sheet: Option<String>,
}
