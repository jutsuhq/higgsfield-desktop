//! The tools of Higgsfield's MCP server as typed calls. Which tools exist is read from `tools/list` and kept; a tool
//! missing from that list is looked for once more in a fresh one, so a server that gained it since recovers without
//! reconnecting. Every failure surfaces as `HiggsfieldError`.

mod catalog;
mod elements;
mod influencer;
mod library;
mod models;
mod operations;
mod video;

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use time::OffsetDateTime;
use url::Url;

use catalog::ToolCatalog;
pub use catalog::ToolRole;
pub use elements::{ReferenceElement, ReferenceElementCategory, ReferenceElementImage, ReferenceElementPage};
pub use influencer::{
    InfluencerDesign, InfluencerImage, InfluencerImageKind, InfluencerPreset, InfluencerPresetDesign,
    InfluencerPresetPage, InfluencerQuote, InfluencerSubmission, InfluencerTraitGroup, InfluencerTraitOption,
};
pub use library::{
    AccountAsset, AccountAssetPage, AssetKind, CreditTransaction, CreditTransactionPage, GenerationPage,
    GenerationType, GenjutsuCategory, GenjutsuPreset, GenjutsuPresetPage, PastGeneration, PastMedia,
    StyleRun,
};
pub use models::{AccountImageModel, AccountImageRequest, ModelLimits, UnlimitedAllowance};
pub use operations::{
    JobsWait, VideoEdit, VideoEditInput, VideoEditOperation, VideoEditProvider, ViralityJob,
    ViralityPrediction, ViralityStatus, Voice, VoicePage,
};
pub use video::{AccountBalance, Generation, MediaRole, UploadedMedia, VideoMedia, VideoRequest};

use super::client::{McpClient, McpTool, McpToolResult};
use super::error::McpError;
use super::json::{self, Object};
use super::oauth::Clock;
use crate::api::file_put;
use crate::core::{GenerationStatus, HiggsfieldError, Moderation, is_https, send};

/// The account behind one MCP client: its tools as typed calls.
pub struct HiggsfieldAccount {
    client: Arc<McpClient>,
    http: reqwest::Client,
    now: Clock,
    catalog: Mutex<Option<(ToolCatalog, OffsetDateTime)>>,
}

impl std::fmt::Debug for HiggsfieldAccount {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HiggsfieldAccount")
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

impl HiggsfieldAccount {
    /// How old the catalog must be before an optional tool's absence (`balance`) is checked again. A missing tool
    /// that a call needs is always checked again, as the call fails otherwise; `balance` is asked for routinely
    /// and its absence is a normal answer, so without this floor it would re-list every time.
    pub const OPTIONAL_TOOL_RECHECK: Duration = Duration::from_secs(60);
    /// `media_import_url`'s limit: "Max URL payload: 50 MB".
    pub const IMPORT_LIMIT: i64 = 50_000_000;
    /// How Higgsfield words an `isError` answer that started nothing, as seen live on 2026-10-15: "Invalid
    /// request:" (an unknown model), and the refused unlimited opt-in.
    pub const REFUSAL_PREFIXES: [&'static str; 2] = [
        "Invalid request:",
        "Error starting generation: Unlimited generations aren't supported",
    ];

    pub fn new(client: Arc<McpClient>) -> Self {
        Self {
            client,
            http: reqwest::Client::new(),
            now: Arc::new(OffsetDateTime::now_utc),
            catalog: Mutex::new(None),
        }
    }

    /// The client that PUTs files to the upload URLs the server hands out; nothing else goes through it.
    #[must_use]
    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    #[must_use]
    pub fn with_clock(mut self, now: Clock) -> Self {
        self.now = now;
        self
    }

    pub fn client(&self) -> &McpClient {
        &self.client
    }

    /// Re-reads `tools/list`. `true` when every tool a generation needs is there with the arguments it takes.
    pub async fn refresh_tools(&self) -> Result<bool, HiggsfieldError> {
        let tools = self.client.list_tools().await.map_err(map_error)?;
        let catalog = ToolCatalog::new(&tools);
        let can_generate = catalog.can_generate();
        *self.catalog.lock().unwrap_or_else(PoisonError::into_inner) = Some((catalog, (self.now)()));
        Ok(can_generate)
    }

    /// Whether the last `tools/list` had every tool a Genjutsu Style needs; `false` before one was read.
    pub fn can_run_styles(&self) -> bool {
        self.catalog
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|(catalog, _)| catalog.can_run_styles())
    }

    /// `HiggsfieldError::ToolUnavailable` when the account has no usable `generate_video`.
    pub async fn check_generate(&self) -> Result<(), HiggsfieldError> {
        self.tool(ToolRole::Generate).await.map(drop)
    }

    /// The exact price in credits; submits nothing. The server prices only uploaded files.
    pub async fn cost(&self, request: &VideoRequest) -> Result<f64, HiggsfieldError> {
        credits(json::get(&self.send_video(request, true).await?, "cost"))
            .ok_or_else(|| unexpected(ToolRole::Generate))
    }

    /// Not idempotent: the tool takes no idempotency key, so sending a request again may pay twice. A reply that
    /// names no job and gives no reason is `Server { status: 200 }`: the call went through and may have started a
    /// generation, so it must not read as a refusal that is safe to send again. Every `results[].id` is a job to
    /// poll: the first is the generation's `id`, the others, one per further variant (`count`), its `variant_ids`.
    pub async fn generate(&self, request: &VideoRequest) -> Result<Generation, HiggsfieldError> {
        let value = self.send_video(request, false).await?;
        let mut ids: Vec<String> = Vec::new();
        for id in json::array(&value, "results")
            .into_iter()
            .flatten()
            .filter_map(|result| json::string(result, "id"))
        {
            if !ids.iter().any(|known| known == id) {
                ids.push(id.to_owned());
            }
        }
        if ids.is_empty() {
            return Err(no_job_started(&value));
        }
        let id = ids.remove(0);
        Ok(Generation {
            id,
            request_id: json::string(&value, "request_id").map(str::to_owned),
            credits: credits(json::get(&value, "cost")),
            variant_ids: ids,
        })
    }

    /// Asks for an upload URL, PUTs the file there and confirms it as a video, an audio file or an image, by its
    /// content type. The server tells them apart by the file name's extension. An upload URL that isn't https is
    /// never opened.
    pub async fn upload(&self, file: &Path, content_type: &str) -> Result<UploadedMedia, HiggsfieldError> {
        let filename = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let value = self
            .call(
                ToolRole::Upload,
                object(json!({ "filename": filename, "content_type": content_type })),
            )
            .await?;
        let slot = json::array(&value, "uploads").and_then(|uploads| uploads.first());
        let parsed = slot.and_then(|slot| {
            let media_id = json::string(slot, "media_id")?.to_owned();
            let upload_url = Url::parse(json::string(slot, "upload_url")?)
                .ok()
                .filter(is_https)?;
            let url = Url::parse(json::string(slot, "url")?).ok()?;
            Some((media_id, upload_url, url))
        });
        let Some((media_id, upload_url, url)) = parsed else {
            return Err(failure(&value, ToolRole::Upload));
        };
        let mut headers = HeaderMap::new();
        let content = HeaderValue::from_str(content_type)
            .map_err(|_| HiggsfieldError::validation("The file type can't be sent"))?;
        headers.insert(CONTENT_TYPE, content);
        let response = send(file_put(&self.http, upload_url, headers, file).await?).await?;
        if !response.is_success() {
            return Err(HiggsfieldError::Server {
                status: response.status,
            });
        }
        let kind = ["video", "audio"]
            .into_iter()
            .find(|kind| content_type.starts_with(&format!("{kind}/")))
            .unwrap_or("image");
        let confirmed = self
            .call(
                ToolRole::ConfirmUpload,
                object(json!({ "media_id": media_id, "type": kind })),
            )
            .await?;
        if let Some(error) = json::string(&confirmed, "error") {
            return Err(HiggsfieldError::validation(error));
        }
        Ok(UploadedMedia { id: media_id, url })
    }

    /// `media_import_url`: the server fetches an https file of at most `IMPORT_LIMIT` bytes itself and answers with
    /// a confirmed `media_id`, which `medias` take like an upload's. `kind` is "video" or "image"; the server checks
    /// it against the file's content type. A refusal (`isError`, `error`, or no `media_id`) is `Validation`.
    pub async fn import_media(&self, url: &Url, kind: &str) -> Result<String, HiggsfieldError> {
        let value = self
            .call(
                ToolRole::ImportMedia,
                object(json!({ "url": url.as_str(), "type": kind })),
            )
            .await?;
        match json::string(&value, "media_id") {
            Some(id) if !id.is_empty() => Ok(id.to_owned()),
            _ => Err(failure(&value, ToolRole::ImportMedia)),
        }
    }

    pub async fn status(&self, generation_id: &str) -> Result<GenerationStatus, HiggsfieldError> {
        let value = self
            .call(ToolRole::Status, object(json!({ "jobId": generation_id })))
            .await?;
        let Some(generation) = json::get(&value, "generation") else {
            // A stated error ("Job not found") ends the job; an answer with neither is polled again, since the job
            // may be paid and running.
            return Err(if json::get(&value, "error").is_none() {
                HiggsfieldError::Server { status: 200 }
            } else {
                unexpected(ToolRole::Status)
            });
        };
        // Ends the job, as REST's `Failed` does for a completed job without a URL (or one that isn't https):
        // polling again won't bring one. Reported as a broken answer: the tool's `outputSchema` promises `rawUrl`.
        status_of(generation, json::get(&value, "error")).ok_or_else(|| unexpected(ToolRole::Status))
    }

    /// `None` when the account exposes no balance tool.
    pub async fn balance(&self) -> Result<Option<AccountBalance>, HiggsfieldError> {
        if self
            .look_up(ToolRole::Balance, Self::OPTIONAL_TOOL_RECHECK)
            .await?
            .is_none()
        {
            return Ok(None);
        }
        let value = self.call(ToolRole::Balance, Object::new()).await?;
        let credits = json::num(&value, "credits").ok_or_else(|| unexpected(ToolRole::Balance))?;
        Ok(Some(AccountBalance {
            credits,
            plan: json::string(&value, "subscription_plan_type").map(str::to_owned),
        }))
    }

    /// `generate_video`, quoted or submitted. A reply that is only a `notice`, with neither `results` nor `cost`,
    /// submitted nothing and fails with its message.
    async fn send_video(&self, request: &VideoRequest, get_cost: bool) -> Result<Value, HiggsfieldError> {
        let arguments = request.arguments(get_cost).map_err(map_error)?;
        let value = self.call(ToolRole::Generate, arguments).await?;
        if let Some(notice) = json::get(&value, "notice")
            && json::get(&value, "results").is_none()
            && json::get(&value, "cost").is_none()
        {
            let message = json::string(notice, "message").unwrap_or("Higgsfield didn't take the request");
            return Err(HiggsfieldError::validation(message));
        }
        Ok(value)
    }

    pub(crate) async fn call(&self, role: ToolRole, arguments: Object) -> Result<Value, HiggsfieldError> {
        Ok(self
            .call_result(role, arguments)
            .await?
            .value()
            .unwrap_or(Value::Null))
    }

    pub(crate) async fn call_result(
        &self,
        role: ToolRole,
        arguments: Object,
    ) -> Result<McpToolResult, HiggsfieldError> {
        let quote = arguments
            .get("params")
            .is_some_and(|params| json::is_bool(params, "get_cost", true));
        let name = self.ready_to_send(role).await?;
        let result = match self.client.call_tool(&name, arguments).await.map_err(map_error) {
            // A refused poll says nothing about the job, which may be paid and still running.
            Err(HiggsfieldError::Validation { .. }) if role.reads_job_status() => {
                return Err(HiggsfieldError::Server { status: 200 });
            }
            result => result?,
        };
        if result.is_error {
            if role.reads_job_status() {
                return Err(HiggsfieldError::Server { status: 200 });
            }
            let text = result.text();
            // An execution error does not prove that a paid tool had no side effects, unless it is one of
            // Higgsfield's refusals before execution (`REFUSAL_PREFIXES`): that one keeps its reason.
            if role.may_start_generation() && !quote && !is_refusal(&text) {
                return Err(HiggsfieldError::Server { status: 200 });
            }
            return Err(HiggsfieldError::validation(text));
        }
        Ok(result)
    }

    /// The tool's name, with the session and the token in place first for a paid tool: a failure here happens
    /// before anything is sent, so it is `NotSent` rather than an error that could mean the generation started.
    async fn ready_to_send(&self, role: ToolRole) -> Result<String, HiggsfieldError> {
        if !role.may_start_generation() {
            return Ok(self.tool(role).await?.name);
        }
        let ready = async {
            let name = self.tool(role).await?.name;
            self.client.prepare().await.map_err(map_error)?;
            Ok(name)
        };
        ready.await.map_err(|error| match error {
            HiggsfieldError::Network | HiggsfieldError::Server { .. } => HiggsfieldError::NotSent,
            other => other,
        })
    }

    async fn tool(&self, role: ToolRole) -> Result<McpTool, HiggsfieldError> {
        self.look_up(role, Duration::ZERO)
            .await?
            .ok_or_else(|| HiggsfieldError::ToolUnavailable(role.raw_value().to_owned()))
    }

    /// Reads `tools/list` at most once: when nothing is cached yet, or when `role` is missing from a catalog at
    /// least `minimum_age` old.
    pub(crate) async fn look_up(
        &self,
        role: ToolRole,
        minimum_age: Duration,
    ) -> Result<Option<McpTool>, HiggsfieldError> {
        {
            let catalog = self.catalog.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((catalog, loaded_at)) = catalog.as_ref() {
                if let Some(tool) = catalog.get(role) {
                    return Ok(Some(tool.clone()));
                }
                if (self.now)() - *loaded_at < minimum_age {
                    return Ok(None);
                }
            }
        }
        self.refresh_tools().await?;
        let catalog = self.catalog.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(catalog
            .as_ref()
            .and_then(|(catalog, _)| catalog.get(role).cloned()))
    }
}

/// `value`'s object, for arguments built with `json!`.
fn object(value: Value) -> Object {
    match value {
        Value::Object(object) => object,
        _ => Object::new(),
    }
}

/// The server's own reason when it gave one, as for a declined generation or upload.
fn failure(value: &Value, role: ToolRole) -> HiggsfieldError {
    json::string(value, "error").map_or_else(|| unexpected(role), HiggsfieldError::validation)
}

fn unexpected(role: ToolRole) -> HiggsfieldError {
    HiggsfieldError::validation(format!("Unexpected result from the {} tool", role.raw_value()))
}

/// Every MCP failure as the `HiggsfieldError` the app shows.
pub(crate) fn map_error(error: McpError) -> HiggsfieldError {
    match error {
        McpError::Higgsfield(error) => error,
        // Invalid requests, missing methods and invalid arguments are refused before execution.
        McpError::JsonRpc(error) if [-32600, -32601, -32602].contains(&error.code) => {
            HiggsfieldError::validation(error.message)
        }
        // The client decodes only 2xx replies.
        McpError::JsonRpc(_) | McpError::Decoding(_) => HiggsfieldError::Server { status: 200 },
        // A request that can't be written, such as a NaN duration, is never sent.
        McpError::Encoding => HiggsfieldError::validation("The request has a value that can't be sent"),
        McpError::OAuth(_) | McpError::Browser(_) => HiggsfieldError::Unauthorized,
    }
}

/// What a paid tool's answer that names no job means. With a stated reason (`error`) it is a refusal that started
/// nothing, `Validation` with that reason. Without one the call went through and may have started a paid job, so
/// it is `Server { status: 200 }`, never an error that reads as safe to send again.
fn no_job_started(value: &Value) -> HiggsfieldError {
    json::string(value, "error").map_or(
        HiggsfieldError::Server { status: 200 },
        HiggsfieldError::validation,
    )
}

/// Whether `text` is one of Higgsfield's refusals before execution (`REFUSAL_PREFIXES`).
fn is_refusal(text: &str) -> bool {
    HiggsfieldAccount::REFUSAL_PREFIXES
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

/// The exact credits of a `cost` block, else its rounded ones.
fn credits(cost: Option<&Value>) -> Option<f64> {
    let cost = cost?;
    json::get(cost, "credits_exact")
        .or_else(|| json::get(cost, "credits"))?
        .as_f64()
}

/// A `generation` object's status; `None` for a completed one without an https `rawUrl`. A status added later, or
/// none at all, is most likely another intermediate stage: it reads as in progress, as REST does.
pub(crate) fn status_of(generation: &Value, error: Option<&Value>) -> Option<GenerationStatus> {
    let message = json::get(generation, "error")
        .or(error)
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(match json::string(generation, "status").unwrap_or("") {
        "queued" | "pending" => GenerationStatus::Queued,
        "failed" => GenerationStatus::Failed { message },
        "nsfw" => GenerationStatus::Rejected(Moderation::ExplicitContent),
        "ip_detected" => GenerationStatus::Rejected(Moderation::ProtectedContent),
        "canceled" | "cancelled" => GenerationStatus::Canceled,
        "completed" => {
            let url =
                json::get(generation, "results").and_then(|results| json::media_url(results, "rawUrl"))?;
            GenerationStatus::Completed { video_url: url }
        }
        _ => GenerationStatus::InProgress,
    })
}

/// A cursor argument: sent unless `None` or `null`.
fn insert_cursor(arguments: &mut Object, cursor: Option<&Value>) {
    if let Some(cursor) = cursor.filter(|cursor| !cursor.is_null()) {
        arguments.insert("cursor".into(), cursor.clone());
    }
}

/// `size` kept within `low..=high`, as JSON.
fn clamped(size: i64, low: i64, high: i64) -> Value {
    Value::from(size.clamp(low, high))
}

/// Unix seconds with a fraction, as the server's `createdAt`.
fn unix_date(seconds: Option<f64>) -> OffsetDateTime {
    let seconds = seconds.filter(|seconds| seconds.is_finite()).unwrap_or(0.0);
    #[allow(clippy::cast_possible_truncation)]
    let nanos = (seconds * 1e9).round() as i128;
    OffsetDateTime::from_unix_timestamp_nanos(nanos).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}
