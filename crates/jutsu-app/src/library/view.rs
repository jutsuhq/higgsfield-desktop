//! What the Library screen receives over IPC: the grid's snapshot, a result's page and an image's details.

use jutsu_core::{Culling, MaterialReference};
use serde::{Deserialize, Serialize};

use super::chain::ChainStep;
use super::filter::{FilterField, FilterOptions, LibraryFilter};
use super::history::HistoryPlaceholder;
use super::media::{KindCounts, LibraryKind, LibraryOrigin};
use super::relations::LinkAvailability;
use super::thumbnail::ThumbnailFit;

/// What the grid asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct LibraryRequest {
    pub origin: LibraryOrigin,
    pub kind: LibraryKind,
    pub filter: LibraryFilter,
    /// Variant groups shown as every variant instead of one card.
    pub expanded_groups: Vec<String>,
}

/// Video or image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum CardKind {
    Video,
    Image,
}

/// One card of the grid.
#[allow(clippy::struct_excessive_bools, reason = "the wire's fields")]
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryCard {
    /// The material's ID.
    pub id: String,
    pub kind: CardKind,
    /// "rooftop · Claymation"; an image's title or prompt.
    pub title: String,
    /// "480p · $1.91 · 36 min ago"; "soul-2.0 · 15 Oct 2026" for an image.
    pub subtitle: String,
    pub accessibility_label: String,
    /// "Motion", "Restyle", "Swap"…; "Image" for an image.
    pub mode_tag: String,
    pub is_draft: bool,
    /// "6 s"; `None` for an image.
    pub duration_text: Option<String>,
    /// An image's aspect ratio, "16:9", its tile's tag where a video's duration is; `None` for a video.
    pub aspect_text: Option<String>,
    pub prompt: String,
    pub is_starred: bool,
    /// The star can change here: a local result, or one whose like on the connected account it follows.
    pub can_star: bool,
    pub culling: Culling,
    /// The file here; `None` for a result only on Higgsfield.
    pub file_path: Option<String>,
    /// Where `<video>` or `<img>` loads it: the media server for a file here, the account's link otherwise.
    pub media_url: Option<String>,
    /// Only on Higgsfield so far, so not downloaded.
    pub needs_import: bool,
    pub is_downloading: bool,
    /// The file is listed but gone, or on a drive that isn't connected.
    pub is_unavailable: bool,
    /// Its variant group on screen, when it has one.
    pub variant_group: Option<String>,
    /// How many variants a collapsed group's face stands for.
    pub variant_count: Option<i32>,
    pub is_variant_primary: bool,
    pub fit: ThumbnailFit,
    /// The account's generation, for Copy Request ID.
    pub request_id: Option<String>,
    pub menu: CardMenu,
    /// The card stands for a batch of results.
    pub batch: Option<BatchSummary>,
    /// The virality badge, "72".
    pub virality: Option<String>,
}

/// What shows instead of an empty grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEmpty {
    pub title: String,
    pub detail: Option<String>,
    pub action: Option<LibraryEmptyAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum LibraryEmptyAction {
    ClearFilters,
    GoToCreate,
}

/// The Higgsfield origin's paging state, under the grid.
#[allow(clippy::struct_excessive_bools, reason = "the wire's fields")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HistoryFooter {
    pub is_loading: bool,
    pub has_more: bool,
    /// The last load failed: what to say next to Try Again.
    pub failure: Option<String>,
    /// Loading stopped after pages that added nothing; Load More goes on.
    pub is_waiting: bool,
    /// The account's history can be read: the header shows Refresh.
    pub can_refresh: bool,
    /// After the last page: that Styles made elsewhere aren't listed.
    pub note: Option<String>,
}

/// Everything the grid shows now. `LibraryChanged` says when to ask again.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySnapshot {
    pub cards: Vec<LibraryCard>,
    pub counts: KindCounts,
    pub options: FilterOptions,
    /// The filter bar's controls for this kind, in order.
    pub fields: Vec<FilterField>,
    pub is_filter_active: bool,
    /// The origin has nothing at all, rejected results included: the header hides search.
    pub is_origin_empty: bool,
    pub empty: Option<LibraryEmpty>,
    pub placeholder: Option<HistoryPlaceholder>,
    pub history: Option<HistoryFooter>,
    /// This session has a keep or reject to take back.
    pub can_undo: bool,
}

/// A local file and where the window loads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaFile {
    pub path: Option<String>,
    pub url: Option<String>,
    pub exists: bool,
}

/// The source side of the before/after player: the original, limited to the range that was sent.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SourcePlayback {
    pub file: MediaFile,
    pub start: f64,
    pub end: f64,
}

/// One end of a "made from" relation.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LinkView {
    pub reference: MaterialReference,
    pub is_video: bool,
    /// "Start frame", "Reference 2".
    pub role_title: String,
    pub prompt: String,
    pub preview_url: Option<String>,
    pub availability: LinkAvailability,
    /// "Unavailable", "Not in the library".
    pub note: Option<String>,
}

/// An input of the result that can be saved back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct InputView {
    /// "Source video", "Reference 2".
    pub title: String,
    /// "rooftop – Source.mp4".
    pub file_name: String,
    /// The source video's tile.
    pub is_source: bool,
    /// The 1-based reference tile it belongs to.
    pub reference: Option<u32>,
    /// A copy is on this computer: Show in File Manager on its tile.
    pub is_here: bool,
}

/// A result's page.
#[allow(clippy::struct_excessive_bools, reason = "the wire's fields")]
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ResultDetail {
    pub id: String,
    pub title: String,
    /// "36 min ago · Draft".
    pub headline: String,
    pub prompt: String,
    pub facts: Vec<String>,
    /// "00:04.0 – 00:10.0".
    pub range_text: Option<String>,
    /// Under the inputs: the source file's name, "Source on Higgsfield", or "No source video · made from the prompt"
    /// (`ResultSummary.sourceLine`).
    pub source_line: String,
    /// The reference images in the order the job sent them, each where the window loads it: the file here, else
    /// Higgsfield's copy.
    pub reference_previews: Vec<Option<String>>,
    pub billing: String,
    pub timing: Option<String>,
    pub request_id: Option<String>,
    pub correlation_id: Option<String>,
    pub is_starred: bool,
    /// The Library menu's Star item: "Star", or "Unstar" on a starred result (`LibraryMenuTargets.starTitle`).
    pub star_title: String,
    pub can_star: bool,
    pub culling: Culling,
    pub is_draft: bool,
    pub result: MediaFile,
    /// The account's link while it is only on Higgsfield.
    pub remote_url: Option<String>,
    pub needs_import: bool,
    pub source: Option<SourcePlayback>,
    /// Width over height, for sizing the player.
    pub aspect_ratio: Option<f64>,
    pub chain: Vec<ChainStep>,
    pub made_from: Vec<LinkView>,
    pub used_in: Vec<LinkView>,
    pub inputs: Vec<InputView>,
    pub menu: CardMenu,
    pub virality: super::virality::ViralityView,
    /// The Enhance menu; `None` when it is hidden.
    pub enhance: Option<super::enhance::EnhanceMenu>,
    /// "Upscaled from rooftop" on a result an Enhance operation made.
    pub operation_source: Option<super::enhance::MadeFromLink>,
}

/// An image's Details….
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageDetails {
    pub id: String,
    pub reference: MaterialReference,
    /// The prompt, or "No prompt".
    pub prompt: String,
    pub has_prompt: bool,
    pub model: String,
    pub created_text: String,
    pub image: MediaFile,
    pub remote_url: Option<String>,
    pub is_starred: bool,
    pub made_from: Vec<LinkView>,
    pub used_in: Vec<LinkView>,
}

/// What a result card's menu offers past the actions every card has (`LibraryCardMenu`).
#[allow(clippy::struct_excessive_bools, reason = "the wire's fields")]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CardMenu {
    /// Compare…: a file here, or one to download. On an image, it starts picking two images with this one picked.
    pub can_compare: bool,
    /// Copy Image: an image whose file is here (`LibraryImageMenu.availableMenu`).
    pub can_copy_image: bool,
    /// Compare with Original: the material has a version this one was made from.
    pub has_original: bool,
    pub follow_up: Option<FollowUp>,
    /// Edit & Re-run: a result made here that isn't an operation on another.
    pub can_rerun: bool,
    /// Edit… and Extend… with Seedance 2.5: a downloaded result.
    pub can_edit_video: bool,
    /// Edit in… and Import as New Version…: a file here.
    pub can_edit_in: bool,
    /// Join with Original…: an Extend result with its source here.
    pub can_join: bool,
}

/// The card menu's follow-up of a draft or of an account history item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum FollowUp {
    /// Edit Inputs: a result only on Higgsfield.
    EditInputs,
    /// Finalize in 1080p…: a native draft within its seven days.
    Finalize,
    /// Render Final…: another draft.
    RenderFinal,
}

/// The card of a batch: its results fold into one card at the first of them (`LibraryBatchCard`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchSummary {
    pub count: i32,
    /// "Batch × 4".
    pub tag: String,
    pub member_ids: Vec<String>,
    /// What Delete Unstarred… deletes.
    pub unstarred_ids: Vec<String>,
    /// Render Finals for Starred…: a member can render a final.
    pub can_render_finals: bool,
}

/// One video of Compare: the result, its chips and its file.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CompareTrack {
    pub id: String,
    pub title: String,
    /// "Original", then the settings chips: "Restyle", "Claymation", "480p", "6 s".
    pub labels: Vec<String>,
    pub file: MediaFile,
}

/// A layout the compare page offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CompareLayoutOption {
    pub value: super::compare::CompareLayout,
    pub title: String,
}

/// Compare (`LibraryCompare`): two or more results on one clock.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CompareView {
    pub tracks: Vec<CompareTrack>,
    pub layouts: Vec<CompareLayoutOption>,
    /// Columns of the side-by-side grid.
    pub columns: i32,
}

/// What "Compare…" offers for a result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CompareCandidate {
    pub id: String,
    /// "rooftop · Claymation — 480p draft".
    pub title: String,
}

/// One cell of a batch comparison.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchCellView {
    pub id: String,
    /// "Film Noir", "Style 2", "Character 1", "Variant 3".
    pub label: String,
    pub file: MediaFile,
    pub is_starred: bool,
    pub culling: Culling,
    pub virality: super::virality::ViralityView,
    /// Pick winner: the result opens to render its final; else Open.
    pub can_render_final: bool,
}

/// Score Variants on a batch comparison's header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScoreButton {
    pub availability: super::virality::ViralityAvailability,
    /// "Score Variants…" or "Score Again…".
    pub title: String,
    pub blocker: Option<String>,
    /// A run is going on: the button stays off until it ends.
    pub is_running: bool,
}

/// A batch opened as a comparison (`BatchComparison`).
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchView {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub cells: Vec<BatchCellView>,
    pub columns: i32,
    pub unstarred_ids: Vec<String>,
    pub score: ScoreButton,
    pub can_render_finals: bool,
    /// Off, with the help below, until a draft is starred.
    pub has_starred_drafts: bool,
    /// "Star the drafts to render as finals" or "Render the starred drafts as finals at one resolution".
    pub finals_help: String,
    /// The member IDs, for the finals sheet.
    pub member_ids: Vec<String>,
}

/// The Score Variants sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScoreSheet {
    /// "Score Variants" or "Score Again".
    pub title: String,
    /// What a re-score replaces.
    pub notice: Option<String>,
    /// "3 videos · Free".
    pub price_text: String,
    /// "Score" or "Score Again".
    pub button_title: String,
    /// "Free", "12 cr".
    pub price_tag: Option<String>,
    /// The price isn't known: the consent checkbox shows.
    pub needs_consent: bool,
    pub consent_text: String,
    pub blocker: Option<String>,
}

/// What Score did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScoreOutcome {
    Started,
    /// Over today's cap, or the price isn't known under one: ask, then score again approved.
    Confirm {
        confirmation: crate::queue::QueueConfirmation,
    },
    Blocked {
        reason: String,
    },
}
