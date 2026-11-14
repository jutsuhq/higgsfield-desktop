//! `models_explore`: model limits, the unlimited allowance, image models and the catalog export.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;

use super::{Generation, HiggsfieldAccount, ToolRole, credits, map_error, no_job_started, object, status_of};
use crate::core::{GenerationStatus, HiggsfieldError, ImageStatus, sorted_pretty_json};
use crate::mcp::client::{McpClient, McpToolResult};
use crate::mcp::json::{self, Object};

/// A model's limits as a `models_explore` `get` answer states them: the `duration` parameter's range and how many
/// reference images the `medias` take. Every bound is optional, `None` where the answer doesn't state it.
///
/// The catalog snapshot of 2026-10-15 confirms the shape: the model itself, its `parameters` an array of `{name, min,
/// max, default, options, required}`, and `medias` entries with `roles` but no counts. Other shapes are still read:
/// the model under `model` or `data`; parameters as an object by name (or its `properties`); a bound as
/// `minimum`/`maximum`, or an `enum`/`values` of numbers; media entries with `role` or `type: "image"`, counted by
/// `min`/`min_count`/`minItems`/`minimum` and `max`/`max_count`/`maxItems`/`maximum`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ModelLimits {
    pub min_duration: Option<f64>,
    pub max_duration: Option<f64>,
    pub min_images: Option<i64>,
    pub max_images: Option<i64>,
}

const PARAMETER_KEYS: [&str; 2] = ["parameters", "params"];
const MIN_KEYS: [&str; 2] = ["minimum", "min"];
const MAX_KEYS: [&str; 2] = ["maximum", "max"];
const MIN_COUNT_KEYS: [&str; 4] = ["min", "min_count", "minItems", "minimum"];
const MAX_COUNT_KEYS: [&str; 4] = ["max", "max_count", "maxItems", "maximum"];

impl ModelLimits {
    /// From one `get` answer.
    pub fn from_answer(answer: &Value) -> Self {
        let model = ["model", "data"]
            .iter()
            .filter_map(|key| json::get(answer, key))
            .find(|model| model.is_object())
            .unwrap_or(answer);
        let mut limits = Self::default();
        if let Some(duration) = parameter("duration", model) {
            (limits.min_duration, limits.max_duration) = bounds(duration, &MIN_KEYS, &MAX_KEYS);
        }
        if let Some(images) = image_media(model) {
            let (minimum, maximum) = bounds(images, &MIN_COUNT_KEYS, &MAX_COUNT_KEYS);
            limits.min_images = minimum.and_then(json::count);
            limits.max_images = maximum.and_then(json::count);
        }
        limits
    }
}

/// The parameter named `name`, from `parameters` (or `params`) as an object by name, its JSON-schema `properties`,
/// or an array of objects with `name`.
fn parameter<'a>(name: &str, model: &'a Value) -> Option<&'a Value> {
    for key in PARAMETER_KEYS {
        let Some(parameters) = json::get(model, key) else {
            continue;
        };
        let found = json::get(parameters, "properties")
            .and_then(|properties| json::get(properties, name))
            .or_else(|| json::get(parameters, name));
        if found.is_some() {
            return found;
        }
        if let Some(found) = parameters
            .as_array()
            .and_then(|items| items.iter().find(|item| json::string(item, "name") == Some(name)))
        {
            return Some(found);
        }
    }
    None
}

/// The `medias` entry for reference images, at the top or in the parameters.
fn image_media(model: &Value) -> Option<&Value> {
    let holders = std::iter::once(model).chain(PARAMETER_KEYS.iter().filter_map(|key| json::get(model, key)));
    holders
        .flat_map(|holder| {
            ["medias", "media"]
                .into_iter()
                .filter_map(|key| json::array(holder, key))
                .flatten()
        })
        .find(|entry| {
            let roles: Vec<&str> = json::string(entry, "role")
                .into_iter()
                .chain(
                    json::array(entry, "roles")
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str),
                )
                .collect();
            roles
                .iter()
                .any(|role| ["image_references", "image"].contains(role))
                || (roles.is_empty() && json::string(entry, "type") == Some("image"))
        })
}

/// `lower` and `upper` as numbers; else the least and greatest of an enum of numbers whose whole numbers run without
/// a gap (4, 5, 6), since a range can't say "5 or 10".
fn bounds(spec: &Value, lower: &[&str], upper: &[&str]) -> (Option<f64>, Option<f64>) {
    let first = |keys: &[&str]| {
        keys.iter()
            .filter_map(|key| json::num(spec, key))
            .find(|value| value.is_finite())
    };
    let (minimum, maximum) = (first(lower), first(upper));
    if minimum.is_some() || maximum.is_some() {
        return (minimum, maximum);
    }
    let Some(values) = ["enum", "values"].iter().find_map(|key| json::array(spec, key)) else {
        return (None, None);
    };
    let numbers: Vec<f64> = values.iter().filter_map(Value::as_f64).collect();
    if numbers.is_empty()
        || numbers.len() != values.len()
        || !numbers
            .iter()
            .all(|number| number.is_finite() && number.fract() == 0.0)
    {
        return (None, None);
    }
    let low = numbers.iter().copied().fold(f64::INFINITY, f64::min);
    let high = numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    #[allow(clippy::cast_possible_truncation)]
    let distinct: HashSet<i64> = numbers.iter().map(|number| *number as i64).collect();
    #[allow(clippy::cast_possible_truncation)]
    let span = ((high - low) as i64).checked_add(1);
    let span_matches = i64::try_from(distinct.len()).is_ok_and(|count| Some(count) == span);
    if span_matches {
        (Some(low), Some(high))
    } else {
        (None, None)
    }
}

/// Whether the account can pay a `generate_video` model from its plan's unlimited allowance (`use_unlim`), as
/// `models_explore` reports it: each model item's `supports_unlim`, and the answer's top-level `unlim` block for the
/// account right now. A field that is missing or has another shape counts as "no", so the allowance is never used
/// unless both signals were read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnlimitedAllowance {
    /// The `unlim` block says the account can spend unlimited generations now.
    pub is_available: bool,
    /// IDs of the models whose item carries `supports_unlim: true`.
    pub supported_models: HashSet<String>,
}

impl UnlimitedAllowance {
    /// The `unlim` block's flags that say the allowance can be spent now; the server sends `available`.
    const AVAILABILITY_FLAGS: [&'static str; 5] = ["available", "can_use", "enabled", "active", "eligible"];
    /// The `unlim` block's counts of what is left; the server sends `remaining`, `null` when unmetered.
    const REMAINING_COUNTS: [&'static str; 2] = ["remaining", "left"];

    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Both signals say yes for `model`.
    #[must_use]
    pub fn covers(&self, model: &str) -> bool {
        self.is_available && self.supported_models.contains(model)
    }

    /// From the answers of `models_explore` `list` pages: the first `unlim` block found, and the model items of
    /// every page.
    pub fn from_pages(pages: &[Value]) -> Self {
        let is_available = pages
            .iter()
            .find_map(|page| json::get(page, "unlim"))
            .is_some_and(Self::block_is_available);
        let supported_models = pages
            .iter()
            .flat_map(model_items)
            .filter(|item| json::is_bool(item, "supports_unlim", true))
            .filter_map(|item| model_id(item).map(str::to_owned))
            .collect();
        Self {
            is_available,
            supported_models,
        }
    }

    /// `true` alone, or an object with at least one known flag or count, all of them saying yes: a flag `true`, a
    /// count above 0. A `null` count is unmetered and says nothing either way. Anything else is "no".
    fn block_is_available(block: &Value) -> bool {
        if let Value::Bool(value) = block {
            return *value;
        }
        let Some(object) = block.as_object() else {
            return false;
        };
        let flags: Vec<&Value> = Self::AVAILABILITY_FLAGS
            .iter()
            .filter_map(|key| object.get(*key))
            .collect();
        let counts: Vec<&Value> = Self::REMAINING_COUNTS
            .iter()
            .filter_map(|key| object.get(*key))
            .filter(|count| !count.is_null())
            .collect();
        if flags.is_empty() && counts.is_empty() {
            return false;
        }
        flags.iter().all(|flag| **flag == Value::Bool(true))
            && counts.iter().all(|count| count.as_f64().unwrap_or(0.0) > 0.0)
    }
}

/// The model items of a page: its `items` array, or `models` or `data`, or the answer itself when it is one.
pub(crate) fn model_items(page: &Value) -> &[Value] {
    if let Some(items) = page.as_array() {
        return items;
    }
    ["items", "models", "data"]
        .iter()
        .find_map(|key| json::array(page, key))
        .map_or(&[], Vec::as_slice)
}

/// `id`, else `model_id`, else `model`.
pub(crate) fn model_id(item: &Value) -> Option<&str> {
    ["id", "model_id", "model"]
        .iter()
        .find_map(|key| json::string(item, key))
}

/// The `params` of one `generate_image` call, as the catalog snapshot of 2026-10-15 states its `inputSchema`.
/// `use_unlim` is always sent, `false`: left out, the server submits nothing for a model the plan's allowance covers
/// and answers with `unlim_choice`, a question for the user.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountImageRequest {
    /// An ID from `models_explore {type: "image"}`.
    pub model: String,
    pub prompt: String,
    /// Left out for the model's default.
    pub aspect_ratio: Option<String>,
    /// Variants, 1–4.
    pub count: i64,
    /// `media_id`s of images to edit.
    pub image_ids: Vec<String>,
    /// The role the images go in under: the model's own (`AccountImageModel::image_role`).
    pub image_role: String,
}

impl AccountImageRequest {
    pub fn new(
        model: impl Into<String>,
        prompt: impl Into<String>,
        aspect_ratio: Option<String>,
        count: i64,
    ) -> Self {
        Self {
            model: model.into(),
            prompt: prompt.into(),
            aspect_ratio,
            count,
            image_ids: Vec::new(),
            image_role: AccountImageModel::IMAGE_ROLES[0].to_owned(),
        }
    }

    pub(crate) fn arguments(&self, get_cost: bool) -> Object {
        let mut params = object(
            json!({ "model": self.model, "prompt": self.prompt, "count": self.count, "use_unlim": false }),
        );
        if let Some(aspect_ratio) = &self.aspect_ratio {
            params.insert("aspect_ratio".into(), aspect_ratio.clone().into());
        }
        if !self.image_ids.is_empty() {
            let medias = self
                .image_ids
                .iter()
                .map(|id| json!({ "role": self.image_role, "value": id }))
                .collect();
            params.insert("medias".into(), Value::Array(medias));
        }
        if get_cost {
            params.insert("get_cost".into(), true.into());
        }
        object(json!({ "params": params }))
    }
}

/// An image model as `models_explore {action: "list", type: "image"}` (or `get`) names it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountImageModel {
    pub id: String,
    pub name: String,
    /// The frames the model takes, as "W:H" (some list "auto" too).
    pub aspect_ratios: Vec<String>,
    /// The `medias` role for an image to edit; `None` when the model takes none.
    pub image_role: Option<String>,
    /// The most images the model takes; `None` when the answer doesn't say.
    pub max_images: Option<i64>,
}

impl AccountImageModel {
    pub(crate) const NAME_KEYS: [&'static str; 3] = ["name", "title", "display_name"];
    /// The roles an image to edit goes in under, in the order they are preferred.
    pub const IMAGE_ROLES: [&'static str; 2] = ["image", "image_references"];

    pub fn from_item(item: &Value) -> Option<Self> {
        let id = model_id(item).filter(|id| !id.is_empty())?.to_owned();
        let name = Self::NAME_KEYS
            .iter()
            .find_map(|key| json::string(item, key))
            .unwrap_or(&id)
            .to_owned();
        let aspect_ratios = json::array(item, "aspect_ratios")
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let roles_of = |entry: &Value| -> Vec<String> {
            json::array(entry, "roles")
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        };
        let media = json::array(item, "medias").into_iter().flatten().find(|entry| {
            roles_of(entry)
                .iter()
                .any(|role| Self::IMAGE_ROLES.contains(&role.as_str()))
        });
        let roles = media.map(roles_of).unwrap_or_default();
        Some(Self {
            id,
            name,
            aspect_ratios,
            image_role: Self::IMAGE_ROLES
                .iter()
                .find(|role| roles.iter().any(|known| known == *role))
                .map(|role| (*role).to_owned()),
            max_images: media
                .and_then(|media| json::num(media, "max"))
                .and_then(json::count),
        })
    }
}

impl HiggsfieldAccount {
    /// `models_explore` pages read per media type, at most, so a token chain that never ends can't loop.
    pub const MAX_MODEL_PAGES: usize = 20;
    /// The most items a `models_explore` `list` page holds (its schema's `limit` maximum; the default is 20).
    pub const MODEL_PAGE_SIZE: i64 = 100;
    /// The `type` values `models_explore` lists models for.
    pub const EXPLORED_TYPES: [&'static str; 3] = ["video", "image", "audio"];
    /// The `type`s whose listed models are each described with `get` too.
    pub const DESCRIBED_TYPES: [&'static str; 2] = ["video", "image"];
    /// `models_explore` `list` pages read for the allowance, at most.
    pub const MAX_ALLOWANCE_PAGES: usize = 5;
    /// `models_explore` `list` pages read for the image models, at most.
    pub const MAX_IMAGE_MODEL_PAGES: usize = 3;

    /// The limits of each of `models` from `models_explore` `get`; `None` when the account has no
    /// `models_explore`. A model the server refuses to describe, or answers with an error, is left out. Free and
    /// read-only.
    pub async fn model_limits(
        &self,
        models: &[String],
    ) -> Result<Option<HashMap<String, ModelLimits>>, HiggsfieldError> {
        if self
            .look_up(ToolRole::ModelsExplore, Self::OPTIONAL_TOOL_RECHECK)
            .await?
            .is_none()
        {
            return Ok(None);
        }
        let mut limits = HashMap::new();
        for model in models {
            let result = match self
                .explore_models(object(json!({ "action": "get", "model_id": model })))
                .await
            {
                Ok((result, _)) => result,
                Err(HiggsfieldError::Validation { .. }) => continue,
                Err(error) => return Err(error),
            };
            if result.is_error {
                continue;
            }
            if let Some(answer) = result.value() {
                limits.insert(model.clone(), ModelLimits::from_answer(&answer));
            }
        }
        Ok(Some(limits))
    }

    /// What `models_explore` says about the unlimited allowance; `None` when the account has no `models_explore`.
    /// Reads `list` pages of video models until each of `models` was seen, a page says the account has no allowance
    /// now, or the pages end. Free and read-only.
    pub async fn unlimited_allowance(
        &self,
        models: &HashSet<String>,
    ) -> Result<Option<UnlimitedAllowance>, HiggsfieldError> {
        if self
            .look_up(ToolRole::ModelsExplore, Self::OPTIONAL_TOOL_RECHECK)
            .await?
            .is_none()
        {
            return Ok(None);
        }
        let mut arguments = model_list("video");
        let mut pages = Vec::new();
        let mut seen_tokens = HashSet::new();
        for _ in 0..Self::MAX_ALLOWANCE_PAGES {
            let (result, next) = self.explore_models(arguments.clone()).await?;
            let Some(page) = result.value().filter(|_| !result.is_error) else {
                break;
            };
            pages.push(page);
            let read = UnlimitedAllowance::from_pages(&pages);
            let seen: HashSet<&str> = pages.iter().flat_map(model_items).filter_map(model_id).collect();
            let all_seen = models.iter().all(|model| seen.contains(model.as_str()));
            let Some(next) =
                next.filter(|next| read.is_available && !all_seen && seen_tokens.insert(next.clone()))
            else {
                break;
            };
            arguments.insert("after".into(), next.into());
        }
        Ok(Some(UnlimitedAllowance::from_pages(&pages)))
    }

    /// The account's image models in the server's order; `None` when the account has no `models_explore`.
    pub async fn image_models(&self) -> Result<Option<Vec<AccountImageModel>>, HiggsfieldError> {
        if self
            .look_up(ToolRole::ModelsExplore, Self::OPTIONAL_TOOL_RECHECK)
            .await?
            .is_none()
        {
            return Ok(None);
        }
        let mut arguments = model_list("image");
        let mut models: Vec<AccountImageModel> = Vec::new();
        let mut seen_tokens = HashSet::new();
        for _ in 0..Self::MAX_IMAGE_MODEL_PAGES {
            let (result, next) = self.explore_models(arguments.clone()).await?;
            let Some(page) = result.value().filter(|_| !result.is_error) else {
                break;
            };
            for model in model_items(&page).iter().filter_map(AccountImageModel::from_item) {
                if !models.iter().any(|known| known.id == model.id) {
                    models.push(model);
                }
            }
            let Some(next) = next.filter(|next| seen_tokens.insert(next.clone())) else {
                break;
            };
            arguments.insert("after".into(), next.into());
        }
        Ok(Some(models))
    }

    /// One image model by its ID (`models_explore get`); `None` when the account has no `models_explore` or the
    /// server doesn't describe the model.
    pub async fn image_model(&self, id: &str) -> Result<Option<AccountImageModel>, HiggsfieldError> {
        if self
            .look_up(ToolRole::ModelsExplore, Self::OPTIONAL_TOOL_RECHECK)
            .await?
            .is_none()
        {
            return Ok(None);
        }
        let (result, _) = self
            .explore_models(object(json!({ "action": "get", "model_id": id })))
            .await?;
        Ok(result
            .value()
            .filter(|_| !result.is_error)
            .and_then(|answer| AccountImageModel::from_item(&answer)))
    }

    /// `HiggsfieldError::ToolUnavailable` when the account has no `generate_image`.
    pub async fn check_image_tool(&self) -> Result<(), HiggsfieldError> {
        match self
            .look_up(ToolRole::GenerateImage, Self::OPTIONAL_TOOL_RECHECK)
            .await?
        {
            Some(_) => Ok(()),
            None => Err(HiggsfieldError::ToolUnavailable(
                ToolRole::GenerateImage.raw_value().to_owned(),
            )),
        }
    }

    /// The price in credits, read like `generate_video`'s quote; `None` when the answer names none. Submits nothing.
    pub async fn image_cost(&self, request: &AccountImageRequest) -> Result<Option<f64>, HiggsfieldError> {
        let value = self
            .call(ToolRole::GenerateImage, request.arguments(true))
            .await?;
        Ok(credits(json::get(&value, "cost")))
    }

    /// Not idempotent, like `generate`. Every `results[].id` is a job to poll: one per variant, or one job for all
    /// of them. A reply that names no job and gives no reason is `Server { status: 200 }`, as it may have started
    /// one.
    pub async fn generate_images(
        &self,
        request: &AccountImageRequest,
    ) -> Result<Vec<Generation>, HiggsfieldError> {
        let value = self
            .call(ToolRole::GenerateImage, request.arguments(false))
            .await?;
        let ids: Vec<&str> = json::array(&value, "results")
            .into_iter()
            .flatten()
            .filter_map(|result| json::string(result, "id"))
            .collect();
        if ids.is_empty() {
            return Err(no_job_started(&value));
        }
        let request_id = json::string(&value, "request_id").map(str::to_owned);
        let cost = credits(json::get(&value, "cost"));
        // The answer's cost is the whole request's; it is kept on the first job only, so it isn't counted twice.
        Ok(ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                Generation::new(
                    (*id).to_owned(),
                    request_id.clone(),
                    if index == 0 { cost } else { None },
                )
            })
            .collect())
    }

    /// `job_status` of an image job: `generation.results.rawUrl`, as for a video. Each variant of a request is its
    /// own job, so a job holds one image.
    pub async fn image_status(&self, generation_id: &str) -> Result<ImageStatus, HiggsfieldError> {
        let value = self
            .call(ToolRole::Status, object(json!({ "jobId": generation_id })))
            .await?;
        // As for a video: a stated error ends the job, an answer with neither is polled again.
        let Some(generation) = json::get(&value, "generation") else {
            return Err(if json::get(&value, "error").is_none() {
                HiggsfieldError::Server { status: 200 }
            } else {
                HiggsfieldError::validation("Unexpected result from the job_status tool")
            });
        };
        Ok(image_status_of(generation, json::get(&value, "error")))
    }

    /// The server's tool and model catalogs as pretty-printed, key-sorted JSON, to check the docs and the arguments
    /// the app sends against the live server. Calls only free, read-only tools: `tools/list`, then `models_explore`
    /// with `list` for each type in `EXPLORED_TYPES` and `get` for each video and image model the lists name.
    ///
    /// Holds only what the server answered and the time of the export: never the access token or the session ID.
    /// Every occurrence of a string in `redacting` (the account's email, say) is replaced, in case an answer echoes
    /// it. Without `models_explore`, `models` is `null`; a `models_explore` call the server refuses is kept with its
    /// error rather than ending the export.
    pub async fn export_catalog(&self, redacting: &[String]) -> Result<Vec<u8>, HiggsfieldError> {
        let tools = self.client.list_tool_values().await.map_err(map_error)?;
        let models = if tools
            .iter()
            .any(|tool| json::string(tool, "name") == Some("models_explore"))
        {
            self.explore_catalog().await?
        } else {
            Value::Null
        };
        let exported_at = (self.now)()
            .replace_nanosecond(0)
            .ok()
            .and_then(|now| now.format(&Rfc3339).ok());
        let export = json!({
            "exportedAt": exported_at,
            "protocolVersion": McpClient::PROTOCOL_VERSION,
            "tools": tools,
            "models": models,
        });
        let secrets: Vec<String> = redacting
            .iter()
            .filter(|secret| !secret.is_empty())
            .cloned()
            .collect();
        Ok(sorted_pretty_json(&json::redacting(&export, &secrets)))
    }

    /// `{"list": {type: [page…]}, "get": {model_id: answer}}`.
    async fn explore_catalog(&self) -> Result<Value, HiggsfieldError> {
        let mut lists = Object::new();
        let mut described: Vec<String> = Vec::new();
        for kind in Self::EXPLORED_TYPES {
            let mut pages = Vec::new();
            let mut arguments = model_list(kind);
            let mut seen_tokens = HashSet::new();
            for _ in 0..Self::MAX_MODEL_PAGES {
                let (record, next) = self.explore(arguments.clone()).await?;
                pages.push(record);
                let Some(next) = next.filter(|next| seen_tokens.insert(next.clone())) else {
                    break;
                };
                arguments.insert("after".into(), next.into());
            }
            if Self::DESCRIBED_TYPES.contains(&kind) {
                described.extend(pages.iter().flat_map(|page| {
                    model_items(json::get(page, "structuredContent").unwrap_or(&Value::Null))
                        .iter()
                        .filter_map(|item| model_id(item).map(str::to_owned))
                        .collect::<Vec<_>>()
                }));
            }
            lists.insert(kind.into(), Value::Array(pages));
        }
        let mut models = Object::new();
        for id in described {
            if models.contains_key(&id) {
                continue;
            }
            let (record, _) = self
                .explore(object(json!({ "action": "get", "model_id": id })))
                .await?;
            models.insert(id, record);
        }
        Ok(json!({ "list": lists, "get": models }))
    }

    /// One `models_explore` call: its arguments with the whole answer, and the next page's token if it names one.
    async fn explore(&self, arguments: Object) -> Result<(Value, Option<String>), HiggsfieldError> {
        let (result, next) = match self.explore_models(arguments.clone()).await {
            Ok(page) => page,
            Err(HiggsfieldError::Validation { message, .. }) => {
                return Ok((json!({ "arguments": arguments, "error": message }), None));
            }
            Err(error) => return Err(error),
        };
        let record = json!({
            "arguments": arguments,
            "isError": result.is_error,
            "content": result.content,
            "structuredContent": result.structured_content,
        });
        Ok((record, next))
    }

    /// One `models_explore` call by the tool's name, and the token of the next page, which goes back in `after`: the
    /// answer's `next_page_token`, unless `has_more` is `false`.
    pub(crate) async fn explore_models(
        &self,
        arguments: Object,
    ) -> Result<(McpToolResult, Option<String>), HiggsfieldError> {
        let result = self
            .client
            .call_tool("models_explore", arguments)
            .await
            .map_err(map_error)?;
        let next = result
            .value()
            .filter(|value| !result.is_error && !json::is_bool(value, "has_more", false))
            .and_then(|value| json::string(&value, "next_page_token").map(str::to_owned))
            .filter(|token| !token.is_empty());
        Ok((result, next))
    }
}

/// The arguments of the first `models_explore` `list` page of `kind`, `MODEL_PAGE_SIZE` models long.
pub(crate) fn model_list(kind: &str) -> Object {
    object(json!({ "action": "list", "type": kind, "limit": HiggsfieldAccount::MODEL_PAGE_SIZE }))
}

pub(crate) fn image_status_of(generation: &Value, error: Option<&Value>) -> ImageStatus {
    match status_of(generation, error) {
        Some(GenerationStatus::Queued) => ImageStatus::Queued,
        Some(GenerationStatus::InProgress) => ImageStatus::InProgress,
        Some(GenerationStatus::Completed { video_url }) => ImageStatus::Completed {
            image_urls: vec![video_url],
        },
        Some(GenerationStatus::Failed { message }) => ImageStatus::Failed { message },
        Some(GenerationStatus::Rejected(moderation)) => ImageStatus::Rejected(moderation),
        Some(GenerationStatus::Canceled) => ImageStatus::Canceled,
        None => ImageStatus::Failed {
            message: Some("Completed without an image URL.".into()),
        },
    }
}
