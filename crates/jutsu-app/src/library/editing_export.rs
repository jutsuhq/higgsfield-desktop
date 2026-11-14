//! Export for Editing (`EditingExport.swift`, `EditingExportButton.swift`): the presets an editor takes, how the
//! menu names them, and the menu's state for one result.
//!
//! Every preset has an encoder in the bundled LGPL ffmpeg: `ProRes` through the native `prores_ks`, H.264 through
//! `FfmpegTools::h264_encoder`, the platform's hardware encoder or `OpenH264`.

pub use jutsu_core::ExportPreset;
use jutsu_core::{AudioCodec, VideoCodec};
use serde::Serialize;

pub const MENU_TITLE: &str = "Export for Editing…";
pub const MENU_HINT: &str = "Export for editing";
pub const SIDECAR_TOGGLE: &str = "Include JSON Sidecar";
pub const SUBTITLES_TOGGLE: &str = "Include Subtitles (SRT)";
pub const FOLDER_PROMPT: &str = "Export Here";
pub const FOLDER_MESSAGE: &str = "Choose a folder for the exported take. Existing files are never replaced.";
pub const FAILURE_TITLE: &str = "Can't export the result";
pub const SIDECAR_FAILURE_TITLE: &str = "The JSON sidecar wasn't written";
pub const SUBTITLES_FAILURE_TITLE: &str = "The subtitles weren't written";

/// "Premiere Pro".
pub fn preset_title(preset: ExportPreset) -> &'static str {
    match preset {
        ExportPreset::Premiere => "Premiere Pro",
        ExportPreset::DavinciResolve => "DaVinci Resolve",
        ExportPreset::FinalCutPro => "Final Cut Pro",
        ExportPreset::EditingH264 => "Editing H.264",
    }
}

/// `ProRes 422 · PCM · MOV`, under the title in the export menu.
pub fn format_summary(preset: ExportPreset) -> String {
    let video = match preset.video_codec() {
        VideoCodec::ProRes422 => "ProRes 422",
        VideoCodec::H264 => "H.264",
    };
    let audio = match preset.audio_codec() {
        AudioCodec::LinearPcm => "PCM",
        AudioCodec::Aac => "AAC",
    };
    format!("{video} · {audio} · {}", preset.file_extension().to_uppercase())
}

/// `Premiere Pro (ProRes 422 · PCM · MOV)`, as the menu and a delivery output's preset picker list it.
pub fn preset_item_title(preset: ExportPreset) -> String {
    format!("{} ({})", preset_title(preset), format_summary(preset))
}

/// One preset of the menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportPresetItem {
    pub preset: ExportPreset,
    pub title: String,
}

/// Every preset, in order.
pub fn preset_items() -> Vec<ExportPresetItem> {
    ExportPreset::ALL
        .iter()
        .map(|preset| ExportPresetItem {
            preset: *preset,
            title: preset_item_title(*preset),
        })
        .collect()
}

/// The Export for Editing menu of one result.
#[allow(clippy::struct_excessive_bools, reason = "the wire's fields")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EditingExportMenu {
    pub presets: Vec<ExportPresetItem>,
    /// Off without a downloaded file.
    pub is_enabled: bool,
    pub writes_sidecar: bool,
    /// The subtitles toggle shows only for a version with subtitles.
    pub has_subtitles: bool,
    pub writes_subtitles: bool,
    /// An export runs: the menu shows its progress and Cancel Export.
    pub is_exporting: bool,
}
