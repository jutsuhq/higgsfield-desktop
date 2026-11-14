//! The image form (`ReferenceGenerator.swift`'s form, `ImageDraft.swift`): a prompt, a model of the connection, a
//! frame, the variants and an optional image to edit; its price text and the daily cap's question.

use jutsu_core::{
    Budget, BudgetDecision, Cost, CostUnit, ImageModel, ImageModelInput, JobTemplate, LocalImage, Mode,
    PixelSize, ProviderKind, SpendCap, TemplateKind,
};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::create::Notice;
use crate::queue::QueueConfirmation;
use crate::queue::format;

/// Under the Create title on the Image side.
pub const IMAGE_TAGLINE: &str = "Make still images from a prompt. Every image is saved to Library › Images.";
/// Why there is no model to pick.
pub const NO_MODELS: &str = "This connection offers no image models";
/// An account's price that only the run tells (`PostOperationText.unknownPriceNotice`).
pub const UNKNOWN_PRICE_NOTICE: &str =
    "Price unknown — charged on completion. Higgsfield gives no quote for this before it runs.";

/// The Create switch label (`TemplateKind.title`).
pub fn kind_title(kind: TemplateKind) -> &'static str {
    match kind {
        TemplateKind::Video => "Video",
        TemplateKind::Image => "Image",
    }
}

/// The Image side of the Create draft, saved between launches beside the video draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageDraft {
    #[serde(default)]
    pub prompt: String,
    /// `None` until the models are known; then one the connection offers.
    #[serde(default, rename = "modelID")]
    pub model_id: Option<String>,
    /// The frame the user picked; `None` follows the model default.
    #[serde(default)]
    pub chosen_aspect_ratio: Option<String>,
    #[serde(default = "one")]
    pub count: i64,
    /// The image to edit, for a model that takes one.
    #[serde(default)]
    pub input_image: Option<LocalImage>,
}

const fn one() -> i64 {
    1
}

impl Default for ImageDraft {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            model_id: None,
            chosen_aspect_ratio: None,
            count: 1,
            input_image: None,
        }
    }
}

impl ImageDraft {
    /// Something the user put into the form: a prompt or an image to edit. The model, frame and count alone restore
    /// as a fresh form.
    pub fn has_inputs(&self) -> bool {
        !self.prompt.trim().is_empty() || self.input_image.is_some()
    }

    /// The local file the form points at; Clear Cache keeps it.
    pub fn local_files(&self) -> Vec<Url> {
        self.input_image
            .iter()
            .map(|image| image.file_url.clone())
            .filter(|url| url.scheme() == "file")
            .collect()
    }

    /// The draft with its file reopened by `reopen`, which returns where it is now or `None` when it can't be read
    /// any more. Files on the account (not file URLs) are kept as they are.
    pub fn reopening_files(&self, reopen: impl Fn(&LocalImage) -> Option<LocalImage>) -> Self {
        let mut draft = self.clone();
        draft.input_image = self.input_image.as_ref().and_then(|image| {
            if image.file_url.scheme() == "file" {
                reopen(image)
            } else {
                Some(image.clone())
            }
        });
        draft
    }
}

/// The form as it stands, before the input is uploaded; what a request is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRequestDraft {
    pub model: ImageModel,
    pub prompt: String,
    pub aspect_ratio: String,
    pub count: i64,
    /// The images to edit, in order.
    pub inputs: Vec<LocalImage>,
    /// An agent's name for the images; `None` for the person's own.
    pub title: Option<String>,
}

/// The image form. Without models (disconnected, or before they load) the last model choice is kept for when they
/// come back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageForm {
    pub prompt: String,
    model_id: Option<String>,
    /// The frame the user picked; `None` follows `default_aspect_ratio`.
    pub chosen_aspect_ratio: Option<String>,
    pub count: i64,
    /// The image to edit, for a model that takes one.
    pub input_image: Option<LocalImage>,
    models: Vec<ImageModel>,
    /// Why no model can be offered; `None` while loading or when there are models.
    unavailable_reason: Option<String>,
    /// `models` are the ones saved at the last load; Generate waits for the connection's own.
    models_are_saved: bool,
}

impl Default for ImageForm {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            model_id: None,
            chosen_aspect_ratio: None,
            count: 1,
            input_image: None,
            models: Vec::new(),
            unavailable_reason: None,
            models_are_saved: false,
        }
    }
}

impl ImageForm {
    pub fn model_id(&self) -> Option<&str> {
        self.model_id.as_deref()
    }

    pub fn models(&self) -> &[ImageModel] {
        &self.models
    }

    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }

    pub fn models_are_saved(&self) -> bool {
        self.models_are_saved
    }

    /// The connection's own list isn't in: never read, only the saved one showing, or its read failed. Not when the
    /// connection said it has none.
    pub fn needs_model_read(&self) -> bool {
        if self.models.is_empty() {
            self.unavailable_reason.as_deref() != Some(NO_MODELS)
        } else {
            self.models_are_saved
        }
    }

    /// What an agent is told beside the image models while the connection's own list isn't in, with what to do;
    /// `None` once it is, or when the connection has none.
    pub fn agent_models_note(&self) -> Option<String> {
        if !self.needs_model_read() {
            return None;
        }
        let state = if self.models_are_saved {
            "These are the image models saved at the last read; Jutsu Studio is reading the connection's own."
                .to_owned()
        } else if let Some(reason) = &self.unavailable_reason {
            format!(
                "Jutsu Studio couldn't read the image models: {}.",
                reason.trim_matches('.')
            )
        } else {
            "Jutsu Studio hasn't read this connection's image models yet.".to_owned()
        };
        Some(format!(
            "{state} Ask again in a moment; if the list stays empty, call show with page create, which reads it too, \
             then try again."
        ))
    }

    pub fn model(&self) -> Option<&ImageModel> {
        let id = self.model_id.as_deref()?;
        self.models.iter().find(|model| model.id == id)
    }

    /// The frame without a choice: the source clip's nearest, or 1:1 for Object Swap (the image is an object) and
    /// without a source.
    pub fn default_aspect_ratio(&self, source: Option<PixelSize>, mode: Mode) -> Option<String> {
        let model = self.model()?;
        if let Some(source) = source
            && mode != Mode::ObjectSwap
            && source.is_valid()
        {
            return model.nearest_aspect_ratio(source.width, source.height);
        }
        if model.aspect_ratios.iter().any(|ratio| ratio == "1:1") {
            Some("1:1".to_owned())
        } else {
            model.aspect_ratios.first().cloned()
        }
    }

    /// The chosen frame when the model takes it, else the default.
    pub fn aspect_ratio(&self, source: Option<PixelSize>, mode: Mode) -> Option<String> {
        if let (Some(chosen), Some(model)) = (&self.chosen_aspect_ratio, self.model())
            && model.aspect_ratios.contains(chosen)
        {
            return Some(chosen.clone());
        }
        self.default_aspect_ratio(source, mode)
    }

    /// Picks the model, keeping the count when it makes it, else its first.
    pub fn select_model(&mut self, id: &str) {
        let Some(model) = self.models.iter().find(|model| model.id == id) else {
            return;
        };
        if !model.variant_counts.contains(&self.count) {
            self.count = model.variant_counts.first().copied().unwrap_or(1);
        }
        self.model_id = Some(id.to_owned());
    }

    /// Takes the connection's models, keeping the chosen one when it is still offered. Without models the choice is
    /// kept, so a restored draft or a template applied before they load keeps its model.
    pub fn use_models(&mut self, models: Vec<ImageModel>, unavailable_reason: Option<String>) {
        self.models = models;
        self.models_are_saved = false;
        self.unavailable_reason = if self.models.is_empty() {
            Some(unavailable_reason.unwrap_or_else(|| NO_MODELS.to_owned()))
        } else {
            None
        };
        let Some(first) = self.models.first().map(|model| model.id.clone()) else {
            return;
        };
        let keeps = self
            .model_id
            .as_ref()
            .filter(|id| self.models.iter().any(|model| &&model.id == id))
            .cloned();
        self.select_model(&keeps.unwrap_or(first));
    }

    /// Shows the models saved at the last load until `use_models` hands over the connection's own; none saved keeps
    /// the current ones.
    pub fn use_saved_models(&mut self, saved: Vec<ImageModel>) {
        if saved.is_empty() {
            return;
        }
        self.use_models(saved, None);
        self.models_are_saved = true;
    }

    /// Whether the model sends `input_image`.
    pub fn sends_input(&self) -> bool {
        self.input_image.is_some()
            && self
                .model()
                .is_some_and(|model| model.input != ImageModelInput::None)
    }

    /// Why Generate is off; `None` when it can run.
    pub fn blocker(&self) -> Option<String> {
        let Some(model) = self.model() else {
            return Some(
                self.unavailable_reason
                    .clone()
                    .unwrap_or_else(|| NO_MODELS.to_owned()),
            );
        };
        if self.prompt.trim().is_empty() {
            return Some("Describe the image to make".to_owned());
        }
        if model.input == ImageModelInput::Required && self.input_image.is_none() {
            return Some(format!("{} edits an image: add one", model.title));
        }
        if self.models_are_saved {
            return Some("Checking the image models…".to_owned());
        }
        None
    }

    /// The request as it stands; `None` while Generate is off.
    pub fn draft(&self, source: Option<PixelSize>, mode: Mode) -> Option<ImageRequestDraft> {
        if self.blocker().is_some() {
            return None;
        }
        let model = self.model()?.clone();
        let aspect_ratio = self.aspect_ratio(source, mode)?;
        let inputs = if self.sends_input() {
            self.input_image.iter().cloned().collect()
        } else {
            Vec::new()
        };
        Some(ImageRequestDraft {
            model,
            prompt: self.prompt.clone(),
            aspect_ratio,
            count: self.count,
            inputs,
            title: None,
        })
    }

    /// What the price covers: everything the request is made of.
    pub fn price_key(&self, source: Option<PixelSize>, mode: Mode) -> String {
        let input = if self.sends_input() {
            self.input_image
                .as_ref()
                .map(|image| image.file_url.to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        [
            self.model_id.clone().unwrap_or_default(),
            self.prompt.clone(),
            self.aspect_ratio(source, mode).unwrap_or_default(),
            self.count.to_string(),
            input,
        ]
        .join("\u{1F}")
    }

    /// The model's list price for `count` images, an estimate.
    pub fn list_price(&self) -> Option<Cost> {
        let each = self.model()?.list_price.as_ref()?;
        let amount = each.amount() * Decimal::from(self.count);
        Some(match each.unit() {
            CostUnit::Usd => Cost::usd(amount, true),
            CostUnit::Credits => Cost::credits(amount, true),
        })
    }

    // MARK: Draft

    /// The form as a value, for the Create draft.
    pub fn snapshot(&self) -> ImageDraft {
        ImageDraft {
            prompt: self.prompt.clone(),
            model_id: self.model_id.clone(),
            chosen_aspect_ratio: self.chosen_aspect_ratio.clone(),
            count: self.count,
            input_image: self.input_image.clone(),
        }
    }

    /// Restores a saved form. The saved model is kept when it is offered, else the current one stays; the count is
    /// kept when that model makes it. Before the models are loaded the choice is kept as it is.
    pub fn restore(&mut self, draft: &ImageDraft) {
        self.prompt.clone_from(&draft.prompt);
        self.chosen_aspect_ratio.clone_from(&draft.chosen_aspect_ratio);
        self.input_image.clone_from(&draft.input_image);
        self.count = draft.count.max(1);
        let Some(first) = self.models.first().map(|model| model.id.clone()) else {
            self.model_id.clone_from(&draft.model_id);
            return;
        };
        let offered = draft
            .model_id
            .as_ref()
            .filter(|id| self.models.iter().any(|model| &&model.id == id))
            .cloned();
        let id = offered.or_else(|| self.model_id.clone()).unwrap_or(first);
        self.select_model(&id);
    }

    /// Fills the form from an image template. A model the connection doesn't offer runs on the current one, and the
    /// notice says so; the count is kept when that model makes it.
    pub fn apply_image_template(&mut self, template: &JobTemplate) -> Option<Notice> {
        let is_offered = self.models.is_empty()
            || self
                .models
                .iter()
                .any(|model| Some(&model.id) == template.image_model_id.as_ref());
        self.restore(&ImageDraft {
            prompt: template.prompt.clone(),
            model_id: template.image_model_id.clone(),
            chosen_aspect_ratio: template.image_aspect_ratio.clone(),
            count: template.image_count,
            input_image: template.references.first().cloned(),
        });
        if is_offered {
            return None;
        }
        let model = self.model()?;
        Some(Notice {
            title: format!("Running on {}", model.title),
            detail: format!(
                "\"{}\" uses an image model this connection doesn't offer, so it runs on {}.",
                template.name, model.title
            ),
        })
    }

    /// The form's inputs as an image template.
    pub fn make_image_template(&self, name: &str, id: Uuid, created_at: OffsetDateTime) -> JobTemplate {
        JobTemplate::image(
            id,
            name,
            created_at,
            self.model_id.clone(),
            self.model().map(|model| model.title.clone()),
            self.aspect_ratio(None, Mode::MotionTransfer),
            self.count,
            self.prompt.clone(),
            self.input_image.iter().cloned().collect(),
        )
    }
}

/// "$0.0032" or "$0.08": cents, or four decimals when cents would round the price; credits as elsewhere.
pub fn image_price(cost: &Cost) -> String {
    if cost.unit() != CostUnit::Usd {
        return format::cost_text(cost);
    }
    let amount = cost.amount();
    let cents = amount * Decimal::ONE_HUNDRED;
    let digits: u32 = if cents.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero) == cents {
        2
    } else {
        4
    };
    let rounded = amount.round_dp_with_strategy(digits, RoundingStrategy::MidpointAwayFromZero);
    format!("${rounded:.prec$}", prec = digits as usize)
}

/// "$0.0128 · Price from Higgsfield", "About $0.08 · Estimate".
pub fn price_text(cost: &Cost) -> String {
    if cost.is_unknown_price() {
        return UNKNOWN_PRICE_NOTICE.to_owned();
    }
    let amount = image_price(cost);
    if cost.is_estimate() {
        format!("About {amount} · Estimate")
    } else {
        format!("{amount} · Price from Higgsfield")
    }
}

/// The question to ask before Generate when the price takes today past the cap, or isn't known under one; `None` to
/// go ahead.
pub fn cap_confirmation(
    cost: Option<&Cost>,
    committed_today: Decimal,
    cap: Option<&SpendCap>,
) -> Option<QueueConfirmation> {
    let cap = cap?;
    let cap_text = format::amount(cap.amount, cap.unit);
    let Some(cost) = cost.filter(|cost| !cost.is_unknown_price()) else {
        return Some(QueueConfirmation {
            title: "Price not known yet".to_owned(),
            message: format!(
                "These images' price isn't known, so they may take today past your {cap_text} cap. Today is at {} so far.",
                format::amount(committed_today, cap.unit)
            ),
            confirm_title: "Generate anyway".to_owned(),
        });
    };
    let BudgetDecision::NeedsConfirmation { total } = Budget::allows(cost, committed_today, Some(cap)) else {
        return None;
    };
    Some(QueueConfirmation {
        title: "Over today's spend cap".to_owned(),
        message: format!(
            "These images add {}, bringing today to {} against your {cap_text} cap.",
            image_price(cost),
            format::amount(total, cap.unit)
        ),
        confirm_title: "Generate anyway".to_owned(),
    })
}

/// "Soul 2 · 4 images".
pub fn run_title(model: &str, count: i64) -> String {
    format!("{model} · {count} images")
}

/// Whether the account's image generation is confirmed in its tool catalog (`HiggsfieldMCPProvider.offersImageGeneration`).
pub const OFFERS_IMAGE_GENERATION: bool = true;

/// Why images can't be made on the connection now (`AppEnvironment.referenceGenerationBlocker`): an account whose
/// catalog isn't confirmed to offer image generation waits for it.
pub fn generation_blocker(kind: ProviderKind, offers_image_generation: bool) -> Option<&'static str> {
    (kind == ProviderKind::Account && !offers_image_generation)
        .then_some(crate::library::enhance::WAITING_FOR_CATALOG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_waits_for_its_catalog_to_offer_image_generation() {
        assert_eq!(
            generation_blocker(ProviderKind::Account, false),
            Some("Waiting for the account's tool catalog to be confirmed")
        );
        assert_eq!(generation_blocker(ProviderKind::Account, true), None);
        assert_eq!(generation_blocker(ProviderKind::ApiKey, false), None);
    }

    fn usd(text: &str, is_estimate: bool) -> Cost {
        Cost::usd(text.parse().unwrap(), is_estimate)
    }

    pub(crate) fn soul() -> ImageModel {
        let mut model = ImageModel::new(
            "soul",
            "Soul 2",
            ["1:1", "3:4", "9:16", "16:9"].map(str::to_owned).to_vec(),
        )
        .with_variant_counts(vec![1, 4]);
        model.list_price = Some(usd("0.0032", true));
        model
    }

    pub(crate) fn qwen() -> ImageModel {
        let mut model = ImageModel::new(
            "qwen",
            "Qwen Image 3 Edit",
            vec!["1:1".to_owned(), "21:9".to_owned()],
        );
        model.input = ImageModelInput::Required;
        model.list_price = Some(usd("0.04", true));
        model
    }

    fn input() -> LocalImage {
        LocalImage::new(Url::parse("file:///tmp/knight.png").unwrap())
    }

    const LANDSCAPE: PixelSize = PixelSize {
        width: 1920,
        height: 1080,
    };

    fn form() -> ImageForm {
        let mut form = ImageForm::default();
        form.use_models(vec![soul(), qwen()], None);
        form.prompt = "a knight in silver armor".to_owned();
        form
    }

    #[test]
    fn the_first_model_is_picked_and_the_count_fits_the_model() {
        let mut form = form();
        assert_eq!(form.model_id(), Some("soul"));
        form.count = 4;
        form.select_model("qwen");
        assert_eq!(form.count, 1);
        form.use_models(vec![qwen()], None);
        assert_eq!(form.model_id(), Some("qwen"));
        form.use_models(Vec::new(), Some("Waiting".to_owned()));
        assert!(form.model().is_none());
        assert_eq!(form.blocker().as_deref(), Some("Waiting"));
        // Without models, as before the connection is up at launch, the choice is kept for when they come back.
        form.use_models(vec![soul(), qwen()], None);
        assert_eq!(form.model_id(), Some("qwen"));
    }

    #[test]
    fn saved_models_show_until_the_connections_own_arrive_and_hold_generate_meanwhile() {
        let mut form = ImageForm {
            prompt: "a knight in silver armor".to_owned(),
            ..ImageForm::default()
        };
        form.use_saved_models(Vec::new());
        assert!(form.model().is_none());
        form.use_saved_models(vec![soul(), qwen()]);
        assert_eq!(form.model_id(), Some("soul"));
        assert!(form.models_are_saved());
        assert_eq!(form.blocker().as_deref(), Some("Checking the image models…"));
        form.use_models(vec![soul()], None);
        assert!(!form.models_are_saved());
        assert_eq!(form.blocker(), None);
    }

    #[test]
    fn the_frame_follows_the_source_except_for_object_swap() {
        let mut form = form();
        let portrait = PixelSize {
            width: 1080,
            height: 1920,
        };
        assert_eq!(
            form.aspect_ratio(Some(LANDSCAPE), Mode::MotionTransfer)
                .as_deref(),
            Some("16:9")
        );
        assert_eq!(
            form.aspect_ratio(Some(portrait), Mode::Restyle).as_deref(),
            Some("9:16")
        );
        assert_eq!(
            form.aspect_ratio(Some(LANDSCAPE), Mode::ObjectSwap).as_deref(),
            Some("1:1")
        );
        assert_eq!(
            form.aspect_ratio(None, Mode::MotionTransfer).as_deref(),
            Some("1:1")
        );
        form.chosen_aspect_ratio = Some("3:4".to_owned());
        assert_eq!(
            form.aspect_ratio(Some(LANDSCAPE), Mode::MotionTransfer)
                .as_deref(),
            Some("3:4")
        );
        form.select_model("qwen");
        assert_eq!(
            form.aspect_ratio(Some(LANDSCAPE), Mode::MotionTransfer)
                .as_deref(),
            Some("21:9")
        );
    }

    #[test]
    fn generate_needs_a_prompt_and_the_image_an_edit_model_takes() {
        let mut form = form();
        form.prompt = "  ".to_owned();
        assert_eq!(form.blocker().as_deref(), Some("Describe the image to make"));
        form.prompt = "add a cape".to_owned();
        form.select_model("qwen");
        assert_eq!(
            form.blocker().as_deref(),
            Some("Qwen Image 3 Edit edits an image: add one")
        );
        form.input_image = Some(input());
        assert_eq!(form.blocker(), None);
        assert!(form.sends_input());
        form.select_model("soul");
        assert!(!form.sends_input());
    }

    #[test]
    fn the_list_price_counts_every_image() {
        let mut form = form();
        form.count = 4;
        assert_eq!(form.list_price(), Some(usd("0.0128", true)));
    }

    #[test]
    fn the_price_text_keeps_small_amounts() {
        assert_eq!(price_text(&usd("0.0032", true)), "About $0.0032 · Estimate");
        assert_eq!(price_text(&usd("0.08", false)), "$0.08 · Price from Higgsfield");
        assert_eq!(
            price_text(&Cost::credits(Decimal::from(3), false)),
            "3 cr · Price from Higgsfield"
        );
        assert_eq!(price_text(&Cost::UNKNOWN_CREDITS), UNKNOWN_PRICE_NOTICE);
    }

    #[test]
    fn the_cap_asks_when_the_price_is_over_or_unknown() {
        let cap = SpendCap::usd(Decimal::ONE);
        let price = usd("0.08", true);
        assert_eq!(cap_confirmation(Some(&price), Decimal::ZERO, None), None);
        assert_eq!(cap_confirmation(Some(&price), Decimal::ZERO, Some(&cap)), None);
        let over = cap_confirmation(Some(&price), "0.95".parse().unwrap(), Some(&cap));
        assert_eq!(
            over.map(|over| over.message),
            Some("These images add $0.08, bringing today to $1.03 against your $1.00 cap.".to_owned())
        );
        assert_eq!(
            cap_confirmation(None, Decimal::ZERO, Some(&cap)).map(|question| question.title),
            Some("Price not known yet".to_owned())
        );
        assert!(
            cap_confirmation(
                Some(&Cost::UNKNOWN_CREDITS),
                Decimal::ZERO,
                Some(&SpendCap::credits(100))
            )
            .is_some()
        );
    }

    #[test]
    fn restore_keeps_the_saved_model_and_a_fitting_count() {
        let mut form = form();
        form.restore(&ImageDraft {
            prompt: "knight".to_owned(),
            model_id: Some("qwen".to_owned()),
            count: 4,
            ..ImageDraft::default()
        });
        // Qwen makes one image: the count falls back to what it makes.
        assert_eq!(form.model_id(), Some("qwen"));
        assert_eq!(form.count, 1);
        assert_eq!(form.prompt, "knight");

        form.restore(&ImageDraft {
            prompt: "knight".to_owned(),
            model_id: Some("soul".to_owned()),
            chosen_aspect_ratio: Some("9:16".to_owned()),
            count: 4,
            input_image: None,
        });
        assert_eq!(form.model_id(), Some("soul"));
        assert_eq!(form.count, 4);
        assert_eq!(
            form.aspect_ratio(None, Mode::MotionTransfer).as_deref(),
            Some("9:16")
        );
    }

    #[test]
    fn restore_before_the_models_arrive_keeps_the_choice() {
        let mut form = ImageForm::default();
        form.restore(&ImageDraft {
            prompt: "knight".to_owned(),
            model_id: Some("soul".to_owned()),
            count: 4,
            ..ImageDraft::default()
        });
        form.use_models(vec![qwen(), soul()], None);
        assert_eq!(form.model_id(), Some("soul"));
        assert_eq!(form.count, 4);

        let mut unknown = ImageForm::default();
        unknown.restore(&ImageDraft {
            prompt: "knight".to_owned(),
            model_id: Some("flux".to_owned()),
            ..ImageDraft::default()
        });
        unknown.use_models(vec![soul(), qwen()], None);
        assert_eq!(unknown.model_id(), Some("soul"));
    }

    fn template(
        name: &str,
        model: &str,
        title: Option<&str>,
        ratio: &str,
        count: i64,
        prompt: &str,
    ) -> JobTemplate {
        JobTemplate::image(
            Uuid::new_v4(),
            name,
            OffsetDateTime::UNIX_EPOCH,
            Some(model.to_owned()),
            title.map(str::to_owned),
            Some(ratio.to_owned()),
            count,
            prompt,
            Vec::new(),
        )
    }

    #[test]
    fn applying_an_image_template_fills_the_form() {
        let mut form = form();
        let mut cape = template("Cape", "qwen", Some("Qwen Image 3 Edit"), "21:9", 1, "add a cape");
        cape.references = vec![input()];
        assert_eq!(form.apply_image_template(&cape), None);
        assert_eq!(form.model_id(), Some("qwen"));
        assert_eq!(form.prompt, "add a cape");
        assert_eq!(form.chosen_aspect_ratio.as_deref(), Some("21:9"));
        assert_eq!(form.count, 1);
        assert_eq!(form.input_image, Some(input()));
        assert_eq!(form.blocker(), None);
    }

    /// The menu's New from Template can run before Create has loaded the models.
    #[test]
    fn an_image_template_applied_before_the_models_arrive_keeps_its_model() {
        let mut form = ImageForm::default();
        assert_eq!(
            form.apply_image_template(&template("Knight", "soul", None, "16:9", 4, "a knight")),
            None
        );
        form.use_models(vec![qwen(), soul()], None);
        assert_eq!(form.model_id(), Some("soul"));
        assert_eq!(form.count, 4);
        assert_eq!(form.prompt, "a knight");
    }

    #[test]
    fn applying_a_template_of_an_unoffered_model_keeps_the_current_one_with_a_notice() {
        let mut form = form();
        form.select_model("soul");
        // Flux isn't offered, and Soul doesn't make 2: the count stays at what it makes.
        let notice = form.apply_image_template(&template("Cape", "flux", Some("Flux"), "16:9", 2, "a cape"));
        assert_eq!(
            notice.map(|notice| notice.title),
            Some("Running on Soul 2".to_owned())
        );
        assert_eq!(form.model_id(), Some("soul"));
        assert_eq!(form.count, 1);
        assert_eq!(form.prompt, "a cape");
        assert_eq!(form.chosen_aspect_ratio.as_deref(), Some("16:9"));
    }

    #[test]
    fn an_image_template_round_trips_through_the_form() {
        let mut form = form();
        form.count = 4;
        form.chosen_aspect_ratio = Some("3:4".to_owned());
        let template = form.make_image_template("Knight", Uuid::new_v4(), OffsetDateTime::UNIX_EPOCH);
        assert_eq!(template.kind, TemplateKind::Image);
        assert_eq!(template.image_model_id.as_deref(), Some("soul"));
        assert_eq!(template.image_model_title.as_deref(), Some("Soul 2"));
        assert_eq!(template.image_aspect_ratio.as_deref(), Some("3:4"));
        assert_eq!(template.image_count, 4);

        let mut fresh = self::form();
        assert_eq!(fresh.apply_image_template(&template), None);
        assert_eq!(fresh.snapshot(), form.snapshot());
    }

    #[test]
    fn a_restored_form_makes_the_same_request() {
        let mut sheet = form();
        sheet.count = 4;
        sheet.chosen_aspect_ratio = Some("16:9".to_owned());
        let mut panel = ImageForm::default();
        panel.use_models(vec![soul(), qwen()], None);
        panel.restore(&sheet.snapshot());
        assert_eq!(panel.snapshot(), sheet.snapshot());
        let panel_draft = panel.draft(None, Mode::MotionTransfer).unwrap();
        assert_eq!(
            panel_draft,
            sheet.draft(Some(LANDSCAPE), Mode::MotionTransfer).unwrap()
        );
        assert_eq!(panel_draft.aspect_ratio, "16:9");
        assert_eq!(panel_draft.count, 4);
        assert_eq!(
            panel.price_key(None, Mode::MotionTransfer),
            sheet.price_key(Some(LANDSCAPE), Mode::MotionTransfer)
        );
    }

    #[test]
    fn a_draft_has_inputs_only_with_a_prompt_or_an_image() {
        assert!(!ImageDraft::default().has_inputs());
        assert!(
            !ImageDraft {
                model_id: Some("soul".to_owned()),
                prompt: "  ".to_owned(),
                ..ImageDraft::default()
            }
            .has_inputs()
        );
        let draft = ImageDraft {
            input_image: Some(input()),
            ..ImageDraft::default()
        };
        assert!(draft.has_inputs());
        assert_eq!(draft.local_files().len(), 1);
        assert_eq!(draft.reopening_files(|_| None).input_image, None);
    }
}
