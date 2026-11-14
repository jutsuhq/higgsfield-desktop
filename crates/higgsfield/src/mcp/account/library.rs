//! The account's history, credits, uploads and Genjutsu galleries, read from the live server on 2026-10-14.

use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;
use url::Url;
use uuid::Uuid;

use super::{
    Generation, HiggsfieldAccount, MediaRole, ToolRole, clamped, insert_cursor, no_job_started, object,
    unexpected, unix_date,
};
use crate::core::HiggsfieldError;
use crate::mcp::client::McpToolResult;
use crate::mcp::json::{self, Object};

/// One movement of the account's credits.
#[derive(Debug, Clone, PartialEq)]
pub struct CreditTransaction {
    pub date: OffsetDateTime,
    /// Negative when spent, positive when refunded or granted.
    pub credits: f64,
    /// "spend", "refund", "grant" or "deduct".
    pub action: String,
    /// What it was for, as Higgsfield names it: "Higgsfield Genjutsu - Object Swap".
    pub title: String,
}

/// One page of the account's credit history, newest first.
#[derive(Debug, Clone, PartialEq)]
pub struct CreditTransactionPage {
    pub items: Vec<CreditTransaction>,
    /// Opaque: sent back unchanged for the next page. `None` on the last page.
    pub next_cursor: Option<Value>,
}

/// The `kind` filter of the account's uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Image,
    Video,
}

impl AssetKind {
    #[must_use]
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
        }
    }

    fn from_raw_value(text: &str) -> Option<Self> {
        match text {
            "image" => Some(Self::Image),
            "video" => Some(Self::Video),
            _ => None,
        }
    }
}

/// A file the account uploaded, as a generation input or on higgsfield.ai.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountAsset {
    /// The `media_id` generations take.
    pub id: String,
    pub kind: AssetKind,
    pub url: Url,
    /// Videos only.
    pub thumbnail_url: Option<Url>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// Videos only, in seconds.
    pub duration: Option<f64>,
    pub created_at: OffsetDateTime,
}

/// One page of the account's uploads, newest first.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountAssetPage {
    pub items: Vec<AccountAsset>,
    /// Opaque: sent back unchanged for the next page. `None` on the last page.
    pub next_cursor: Option<Value>,
}

/// The `type` filter of `show_generations`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GenerationType {
    Video,
    Image,
    Audio,
    ThreeD,
}

impl GenerationType {
    #[must_use]
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Image => "image",
            Self::Audio => "audio",
            Self::ThreeD => "3d",
        }
    }
}

/// One page of the account's history, newest first.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerationPage {
    pub items: Vec<PastGeneration>,
    /// Opaque: sent back unchanged for the next page. `None` on the last page.
    pub next_cursor: Option<Value>,
}

/// A generation from `show_generations`. Its `id` is the job ID `generate` returned and `status` takes.
#[derive(Debug, Clone, PartialEq)]
pub struct PastGeneration {
    pub id: String,
    /// "video", "image", "audio" or "3d".
    pub kind: String,
    pub status: String,
    /// The server's model ID.
    pub model: String,
    pub created_at: OffsetDateTime,
    pub is_liked: bool,
    /// `results.rawUrl`; `None` until the generation finished.
    pub output_url: Option<Url>,
    /// The output's size, not the source's.
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// The output's length in seconds.
    pub duration: Option<f64>,
    /// What the user typed; the server's rewrite (`enhanced_prompt`) is not kept.
    pub prompt: String,
    pub resolution: Option<String>,
    pub media: Vec<PastMedia>,
    /// `params.draft`: Seedance 2.5's native draft, finalizable at 1080p within seven days.
    pub is_draft: bool,
    /// `params.draft_job_id`: the draft this generation finalized.
    pub draft_job_id: Option<String>,
    /// `params.mode` as the generation was sent; `None` when it had none.
    pub mode: Option<String>,
}

/// An input of a past generation, as uploaded.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PastMedia {
    /// `None` for a role `VideoRequest` has no use for.
    pub role: Option<MediaRole>,
    /// What `VideoMedia` takes, while the server still keeps the file.
    pub id: String,
    pub url: Option<Url>,
}

/// The Genjutsu galleries of `get_presets`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GenjutsuCategory {
    /// Motion Library examples picked by Higgsfield.
    Higgsfield,
    /// Community examples, most popular first.
    Trending,
    /// Community examples, newest first.
    New,
    /// Looks that redraw the user's own video.
    Styles,
}

impl GenjutsuCategory {
    #[must_use]
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Higgsfield => "genjutsu",
            Self::Trending => "genjutsu-trending",
            Self::New => "genjutsu-new",
            Self::Styles => "genjutsu-styles",
        }
    }
}

/// One gallery entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenjutsuPreset {
    /// As `execute_preset` takes it: `genjutsu:<uuid>`, `genjutsu:trending:<uuid>`, `genjutsu-style:<uuid>`.
    pub id: String,
    pub name: String,
    pub thumbnail_url: Option<Url>,
    /// The example result, a short MP4.
    pub preview_url: Option<Url>,
    /// Also says what the example did; see `driving_video_url` and `example_model`.
    pub description: String,
}

impl GenjutsuPreset {
    /// The UUID that ends `id`.
    #[must_use]
    pub fn uuid(&self) -> Option<Uuid> {
        self.id
            .rsplit(':')
            .next()
            .and_then(|last| Uuid::parse_str(last).ok())
    }

    /// A motion entry's source clip, publicly downloadable. The server names it only inside the description:
    /// "… Driving video: https://….mp4. Use the user's …".
    pub fn driving_video_url(&self) -> Option<Url> {
        let start = self.description.find("Driving video: ")? + "Driving video: ".len();
        let rest = &self.description[start..];
        let link = rest
            .split(char::is_whitespace)
            .next()
            .unwrap_or("")
            .trim_end_matches('.');
        json::server_media(link)
    }

    /// What the example made from its driving video. The user's request decides the job's model, not this.
    #[must_use]
    pub fn example_model(&self) -> Option<&'static str> {
        if self.description.starts_with("Motion transfer") {
            return Some(super::VideoRequest::MOTION_CONTROL);
        }
        if self.description.starts_with("Object replacement") {
            return Some(super::VideoRequest::REPLACE_OBJECT);
        }
        None
    }
}

/// One page of a gallery.
#[derive(Debug, Clone, PartialEq)]
pub struct GenjutsuPresetPage {
    pub items: Vec<GenjutsuPreset>,
    /// Opaque: sent back unchanged for the next page. `None` on the last page.
    pub next_cursor: Option<Value>,
}

/// A Genjutsu Style run on uploaded files.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StyleRun {
    /// `genjutsu-style:<uuid>`.
    pub style_id: String,
    pub video_id: String,
    /// Up to 5 images of subjects to put into the result.
    pub image_ids: Vec<String>,
    /// Optional; empty is left out.
    pub prompt: String,
    pub resolution: String,
}

impl StyleRun {
    /// The `inputs` of `execute_preset`, and the `initial_inputs` of a priced `get_presets` detail.
    pub(crate) fn inputs(&self) -> Value {
        let mut inputs = object(json!({ "video": [self.video_id], "resolution": self.resolution }));
        if !self.image_ids.is_empty() {
            inputs.insert("references".into(), json!(self.image_ids));
        }
        if !self.prompt.is_empty() {
            inputs.insert("prompt".into(), self.prompt.clone().into());
        }
        Value::Object(inputs)
    }
}

/// The `items` of a page and its `next_cursor`; `None` without an `items` array.
fn page<T>(value: &Value, item: impl Fn(&Value) -> Option<T>) -> Option<(Vec<T>, Option<Value>)> {
    let items = json::array(value, "items")?.iter().filter_map(item).collect();
    Some((items, json::cursor(value, "next_cursor")))
}

impl CreditTransactionPage {
    /// `None` when there is no `items` array. Items without a time or amount are left out.
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        let (items, next_cursor) = page(value, CreditTransaction::from_value)?;
        Some(Self { items, next_cursor })
    }
}

impl CreditTransaction {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            date: json::string(value, "created_at").and_then(iso_date)?,
            credits: json::num(value, "credits")?,
            action: json::string(value, "action").unwrap_or("").to_owned(),
            title: json::string(value, "display_name").unwrap_or("").to_owned(),
        })
    }
}

/// "2026-10-14T12:33:58.452235Z", with or without its fraction.
pub(crate) fn iso_date(text: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(text, &Iso8601::DEFAULT).ok()
}

impl AccountAssetPage {
    /// `None` when there is no `items` array. Items without an ID, kind or URL are left out.
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        let (items, next_cursor) = page(value, AccountAsset::from_value)?;
        Some(Self { items, next_cursor })
    }
}

impl AccountAsset {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: json::string(value, "id")?.to_owned(),
            kind: AssetKind::from_raw_value(json::string(value, "kind")?)?,
            url: json::media_url(value, "url")?,
            thumbnail_url: json::media_url(value, "thumbnailUrl"),
            width: json::num(value, "width").and_then(json::count),
            height: json::num(value, "height").and_then(json::count),
            duration: json::num(value, "duration"),
            created_at: unix_date(json::num(value, "createdAt")),
        })
    }
}

impl GenerationPage {
    /// `None` when there is no `items` array. Items without an ID are left out.
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        let (items, next_cursor) = page(value, PastGeneration::from_value)?;
        Some(Self { items, next_cursor })
    }
}

impl PastGeneration {
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        let id = json::string(value, "id")?.to_owned();
        let params = json::get(value, "params");
        let param = |key: &str| params.and_then(|params| json::get(params, key));
        let text = |key: &str| param(key).and_then(Value::as_str).map(str::to_owned);
        let number = |key: &str| param(key).and_then(Value::as_f64);
        Some(Self {
            id,
            kind: json::string(value, "type").unwrap_or("").to_owned(),
            status: json::string(value, "status").unwrap_or("").to_owned(),
            model: json::string(value, "model").unwrap_or("").to_owned(),
            created_at: unix_date(json::num(value, "createdAt")),
            is_liked: json::is_bool(value, "liked", true),
            output_url: json::get(value, "results").and_then(|results| json::media_url(results, "rawUrl")),
            width: number("width").and_then(json::count),
            height: number("height").and_then(json::count),
            duration: number("duration"),
            prompt: text("prompt").unwrap_or_default(),
            resolution: text("resolution"),
            media: param("medias")
                .and_then(Value::as_array)
                .map(|medias| medias.iter().filter_map(PastMedia::from_value).collect())
                .unwrap_or_default(),
            is_draft: param("draft") == Some(&Value::Bool(true)),
            draft_job_id: text("draft_job_id"),
            mode: text("mode"),
        })
    }
}

impl PastMedia {
    fn from_value(value: &Value) -> Option<Self> {
        let data = json::get(value, "data")?;
        Some(Self {
            role: json::string(value, "role").and_then(MediaRole::from_history_role),
            id: json::string(data, "id")?.to_owned(),
            url: json::media_url(data, "url"),
        })
    }
}

impl GenjutsuPresetPage {
    /// `None` when there is no `items` array. Items without an ID or name are left out.
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        let (items, next_cursor) = page(value, GenjutsuPreset::from_value)?;
        Some(Self { items, next_cursor })
    }
}

impl GenjutsuPreset {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: json::string(value, "id")?.to_owned(),
            name: json::string(value, "name")?.to_owned(),
            thumbnail_url: json::media_url(value, "thumbnail_url"),
            preview_url: json::media_url(value, "preview_url"),
            description: json::string(value, "description").unwrap_or("").to_owned(),
        })
    }
}

/// The price a `get_presets` detail gives for its `initial_inputs`. The server puts it in the text answer only
/// ("`cost_credits`: 57.5"), and leaves it out when the inputs aren't ready.
pub(crate) fn preset_credits(result: &McpToolResult) -> Option<f64> {
    let structured = result
        .structured_content
        .as_ref()
        .and_then(|content| json::get(content, "execution"))
        .and_then(|execution| json::num(execution, "cost_credits"));
    if structured.is_some() {
        return structured;
    }
    let text = result.text();
    let start = text.find("cost_credits:")? + "cost_credits:".len();
    let number: String = text[start..]
        .trim_start_matches(' ')
        .chars()
        .take_while(|character| character.is_numeric() || *character == '.')
        .collect();
    number.parse().ok()
}

/// The first job ID an `execute_preset` answer names. A live Style run on 2026-10-15 answered `{source, preset_id,
/// job_ids: [id], status: "submitted"}`, so `job_ids[0]` is read first; the other forms the tool describes and
/// `generate_video`'s are still read after it.
pub(crate) fn preset_job_id(value: &Value) -> Option<String> {
    let first = |key: &str| json::array(value, key).and_then(|items| items.first());
    first("job_ids")
        .and_then(Value::as_str)
        .or_else(|| first("job_ids").and_then(|item| json::string(item, "id")))
        .or_else(|| first("jobs").and_then(|item| json::string(item, "id")))
        .or_else(|| first("results").and_then(|item| json::string(item, "id")))
        .or_else(|| json::string(value, "job_id"))
        .map(str::to_owned)
}

impl HiggsfieldAccount {
    /// A page of the account's history in its selected workspace, newest first. `kind` `None` lists every media
    /// type; `cursor` is `next_cursor` of the page before, as it came; `size` is 1–100.
    pub async fn generations(
        &self,
        kind: Option<GenerationType>,
        only_completed: bool,
        cursor: Option<&Value>,
        size: i64,
    ) -> Result<GenerationPage, HiggsfieldError> {
        let mut arguments =
            object(json!({ "size": clamped(size, 1, 100), "only_completed": only_completed }));
        if let Some(kind) = kind {
            arguments.insert("type".into(), kind.raw_value().into());
        }
        insert_cursor(&mut arguments, cursor);
        let value = self.call(ToolRole::Generations, arguments).await?;
        GenerationPage::from_value(&value).ok_or_else(|| unexpected(ToolRole::Generations))
    }

    /// Likes or unlikes a generation of the account; setting the state it already has is harmless.
    pub async fn set_liked(&self, generation_id: &str, liked: bool) -> Result<(), HiggsfieldError> {
        self.call(
            ToolRole::Like,
            object(json!({ "job_id": generation_id, "liked": liked })),
        )
        .await
        .map(drop)
    }

    /// A page of the account's credit movements, newest first. `size` is 1–100.
    pub async fn transactions(
        &self,
        cursor: Option<&Value>,
        size: i64,
    ) -> Result<CreditTransactionPage, HiggsfieldError> {
        let mut arguments = object(json!({ "size": clamped(size, 1, 100) }));
        insert_cursor(&mut arguments, cursor);
        let value = self.call(ToolRole::Transactions, arguments).await?;
        CreditTransactionPage::from_value(&value).ok_or_else(|| unexpected(ToolRole::Transactions))
    }

    /// A page of the files the account uploaded, newest first. `size` is 1–50.
    pub async fn uploads(
        &self,
        kind: AssetKind,
        cursor: Option<&Value>,
        size: i64,
    ) -> Result<AccountAssetPage, HiggsfieldError> {
        self.assets("uploads", kind, cursor, size).await
    }

    /// A page of `assets_picker_list` of `source` (`uploads`, `generations`, `liked`, `influencer`,
    /// `influencer-liked`), newest first.
    pub(crate) async fn assets(
        &self,
        source: &str,
        kind: AssetKind,
        cursor: Option<&Value>,
        size: i64,
    ) -> Result<AccountAssetPage, HiggsfieldError> {
        let mut arguments =
            object(json!({ "source": source, "kind": kind.raw_value(), "size": clamped(size, 1, 50) }));
        insert_cursor(&mut arguments, cursor);
        let value = self.call(ToolRole::Assets, arguments).await?;
        AccountAssetPage::from_value(&value).ok_or_else(|| unexpected(ToolRole::Assets))
    }

    /// A page of a Genjutsu gallery, in the server's order. `limit` is 1–50.
    pub async fn genjutsu_presets(
        &self,
        category: GenjutsuCategory,
        cursor: Option<&Value>,
        limit: i64,
    ) -> Result<GenjutsuPresetPage, HiggsfieldError> {
        let mut arguments = object(json!({
            "source": "genjutsu", "category": category.raw_value(), "limit": clamped(limit, 1, 50),
        }));
        insert_cursor(&mut arguments, cursor);
        let value = self.call(ToolRole::Presets, arguments).await?;
        GenjutsuPresetPage::from_value(&value).ok_or_else(|| unexpected(ToolRole::Presets))
    }

    /// The price in credits of `run`; `None` when the server gives none, as for inputs it doesn't accept. Opens the
    /// style's detail with the inputs filled in; submits nothing.
    pub async fn style_cost(&self, run: &StyleRun) -> Result<Option<f64>, HiggsfieldError> {
        let arguments = object(json!({
            "source": "genjutsu", "preset_id": run.style_id, "category": "all", "initial_inputs": run.inputs(),
        }));
        Ok(preset_credits(
            &self.call_result(ToolRole::Presets, arguments).await?,
        ))
    }

    /// Not idempotent, like `generate`: a reply that names no job and gives no reason is `Server { status: 200 }`,
    /// as it may have started a run.
    pub async fn run_style(&self, style: &StyleRun) -> Result<Generation, HiggsfieldError> {
        let arguments: Object =
            object(json!({ "source": "genjutsu", "preset_id": style.style_id, "inputs": style.inputs() }));
        let value = self.call(ToolRole::ExecutePreset, arguments).await?;
        let id = preset_job_id(&value).ok_or_else(|| no_job_started(&value))?;
        Ok(Generation::new(
            id,
            json::string(&value, "request_id").map(str::to_owned),
            None,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `params.mode` is kept, so a model with several modes on one server model comes back in its own.
    #[test]
    fn the_mode_sent_is_kept() {
        let edit =
            PastGeneration::from_value(&json!({"id": "job", "params": {"mode": "video-edit"}})).unwrap();
        assert_eq!(edit.mode.as_deref(), Some("video-edit"));
        let plain = PastGeneration::from_value(&json!({"id": "job", "params": {"prompt": "x"}})).unwrap();
        assert_eq!(plain.mode, None);
    }

    // `GenjutsuPresetTests`: entries as `get_presets` gave them on 2026-10-14, trimmed to the keys the client reads.

    fn motion() -> Value {
        json!({
            "id": "genjutsu:27182599-2cee-4b27-be1a-d13e9d713d37", "name": "Countryside Duo", "kind": "genjutsu-motion",
            "thumbnail_url": "https://cdn.higgsfield.ai/higgsfield_multiplier_video_explore_variant/54b6f52b.webp",
            "preview_url": "https://cdn.higgsfield.ai/genjutsu/preview/genjutsu-h264-600-v1/90d01517.mp4",
            "description": "Object replacement reference for /genjutsu. Driving video: https://d2ol7oe51mr4n9.cloudfront.net/content_user_id/baed0aeb.mp4. Use the user's reference image, never the catalog example's character.",
        })
    }

    fn trending() -> Value {
        json!({
            "id": "genjutsu:trending:846e6f7b-53c1-4132-8d0f-ca05bfa826c8", "name": "Studio Duo Exchange",
            "description": "Motion transfer reference for /genjutsu. Driving video: https://example.com/a.mp4. Use …",
        })
    }

    fn style() -> Value {
        json!({
            "id": "genjutsu-style:2b5b68c4-9253-4d79-9d05-81e7718d8d7a", "name": "Knitted Yarn World",
            "kind": "genjutsu-style",
            "thumbnail_url": "https://static-public-media.higgsfield.ai/restyle-presets/503426a7.webp",
            "preview_url": "https://static-public-media.higgsfield.ai/restyle-presets/5c2f5251.mp4",
            "description": "Genjutsu Style (Restyle): redraws the user's own video in this look.",
        })
    }

    fn run() -> StyleRun {
        StyleRun {
            style_id: "genjutsu-style:2b5b68c4-9253-4d79-9d05-81e7718d8d7a".into(),
            video_id: "video-1".into(),
            image_ids: vec!["image-1".into()],
            prompt: "warm light".into(),
            resolution: "720p".into(),
        }
    }

    #[test]
    fn motion_entry_names_its_driving_video_and_model() {
        let preset = GenjutsuPreset::from_value(&motion()).unwrap();
        let driving =
            Url::parse("https://d2ol7oe51mr4n9.cloudfront.net/content_user_id/baed0aeb.mp4").unwrap();
        assert_eq!(preset.driving_video_url(), Some(driving));
        assert_eq!(
            preset.example_model(),
            Some(crate::mcp::account::VideoRequest::REPLACE_OBJECT)
        );
        assert_eq!(
            preset.uuid(),
            Uuid::parse_str("27182599-2cee-4b27-be1a-d13e9d713d37").ok()
        );
        assert_eq!(
            std::path::Path::new(preset.preview_url.unwrap().path()).extension(),
            Some("mp4".as_ref())
        );
    }

    #[test]
    fn trending_ids_keep_their_uuid_at_the_end() {
        let preset = GenjutsuPreset::from_value(&trending()).unwrap();
        assert_eq!(
            preset.uuid(),
            Uuid::parse_str("846e6f7b-53c1-4132-8d0f-ca05bfa826c8").ok()
        );
        assert_eq!(
            preset.example_model(),
            Some(crate::mcp::account::VideoRequest::MOTION_CONTROL)
        );
        assert_eq!(preset.thumbnail_url, None);
    }

    #[test]
    fn style_has_no_driving_video() {
        let preset = GenjutsuPreset::from_value(&style()).unwrap();
        assert_eq!(preset.driving_video_url(), None);
        assert_eq!(preset.example_model(), None);
    }

    #[test]
    fn entries_without_id_or_name_are_left_out() {
        let page = GenjutsuPresetPage::from_value(
            &json!({"items": [style(), {"name": "No ID"}, {"id": "x"}], "next_cursor": null}),
        )
        .unwrap();
        let names: Vec<_> = page.items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["Knitted Yarn World"]);
        assert_eq!(page.next_cursor, None);
        assert_eq!(GenjutsuPresetPage::from_value(&json!({"presets": []})), None);
    }

    #[test]
    fn style_inputs_leave_out_what_is_empty() {
        let mut run = run();
        assert_eq!(
            run.inputs(),
            json!({"video": ["video-1"], "references": ["image-1"], "resolution": "720p", "prompt": "warm light"})
        );
        run.image_ids.clear();
        run.prompt.clear();
        assert_eq!(run.inputs(), json!({"video": ["video-1"], "resolution": "720p"}));
    }

    fn tool_result(text: &str) -> McpToolResult {
        McpToolResult::from_value(&serde_json::from_str(text).unwrap()).unwrap()
    }

    #[test]
    fn quote_is_read_from_structured_content_or_text() {
        let cases = [
            (
                r#"{"content":[{"type":"text","text":"readiness:\n  ready: true\n  issues: []\n  cost_credits: 57.5\nA bare preset"}]}"#,
                57.5,
            ),
            (
                r#"{"content":[{"type":"text","text":"x"}],"structuredContent":{"execution":{"cost_credits":12}}}"#,
                12.0,
            ),
            // A Style's priced detail in TOON, the price only in the text and nested under `execution` (the live
            // quote for 12 s at 720p on 2026-10-15 was 85.5).
            (
                r#"{"content":[{"type":"text","text":"preset:\n  id: style_1\n  name: Film Noir\ninputs[2]{name,required}:\n  video,true\n  resolution,false\nexecution:\n  tool: execute_preset\n  readiness:\n    ready: true\n    issues[0]:\n  cost_credits: 85.5\n  currency: credits"}],"structuredContent":{"preset":{"id":"style_1"}}}"#,
                85.5,
            ),
        ];
        for (text, credits) in cases {
            assert_eq!(preset_credits(&tool_result(text)), Some(credits), "{text}");
        }
    }

    #[test]
    fn no_quote_when_the_inputs_are_not_ready() {
        let text = r#"{"content":[{"type":"text","text":"readiness:\n  ready: false\nPrice unavailable."}]}"#;
        assert_eq!(preset_credits(&tool_result(text)), None);
    }

    #[test]
    fn every_described_job_shape_is_read() {
        for value in [
            json!({"job_ids": ["job-1"]}),
            json!({"job_ids": [{"id": "job-1"}]}),
            json!({"jobs": [{"id": "job-1"}]}),
            json!({"results": [{"id": "job-1"}]}),
            json!({"job_id": "job-1"}),
        ] {
            assert_eq!(preset_job_id(&value).as_deref(), Some("job-1"), "{value}");
        }
        // `StyleJobTests.executePresetsAnswerGivesItsFirstJobID`: `job_ids` comes before `job_id`.
        assert_eq!(
            preset_job_id(&json!({"job_id": "job-9", "job_ids": ["job-1", "job-2"]})).as_deref(),
            Some("job-1")
        );
    }
}
