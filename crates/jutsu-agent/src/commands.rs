//! The fields of `capabilities`, `show` and `upload`, and what they answer. Each command checks its own fields once it
//! has decoded (`check`), so the app refuses a malformed command with `bad_request` before any of it runs, whatever
//! sent it.

use crate::{AgentError, Uuid, inbox, names, values::wire_enum};
use serde::{Deserialize, Serialize};

wire_enum! {
    /// The pages `show` can open, by the names the sidebar uses.
    pub enum AgentPage { Create = "create", Queue = "queue", Library = "library", Settings = "settings" }
}

#[allow(clippy::trivially_copy_pass_by_ref, reason = "serde passes a reference")]
pub(crate) fn is_false(flag: &bool) -> bool {
    !*flag
}

#[allow(clippy::trivially_copy_pass_by_ref, reason = "serde passes a reference")]
pub(crate) fn is_zero(number: &i64) -> bool {
    *number == 0
}

#[allow(clippy::struct_excessive_bools, reason = "the wire's fields")]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesCommand {
    /// Lists the video models; left out, the answer stays short.
    #[serde(default, skip_serializing_if = "is_false")]
    pub models: bool,
    /// Only this model, by its ID; implies `models`.
    pub model: Option<String>,
    /// Lists the connection's image models, which `create_image` takes.
    #[serde(default, skip_serializing_if = "is_false")]
    pub image_models: bool,
    /// Lists Restyle's styles from the catalog Create last read.
    #[serde(default, skip_serializing_if = "is_false")]
    pub styles: bool,
    /// Lists the voices Change Voice offers on an account.
    #[serde(default, skip_serializing_if = "is_false")]
    pub voices: bool,
}

impl CapabilitiesCommand {
    /// Why `model` can't name a model; `None` when it can. Shared by the helper and the app.
    pub fn problem(model: Option<&str>) -> Option<String> {
        let model = model?;
        (!names::is_valid_model_id(model)).then(|| "model is a model ID, such as seedance25".to_owned())
    }

    pub(crate) fn check(&self) -> Result<(), AgentError> {
        Self::problem(self.model.as_deref()).map_or(Ok(()), |problem| Err(AgentError::bad_request(problem)))
    }
}

wire_enum! {
    /// The sheets `show` can open for a Library item.
    pub enum AgentSheet {
        /// Upscale on This Computer, where the person agrees to the system downloading its model.
        UpscaleOnMac = "upscale_on_mac",
        /// Subtitles, where the person agrees to downloading a language's model.
        Subtitles = "subtitles",
    }
}

/// Exactly one of `page`, `job` or `item`; `item` comes with `sheet`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShowCommand {
    pub page: Option<AgentPage>,
    /// A job's ID: a finished one opens its result in Library, any other its row in the Queue.
    pub job: Option<Uuid>,
    /// A Library item, which `sheet` is opened for.
    pub item: Option<Uuid>,
    pub sheet: Option<AgentSheet>,
}

impl ShowCommand {
    pub fn page(page: AgentPage) -> Self {
        Self {
            page: Some(page),
            ..Self::default()
        }
    }

    pub fn job(id: Uuid) -> Self {
        Self {
            job: Some(id),
            ..Self::default()
        }
    }

    pub fn sheet(sheet: AgentSheet, item: Uuid) -> Self {
        Self {
            item: Some(item),
            sheet: Some(sheet),
            ..Self::default()
        }
    }

    /// Why `show` can't open these; `None` when it can. It names the tool itself, so the helper sends it as it is.
    #[allow(clippy::fn_params_excessive_bools, reason = "the Swift app's signature")]
    pub fn problem(has_page: bool, has_job: bool, has_item: bool, has_sheet: bool) -> Option<String> {
        if [has_page, has_job, has_item].iter().filter(|flag| **flag).count() != 1 {
            return Some(format!(
                "show takes exactly one of page ({}), job (a job ID) or item (a Library item ID, with sheet)",
                AgentPage::names()
            ));
        }
        if has_item != has_sheet {
            return Some(format!(
                "show opens a sheet for an item: send item with sheet ({})",
                AgentSheet::names()
            ));
        }
        None
    }

    pub(crate) fn check(&self) -> Result<(), AgentError> {
        Self::problem(
            self.page.is_some(),
            self.job.is_some(),
            self.item.is_some(),
            self.sheet.is_some(),
        )
        .map_or(Ok(()), |problem| Err(AgentError::bad_request(problem)))
    }
}

/// What `show` opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentShown {
    pub page: AgentPage,
    pub job: Option<Uuid>,
    pub item: Option<Uuid>,
    pub sheet: Option<AgentSheet>,
}

/// A file an agent named, sent by the helper as `size` raw bytes right after this line. The app stores it in a
/// folder of its own in the inbox and answers with the copy's path, which the command that needs the file then names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadCommand {
    /// The file's own name, kept so its extension says what it is.
    pub name: String,
    pub size: i64,
    /// The request the file belongs to, checked before a single byte is read.
    pub request: Option<UploadRequest>,
}

impl UploadCommand {
    /// Why an upload of `name` with `size` bytes can't be taken; `None` when it can.
    pub fn problem(name: &str, size: i64) -> Option<String> {
        if !names::is_valid_file_name(name) {
            return Some("name is one file name, not hidden".to_owned());
        }
        if !(0..=inbox::MAX_FILE_SIZE).contains(&size) {
            return Some(format!("size is 0 to {} GiB", inbox::MAX_FILE_SIZE >> 30));
        }
        None
    }

    pub(crate) fn check(&self) -> Result<(), AgentError> {
        if let Some(problem) = Self::problem(&self.name, self.size) {
            return Err(AgentError::bad_request(problem));
        }
        self.request.as_ref().map_or(Ok(()), UploadRequest::check)
    }
}

/// Names the request an upload is for: the command, the agent's `request_id` and a digest of the parameters, so the
/// app can tell a retry of the same request from a reused ID.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadRequest {
    pub command: String,
    pub request_id: String,
    /// Lowercase hex SHA-256 of the request's parameters as the helper normalized them.
    pub fingerprint: String,
    /// `create_video`'s digest without `max_cost`.
    pub parameters: Option<String>,
}

impl UploadRequest {
    pub(crate) fn check(&self) -> Result<(), AgentError> {
        check_request_id(&self.request_id)?;
        check_fingerprints(&self.fingerprint, self.parameters.as_deref())
    }
}

pub(crate) fn check_request_id(id: &str) -> Result<(), AgentError> {
    if names::is_valid_request_id(id) {
        Ok(())
    } else {
        Err(AgentError::bad_request(names::REQUEST_ID_RULE))
    }
}

pub(crate) fn check_fingerprints(fingerprint: &str, parameters: Option<&str>) -> Result<(), AgentError> {
    if names::is_valid_fingerprint(fingerprint) && parameters.is_none_or(names::is_valid_fingerprint) {
        Ok(())
    } else {
        Err(AgentError::bad_request("fingerprint is 64 lowercase hex digits"))
    }
}
