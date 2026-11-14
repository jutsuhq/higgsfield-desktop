//! The Image side's service, against a fake provider: the glue between the window, the form and the generator. The
//! runs themselves are `generator_tests`'.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use jutsu_core::{
    Connectivity, Cost, Credential, GenerationProvider, ImageJobStatus, ImageModel, ImageProvider,
    ImageRequest, MediaPreparing, PortError, PreparedFile, ProviderError, ProviderKind, RemoteAsset,
    RemoteJob,
};
use time::UtcOffset;
use uuid::Uuid;

use super::*;
use crate::connection::MemoryVault;
use crate::provider::{PersistTokens, ProviderFactory, UnlimitedOptIn};
use crate::queue::MissingFfmpeg;
use crate::testing::{BalanceProvider, FakeServices, eventually};
use crate::{AppDirectories, AppEnvironment, PlatformServices};

struct AlwaysOnline;

impl Connectivity for AlwaysOnline {
    fn updates(&self) -> BoxStream<'static, bool> {
        futures::stream::once(async { true })
            .chain(futures::stream::pending())
            .boxed()
    }
}

struct Factory;

impl ProviderFactory for Factory {
    fn provider(&self, _: Credential, _: PersistTokens, _: UnlimitedOptIn) -> Arc<dyn GenerationProvider> {
        Arc::new(BalanceProvider::credits(0, None))
    }
}

/// Offers Soul 2 and answers nothing else; no request is sent in these tests.
struct Models;

fn offline() -> PortError {
    PortError::from(ProviderError::Network)
}

#[async_trait]
impl ImageProvider for Models {
    fn kind(&self) -> ProviderKind {
        ProviderKind::ApiKey
    }
    fn owner(&self) -> Option<String> {
        Some("key-1".to_owned())
    }
    async fn image_models(&self) -> Result<Vec<ImageModel>, PortError> {
        let mut model = ImageModel::new("soul", "Soul 2", vec!["1:1".to_owned(), "16:9".to_owned()])
            .with_variant_counts(vec![1, 4]);
        model.input = jutsu_core::ImageModelInput::Optional;
        Ok(vec![model])
    }
    async fn image_estimate(&self, _: &ImageRequest) -> Result<Cost, PortError> {
        Err(offline())
    }
    async fn upload(&self, _: PreparedFile) -> Result<RemoteAsset, PortError> {
        Err(offline())
    }
    async fn submit_images(&self, _: &ImageRequest, _: Uuid) -> Result<Vec<RemoteJob>, PortError> {
        Err(offline())
    }
    async fn image_status(&self, _: &RemoteJob) -> Result<ImageJobStatus, PortError> {
        Err(offline())
    }
    async fn cancel_images(&self, _: &RemoteJob) -> Result<(), PortError> {
        Ok(())
    }
}

async fn environment(folder: &tempfile::TempDir) -> Arc<AppEnvironment> {
    let media: Arc<dyn MediaPreparing> = Arc::new(MissingFfmpeg);
    AppEnvironment::open(
        AppDirectories {
            data_dir: folder.path().join("data"),
            default_output_dir: folder.path().join("Videos"),
        },
        PlatformServices {
            vault: Arc::new(MemoryVault::new()),
            connection: Arc::new(FakeServices::default()),
            providers: Arc::new(Factory),
            connectivity: Arc::new(AlwaysOnline),
            media: Some(media),
            utc_offset: UtcOffset::UTC,
        },
    )
    .await
    .unwrap()
}

/// The service with Soul 2 loaded.
async fn connected(environment: &AppEnvironment) -> Arc<ImageService> {
    let images = environment.images.clone();
    images.set_provider(Some(Arc::new(Models)));
    let watched = images.clone();
    eventually(move || watched.snapshot().model_id.is_some()).await;
    images
}

#[tokio::test]
async fn the_models_of_the_connection_fill_the_form() {
    let folder = tempfile::tempdir().unwrap();
    let environment = environment(&folder).await;
    let images = connected(&environment).await;
    let snapshot = images.snapshot();
    assert_eq!(snapshot.models.len(), 1);
    assert_eq!(snapshot.model_id.as_deref(), Some("soul"));
    assert_eq!(snapshot.variant_counts, vec![1, 4]);
    assert_eq!(snapshot.aspect_ratios, vec!["1:1".to_owned(), "16:9".to_owned()]);
    assert!(!snapshot.loading_models);
}

#[tokio::test]
async fn without_a_connection_generate_says_so() {
    let folder = tempfile::tempdir().unwrap();
    let environment = environment(&folder).await;
    let images = connected(&environment).await;
    images.edit(ImageEdit::SetPrompt {
        text: "A red fox".to_owned(),
    });
    // The form keeps its models while the provider is gone for a moment, as during a reconnect.
    images.locked().provider = None;
    assert_eq!(
        images.generate(false).await,
        ImageGenerateOutcome::Failed {
            notice: Notice {
                title: "Not connected".to_owned(),
                detail: "Connect Higgsfield again to generate images.".to_owned(),
            }
        }
    );
}

#[tokio::test]
async fn the_side_follows_the_switch_and_an_image_template() {
    let folder = tempfile::tempdir().unwrap();
    let environment = environment(&folder).await;
    let images = connected(&environment).await;
    assert_eq!(images.snapshot().side, CreateSide::Video);
    images.set_side(CreateSide::Image);
    assert_eq!(images.snapshot().side, CreateSide::Image);

    images.edit(ImageEdit::SetPrompt {
        text: "A knight".to_owned(),
    });
    assert_eq!(images.suggested_template_name(), "Image · A knight");
    let template = images.make_template("Knights");
    images.edit(ImageEdit::SetPrompt {
        text: "Something else".to_owned(),
    });
    images.set_side(CreateSide::Video);
    assert_eq!(images.apply_template(&template), None);
    let snapshot = images.snapshot();
    assert_eq!(snapshot.prompt, "A knight");
    assert_eq!(snapshot.side, CreateSide::Image);
}

#[tokio::test]
async fn a_drop_on_the_image_to_edit_row_takes_its_first_image() {
    let folder = tempfile::tempdir().unwrap();
    let environment = environment(&folder).await;
    let images = connected(&environment).await;
    let snapshot = images.edit(ImageEdit::DropInput {
        paths: vec!["/tmp/notes.txt".to_owned()],
    });
    assert_eq!(snapshot.input.and_then(|input| input.path), None);
    let snapshot = images.edit(ImageEdit::DropInput {
        paths: vec![
            "/tmp/clip.mov".to_owned(),
            "/tmp/face.png".to_owned(),
            "/tmp/other.jpg".to_owned(),
        ],
    });
    assert_eq!(
        snapshot.input.and_then(|input| input.name).as_deref(),
        Some("face.png")
    );
}

#[tokio::test]
async fn generate_for_references_opens_with_a_connection_and_picks_up_to_the_room() {
    let folder = tempfile::tempdir().unwrap();
    let environment = environment(&folder).await;
    let images = connected(&environment).await;
    let picks = images.reference_picks(2);
    assert!(picks.offered && picks.can_pick);
    assert_eq!(picks.detail, None);
    assert!(!images.reference_picks(0).can_pick);
    // An image that isn't among the results can't be picked.
    assert_eq!(images.toggle_reference_pick(Uuid::new_v4(), 2).picked.len(), 0);
}
