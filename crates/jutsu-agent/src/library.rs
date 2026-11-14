//! `create_image`, `library_search`, `library_mark`, `library_get` and `library_download`, and what they answer.
//! Each command checks its own fields once they decode, so the app refuses a malformed one with `bad_request`
//! whatever sent it; the helper runs the same checks before it reads any file.

use crate::{
    AgentAmount, AgentError, Timestamp, Uuid,
    elements::AgentElementNames,
    error::Refusal,
    names,
    values::wire_enum,
    video::{AgentVideoRequest, MAX_PROMPT_LENGTH, MAX_TITLE_LENGTH, MODES, count},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser::SerializeMap};
use std::collections::HashSet;

/// More than any model makes, so the model's own counts refuse the rest with a reason.
pub const MAX_COUNT: i64 = 16;
/// A bound of the protocol: the most a model may take where its catalog states no limit.
pub const MAX_INPUTS: i64 = 14;

/// The image an agent asks for, made as the Image side of Create would make it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentImageRequest {
    /// An image model ID from `capabilities`; left out, the connection's first.
    pub model: Option<String>,
    pub prompt: String,
    pub aspect_ratio: Option<String>,
    /// Images the request makes; 1 when left out.
    pub count: Option<i64>,
    /// For `create_image`: the images to edit, in order, paths in the app's inbox.
    pub inputs: Vec<String>,
    /// The account's reference elements by name: their images go first, before `inputs`.
    pub elements: Vec<String>,
    pub title: Option<String>,
}

impl AgentImageRequest {
    pub const BOTH_INPUTS: &str = "send inputs, the images in order, or input, one image; not both";

    /// Shared by the helper and the app.
    pub fn problem(&self) -> Option<String> {
        if self
            .model
            .as_deref()
            .is_some_and(|model| !names::is_valid_image_model_id(model))
        {
            return Some("model is an image model ID from capabilities".to_owned());
        }
        if names::trim_whitespace_and_newlines(&self.prompt).is_empty() {
            return Some("prompt describes the image".to_owned());
        }
        if count(names::scalar_count(&self.prompt)) > MAX_PROMPT_LENGTH {
            return Some(format!(
                "prompt is at most {} characters",
                names::spaced(MAX_PROMPT_LENGTH)
            ));
        }
        if self
            .title
            .as_deref()
            .is_some_and(|title| count(names::scalar_count(title)) > MAX_TITLE_LENGTH)
        {
            return Some(format!(
                "title is at most {} characters",
                names::spaced(MAX_TITLE_LENGTH)
            ));
        }
        if self.aspect_ratio.as_deref().is_some_and(|ratio| ratio.len() > 16) {
            return Some("aspect_ratio is a ratio such as 16:9".to_owned());
        }
        if self
            .count
            .is_some_and(|images| !(1..=MAX_COUNT).contains(&images))
        {
            return Some(format!("count is 1 to {MAX_COUNT}"));
        }
        if count(self.inputs.len()) > MAX_INPUTS {
            return Some(format!("inputs takes at most {MAX_INPUTS} images"));
        }
        if let Some(path) = self
            .inputs
            .iter()
            .find(|path| !AgentVideoRequest::is_valid_path(path))
        {
            return Some(format!(
                "inputs has a path that isn't one: {}",
                names::prefix(path, 80)
            ));
        }
        AgentElementNames::problem(&self.elements)
    }
}

/// One input goes as `input` and several as `inputs`, so a request of one input has the fingerprint 0.2.0 recorded.
impl Serialize for AgentImageRequest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if let Some(model) = &self.model {
            map.serialize_entry("model", model)?;
        }
        map.serialize_entry("prompt", &self.prompt)?;
        if let Some(ratio) = &self.aspect_ratio {
            map.serialize_entry("aspectRatio", ratio)?;
        }
        if let Some(images) = self.count {
            map.serialize_entry("count", &images)?;
        }
        match self.inputs.as_slice() {
            [] => {}
            [input] => map.serialize_entry("input", input)?,
            inputs => map.serialize_entry("inputs", inputs)?,
        }
        if !self.elements.is_empty() {
            map.serialize_entry("elements", &self.elements)?;
        }
        if let Some(title) = &self.title {
            map.serialize_entry("title", title)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for AgentImageRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Fields {
            model: Option<String>,
            prompt: String,
            aspect_ratio: Option<String>,
            count: Option<i64>,
            input: Option<String>,
            inputs: Option<Vec<String>>,
            elements: Option<Vec<String>>,
            title: Option<String>,
        }
        let fields = Fields::deserialize(deserializer)?;
        let inputs = match (fields.input, fields.inputs) {
            (Some(_), Some(_)) => {
                return Err(de::Error::custom(Refusal(AgentError::bad_request(
                    Self::BOTH_INPUTS,
                ))));
            }
            (_, Some(inputs)) => inputs,
            (Some(input), None) => vec![input],
            (None, None) => Vec::new(),
        };
        Ok(Self {
            model: fields.model,
            prompt: fields.prompt,
            aspect_ratio: fields.aspect_ratio,
            count: fields.count,
            inputs,
            elements: fields.elements.unwrap_or_default(),
            title: fields.title,
        })
    }
}

/// Makes images, idempotent by its request ID as `create_video` is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateImageCommand {
    pub request_id: String,
    pub fingerprint: String,
    pub image: AgentImageRequest,
    pub max_cost: AgentAmount,
}

impl CreateImageCommand {
    pub fn problem(&self) -> Option<String> {
        if !names::is_valid_request_id(&self.request_id) {
            return Some(names::REQUEST_ID_RULE.to_owned());
        }
        if !names::is_valid_fingerprint(&self.fingerprint) {
            return Some("fingerprint is 64 lowercase hex digits".to_owned());
        }
        self.image.problem()
    }

    pub(crate) fn check(&self) -> Result<(), AgentError> {
        self.max_cost.check()?;
        self.problem()
            .map_or(Ok(()), |problem| Err(AgentError::bad_request(problem)))
    }
}

wire_enum! {
    /// What the Library holds, by the names agents use.
    pub enum AgentLibraryKind { All = "all", Video = "video", Image = "image" }
}

wire_enum! {
    /// A culling verdict, as the Library's Keep and Reject set it.
    pub enum AgentVerdict { Unreviewed = "unreviewed", Kept = "kept", Rejected = "rejected" }
}

/// Finds results in the Library on this computer, newest first, a page at a time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySearchCommand {
    pub query: Option<String>,
    pub kind: Option<AgentLibraryKind>,
    pub starred: Option<bool>,
    pub verdict: Option<AgentVerdict>,
    pub include_rejected: Option<bool>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub from: Option<Timestamp>,
    pub to: Option<Timestamp>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
    pub include_higgsfield: Option<bool>,
    pub refresh_higgsfield: Option<bool>,
}

impl LibrarySearchCommand {
    pub const DEFAULT_LIMIT: i64 = 20;
    pub const MAX_LIMIT: i64 = 50;
    /// Prompts are cut to this many characters in the answer.
    pub const PROMPT_LENGTH: usize = 200;
    /// In Unicode scalars.
    pub const MAX_QUERY_LENGTH: usize = 500;
    /// In UTF-8 bytes.
    pub const MAX_MODEL_LENGTH: usize = 128;

    pub fn problem(&self) -> Option<String> {
        if self.refresh_higgsfield == Some(true) && self.include_higgsfield != Some(true) {
            return Some(
                "refresh_higgsfield loads the history include_higgsfield lists; send both".to_owned(),
            );
        }
        if self
            .query
            .as_deref()
            .is_some_and(|query| names::scalar_count(query) > Self::MAX_QUERY_LENGTH)
        {
            return Some(format!("query is at most {} characters", Self::MAX_QUERY_LENGTH));
        }
        if self
            .model
            .as_deref()
            .is_some_and(|model| model.len() > Self::MAX_MODEL_LENGTH)
        {
            return Some("model is a model ID or an image model's name".to_owned());
        }
        if self.mode.as_deref().is_some_and(|mode| !MODES.contains(&mode)) {
            return Some(format!("mode is one of {}", MODES.join(", ")));
        }
        if let (Some(from), Some(to)) = (self.from, self.to)
            && to <= from
        {
            return Some("to comes after from".to_owned());
        }
        if self
            .limit
            .is_some_and(|limit| !(1..=Self::MAX_LIMIT).contains(&limit))
        {
            return Some(format!("limit is 1 to {}", Self::MAX_LIMIT));
        }
        if self
            .cursor
            .as_deref()
            .is_some_and(|cursor| !names::is_valid_cursor(cursor))
        {
            return Some("cursor is a nextCursor from an earlier answer".to_owned());
        }
        None
    }
}

/// One result of the Library, as an agent sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLibraryItem {
    pub id: Uuid,
    pub kind: AgentLibraryKind,
    /// The current version's file on this computer; `None` for a result only on Higgsfield.
    pub file: Option<String>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub aspect_ratio: Option<String>,
    pub duration: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub price: Option<AgentAmount>,
    pub verdict: AgentVerdict,
    pub starred: bool,
    pub created_at: Timestamp,
    pub prompt: String,
    pub prompt_truncated: bool,
    pub version_id: Uuid,
    pub version: i64,
    pub version_count: i64,
    pub job_id: Option<Uuid>,
    pub on_higgsfield: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLibraryPage {
    pub items: Vec<AgentLibraryItem>,
    pub next_cursor: Option<String>,
}

wire_enum! {
    /// What `library_mark` sets on a culling verdict.
    pub enum AgentCulling { Keep = "keep", Reject = "reject", Clear = "clear" }
}

/// Sets the culling verdict and, apart from it, the star of Library items. Nothing is deleted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryMarkCommand {
    pub ids: Vec<Uuid>,
    pub verdict: Option<AgentCulling>,
    pub star: Option<bool>,
}

impl LibraryMarkCommand {
    pub const MAX_IDS: usize = 100;

    pub fn problem(&self) -> Option<String> {
        if let Some(problem) = library_ids_problem(&self.ids, Self::MAX_IDS, true) {
            return Some(problem);
        }
        (self.verdict.is_none() && self.star.is_none())
            .then(|| "library_mark needs verdict, star or both".to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMarkedItem {
    pub id: Uuid,
    pub verdict: AgentVerdict,
    pub starred: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMarked {
    /// Each item as it is now.
    pub items: Vec<AgentMarkedItem>,
    /// How many items this call changed; 0 when everything was already so.
    pub changed: i64,
}

/// The Library item IDs a command takes: 1 to `limit`, and, unless `allows_repeats`, none named twice.
pub(crate) fn library_ids_problem(ids: &[Uuid], limit: usize, allows_repeats: bool) -> Option<String> {
    if !(1..=limit).contains(&ids.len()) {
        return Some(format!("ids takes 1 to {limit} Library item IDs"));
    }
    (!allows_repeats && ids.iter().collect::<HashSet<_>>().len() != ids.len())
        .then(|| "ids names an item twice".to_owned())
}

/// The full details of Library items by ID. Read-only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryGetCommand {
    pub ids: Vec<Uuid>,
}

impl LibraryGetCommand {
    pub const MAX_IDS: usize = 20;

    pub fn problem(&self) -> Option<String> {
        library_ids_problem(&self.ids, Self::MAX_IDS, false)
    }
}

wire_enum! {
    pub enum AgentInputRole {
        Source = "source",
        StartFrame = "start_frame",
        EndFrame = "end_frame",
        Reference = "reference",
        /// The result an Enhance operation or an edit was made from.
        EditedFrom = "edited_from",
        /// The recording the characters said.
        Speech = "speech",
    }
}

/// One file that went into a result, in the order its job sent it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLibraryInput {
    pub role: AgentInputRole,
    /// 1-based among the images the job sent, the N of its prompt's `@ImageN`.
    pub position: Option<i64>,
    pub file: Option<String>,
    pub item: Option<Uuid>,
    pub on_higgsfield: bool,
    pub start: Option<f64>,
    pub end: Option<f64>,
}

/// How a result was made, beside its model and mode; a setting that doesn't apply to it is left out.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLibrarySettings {
    pub resolution: Option<String>,
    pub draft: Option<bool>,
    pub audio: Option<String>,
    pub style: Option<Uuid>,
    pub length: Option<f64>,
    pub aspect_ratio: Option<String>,
    pub paid_with_unlimited: Option<bool>,
    pub operation: Option<String>,
    pub count: Option<i64>,
}

/// The version of the same item that the current one was made from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMadeFrom {
    pub item: Uuid,
    pub version_id: Uuid,
    pub version: i64,
    pub file: Option<String>,
}

/// One Library item in full: its row with the whole prompt, its inputs and its settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLibraryDetails {
    pub item: AgentLibraryItem,
    pub inputs: Vec<AgentLibraryInput>,
    pub settings: AgentLibrarySettings,
    pub made_from: Option<AgentMadeFrom>,
    pub next: Option<String>,
}

impl AgentLibraryDetails {
    /// What brings a result only on Higgsfield to this computer.
    pub const DOWNLOAD_NEXT: &str = "Call library_download with its ID to bring the file to this computer.";
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLibraryDetailsPage {
    /// In the order asked.
    pub items: Vec<AgentLibraryDetails>,
    /// The IDs asked for that no Library item has; left out when there are none.
    pub not_found: Option<Vec<Uuid>>,
    pub next: Option<String>,
}

impl AgentLibraryDetailsPage {
    /// What finds the IDs `not_found` lists.
    pub const NOT_FOUND_NEXT: &str =
        "Call library_search, with include_higgsfield for the history, for the item IDs.";

    pub fn new(items: Vec<AgentLibraryDetails>, not_found: Vec<Uuid>) -> Self {
        let next = (!not_found.is_empty()).then(|| Self::NOT_FOUND_NEXT.to_owned());
        Self {
            items,
            not_found: (!not_found.is_empty()).then_some(not_found),
            next,
        }
    }
}

/// Downloads results of the Higgsfield history into the Library. Free; never spends credits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryDownloadCommand {
    pub ids: Vec<Uuid>,
}

impl LibraryDownloadCommand {
    pub const MAX_IDS: usize = 20;

    pub fn problem(&self) -> Option<String> {
        library_ids_problem(&self.ids, Self::MAX_IDS, false)
    }
}

wire_enum! {
    pub enum AgentDownloadStatus {
        Downloaded = "downloaded",
        /// It had a file on this computer already; nothing was downloaded.
        AlreadyHere = "already_here",
        Failed = "failed",
    }
}

/// What came of one download, in the order asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDownload {
    pub id: Uuid,
    pub status: AgentDownloadStatus,
    pub file: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDownloads {
    pub items: Vec<AgentDownload>,
}
