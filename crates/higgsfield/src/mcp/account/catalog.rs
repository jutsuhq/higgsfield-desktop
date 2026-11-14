//! The server's tool contract: Genjutsu runs through the generic `generate_video` tool, inputs go up through
//! `media_upload` and `media_confirm`, and jobs are read with `job_status`. No tool cancels a generation.

use std::collections::HashMap;

use crate::mcp::client::McpTool;

/// What `HiggsfieldAccount` needs a tool for, by the tool's exact name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolRole {
    Generate,
    Upload,
    ConfirmUpload,
    Status,
    Balance,
    Generations,
    Presets,
    ExecutePreset,
    Like,
    Transactions,
    Assets,
    ModelsExplore,
    /// `jobs: [{job_id, index}]` per the catalog snapshot of 2026-10-15.
    JobsWait,
    /// Operations on a result (`VideoEdit`) and the voices they take.
    UpscaleVideo,
    RemoveBackground,
    Reframe,
    Dubbing,
    VoiceChange,
    ListVoices,
    /// `{params: {model, prompt, count, aspect_ratio, medias, get_cost, use_unlim}}` (`AccountImageRequest`).
    GenerateImage,
    /// `list`, `get` and `create` of the account's elements.
    ReferenceElements,
    /// AI Influencer character sheets: the traits, the presets, the free quote and the paid submit.
    InfluencerOptions,
    InfluencerPresets,
    InfluencerPrepare,
    InfluencerGenerate,
    /// `{action: "create", params: {model, medias}}`.
    ViralityPredictor,
    /// `url` and `type` in, a confirmed `media_id` out.
    ImportMedia,
}

impl ToolRole {
    pub const ALL: [Self; 27] = [
        Self::Generate,
        Self::Upload,
        Self::ConfirmUpload,
        Self::Status,
        Self::Balance,
        Self::Generations,
        Self::Presets,
        Self::ExecutePreset,
        Self::Like,
        Self::Transactions,
        Self::Assets,
        Self::ModelsExplore,
        Self::JobsWait,
        Self::UpscaleVideo,
        Self::RemoveBackground,
        Self::Reframe,
        Self::Dubbing,
        Self::VoiceChange,
        Self::ListVoices,
        Self::GenerateImage,
        Self::ReferenceElements,
        Self::InfluencerOptions,
        Self::InfluencerPresets,
        Self::InfluencerPrepare,
        Self::InfluencerGenerate,
        Self::ViralityPredictor,
        Self::ImportMedia,
    ];

    /// Every one of these is needed to run a job.
    pub const GENERATION: [Self; 4] = [Self::Generate, Self::Upload, Self::ConfirmUpload, Self::Status];
    /// Every one of these is needed to run a Genjutsu Style.
    pub const STYLES: [Self; 5] = [
        Self::Presets,
        Self::ExecutePreset,
        Self::Upload,
        Self::ConfirmUpload,
        Self::Status,
    ];

    /// The tool's name.
    #[must_use]
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Generate => "generate_video",
            Self::Upload => "media_upload",
            Self::ConfirmUpload => "media_confirm",
            Self::Status => "job_status",
            Self::Balance => "balance",
            Self::Generations => "show_generations",
            Self::Presets => "get_presets",
            Self::ExecutePreset => "execute_preset",
            Self::Like => "set_generation_like",
            Self::Transactions => "transactions",
            Self::Assets => "assets_picker_list",
            Self::ModelsExplore => "models_explore",
            Self::JobsWait => "jobs_wait",
            Self::UpscaleVideo => "upscale_video",
            Self::RemoveBackground => "remove_background",
            Self::Reframe => "reframe",
            Self::Dubbing => "dubbing",
            Self::VoiceChange => "voice_change",
            Self::ListVoices => "list_voices",
            Self::GenerateImage => "generate_image",
            Self::ReferenceElements => "show_reference_elements",
            Self::InfluencerOptions => "ai_influencer_get_options",
            Self::InfluencerPresets => "ai_influencer_list_presets",
            Self::InfluencerPrepare => "ai_influencer_prepare",
            Self::InfluencerGenerate => "ai_influencer_generate",
            Self::ViralityPredictor => "virality_predictor",
            Self::ImportMedia => "media_import_url",
        }
    }

    /// Top-level arguments the tool's `inputSchema` must declare; otherwise its role counts as unavailable.
    #[must_use]
    pub fn required_arguments(self) -> &'static [&'static str] {
        match self {
            Self::Generate
            | Self::UpscaleVideo
            | Self::RemoveBackground
            | Self::Reframe
            | Self::Dubbing
            | Self::VoiceChange
            | Self::GenerateImage => &["params"],
            Self::Upload => &["filename", "content_type"],
            Self::ConfirmUpload => &["media_id", "type"],
            Self::Status => &["jobId"],
            Self::Balance => &[],
            Self::Generations => &["size", "cursor"],
            Self::Presets => &["source", "category", "preset_id", "initial_inputs", "cursor"],
            Self::ExecutePreset => &["source", "preset_id", "inputs"],
            Self::Like => &["job_id", "liked"],
            Self::Transactions | Self::ListVoices | Self::InfluencerPresets => &["cursor", "size"],
            Self::Assets => &["source", "kind", "cursor", "size"],
            Self::ModelsExplore => &["action", "type"],
            Self::JobsWait => &["jobs"],
            Self::ReferenceElements => &["action", "element_id", "cursor", "size"],
            Self::InfluencerOptions => &["tier"],
            Self::InfluencerPrepare => &[
                "tier",
                "selection",
                "brief",
                "seed",
                "batch_size",
                "medias",
                "item_medias",
                "randomize",
            ],
            Self::InfluencerGenerate => &["params", "max_credits"],
            Self::ViralityPredictor => &["action", "params"],
            Self::ImportMedia => &["url", "type"],
        }
    }

    /// These calls may charge before an execution error is returned.
    #[must_use]
    pub fn may_start_generation(self) -> bool {
        matches!(
            self,
            Self::Generate
                | Self::GenerateImage
                | Self::ExecutePreset
                | Self::UpscaleVideo
                | Self::RemoveBackground
                | Self::Reframe
                | Self::Dubbing
                | Self::VoiceChange
                | Self::ViralityPredictor
                | Self::InfluencerGenerate
        )
    }

    /// Polls of a job that may already be paid: an answer that can't be read is retried, never taken as the end.
    #[must_use]
    pub fn reads_job_status(self) -> bool {
        matches!(self, Self::Status | Self::JobsWait)
    }
}

/// The server's tools, each matched to its role by name and schema.
#[derive(Debug, Clone, Default)]
pub(crate) struct ToolCatalog {
    tools: HashMap<ToolRole, McpTool>,
}

impl ToolCatalog {
    pub fn new(available: &[McpTool]) -> Self {
        let mut tools = HashMap::new();
        for role in ToolRole::ALL {
            let matching = available.iter().find(|tool| {
                let names = tool.argument_names();
                tool.name == role.raw_value()
                    && role
                        .required_arguments()
                        .iter()
                        .all(|argument| names.iter().any(|name| name == argument))
            });
            if let Some(tool) = matching {
                tools.insert(role, tool.clone());
            }
        }
        Self { tools }
    }

    pub fn get(&self, role: ToolRole) -> Option<&McpTool> {
        self.tools.get(&role)
    }

    /// Every tool a job needs is there.
    pub fn can_generate(&self) -> bool {
        ToolRole::GENERATION
            .iter()
            .all(|role| self.tools.contains_key(role))
    }

    pub fn can_run_styles(&self) -> bool {
        ToolRole::STYLES.iter().all(|role| self.tools.contains_key(role))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    /// The catalog snapshot of 2026-11-10 as the client reads it: every role finds its tool by name and arguments.
    fn snapshot() -> Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/mcp-snapshots/2026-11-10.json"
        );
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn snapshot_tool(snapshot: &Value, name: &str) -> McpTool {
        let listed = snapshot["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name);
        McpTool::from_value(listed.unwrap()).unwrap()
    }

    #[test]
    fn every_role_finds_its_tool_in_the_snapshot() {
        let snapshot = snapshot();
        let tools: Vec<McpTool> = ToolRole::ALL
            .iter()
            .map(|role| snapshot_tool(&snapshot, role.raw_value()))
            .collect();
        let catalog = ToolCatalog::new(&tools);
        let missing: Vec<_> = ToolRole::ALL
            .into_iter()
            .filter(|role| catalog.get(*role).is_none())
            .collect();
        assert!(missing.is_empty(), "{missing:?}");
    }

    /// `CharacterSheetsTests`: the AI Influencer tools are found by their snapshot schemas.
    #[test]
    fn every_influencer_tool_is_found_by_its_snapshot_schema() {
        let snapshot = snapshot();
        let names = [
            "ai_influencer_get_options",
            "ai_influencer_list_presets",
            "ai_influencer_prepare",
            "ai_influencer_generate",
            "assets_picker_list",
        ];
        let tools: Vec<McpTool> = names.iter().map(|name| snapshot_tool(&snapshot, name)).collect();
        let catalog = ToolCatalog::new(&tools);
        for role in [
            ToolRole::InfluencerOptions,
            ToolRole::InfluencerPresets,
            ToolRole::InfluencerPrepare,
            ToolRole::InfluencerGenerate,
        ] {
            assert!(catalog.get(role).is_some(), "{role:?}");
        }
    }
}
