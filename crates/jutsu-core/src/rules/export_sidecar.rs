//! The JSON written next to an editing export or a delivery.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::models::{
    AudioOption, DeliveryResultStatus, DeliverySet, ExportPreset, Job, Mode, Resolution, decimal_text,
};
use crate::rules::ExportNaming;
use crate::support::Seconds;
use crate::support::swift_json::pretty_sorted;
use crate::support::url_ext::last_path_component;

/// The name of the app in a sidecar.
const APP: &str = "Jutsu Studio";

/// The source a take was made from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SidecarSource {
    /// The source's file name.
    pub file_name: String,
    /// Seconds from the start of the source file.
    pub range_start: Seconds,
    /// Seconds from the start of the source file.
    pub range_end: Seconds,
}

/// The style preset of a take.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SidecarStyle {
    /// The preset's ID.
    pub id: Uuid,
    /// `None` when the catalog didn't know the preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The price of a take.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SidecarPrice {
    /// Decimal string, so it reads back exactly.
    pub amount: String,
    /// `usd` or `credits`.
    pub unit: String,
    /// Whether the amount is an estimate.
    pub is_estimate: bool,
}

/// The JSON written next to an editing export: what the take was made from, for the edit bay and for
/// support requests. File names only, never paths or bookmarks; no credentials or provider URLs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ExportSidecar {
    /// "Jutsu Studio".
    pub app: String,
    /// The preset the take was exported with.
    pub export_preset: ExportPreset,
    /// When.
    #[serde(with = "crate::support::date::iso8601")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub exported_at: OffsetDateTime,
    /// The job.
    #[serde(rename = "jobID")]
    pub job_id: Uuid,
    /// The job's mode.
    pub mode: Mode,
    /// `None` for a clip made without a source video.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SidecarSource>,
    /// Reference image file names, in the order they were sent.
    pub references: Vec<String>,
    /// The style preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<SidecarStyle>,
    /// The prompt.
    pub prompt: String,
    /// The resolution.
    pub resolution: Resolution,
    /// Whether it was a draft.
    pub is_draft: bool,
    /// The sound option.
    pub audio: AudioOption,
    /// The API's request ID.
    #[serde(rename = "requestID", default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// The price.
    pub cost: SidecarPrice,
    /// When the job was made.
    #[serde(with = "crate::support::date::iso8601")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub created_at: OffsetDateTime,
    /// When it finished.
    #[serde(
        with = "crate::support::date::iso8601_option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub finished_at: Option<OffsetDateTime>,
}

impl ExportSidecar {
    /// The sidecar of `job`, exported with `export_preset` at `exported_at`.
    pub fn new(
        job: &Job,
        preset_name: Option<&str>,
        export_preset: ExportPreset,
        exported_at: OffsetDateTime,
    ) -> Self {
        let spec = &job.spec;
        let price = job.price();
        Self {
            app: APP.to_owned(),
            export_preset,
            exported_at,
            job_id: job.id,
            mode: spec.mode,
            source: spec.source.as_ref().map(|source| SidecarSource {
                file_name: last_path_component(&source.file_url),
                range_start: source.range.lower(),
                range_end: source.range.upper(),
            }),
            references: spec
                .references
                .iter()
                .map(|image| last_path_component(&image.file_url))
                .collect(),
            style: spec.preset_id.map(|id| SidecarStyle {
                id,
                name: preset_name.map(str::to_owned),
            }),
            prompt: spec.prompt.clone(),
            resolution: spec.resolution,
            is_draft: spec.is_draft,
            audio: spec.audio,
            request_id: job.remote.as_ref().map(|remote| remote.request_id.clone()),
            cost: SidecarPrice {
                amount: decimal_text(price.amount()),
                unit: price.unit().raw_value().to_owned(),
                is_estimate: price.is_estimate(),
            },
            created_at: job.created_at,
            finished_at: job.finished_at,
        }
    }

    /// Pretty-printed with sorted keys and ISO 8601 dates.
    pub fn encoded(&self) -> Result<String, serde_json::Error> {
        pretty_sorted(self)
    }

    /// Reads a sidecar back.
    pub fn decode(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
    }
}

/// One file of a delivery manifest.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DeliveryFile {
    /// `None` when nothing was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// The material.
    #[serde(rename = "materialID")]
    pub material_id: Uuid,
    /// The version.
    #[serde(rename = "versionID")]
    pub version_id: Uuid,
    /// The item's name.
    pub item: String,
    /// The output's label.
    pub output: String,
    /// How the file went.
    pub status: DeliveryResultStatus,
    /// Why it failed or was skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// The JSON written next to a delivery set's files: which exact catalog version each file is, in which output,
/// and what failed or was skipped. Names and IDs only, never paths or bookmarks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DeliveryManifest {
    /// "Jutsu Studio".
    pub app: String,
    /// The set.
    #[serde(rename = "setID")]
    pub set_id: Uuid,
    /// The set's name.
    pub name: String,
    /// When.
    #[serde(with = "crate::support::date::iso8601")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub exported_at: OffsetDateTime,
    /// The set's note.
    pub note: String,
    /// The set's review link.
    pub review_link: String,
    /// Every result the set has, in item and output order.
    pub files: Vec<DeliveryFile>,
}

impl DeliveryManifest {
    /// Every result `set` has, in item and output order.
    pub fn new(set: &DeliverySet, exported_at: OffsetDateTime) -> Self {
        let labels = ExportNaming::output_labels(&set.outputs);
        let files = set
            .items()
            .iter()
            .flat_map(|item| {
                set.outputs.iter().filter_map(|output| {
                    let result = set.result(item, output)?;
                    Some(DeliveryFile {
                        file_name: result.file_name.clone(),
                        material_id: item.reference.material_id,
                        version_id: item.reference.version_id,
                        item: item.name.clone(),
                        output: labels
                            .get(&output.id)
                            .cloned()
                            .unwrap_or_else(|| output.summary()),
                        status: result.status,
                        message: result.message.clone(),
                    })
                })
            })
            .collect();
        Self {
            app: APP.to_owned(),
            set_id: set.id,
            name: set.name.clone(),
            exported_at,
            note: set.note.clone(),
            review_link: set.review_link.clone(),
            files,
        }
    }

    /// Pretty-printed with sorted keys and ISO 8601 dates, like the editing sidecar.
    pub fn encoded(&self) -> Result<String, serde_json::Error> {
        pretty_sorted(self)
    }
}
