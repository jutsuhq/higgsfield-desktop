//! Reference image requests and the account's image model catalog.

use std::collections::HashSet;

use async_trait::async_trait;
use higgsfield::mcp::account::{AccountImageModel, AccountImageRequest};
use jutsu_core::{
    Cost, ImageJobStatus, ImageModel, ImageModelInput, ImageProvider, ImageRequest, PortError, PreparedFile,
    ProviderKind, RemoteAsset, RemoteJob,
};
use uuid::Uuid;

use crate::mapping::{image_status, port, server_credits, validation};
use crate::mcp_provider::{HiggsfieldMcpProvider, not_cancellable, unexpected_upload};

/// The account's models offered, by family of ID fragments: Soul, Grok, Qwen, Nano Banana and GPT Image, as the
/// issue lists them, then Seedream and MiniMax (catalog of 2026-11-10: Seedream 5.5, MiniMax H3 Image). Each family
/// offers its newest model first, by the version its IDs name (`version`), as the account lists older ones before
/// newer (`gpt_image_2` before `gpt_image_2_6`, `grok_image` before `grok_image_2_0`). An account that lists none of
/// them offers its first models instead.
pub(crate) const CURATED: [&[&str]; 7] = [
    &["soul"],
    &["grok"],
    &["qwen"],
    &["nano_banana", "nano-banana"],
    &["gpt_image", "gpt-image"],
    &["seedream"],
    &["minimax"],
];
/// The newest model of each of the six families the catalog lists, and a second Soul.
pub(crate) const MAX_MODELS: usize = 7;
/// `generate_image`'s `count`.
pub(crate) const VARIANT_COUNTS: [i64; 4] = [1, 2, 3, 4];
/// The frame offered for a model that lists none (MiniMax H3 Image): the model's own default, sent as no
/// `aspect_ratio` at all, as its schema lists none to send.
pub(crate) const ANY_FRAME: &str = "auto";

/// Each model with the frames it lists, taking images to edit when it names a role for them, in the order the
/// account lists them. As many as its `medias` entry's `max`; where the catalog states none, up to
/// `ImageModel::INPUT_BOUND`, and `get_cost`, quoted before anything is sent, refuses more than the model takes. A
/// model that lists no frame isn't offered, except one of a curated family that takes images, offered with
/// `ANY_FRAME`; nor is a second model by the same name: Higgsfield lists Soul 2.0 as both `soul_2` and `soul_v2`.
pub(crate) fn models(listed: &[AccountImageModel]) -> Vec<ImageModel> {
    let mut names = HashSet::new();
    let usable: Vec<&AccountImageModel> = listed
        .iter()
        .filter(|model| {
            let framed = !model.aspect_ratios.is_empty() || (model.image_role.is_some() && is_curated(model));
            framed && names.insert(model.name.clone())
        })
        .collect();
    let ids = picked(&usable);
    let offered: Vec<&AccountImageModel> = if ids.is_empty() {
        usable.iter().take(MAX_MODELS).copied().collect()
    } else {
        usable
            .iter()
            .filter(|model| ids.contains(&model.id))
            .copied()
            .collect()
    };
    offered
        .into_iter()
        .map(|model| {
            let ratios = if model.aspect_ratios.is_empty() {
                vec![ANY_FRAME.to_owned()]
            } else {
                model.aspect_ratios.clone()
            };
            let mut image = ImageModel::new(model.id.clone(), model.name.clone(), ratios)
                .with_variant_counts(VARIANT_COUNTS.to_vec());
            image.input = if model.image_role.is_none() {
                ImageModelInput::None
            } else {
                ImageModelInput::Optional
            };
            image.max_inputs = model.max_images;
            image.unstated_input_limit =
                (model.image_role.is_some() && model.max_images.is_none()).then_some(ImageModel::INPUT_BOUND);
            image
        })
        .collect()
}

fn is_curated(model: &AccountImageModel) -> bool {
    let id = model.id.to_lowercase();
    CURATED
        .iter()
        .any(|fragments| fragments.iter().any(|fragment| id.contains(fragment)))
}

/// Takes the families' models in turns, newest first within a family, so the account's many Soul models and a
/// family's older versions don't crowd out the other families.
fn picked(usable: &[&AccountImageModel]) -> HashSet<String> {
    let families: Vec<Vec<String>> = CURATED
        .iter()
        .map(|fragments| {
            let mut members: Vec<(Vec<i64>, usize)> = usable
                .iter()
                .enumerate()
                .filter_map(|(index, model)| {
                    let id = model.id.to_lowercase();
                    let fragment = fragments.iter().find(|fragment| id.contains(*fragment))?;
                    Some((version(&id, fragment), index))
                })
                .collect();
            members.sort_by(|first, second| second.0.cmp(&first.0).then(first.1.cmp(&second.1)));
            members
                .into_iter()
                .map(|(_, index)| usable[index].id.clone())
                .collect()
        })
        .collect();
    let mut ids = HashSet::new();
    let turns = families.iter().map(Vec::len).max().unwrap_or(0);
    for turn in 0..turns {
        for family in &families {
            if turn < family.len() && ids.len() < MAX_MODELS {
                ids.insert(family[turn].clone());
            }
        }
    }
    ids
}

/// The version an ID names after its family's fragment: its first run of numbers, a leading "v" dropped.
/// "seedream_5_5" is [5, 5], "seedream_v5_pro" [5], "grok_image_2_0" [2, 0], "nano_banana_pro" none.
pub(crate) fn version(id: &str, fragment: &str) -> Vec<i64> {
    let Some(start) = id.find(fragment) else {
        return Vec::new();
    };
    let mut numbers = Vec::new();
    for part in id[start + fragment.len()..]
        .split(['_', '-', '.'])
        .filter(|part| !part.is_empty())
    {
        if let Ok(number) = part.strip_prefix('v').unwrap_or(part).parse::<i64>() {
            numbers.push(number);
        } else if !numbers.is_empty() {
            break;
        }
    }
    numbers
}

/// `model` is the model as `models_explore` describes it; needed with images, for their role, and with
/// `ANY_FRAME`, which a model that lists no frame doesn't get as `aspect_ratio`.
pub(crate) fn request(
    request: &ImageRequest,
    model: Option<&AccountImageModel>,
) -> Result<AccountImageRequest, PortError> {
    if !VARIANT_COUNTS.contains(&request.count) {
        return Err(validation(Some("count"), "Make 1 to 4 images."));
    }
    let ids = request
        .input_images
        .iter()
        .map(|asset| asset.id.clone().ok_or_else(unexpected_upload))
        .collect::<Result<Vec<_>, _>>()?;
    let lists_no_frame = model.is_some_and(|model| model.aspect_ratios.is_empty());
    let mut account_request = AccountImageRequest::new(
        request.model.clone(),
        request.prompt.clone(),
        (!lists_no_frame).then(|| request.aspect_ratio.clone()),
        request.count,
    );
    if ids.is_empty() {
        return Ok(account_request);
    }
    let Some(role) = model.and_then(|model| model.image_role.clone()) else {
        return Err(validation(None, "This model doesn't take an image."));
    };
    if let Some(max_images) = model.and_then(|model| model.max_images)
        && i64::try_from(ids.len()).unwrap_or(i64::MAX) > max_images
    {
        // The Swift string catalog has a one-image variant of this message.
        let noun = if max_images == 1 { "image" } else { "images" };
        return Err(validation(
            None,
            format!("This model takes at most {max_images} {noun}."),
        ));
    }
    account_request.image_ids = ids;
    account_request.image_role = role;
    Ok(account_request)
}

impl HiggsfieldMcpProvider {
    /// With images, asks `models_explore get` for the model's image role first, and with `ANY_FRAME` whether the
    /// model lists frames at all.
    async fn account_image_request(&self, image: &ImageRequest) -> Result<AccountImageRequest, PortError> {
        if image.input_images.is_empty() && image.aspect_ratio != ANY_FRAME {
            return request(image, None);
        }
        let model = self.account.image_model(&image.model).await.map_err(port)?;
        request(image, model.as_ref())
    }
}

#[async_trait]
impl ImageProvider for HiggsfieldMcpProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Account
    }

    fn owner(&self) -> Option<String> {
        self.owner.clone()
    }

    async fn image_models(&self) -> Result<Vec<ImageModel>, PortError> {
        self.account.check_image_tool().await.map_err(port)?;
        let listed = self.account.image_models().await.map_err(port)?;
        Ok(models(&listed.unwrap_or_default()))
    }

    /// `get_cost` for one image times `count`, exact; `Cost::UNKNOWN_CREDITS` when the answer names no price.
    /// `get_cost` ignores `count`: on 2026-10-15 GPT Image 2 quoted 0.5 credits for 1, 2 and 4 images alike, and
    /// each image of a request was billed 0.5.
    #[allow(clippy::cast_precision_loss, reason = "a count of 1 to 4")]
    async fn image_estimate(&self, image: &ImageRequest) -> Result<Cost, PortError> {
        let account_request = self.account_image_request(image).await?;
        let credits = self.account.image_cost(&account_request).await.map_err(port)?;
        match credits {
            Some(credits) => server_credits(credits * image.count as f64, false, 1),
            None => Ok(Cost::UNKNOWN_CREDITS),
        }
    }

    async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError> {
        jutsu_core::GenerationProvider::upload(self, file).await
    }

    /// Not idempotent: `generate_image` takes no key, like `generate_video`.
    async fn submit_images(
        &self,
        image: &ImageRequest,
        _idempotency_key: Uuid,
    ) -> Result<Vec<RemoteJob>, PortError> {
        let account_request = self.account_image_request(image).await?;
        let generations = self
            .account
            .generate_images(&account_request)
            .await
            .map_err(port)?;
        Ok(generations
            .into_iter()
            .map(|generation| {
                let mut job = RemoteJob::new(generation.id);
                job.correlation_id = generation.request_id;
                job
            })
            .collect())
    }

    async fn image_status(&self, job: &RemoteJob) -> Result<ImageJobStatus, PortError> {
        let status = self.account.image_status(&job.request_id).await.map_err(port)?;
        Ok(image_status(status))
    }

    async fn cancel_images(&self, _job: &RemoteJob) -> Result<(), PortError> {
        Err(not_cancellable())
    }
}
