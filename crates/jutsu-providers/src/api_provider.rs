//! `GenerationProvider` for an API key: pays from the prepaid USD balance of the key's account.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::LazyLock;

use async_trait::async_trait;
use higgsfield::api::{
    GrokImageRequest, HiggsfieldApiClient, ModelEndpoint, QwenEditRequest, SoulImageRequest, SubmittedRequest,
};
use higgsfield::core::{HiggsfieldError, is_https};
use jutsu_core::{
    Balance, Capabilities, Cost, CostUnit, GenerationProvider, ImageJobStatus, ImageModel, ImageModelInput,
    ImageProvider, ImageRequest, JobSpec, MediaLinkKind, Mode, ModelConstraintEntry, ModelConstraintTable,
    ModelConstraints, ModelConstraintsSource, PortError, PreparedFile, Pricing, ProviderError, ProviderKind,
    RemoteAsset, RemoteInputs, RemoteJob, RemoteStatus, Resolution, StylePreset, VideoModel,
};
use rust_decimal::Decimal;
use serde::Serialize;
use url::Url;
use uuid::Uuid;

use crate::api_model::{self, ApiModel};
use crate::mapping::{image_status, port, pricing, remote_status, validation};
use crate::quote_cache::QuoteCache;

/// `GenerationProvider` for an API key: pays from the prepaid USD balance of the key's account.
pub struct HiggsfieldApiProvider {
    client: HiggsfieldApiClient,
    pricing: Option<Pricing>,
    /// `/estimate` quotes before upload (`quote_before_upload`), for as long as this connection lasts, keyed by the
    /// job with its prompt left out.
    quotes: QuoteCache<String, Decimal, HiggsfieldError>,
}

/// The most a linked input may have: Restyle takes at most 200 MiB.
const MAX_LINKED_BYTES: i64 = 200 * 1024 * 1024;

impl HiggsfieldApiProvider {
    /// `pricing` defaults to the bundled `pricing.json`.
    pub fn new(client: HiggsfieldApiClient, pricing: Option<Pricing>) -> Self {
        Self {
            client,
            pricing,
            quotes: QuoteCache::default(),
        }
    }

    /// The client the provider calls.
    pub fn client(&self) -> &HiggsfieldApiClient {
        &self.client
    }

    /// Higgsfield's free `POST /estimate/{endpoint}` quote of a job without a source or images (Seedance from a
    /// prompt), whose request is complete before anything is uploaded. Jobs that differ only in their prompt share
    /// a quote. `None` for a job with media or without a prompt, and when the quote fails: the list price stands
    /// then.
    async fn quote_before_upload(&self, spec: &JobSpec) -> Option<Cost> {
        if spec.source.is_some() || !spec.references.is_empty() || spec.prompt.trim().is_empty() {
            return None;
        }
        let empty = RemoteInputs {
            video: None,
            images: Vec::new(),
            audio: None,
        };
        let request = ApiModel::new(spec, &empty).ok()?;
        let mut key = spec.clone();
        key.prompt = String::new();
        let key = serde_json::to_string(&key).ok()?;
        let client = self.client.clone();
        let (endpoint, body) = (request.endpoint, request.body);
        let usd = self
            .quotes
            .value(key, move || async move { client.estimate(endpoint, &body).await })
            .await
            .ok()?;
        Some(Cost::usd(usd, false))
    }

    /// The REST API makes one generation per request; variants (`JobSpec::variants`) are the account's `count`.
    /// Refused before anything is sent.
    fn check_variants(spec: &JobSpec) -> Result<(), PortError> {
        if spec.variants == 1 {
            Ok(())
        } else {
            Err(validation(Some("count"), "Variants need a Higgsfield account."))
        }
    }

    async fn submit_body(
        &self,
        endpoint: ModelEndpoint,
        body: &(impl Serialize + Sync),
        idempotency_key: Uuid,
    ) -> Result<SubmittedRequest, PortError> {
        self.client
            .submit(endpoint, body, idempotency_key)
            .await
            .map_err(port)
    }
}

impl std::fmt::Debug for HiggsfieldApiProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HiggsfieldApiProvider")
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl GenerationProvider for HiggsfieldApiProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::ApiKey
    }

    fn owner(&self) -> Option<String> {
        Some(self.client.key_id().to_owned())
    }

    /// Every model with an endpoint, each in the modes it has one for (`VideoModel::runs_with_api_key_in`).
    async fn capabilities(&self) -> Result<Capabilities, PortError> {
        let models: BTreeSet<VideoModel> = VideoModel::all()
            .into_iter()
            .filter(VideoModel::runs_with_api_key)
            .collect();
        let mut unserved = BTreeMap::new();
        for model in &models {
            let unserved_modes: BTreeSet<Mode> = model
                .modes()
                .into_iter()
                .filter(|mode| !model.runs_with_api_key_in(*mode))
                .collect();
            if !unserved_modes.is_empty() {
                unserved.insert(model.clone(), unserved_modes);
            }
        }
        let mut capabilities = Capabilities::new(
            Mode::ALL.iter().copied().collect(),
            Resolution::ALL.iter().copied().collect(),
            CostUnit::Usd,
            None,
        );
        capabilities.models = models;
        capabilities.parameters = api_model::parameters();
        capabilities.unserved_modes = unserved;
        Ok(capabilities)
    }

    async fn presets(&self) -> Result<Vec<StylePreset>, PortError> {
        let presets = self.client.presets().await.map_err(port)?;
        Ok(presets
            .into_iter()
            .map(|preset| StylePreset {
                id: preset.id,
                name: preset.name,
                preview_url: preset.preview_url,
                preview_video_url: None,
            })
            .collect())
    }

    /// Higgsfield's quote for a job with nothing to upload (`quote_before_upload`), else the list price from
    /// pricing.json: `POST /estimate/{endpoint}` takes the body `submit` sends, so a job with media is quoted once
    /// they are uploaded (`server_estimate`).
    async fn estimate(&self, spec: &JobSpec) -> Result<Cost, PortError> {
        if spec.operation.is_some() {
            return Err(api_model::needs_account());
        }
        Self::check_variants(spec)?;
        if let Some(quoted) = self.quote_before_upload(spec).await {
            return Ok(quoted);
        }
        let bundled;
        let pricing = if let Some(pricing) = &self.pricing {
            pricing
        } else {
            bundled = Pricing::bundled().map_err(|error| pricing(&error))?;
            &bundled
        };
        pricing.cost(spec).map_err(|error| pricing_error(&error))
    }

    async fn server_estimate(
        &self,
        spec: &JobSpec,
        inputs: &RemoteInputs,
    ) -> Result<Option<Cost>, PortError> {
        Self::check_variants(spec)?;
        let request = ApiModel::new(spec, inputs)?;
        let usd = self
            .client
            .estimate(request.endpoint, &request.body)
            .await
            .map_err(port)?;
        Ok(Some(Cost::usd(usd, false)))
    }

    async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError> {
        upload(&self.client, &file).await
    }

    /// Every REST input is a public HTTPS URL, so a file that is online already is sent by its link. Restyle takes
    /// at most 200 MiB; a larger file is uploaded after the local conversion that brings it under that.
    async fn linked_asset(
        &self,
        origin: &Url,
        _kind: MediaLinkKind,
        byte_count: i64,
    ) -> Result<Option<RemoteAsset>, PortError> {
        if !is_https(origin) || byte_count <= 0 || byte_count > MAX_LINKED_BYTES {
            return Ok(None);
        }
        Ok(Some(RemoteAsset::new(origin.clone())))
    }

    async fn submit(
        &self,
        spec: &JobSpec,
        inputs: &RemoteInputs,
        idempotency_key: Uuid,
    ) -> Result<RemoteJob, PortError> {
        Self::check_variants(spec)?;
        let request = ApiModel::new(spec, inputs)?;
        let submitted = self
            .submit_body(request.endpoint, &request.body, idempotency_key)
            .await?;
        let mut job = remote_job(submitted.clone());
        job.initial_status = submitted.initial_status.map(remote_status);
        Ok(job)
    }

    /// Every submit sends the `Idempotency-Key` header.
    async fn submit_is_idempotent(&self, _spec: &JobSpec) -> Result<bool, PortError> {
        Ok(true)
    }

    async fn status(&self, job: &RemoteJob) -> Result<RemoteStatus, PortError> {
        let status = self
            .client
            .status(&job.request_id, job.status_url.as_ref())
            .await
            .map_err(port)?;
        Ok(remote_status(status))
    }

    async fn cancel(&self, job: &RemoteJob) -> Result<(), PortError> {
        self.client
            .cancel(&job.request_id, job.cancel_url.as_ref())
            .await
            .map_err(port)
    }

    // note: the public REST docs list no balance endpoint, so the UI shows none; read it here once one exists.
    async fn balance(&self) -> Result<Option<Balance>, PortError> {
        Ok(None)
    }
}

/// A price the bundled tariffs can't give.
fn pricing_error(error: &jutsu_core::PricingError) -> PortError {
    pricing(error)
}

fn remote_job(submitted: SubmittedRequest) -> RemoteJob {
    let mut job = RemoteJob::new(submitted.request_id);
    job.correlation_id = submitted.correlation_id;
    job.status_url = submitted.status_url;
    job.cancel_url = submitted.cancel_url;
    job
}

async fn upload(client: &HiggsfieldApiClient, file: &PreparedFile) -> Result<RemoteAsset, PortError> {
    let target = client.upload_target(&file.content_type).await.map_err(port)?;
    let path = file_path(&file.file_url)?;
    client
        .put(
            &path,
            &file.content_type,
            &target.upload_url,
            &target.upload_headers.clone().unwrap_or_default(),
        )
        .await
        .map_err(port)?;
    Ok(RemoteAsset::new(target.public_url))
}

/// The local path of a prepared file's `file:` URL.
pub(crate) fn file_path(url: &Url) -> Result<PathBuf, PortError> {
    url.to_file_path()
        .map_err(|()| PortError::Other(format!("{url} isn't a local file")))
}

/// The REST schemas' limits where they are tighter than the catalog's, which follow the account: Genjutsu Lite's
/// Motion Transfer takes 1–8 `image_urls` (its schema on console.higgsfield.ai, read 2026-11-11), the account up to
/// 10. Nothing is asked of the server.
static CONSTRAINTS: LazyLock<ModelConstraintTable> = LazyLock::new(|| {
    ModelConstraintTable::new(
        VideoModel::from_raw("genjutsuLite")
            .map(|model| ModelConstraintEntry {
                model,
                mode: Mode::MotionTransfer,
                constraints: ModelConstraints::new(None, None, None, Some(8)),
            })
            .into_iter()
            .collect(),
    )
});

#[async_trait]
impl ModelConstraintsSource for HiggsfieldApiProvider {
    async fn model_constraints(&self) -> Result<Option<ModelConstraintTable>, PortError> {
        Ok(Some(CONSTRAINTS.clone()))
    }
}

// MARK: Images

const SOUL_RATIOS: [&str; 7] = ["1:1", "3:4", "4:3", "2:3", "3:2", "9:16", "16:9"];
const GROK_RATIOS: [&str; 9] = ["1:1", "3:4", "4:3", "2:3", "3:2", "9:16", "16:9", "1:2", "2:1"];
const QWEN_RATIOS: [&str; 10] = [
    "1:1", "3:4", "4:3", "2:3", "3:2", "9:16", "16:9", "7:9", "9:7", "21:9",
];

fn ratios(list: &[&str]) -> Vec<String> {
    list.iter().map(|ratio| (*ratio).to_owned()).collect()
}

/// The image models an API key offers for reference images (issue #42): a short list whose request schemas are on
/// their docs.higgsfield.ai pages. List prices per image are open.higgsfield.ai's (2026-10-15), shown as estimates
/// until `/estimate` quotes the exact body.
pub(crate) static API_IMAGE_MODELS: LazyLock<Vec<ImageModel>> = LazyLock::new(|| {
    // $0.0032 an image at 720p, the default resolution; `batch_size` takes 1 or 4.
    let mut soul = ImageModel::new(ModelEndpoint::SoulV2.raw_value(), "Soul 2", ratios(&SOUL_RATIOS))
        .with_variant_counts(vec![1, 4]);
    soul.list_price = Some(Cost::usd(Decimal::new(32, 4), true));
    // $0.04 (1k, low) to $0.08 (2k, medium); the request is 1k medium, whose own rate isn't published, so the
    // estimate is the upper bound.
    let mut grok = ImageModel::new(
        ModelEndpoint::GrokImage2.raw_value(),
        "Grok Image 2",
        ratios(&GROK_RATIOS),
    );
    grok.input = ImageModelInput::Optional;
    grok.list_price = Some(Cost::usd(Decimal::new(8, 2), true));
    // $0.04 an image at 1k, the default resolution.
    let mut qwen = ImageModel::new(
        ModelEndpoint::QwenImage3Edit.raw_value(),
        "Qwen Image 3 Edit",
        ratios(&QWEN_RATIOS),
    );
    qwen.input = ImageModelInput::Required;
    qwen.list_price = Some(Cost::usd(Decimal::new(4, 2), true));
    vec![soul, grok, qwen]
});

/// The body of one image endpoint.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
enum ImageBody {
    Soul(SoulImageRequest),
    Grok(GrokImageRequest),
    Qwen(QwenEditRequest),
}

fn unknown_image_model() -> PortError {
    validation(Some("model"), "This image model can't run with an API key.")
}

/// The endpoint and body of `request`, checked against the model: a known model, one of its variant counts, a
/// prompt, and an input image where the model needs one.
fn image_body(request: &ImageRequest) -> Result<(ModelEndpoint, ImageBody), PortError> {
    let model = API_IMAGE_MODELS
        .iter()
        .find(|model| model.id == request.model)
        .ok_or_else(unknown_image_model)?;
    let endpoint = ModelEndpoint::from_raw_value(&request.model).ok_or_else(unknown_image_model)?;
    if !model.variant_counts.contains(&request.count) {
        return Err(validation(
            Some("batch_size"),
            format!("{} can't make that many images.", model.title),
        ));
    }
    if request.prompt.trim().is_empty() {
        return Err(validation(Some("prompt"), "Describe the image to make."));
    }
    let inputs: Vec<Url> = request
        .input_images
        .iter()
        .map(|image| image.url.clone())
        .collect();
    let body = match endpoint {
        ModelEndpoint::SoulV2 => ImageBody::Soul(SoulImageRequest {
            prompt: request.prompt.clone(),
            aspect_ratio: request.aspect_ratio.clone(),
            batch_size: request.count,
        }),
        ModelEndpoint::GrokImage2 => ImageBody::Grok(GrokImageRequest {
            prompt: request.prompt.clone(),
            aspect_ratio: request.aspect_ratio.clone(),
            image_urls: inputs,
        }),
        ModelEndpoint::QwenImage3Edit => {
            if inputs.is_empty() {
                return Err(validation(
                    Some("image_urls"),
                    format!("{} needs an image to edit.", model.title),
                ));
            }
            ImageBody::Qwen(QwenEditRequest {
                prompt: request.prompt.clone(),
                image_urls: inputs,
                aspect_ratio: request.aspect_ratio.clone(),
            })
        }
        _ => return Err(unknown_image_model()),
    };
    Ok((endpoint, body))
}

/// The list price of `request`: the model's per image times the images.
fn list_price(request: &ImageRequest) -> Option<Cost> {
    let model = API_IMAGE_MODELS.iter().find(|model| model.id == request.model)?;
    match model.list_price {
        Some(Cost::Usd { amount, .. }) => Some(Cost::usd(amount * Decimal::from(request.count), true)),
        _ => None,
    }
}

/// No answer, or one the client can't read: it says nothing about the request itself.
fn is_outage(error: &PortError) -> bool {
    matches!(
        error,
        PortError::Provider(ProviderError::Network | ProviderError::Server { .. })
    )
}

#[async_trait]
impl ImageProvider for HiggsfieldApiProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::ApiKey
    }

    fn owner(&self) -> Option<String> {
        Some(self.client.key_id().to_owned())
    }

    async fn image_models(&self) -> Result<Vec<ImageModel>, PortError> {
        Ok(API_IMAGE_MODELS.clone())
    }

    /// `/estimate` with the exact body; the list price, labelled an estimate, when the server can't answer now.
    async fn image_estimate(&self, request: &ImageRequest) -> Result<Cost, PortError> {
        let (endpoint, body) = image_body(request)?;
        match self.client.estimate(endpoint, &body).await.map_err(port) {
            Ok(usd) => Ok(Cost::usd(usd, false)),
            Err(error) if is_outage(&error) => list_price(request).ok_or(error),
            Err(error) => Err(error),
        }
    }

    async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError> {
        upload(&self.client, &file).await
    }

    async fn submit_images(
        &self,
        request: &ImageRequest,
        idempotency_key: Uuid,
    ) -> Result<Vec<RemoteJob>, PortError> {
        let (endpoint, body) = image_body(request)?;
        let submitted = self.submit_body(endpoint, &body, idempotency_key).await?;
        Ok(vec![remote_job(submitted)])
    }

    async fn image_status(&self, job: &RemoteJob) -> Result<ImageJobStatus, PortError> {
        let status = self
            .client
            .image_status(&job.request_id, job.status_url.as_ref())
            .await
            .map_err(port)?;
        Ok(image_status(status))
    }

    async fn cancel_images(&self, job: &RemoteJob) -> Result<(), PortError> {
        GenerationProvider::cancel(self, job).await
    }
}
