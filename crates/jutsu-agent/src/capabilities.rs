//! The answer to `capabilities`. Never carries a credential or the account's e-mail: whether a connection exists and
//! what it pays with is all an agent needs.

use crate::{
    PROTOCOL_VERSION, Uuid, enhance::AgentSmoothMotion, library::MAX_INPUTS,
    subtitles::AgentSubtitleCapabilities, values::wire_enum,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

wire_enum! {
    pub enum ConnectionKind { None = "none", ApiKey = "apiKey", Account = "account" }
}

wire_enum! {
    /// What an agent budget counts in: cents with an API key, credits on an account.
    pub enum AgentBudgetUnit { UsdCents = "usd_cents", Credits = "credits" }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub kind: ConnectionKind,
    /// The account's plan, when Higgsfield named it.
    pub plan: Option<String>,
    /// The account's credits as last read; `None` with an API key, or before it is read.
    pub credits: Option<i64>,
}

/// What agents may spend today, in whole units of what the connection pays with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub unit: AgentBudgetUnit,
    /// Settings › Agents › Daily agent budget; 0 until the person sets one.
    pub daily_limit: i64,
    pub spent_today: i64,
    pub reserved_today: i64,
    /// What is left of `daily_limit` once spent and reserved are taken, as the app's spending guard counts it.
    pub remaining: i64,
    /// Settings › Agents › Let agents use unlimited generations.
    pub unlimited_allowed: bool,
    /// The person's own daily cap, in the same unit; `None` when they set none.
    pub daily_cap: Option<i64>,
    pub cap_taken_today: Option<i64>,
}

/// Tools that belong together, so an agent can tell what this version of the app offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolGroup {
    pub name: String,
    pub tools: Vec<String>,
}

impl ToolGroup {
    /// Every tool this version offers, in the order `tools/list` gives them.
    pub fn all() -> Vec<Self> {
        let group = |name: &str, tools: &[&str]| Self {
            name: name.to_owned(),
            tools: tools.iter().map(|tool| (*tool).to_owned()).collect(),
        };
        vec![
            group("app", &["capabilities"]),
            group("video", &["quote", "create_video", "jobs", "job_action"]),
            group("image", &["create_image"]),
            group("elements", &["elements", "create_element"]),
            group(
                "library",
                &[
                    "library_search",
                    "library_get",
                    "library_download",
                    "library_mark",
                    "enhance",
                    "delivery_set",
                    "export",
                    "frames",
                    "subtitles",
                ],
            ),
            group("window", &["show"]),
        ]
    }
}

/// A video model, compact: what an agent needs to pick one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub title: String,
    pub modes: Vec<String>,
    pub resolutions: Vec<String>,
    pub available: bool,
    /// Only for the one model asked for: what a job on it takes.
    pub details: Option<ModelDetails>,
}

/// A model's limits in one mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeDetails {
    pub mode: String,
    pub min_seconds: f64,
    pub max_seconds: f64,
    pub min_images: i64,
    pub max_images: i64,
    pub needs_style: bool,
    pub needs_source: bool,
    pub resolutions: Vec<String>,
    pub max_variants: i64,
    pub variants_unavailable: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools, reason = "the wire's fields")]
pub struct ModelDetails {
    pub modes: Vec<ModeDetails>,
    pub has_draft: bool,
    pub makes_clips_without_source: bool,
    pub needs_prompt: bool,
    /// `true` for a model whose characters say a recording; left out for every other model.
    pub takes_speech: Option<bool>,
}

/// An image model of the connection, as `create_image` takes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageModel {
    pub id: String,
    pub title: String,
    pub counts: Vec<i64>,
    pub aspect_ratios: Vec<String>,
    /// "none", "optional" or "required": whether it edits input images.
    pub input: String,
    /// The most images `create_image` takes as `inputs`; 0 when it edits none.
    pub max_inputs: i64,
    /// Set when `max_inputs` is the protocol's bound rather than the model's own.
    pub max_inputs_note: Option<String>,
}

impl ImageModel {
    /// `max_inputs_note` for a model whose catalog states no limit.
    pub fn bound_note() -> String {
        format!(
            "Higgsfield's catalog states no limit for this model, so up to {MAX_INPUTS} are taken; Higgsfield's own \
             quote, made before anything is sent, refuses more than the model takes, and that costs nothing."
        )
    }
}

/// A Restyle style, by the ID `create_video` takes as `style`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Style {
    pub id: Uuid,
    pub name: String,
}

/// A voice of Change Voice, by the ID `enhance` takes as `voice`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
}

/// The app's window, as the person sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub open: bool,
    /// Settings › General › Run in the menu bar.
    pub menu_bar_mode: bool,
}

/// Enhance on this computer: free, no account, no upload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnDevice {
    pub smooth_motion: AgentOnDeviceEngine,
    /// The modes `enhance` `smooth_motion` takes.
    pub modes: Vec<String>,
    pub upscale_video: AgentOnDeviceEngine,
    pub upscale_image: AgentOnDeviceEngine,
    /// `subtitles`: its speech models, their languages and models.
    pub subtitles: Option<AgentSubtitleCapabilities>,
}

impl OnDevice {
    pub fn new(
        smooth_motion: AgentOnDeviceEngine,
        upscale_video: AgentOnDeviceEngine,
        upscale_image: AgentOnDeviceEngine,
    ) -> Self {
        Self {
            smooth_motion,
            modes: AgentSmoothMotion::ALL
                .iter()
                .map(|mode| mode.as_str().to_owned())
                .collect(),
            upscale_video,
            upscale_image,
            subtitles: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub protocol_version: i64,
    pub app_version: String,
    /// Settings › Agents › Allow agents. Always true in an answer.
    pub agents_allowed: bool,
    pub connection: Connection,
    /// `None` while nothing is connected.
    pub budget: Option<Budget>,
    pub tool_groups: Vec<ToolGroup>,
    pub models: Option<Vec<Model>>,
    pub image_models: Option<Vec<ImageModel>>,
    pub image_models_note: Option<String>,
    pub styles: Option<Vec<Style>>,
    pub voices: Option<Vec<Voice>>,
    pub on_device: Option<OnDevice>,
    pub window: Option<Window>,
    /// The `jutsu` helper's own version, added by the helper (`note_helper`); the app leaves it out.
    pub helper_version: Option<String>,
    /// Added by a helper older than the app: what the agent should do about it.
    pub helper_note: Option<String>,
}

impl AgentCapabilities {
    pub fn new(app_version: &str, connection: Connection, budget: Option<Budget>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            app_version: app_version.to_owned(),
            agents_allowed: true,
            connection,
            budget,
            tool_groups: ToolGroup::all(),
            models: None,
            image_models: None,
            image_models_note: None,
            styles: None,
            voices: None,
            on_device: None,
            window: None,
            helper_version: None,
            helper_note: None,
        }
    }

    /// The helper `version` that relays this answer: an MCP client keeps the helper process it started, so after the
    /// app is updated it can run an older helper, with that version's tools, until it reconnects.
    pub fn note_helper(&mut self, version: &str) {
        self.helper_version = Some(version.to_owned());
        self.helper_note = is_older(version, &self.app_version).then(|| {
            format!(
                "This jutsu helper is {version}, older than Jutsu Studio {}: the MCP client still runs the helper it \
                 started before the app was updated, with that version's tools. Reconnect the jutsu MCP server (/mcp \
                 in Claude Code, or restart the client) to get this version's tools.",
                self.app_version
            )
        });
    }
}

/// Whether release `helper` is older than release `app`, compared as numbers (0.2.10 after 0.2.9); a development
/// build or a version that isn't a number is never called older.
pub fn is_older(helper: &str, app: &str) -> bool {
    let is_release = |version: &str| version.chars().next().is_some_and(char::is_numeric);
    is_release(helper) && is_release(app) && numeric_order(helper, app) == Ordering::Less
}

/// Foundation's `.numeric` comparison: runs of digits compare as numbers, the rest character by character.
fn numeric_order(left: &str, right: &str) -> Ordering {
    let (mut left, mut right) = (left.chars().peekable(), right.chars().peekable());
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(a), Some(b)) if a.is_ascii_digit() && b.is_ascii_digit() => {
                let take = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(digit) = chars.next_if(char::is_ascii_digit) {
                        digits.push(digit);
                    }
                    digits.trim_start_matches('0').to_owned()
                };
                let (a, b) = (take(&mut left), take(&mut right));
                let order = a.len().cmp(&b.len()).then_with(|| a.cmp(&b));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(a), Some(b)) => {
                if a != b {
                    return a.cmp(&b);
                }
                left.next();
                right.next();
            }
        }
    }
}

/// One engine of this computer, as `capabilities` reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOnDeviceEngine {
    pub available: bool,
    /// The largest source it takes, either way round; `None` for any size.
    pub max_long_side: Option<i64>,
    pub max_short_side: Option<i64>,
    /// Its model: none, `needs_consent`, downloading, ready, or agreed.
    pub model: String,
}
