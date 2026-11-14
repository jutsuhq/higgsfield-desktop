//! The Enhance menu and sheet on a result's page (`EnhanceMenu.swift`, `EnhanceSheet.swift`, `EnhanceOptions.swift`
//! and `PostOperations.swift`): when the menu shows, what each operation's job is, its options and how its price
//! reads, a price no quote exists for included.
//!
//! The menu's on-device items (Smooth Motion…, Upscale on This Mac…) come with the on-device engines; until then the
//! menu offers only the account's operations, as the macOS app does on a Mac whose engines are unavailable.

pub use jutsu_core::PostActionKind;
use jutsu_core::{
    ClosedRange, Cost, DubLanguage, Job, JobSpec, PixelSize, PostAction, PostOperation, ProviderKind,
    ReframeAspect, SourceClip, UpscaleEngine, UpscaleFrameRate, UpscaleOptions, UpscalePreset, UpscaleTarget,
    VoiceChoice,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::item::LibraryItem;
use crate::on_device::text::{self as on_device_text, OnDeviceOffer};
use crate::queue::format;

/// The menu's title.
pub const MENU_TITLE: &str = "Enhance";
/// Only Reframe has a cost preflight, and no public page states the others' rates, so the user agrees to an open
/// price.
pub const UNKNOWN_PRICE_NOTICE: &str =
    "Price unknown — charged on completion. Higgsfield gives no quote for this before it runs.";
/// An operation Higgsfield prices only once it has run (`PostAction::is_priced_after_run`, Topaz).
pub const PRICE_AFTER_RUN_NOTICE: &str = "Price shown after the run. Higgsfield gives no quote for a Topaz upscale; Jutsu reads what it charged from your credit history when it finishes.";
/// The box the user ticks to queue at an unknown price.
pub const UNKNOWN_PRICE_CONSENT: &str = "Charge whatever Higgsfield bills when it finishes";
pub const WAITING_FOR_CATALOG: &str = "Waiting for the account's tool catalog to be confirmed";
pub const NEEDS_ACCOUNT: &str = "Needs a Higgsfield account";
/// What Remove Background makes, as seen live on 2026-10-15: the subject on black in an MP4 without alpha, which
/// came without sound (the source's is put back, `PostAction::restores_source_sound`).
pub const REMOVE_BACKGROUND_NOTE: &str =
    "The subject stays on a black background, in an MP4 without transparency. The clip's sound is put back.";
/// Whether the account's operations are confirmed (`HiggsfieldMCPProvider.offersPostOperations`).
pub const OFFERS_POST_OPERATIONS: bool = true;

/// "About 12 credits · Estimate · charged on completion", "12 cr · Price from Higgsfield · charged on completion",
/// or the unknown price: shown after the run for `action` when Higgsfield prices it only then.
pub fn price_text(cost: &Cost, action: Option<&PostAction>) -> String {
    if *cost == Cost::UNKNOWN_CREDITS {
        return if action.is_some_and(PostAction::is_priced_after_run) {
            PRICE_AFTER_RUN_NOTICE
        } else {
            UNKNOWN_PRICE_NOTICE
        }
        .to_owned();
    }
    let amount = format::cost_text(cost);
    if cost.is_estimate() {
        format!("About {amount} · Estimate · charged on completion")
    } else {
        format!("{amount} · Price from Higgsfield · charged on completion")
    }
}

/// The sheet's title before its options are chosen.
pub fn kind_title(kind: PostActionKind) -> &'static str {
    match kind {
        PostActionKind::Upscale => "Upscale",
        PostActionKind::Reframe => "Reframe",
        PostActionKind::RemoveBackground => "Remove Background",
        PostActionKind::Dub => "Dub",
        PostActionKind::ChangeVoice => "Change Voice",
    }
}

/// "Upscaled from rooftop", for the result an operation made from `parent`.
pub fn made_from(action: &PostAction, parent: &str) -> String {
    match action {
        PostAction::Upscale(_) => format!("Upscaled from {parent}"),
        PostAction::RemoveBackground => format!("Background removed from {parent}"),
        PostAction::Reframe(_) => format!("Reframed from {parent}"),
        PostAction::Dub(_) => format!("Dubbed from {parent}"),
        PostAction::ChangeVoice(_) => format!("Voice changed from {parent}"),
    }
}

/// Every language, by its English name.
pub fn sorted_languages() -> Vec<DubLanguage> {
    let mut languages = DubLanguage::ALL.to_vec();
    languages.sort_by_key(|language| format::dub_language_title(*language));
    languages
}

/// "Yours" for a voice of the account (a reference element), "Built-in" for the provider's own.
pub fn voice_kind_label(voice: &VoiceChoice) -> &'static str {
    if voice.kind == "element" {
        "Yours"
    } else {
        "Built-in"
    }
}

pub fn target_title(target: UpscaleTarget) -> &'static str {
    match target {
        UpscaleTarget::P1080 => "1080p",
        UpscaleTarget::K2 => "2K",
        UpscaleTarget::K4 => "4K",
    }
}

/// "Keep", "24 fps".
pub fn frame_rate_title(rate: UpscaleFrameRate) -> String {
    match rate {
        UpscaleFrameRate::Keep => "Keep".to_owned(),
        rate => format!("{} fps", rate.raw_value()),
    }
}

pub fn preset_title(preset: UpscalePreset) -> &'static str {
    match preset {
        UpscalePreset::Aigc => "AI video",
        UpscalePreset::Common => "General",
        UpscalePreset::Ugc => "Phone footage",
        UpscalePreset::ShortSeries => "Short series",
        UpscalePreset::OldFilm => "Old film",
    }
}

pub fn engine_title(engine: UpscaleEngine) -> &'static str {
    match engine {
        UpscaleEngine::Standard => "Standard",
        UpscaleEngine::Topaz => "Topaz",
    }
}

/// The canvas Reframe makes around a result: the smallest frame of the new aspect ratio that holds the whole
/// result, which sits in its middle; the rest is filled in.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ReframeCanvas {
    /// The canvas's width over its height.
    pub width_over_height: f64,
    /// Where the result sits in the canvas, as fractions of the canvas's width and height.
    pub frame_x: f64,
    pub frame_y: f64,
    pub frame_width: f64,
    pub frame_height: f64,
}

impl ReframeCanvas {
    /// `None` for a result of unknown size.
    #[allow(clippy::cast_precision_loss, reason = "frame sizes are far below 2^52")]
    pub fn new(source: PixelSize, aspect: ReframeAspect) -> Option<Self> {
        if source.width <= 0 || source.height <= 0 {
            return None;
        }
        let source_ratio = source.width as f64 / source.height as f64;
        let target = aspect.width_over_height();
        let (frame_width, frame_height) = if target >= source_ratio {
            // Wider canvas: the result keeps its height, the sides are filled.
            (source_ratio / target, 1.0)
        } else {
            // Taller canvas: the result keeps its width, top and bottom are filled.
            (1.0, target / source_ratio)
        };
        Some(Self {
            width_over_height: target,
            frame_x: (1.0 - frame_width) / 2.0,
            frame_y: (1.0 - frame_height) / 2.0,
            frame_width,
            frame_height,
        })
    }

    /// Whether the result already has this aspect ratio, so nothing would be filled in.
    pub fn is_unchanged(&self) -> bool {
        (self.frame_width - 1.0).abs() < 0.01 && (self.frame_height - 1.0).abs() < 0.01
    }

    /// What the preview says under the canvas.
    pub fn caption(canvas: Option<&Self>) -> &'static str {
        match canvas {
            None => "Reading the result's size…",
            Some(canvas) if canvas.is_unchanged() => "The result already has this shape; pick another one.",
            Some(_) => {
                "The dashed outline is the new canvas. Higgsfield fills in the area around the result."
            }
        }
    }
}

/// Whether a result's page shows the Enhance menu, and whether its items can run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnhanceAvailability {
    Hidden,
    /// Shown with every item off, for the reason.
    Disabled(&'static str),
    Available,
}

impl EnhanceAvailability {
    /// - `connection`: the active connection's kind; `None` while disconnected.
    /// - `offered`: whether the account's operations are confirmed (`OFFERS_POST_OPERATIONS`).
    pub fn of(item: &LibraryItem, connection: Option<ProviderKind>, offered: bool) -> Self {
        let (Some(_), false, Some(connection)) = (item.output_file(), item.needs_import(), connection) else {
            return Self::Hidden;
        };
        match connection {
            // The REST API has neither operation: the menu only says what it takes, on a result an account made.
            ProviderKind::ApiKey if item.job.provider_kind == ProviderKind::Account => {
                Self::Disabled(NEEDS_ACCOUNT)
            }
            ProviderKind::ApiKey => Self::Hidden,
            ProviderKind::Account if offered => Self::Available,
            ProviderKind::Account => Self::Disabled(WAITING_FOR_CATALOG),
        }
    }
}

/// The spec of `action` on `job`'s result: its downloaded file as the source, whole. A result the signed-in account
/// (`owner`) made is named by its job ID, so nothing is uploaded; any other is uploaded. `None` without a downloaded
/// result.
/// - `duration`, `pixel_size`: the file's own, when probed just now; otherwise `output_duration` and
///   `output_pixel_size`.
pub fn operation_spec(
    job: &Job,
    action: PostAction,
    owner: Option<&str>,
    duration: Option<f64>,
    pixel_size: Option<PixelSize>,
) -> Option<JobSpec> {
    let file = job.output_file.as_ref().filter(|url| url.scheme() == "file")?;
    let length = duration.unwrap_or_else(|| job.output_duration()).max(0.0);
    let size = pixel_size
        .or_else(|| job.output_pixel_size())
        .unwrap_or(PixelSize::new(0, 0));
    let job_id =
        (job.provider_kind == ProviderKind::Account && job.owner.as_deref() == owner && owner.is_some())
            .then(|| job.remote.as_ref().map(|remote| remote.request_id.clone()))
            .flatten();
    let source = SourceClip::new(file.clone(), length, size, ClosedRange::new(0.0, length));
    let mut spec = JobSpec::new(
        job.spec.mode,
        job.spec.model.clone(),
        Some(source),
        job.spec.resolution,
    );
    spec.operation = Some(PostOperation {
        action,
        source_job_id: job_id,
    });
    Some(spec)
}

/// The options of one sheet as the window sends them back: every operation's, so switching keeps what was chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceChoice {
    pub kind: PostActionKind,
    pub target: UpscaleTarget,
    pub engine: UpscaleEngine,
    /// 24, 30 or 60.
    pub frame_rate: i32,
    pub preset: UpscalePreset,
    pub aspect: ReframeAspect,
    pub language: DubLanguage,
    /// `None` until Change Voice has a voice.
    pub voice: Option<VoiceChoice>,
}

impl EnhanceChoice {
    /// What the menu item opens with: `UpscaleOptions::default()`, 9:16, English and no voice.
    pub fn new(kind: PostActionKind) -> Self {
        let options = UpscaleOptions::default();
        Self {
            kind,
            target: options.target,
            engine: options.engine,
            frame_rate: 24,
            preset: options.preset,
            aspect: ReframeAspect::Portrait,
            language: DubLanguage::English,
            voice: None,
        }
    }

    /// Topaz has no 2K; its next target up is 4K.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        if !self.engine.targets().contains(&self.target) {
            self.target = UpscaleTarget::K4;
        }
        if UpscaleFrameRate::try_from(i64::from(self.frame_rate))
            .ok()
            .is_none_or(|rate| !UpscaleFrameRate::OFFERED.contains(&rate))
        {
            self.frame_rate = 24;
        }
        self
    }

    /// The operation; `None` until Change Voice has a voice.
    pub fn action(&self) -> Option<PostAction> {
        Some(match self.kind {
            PostActionKind::Upscale => PostAction::Upscale(UpscaleOptions {
                target: self.target,
                frame_rate: UpscaleFrameRate::try_from(i64::from(self.frame_rate))
                    .unwrap_or(UpscaleFrameRate::Fps24),
                engine: self.engine,
                preset: self.preset,
            }),
            PostActionKind::Reframe => PostAction::Reframe(self.aspect),
            PostActionKind::RemoveBackground => PostAction::RemoveBackground,
            PostActionKind::Dub => PostAction::Dub(self.language),
            PostActionKind::ChangeVoice => PostAction::ChangeVoice(self.voice.clone()?),
        })
    }
}

/// One choice of a picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceOption<T> {
    pub value: T,
    pub title: String,
}

fn options<T: Copy>(values: &[T], title: impl Fn(T) -> String) -> Vec<EnhanceOption<T>> {
    values
        .iter()
        .map(|value| EnhanceOption {
            value: *value,
            title: title(*value),
        })
        .collect()
}

/// The pickers of `choice`'s operation.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceOptions {
    pub engines: Vec<EnhanceOption<UpscaleEngine>>,
    pub targets: Vec<EnhanceOption<UpscaleTarget>>,
    /// Frame rate and Tuned for: the standard engine only.
    pub takes_frame_rate: bool,
    pub frame_rates: Vec<EnhanceOption<i32>>,
    pub presets: Vec<EnhanceOption<UpscalePreset>>,
    pub aspects: Vec<EnhanceOption<ReframeAspect>>,
    pub languages: Vec<EnhanceOption<DubLanguage>>,
}

impl EnhanceOptions {
    pub fn new(choice: &EnhanceChoice) -> Self {
        Self {
            engines: options(UpscaleEngine::ALL, |engine| engine_title(engine).to_owned()),
            targets: options(choice.engine.targets(), |target| target_title(target).to_owned()),
            takes_frame_rate: choice.engine.takes_frame_rate(),
            frame_rates: UpscaleFrameRate::OFFERED
                .iter()
                .map(|rate| EnhanceOption {
                    value: i32::try_from(rate.raw_value()).unwrap_or(24),
                    title: frame_rate_title(*rate),
                })
                .collect(),
            presets: options(UpscalePreset::ALL, |preset| preset_title(preset).to_owned()),
            aspects: options(ReframeAspect::ALL, |aspect| aspect.raw_value().to_owned()),
            languages: options(&sorted_languages(), |language| {
                format::dub_language_title(language).to_owned()
            }),
        }
    }
}

/// The sheet of one operation on one result.
#[allow(clippy::struct_excessive_bools, reason = "the wire's fields")]
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceSheet {
    /// The options as they apply: Topaz moved off 2K.
    pub choice: EnhanceChoice,
    /// "Upscale to 1080p", or the kind's title until Change Voice has a voice.
    pub title: String,
    /// The result's title.
    pub subtitle: String,
    pub options: EnhanceOptions,
    /// Reframe's canvas over the result's first frame; `None` while its size is unknown.
    pub canvas: Option<ReframeCanvas>,
    pub canvas_caption: String,
    /// "Reframe takes results up to a minute long."
    pub is_too_long_to_reframe: bool,
    /// The price line: the price, the unknown price's notice, or why there is none.
    pub price_text: String,
    /// The unknown price asks for agreement first.
    pub needs_consent: bool,
    /// The button's price tag, for a known price.
    pub price_tag: Option<String>,
    /// Queueable once the consent, if any, is given.
    pub can_queue: bool,
}

impl EnhanceSheet {
    /// - `price`: the provider's price for the options; `None` when it can't be had.
    /// - `size`, `duration`: the result file's, as probed.
    pub fn new(
        item: &LibraryItem,
        choice: EnhanceChoice,
        price: Option<&Cost>,
        size: Option<PixelSize>,
        duration: Option<f64>,
        is_connected: bool,
    ) -> Self {
        let choice = choice.normalized();
        let action = choice.action();
        let canvas = size.and_then(|size| ReframeCanvas::new(size, choice.aspect));
        let price_text = match (&action, price) {
            (None, _) => "Pick a voice to see the price.".to_owned(),
            (Some(action), Some(price)) => price_text(price, Some(action)),
            (Some(_), None) => "Higgsfield gave no price for this, so it can't be queued.".to_owned(),
        };
        let unchanged =
            choice.kind == PostActionKind::Reframe && canvas.is_some_and(|canvas| canvas.is_unchanged());
        let known = price.filter(|price| **price != Cost::UNKNOWN_CREDITS);
        Self {
            title: action
                .as_ref()
                .map_or_else(|| kind_title(choice.kind).to_owned(), format::post_action_title),
            subtitle: item.title(),
            options: EnhanceOptions::new(&choice),
            canvas_caption: ReframeCanvas::caption(canvas.as_ref()).to_owned(),
            canvas,
            is_too_long_to_reframe: duration.unwrap_or_else(|| item.job.output_duration())
                > ReframeAspect::MAX_SOURCE_LENGTH,
            price_text,
            needs_consent: action.is_some() && price.is_some_and(|price| *price == Cost::UNKNOWN_CREDITS),
            price_tag: known.map(format::cost_text),
            can_queue: action.is_some() && price.is_some() && is_connected && !unchanged,
            choice,
        }
    }
}

/// The Enhance menu of a result's page; `None` when it is hidden.
#[allow(
    clippy::struct_excessive_bools,
    reason = "the menu's items, each shown or not"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceMenu {
    /// Every Higgsfield item is off, for this reason.
    pub disabled_reason: Option<String>,
    pub hint: String,
    /// The whole button is off: Higgsfield's items are off and this computer offers nothing.
    pub is_disabled: bool,
    /// Higgsfield's items show (Upscale…, Reframe…, …).
    pub shows_remote: bool,
    /// Smooth Motion…, free on this computer.
    pub smooth_motion: bool,
    /// Upscale on This Computer… on its own, as Higgsfield's Upscale isn't on offer.
    pub upscale_on_device: bool,
    /// Upscale is a submenu: With Higgsfield… and On This Computer….
    pub upscale_submenu: bool,
    /// While a run on this computer waits or runs on the item, its items are off under this title: "Wait until
    /// Upscale 4× finishes".
    pub busy: Option<String>,
}

impl EnhanceMenu {
    /// Higgsfield's items only.
    pub fn new(availability: &EnhanceAvailability) -> Option<Self> {
        Self::with_local(availability, OnDeviceOffer::NONE, None)
    }

    /// With this computer's runs (`EnhanceMenu.swift`): shown when either side offers something; Upscale on This
    /// Computer… sits under one Upscale item beside Higgsfield's when both are on offer and nothing runs on the item.
    pub fn with_local(
        availability: &EnhanceAvailability,
        local: OnDeviceOffer,
        busy: Option<String>,
    ) -> Option<Self> {
        if *availability == EnhanceAvailability::Hidden && local.is_empty() {
            return None;
        }
        let reason = match availability {
            EnhanceAvailability::Disabled(reason) => Some((*reason).to_owned()),
            _ => None,
        };
        let hint = if *availability == EnhanceAvailability::Available {
            "Upscale, reframe, dub and more".to_owned()
        } else if local.is_empty() {
            reason.clone().unwrap_or_default()
        } else {
            on_device_text::menu_hint(local).to_owned()
        };
        Some(Self {
            is_disabled: reason.is_some() && local.is_empty(),
            shows_remote: *availability != EnhanceAvailability::Hidden,
            smooth_motion: local.smooth_motion,
            upscale_on_device: local.upscale_video && *availability != EnhanceAvailability::Available,
            upscale_submenu: reason.is_none()
                && *availability == EnhanceAvailability::Available
                && local.upscale_video
                && busy.is_none(),
            busy,
            disabled_reason: reason,
            hint,
        })
    }
}

/// "Upscaled from rooftop" on a result an operation made, opening the result it was made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MadeFromLink {
    pub title: String,
    /// The result it was made from; `None` once it was deleted.
    pub parent_id: Option<String>,
}

impl MadeFromLink {
    pub fn new(item: &LibraryItem, parent: Option<&LibraryItem>) -> Option<Self> {
        let action = &item.spec().operation.as_ref()?.action;
        let parent_id = item.job.parent_id?;
        let name = parent.map_or_else(|| "a deleted result".to_owned(), LibraryItem::source_name);
        Some(Self {
            title: made_from(action, &name),
            parent_id: parent.map(|_| parent_id.to_string()),
        })
    }
}

/// One voice Change Voice offers, with a sample to play.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceRow {
    /// "type/id".
    pub id: String,
    pub choice: VoiceChoice,
    /// "Yours" or "Built-in".
    pub kind_label: String,
    /// The sample's link on Higgsfield.
    pub preview_url: Option<String>,
}

impl VoiceRow {
    pub fn new(voice: &jutsu_core::AccountVoice) -> Self {
        Self {
            id: voice.id(),
            kind_label: voice_kind_label(&voice.choice).to_owned(),
            choice: voice.choice.clone(),
            preview_url: voice.preview_url.as_ref().map(Url::to_string),
        }
    }
}

#[cfg(test)]
mod tests {
    use jutsu_core::JobState;
    use rust_decimal::Decimal;

    use super::*;
    use crate::library::fixture::{ItemFixture, item, minutes_ago};
    use crate::queue::row::{JobRowModel, price_source};
    use crate::queue::service::queued_notice;

    fn account_result() -> LibraryItem {
        let mut item = ItemFixture {
            via: ProviderKind::Account,
            ..ItemFixture::default()
        }
        .item();
        item.job.owner = Some("user_1".to_owned());
        item
    }

    fn voice(kind: &str, name: &str) -> VoiceChoice {
        VoiceChoice {
            id: "v-1".to_owned(),
            kind: kind.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn the_menu_waits_for_the_catalog_on_the_account() {
        assert_eq!(
            EnhanceAvailability::of(&account_result(), Some(ProviderKind::Account), false),
            EnhanceAvailability::Disabled(WAITING_FOR_CATALOG)
        );
        assert_eq!(
            EnhanceAvailability::of(&account_result(), Some(ProviderKind::Account), true),
            EnhanceAvailability::Available
        );
        // A result made with a key runs too, from an upload.
        assert_eq!(
            EnhanceAvailability::of(&item(), Some(ProviderKind::Account), true),
            EnhanceAvailability::Available
        );
    }

    #[test]
    fn an_api_key_hides_it_or_says_it_needs_the_account() {
        assert_eq!(
            EnhanceAvailability::of(&item(), Some(ProviderKind::ApiKey), true),
            EnhanceAvailability::Hidden
        );
        assert_eq!(
            EnhanceAvailability::of(&account_result(), Some(ProviderKind::ApiKey), true),
            EnhanceAvailability::Disabled(NEEDS_ACCOUNT)
        );
    }

    #[test]
    fn without_a_downloaded_result_or_a_connection_there_is_no_menu() {
        let mut remote = account_result();
        remote.job.output_file = Some(Url::parse("https://cdn.example/out.mp4").unwrap());
        assert_eq!(
            EnhanceAvailability::of(&remote, Some(ProviderKind::Account), true),
            EnhanceAvailability::Hidden
        );
        assert_eq!(
            EnhanceAvailability::of(&account_result(), None, true),
            EnhanceAvailability::Hidden
        );
    }

    #[test]
    fn the_signed_in_accounts_result_is_named_by_its_job_id() {
        let job = account_result().job;
        let spec = operation_spec(
            &job,
            PostAction::RemoveBackground,
            Some("user_1"),
            Some(6.4),
            Some(PixelSize::new(3, 2)),
        )
        .unwrap();
        assert_eq!(
            spec.operation,
            Some(PostOperation {
                action: PostAction::RemoveBackground,
                source_job_id: Some("req-123".to_owned()),
            })
        );
        let source = spec.source.as_ref().unwrap();
        assert_eq!(Some(&source.file_url), job.output_file.as_ref());
        assert_eq!(source.range, ClosedRange::new(0.0, 6.4));
        assert_eq!(source.pixel_size, PixelSize::new(3, 2));
        assert_eq!(spec.references, Vec::<jutsu_core::LocalImage>::new());
        assert_eq!(spec.prompt, "");
        assert!(!spec.is_draft);
    }

    #[test]
    fn another_accounts_or_a_keys_result_is_uploaded() {
        let other = operation_spec(
            &account_result().job,
            PostAction::RemoveBackground,
            Some("user_2"),
            None,
            None,
        )
        .unwrap();
        let key = operation_spec(&item().job, PostAction::RemoveBackground, None, None, None).unwrap();
        assert_eq!(other.operation.unwrap().source_job_id, None);
        assert_eq!(key.operation.unwrap().source_job_id, None);
    }

    fn upscale(target: UpscaleTarget, engine: UpscaleEngine) -> PostAction {
        PostAction::Upscale(UpscaleOptions {
            target,
            engine,
            ..UpscaleOptions::default()
        })
    }

    #[test]
    fn an_unknown_price_is_never_shown_as_a_number() {
        let mut job = account_result().job;
        job.spec = operation_spec(
            &job,
            upscale(UpscaleTarget::K4, UpscaleEngine::Standard),
            Some("user_1"),
            None,
            None,
        )
        .unwrap();
        job.cost = Cost::UNKNOWN_CREDITS;
        job.state = JobState::Processing;

        assert!(job.price_is_unknown());
        let row = JobRowModel::new(&job, minutes_ago(0.0), None);
        assert_eq!(row.cost, format::PRICE_UNKNOWN);
        assert_eq!(row.title, "out · Upscale to 4K");
        assert_eq!(price_source(&job), format::CHARGED_ON_COMPLETION);
        assert_eq!(
            queued_notice(std::slice::from_ref(&job)),
            Some(format!("Final queued · {}", format::PRICE_UNKNOWN))
        );
        assert!(
            LibraryItem::new(job.clone(), false)
                .billing_summary()
                .starts_with(format::PRICE_UNKNOWN)
        );
        assert_eq!(price_text(&Cost::UNKNOWN_CREDITS, None), UNKNOWN_PRICE_NOTICE);

        job.server_price = Some(Cost::credits(Decimal::from(14), false));
        assert!(!job.price_is_unknown());
        assert!(price_text(&Cost::credits(Decimal::from(14), false), None).starts_with("14 cr"));
    }

    /// A Topaz upscale has no quote (issue #113): the sheet says its price comes after the run, and once the charge
    /// is read from the credit history it shows as Higgsfield's price.
    #[test]
    fn a_topaz_upscale_is_priced_after_the_run() {
        let topaz = upscale(UpscaleTarget::K4, UpscaleEngine::Topaz);
        assert!(topaz.is_priced_after_run());
        assert!(!upscale(UpscaleTarget::K4, UpscaleEngine::Standard).is_priced_after_run());
        assert!(!PostAction::RemoveBackground.is_priced_after_run());
        assert_eq!(
            price_text(&Cost::UNKNOWN_CREDITS, Some(&topaz)),
            PRICE_AFTER_RUN_NOTICE
        );
        assert!(PRICE_AFTER_RUN_NOTICE.starts_with("Price shown after the run"));
        assert_eq!(
            price_text(&Cost::UNKNOWN_CREDITS, Some(&PostAction::RemoveBackground)),
            UNKNOWN_PRICE_NOTICE
        );

        let mut job = account_result().job;
        job.spec = operation_spec(&job, topaz, Some("user_1"), None, None).unwrap();
        job.cost = Cost::UNKNOWN_CREDITS;
        job.state = JobState::Processing;
        assert_eq!(
            queued_notice(std::slice::from_ref(&job)),
            Some(format!("Final queued · {}", format::PRICE_AFTER_RUN))
        );
        assert_eq!(price_source(&job), format::CHARGED_ON_COMPLETION);

        job.state = JobState::Done;
        job.server_price = Some(Cost::credits(Decimal::from(41), false));
        assert!(!job.price_is_unknown());
        assert_ne!(price_source(&job), format::CHARGED_ON_COMPLETION);
        assert_eq!(
            queued_notice(std::slice::from_ref(&job)),
            Some(format!(
                "Final queued · {}",
                format::format_cost(&Cost::credits(Decimal::from(41), false), true)
            ))
        );
    }

    /// A generation on a model whose rate no page or quote states is unknown too, until the server prices it.
    #[test]
    fn a_generation_without_a_rate_is_unknown_until_quoted() {
        let mut job = account_result().job;
        job.cost = Cost::UNKNOWN_CREDITS;
        assert!(job.price_is_unknown());
        job.cost = Cost::UNKNOWN_USD;
        assert!(job.price_is_unknown());
        job.server_price = Some(Cost::usd(Decimal::new(15, 1), false));
        assert!(!job.price_is_unknown());
        job.cost = Cost::credits(Decimal::ONE, true);
        job.server_price = None;
        assert!(!job.price_is_unknown());
    }

    #[test]
    fn the_new_operations_name_themselves() {
        let amber = voice("element", "Amber");
        assert_eq!(
            format::post_action_title(&PostAction::Reframe(ReframeAspect::Portrait)),
            "Reframe to 9:16"
        );
        assert_eq!(
            format::post_action_title(&PostAction::ChangeVoice(amber.clone())),
            "Change Voice to Amber"
        );
        assert_eq!(
            crate::library::item::post_action_tag(&PostAction::Reframe(ReframeAspect::Square)),
            "Reframe"
        );
        assert_eq!(
            crate::library::item::post_action_tag(&PostAction::Dub(DubLanguage::French)),
            "Dub"
        );
        assert_eq!(
            crate::library::item::post_action_tag(&PostAction::ChangeVoice(amber.clone())),
            "Voice"
        );
        assert_eq!(
            made_from(&PostAction::Reframe(ReframeAspect::Landscape), "rooftop"),
            "Reframed from rooftop"
        );
        assert_eq!(
            made_from(&PostAction::Dub(DubLanguage::French), "rooftop"),
            "Dubbed from rooftop"
        );
        assert_eq!(
            made_from(&PostAction::ChangeVoice(amber), "rooftop"),
            "Voice changed from rooftop"
        );
        assert_eq!(kind_title(PostActionKind::ChangeVoice), "Change Voice");
        assert_eq!(voice_kind_label(&voice("element", "Mine")), "Yours");
        assert_eq!(voice_kind_label(&voice("preset", "Amber")), "Built-in");
    }

    /// Only English is ported, so the names are the English ones (Swift also checks German, which isn't offered).
    #[test]
    fn languages_are_named_and_sorted() {
        assert_eq!(format::dub_language_title(DubLanguage::French), "French");
        assert_eq!(format::dub_language_title(DubLanguage::Chinese), "Chinese");
        assert_eq!(format::dub_language_title(DubLanguage::Filipino), "Filipino");
        let sorted = sorted_languages();
        assert_eq!(sorted.first(), Some(&DubLanguage::Arabic));
        assert_eq!(sorted.last(), Some(&DubLanguage::Turkish));
        assert_eq!(sorted.len(), DubLanguage::ALL.len());
    }

    #[test]
    fn a_taller_canvas_keeps_the_width_and_fills_above_and_below() {
        let canvas = ReframeCanvas::new(PixelSize::new(1920, 1080), ReframeAspect::Portrait).unwrap();
        assert!((canvas.width_over_height - 9.0 / 16.0).abs() < 1e-12);
        assert!((canvas.frame_width - 1.0).abs() < 1e-12);
        assert!((canvas.frame_height - (9.0 / 16.0) / (16.0 / 9.0)).abs() < 1e-9);
        assert!(canvas.frame_x.abs() < 1e-12);
        assert!((canvas.frame_y - (1.0 - canvas.frame_height) / 2.0).abs() < 1e-9);
        assert!(!canvas.is_unchanged());
    }

    #[test]
    fn a_wider_canvas_keeps_the_height_and_fills_the_sides() {
        let canvas = ReframeCanvas::new(PixelSize::new(720, 1280), ReframeAspect::Square).unwrap();
        assert!((canvas.frame_height - 1.0).abs() < 1e-12);
        assert!((canvas.frame_width - 0.5625).abs() < 1e-9);
        assert!((canvas.frame_x - 0.21875).abs() < 1e-9);
        assert!(canvas.frame_y.abs() < 1e-12);
    }

    #[test]
    fn the_same_shape_changes_nothing_and_an_unknown_size_has_no_canvas() {
        assert!(
            ReframeCanvas::new(PixelSize::new(1080, 1080), ReframeAspect::Square)
                .unwrap()
                .is_unchanged()
        );
        assert_eq!(
            ReframeCanvas::new(PixelSize::new(0, 0), ReframeAspect::Square),
            None
        );
    }

    #[test]
    fn the_upscale_options_follow_the_engine() {
        assert_eq!(
            UpscaleEngine::Topaz.targets(),
            &[UpscaleTarget::P1080, UpscaleTarget::K4]
        );
        assert_eq!(UpscaleEngine::Standard.targets(), UpscaleTarget::ALL);
        assert!(!UpscaleEngine::Topaz.takes_frame_rate());
        assert!(!UpscaleFrameRate::OFFERED.contains(&UpscaleFrameRate::Keep));
        assert_eq!(UpscaleOptions::default().frame_rate, UpscaleFrameRate::Fps24);
        // Topaz has no 2K: the sheet moves to 4K.
        let mut choice = EnhanceChoice::new(PostActionKind::Upscale);
        choice.target = UpscaleTarget::K2;
        choice.engine = UpscaleEngine::Topaz;
        assert_eq!(choice.normalized().target, UpscaleTarget::K4);
    }

    #[test]
    fn the_result_says_what_it_was_made_from() {
        let mut upscaled = item();
        upscaled.job.spec.operation = Some(PostOperation {
            action: PostAction::Upscale(UpscaleOptions::default()),
            source_job_id: None,
        });
        assert_eq!(
            made_from(&PostAction::RemoveBackground, "rooftop"),
            "Background removed from rooftop"
        );
        assert_eq!(
            made_from(&PostAction::Upscale(UpscaleOptions::default()), "rooftop"),
            "Upscaled from rooftop"
        );
        assert_eq!(upscaled.mode_tag(), "Upscale");
        assert_eq!(upscaled.title(), "rooftop · Upscale to 1080p");

        let parent = item();
        upscaled.job.parent_id = Some(parent.id());
        assert_eq!(
            MadeFromLink::new(&upscaled, Some(&parent)),
            Some(MadeFromLink {
                title: "Upscaled from rooftop".to_owned(),
                parent_id: Some(parent.id().to_string()),
            })
        );
        assert_eq!(
            MadeFromLink::new(&upscaled, None).map(|link| (link.title, link.parent_id)),
            Some(("Upscaled from a deleted result".to_owned(), None))
        );
    }

    #[test]
    fn the_sheet_waits_for_a_voice_and_asks_before_an_unknown_price() {
        let item = account_result();
        let sheet = EnhanceSheet::new(
            &item,
            EnhanceChoice::new(PostActionKind::ChangeVoice),
            None,
            None,
            None,
            true,
        );
        assert_eq!(sheet.title, "Change Voice");
        assert_eq!(sheet.price_text, "Pick a voice to see the price.");
        assert!(!sheet.can_queue);

        let sheet = EnhanceSheet::new(
            &item,
            EnhanceChoice::new(PostActionKind::RemoveBackground),
            Some(&Cost::UNKNOWN_CREDITS),
            Some(PixelSize::new(1920, 1080)),
            Some(6.0),
            true,
        );
        assert!(sheet.needs_consent && sheet.can_queue);
        assert_eq!(sheet.price_tag, None);
        assert_eq!(sheet.price_text, UNKNOWN_PRICE_NOTICE);

        // A canvas of the result's own shape has nothing to fill in.
        let mut square = EnhanceChoice::new(PostActionKind::Reframe);
        square.aspect = ReframeAspect::Landscape;
        let sheet = EnhanceSheet::new(
            &item,
            square,
            Some(&Cost::credits(Decimal::from(6), false)),
            Some(PixelSize::new(1920, 1080)),
            Some(70.0),
            true,
        );
        assert!(!sheet.can_queue);
        assert!(sheet.is_too_long_to_reframe);
        assert_eq!(sheet.price_tag.as_deref(), Some("6 cr"));
        assert_eq!(sheet.options.aspects.len(), 3);
    }

    #[test]
    fn the_menu_offers_this_computer_s_runs_beside_or_instead_of_higgsfield_s() {
        let both = OnDeviceOffer {
            smooth_motion: true,
            upscale_video: true,
        };
        let menu = EnhanceMenu::with_local(&EnhanceAvailability::Available, both, None).unwrap();
        assert!(menu.upscale_submenu && menu.smooth_motion && menu.shows_remote);
        assert!(!menu.upscale_on_device && !menu.is_disabled);
        assert_eq!(menu.hint, "Upscale, reframe, dub and more");

        let busy = Some("Wait until Upscale 4× finishes".to_owned());
        let running = EnhanceMenu::with_local(&EnhanceAvailability::Available, both, busy.clone()).unwrap();
        assert!(!running.upscale_submenu);
        assert_eq!(running.busy, busy);

        let local = EnhanceMenu::with_local(&EnhanceAvailability::Hidden, both, None).unwrap();
        assert!(!local.shows_remote && local.upscale_on_device);
        assert_eq!(local.hint, "Smooth Motion and Upscale, free on this computer");

        let off = EnhanceMenu::with_local(
            &EnhanceAvailability::Disabled("Sign in first"),
            OnDeviceOffer::NONE,
            None,
        )
        .unwrap();
        assert!(off.is_disabled);
        assert_eq!(off.hint, "Sign in first");
        let smooth_only = OnDeviceOffer {
            smooth_motion: true,
            upscale_video: false,
        };
        let partly =
            EnhanceMenu::with_local(&EnhanceAvailability::Disabled("Sign in first"), smooth_only, None)
                .unwrap();
        assert!(!partly.is_disabled && !partly.upscale_on_device);
        assert_eq!(partly.disabled_reason.as_deref(), Some("Sign in first"));
        assert!(EnhanceMenu::with_local(&EnhanceAvailability::Hidden, OnDeviceOffer::NONE, None).is_none());
    }
}
