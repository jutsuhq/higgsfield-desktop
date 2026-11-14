//! The agent server end to end: the app's environment with a fake provider, the listener on a temporary socket, and
//! the helper's client running capabilities → quote → `create_video` → jobs.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use jutsu_agent::{
    AgentAmount, AgentBudgetUnit, AgentClient, AgentCommand, AgentCulling, AgentEndpoint, AgentEnhancement,
    AgentExportPreset, AgentExportStatus, AgentImageRequest, AgentRequest, AgentResponse, AgentSheet,
    AgentSubtitlesAction, AgentVerdict, AgentVideoRequest, CapabilitiesCommand, CreateImageCommand,
    CreateVideoCommand, EnhanceCommand, ErrorCode, ExportCommand, JobsCommand, LibraryGetCommand,
    LibraryMarkCommand, LibrarySearchCommand, QuoteCommand, ShowCommand, SubtitlesCommand,
};
use jutsu_core::{
    Balance, Capabilities, Connectivity, Cost, CostUnit, Credential, Culling, GenerationProvider,
    ImageJobStatus, ImageModel, ImageModelInput, ImageProvider, ImageRequest, JobSpec, Mode, PortError,
    PreparedFile, ProviderKind, RemoteAsset, RemoteInputs, RemoteJob, RemoteStatus, Resolution, StylePreset,
    VideoModel,
};
use rust_decimal::Decimal;
use time::UtcOffset;
use uuid::Uuid;

use super::app_id;
use super::preferences::AgentPreferences;
use super::runtime::AgentRuntime;
use super::service::{AgentService, AgentShowTarget, AgentWindow, WindowSheet};
use super::settings::AgentStatus;
use super::test_fakes::ImageFake;
use crate::connection::{ApiKeyForm, MemoryVault};
use crate::library::fixture::{catalog_image, video};
use crate::provider::{PersistTokens, ProviderFactory, UnlimitedOptIn};
use crate::queue::MissingFfmpeg;
use crate::testing::{BalanceProvider, FakeServices};
use crate::{AppDirectories, AppEnvironment, PlatformServices};

struct AlwaysOnline;

impl Connectivity for AlwaysOnline {
    fn updates(&self) -> BoxStream<'static, bool> {
        futures::stream::once(async { true })
            .chain(futures::stream::pending())
            .boxed()
    }
}

/// `BalanceProvider` that quotes $1.30 for every job; it sends nothing.
struct Priced(BalanceProvider);

#[async_trait::async_trait]
impl GenerationProvider for Priced {
    fn kind(&self) -> ProviderKind {
        self.0.kind()
    }
    fn owner(&self) -> Option<String> {
        self.0.owner()
    }
    async fn capabilities(&self) -> Result<Capabilities, PortError> {
        self.0.capabilities().await
    }
    async fn presets(&self) -> Result<Vec<StylePreset>, PortError> {
        self.0.presets().await
    }
    async fn estimate(&self, _spec: &JobSpec) -> Result<Cost, PortError> {
        Ok(Cost::usd(Decimal::new(130, 2), false))
    }
    async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError> {
        self.0.upload(file).await
    }
    async fn submit(&self, spec: &JobSpec, inputs: &RemoteInputs, key: Uuid) -> Result<RemoteJob, PortError> {
        self.0.submit(spec, inputs, key).await
    }
    async fn submit_is_idempotent(&self, spec: &JobSpec) -> Result<bool, PortError> {
        self.0.submit_is_idempotent(spec).await
    }
    async fn status(&self, job: &RemoteJob) -> Result<RemoteStatus, PortError> {
        self.0.status(job).await
    }
    async fn cancel(&self, job: &RemoteJob) -> Result<(), PortError> {
        self.0.cancel(job).await
    }
    async fn balance(&self) -> Result<Option<Balance>, PortError> {
        self.0.balance().await
    }
}

/// An API key whose provider offers every model and mode.
struct Factory;

impl ProviderFactory for Factory {
    fn provider(
        &self,
        _credential: Credential,
        _persist: PersistTokens,
        _unlimited_opt_in: UnlimitedOptIn,
    ) -> Arc<dyn GenerationProvider> {
        let mut provider = BalanceProvider::credits(1_000, None);
        provider.kind = ProviderKind::ApiKey;
        provider.owner = Some("kid".to_owned());
        let mut capabilities = Capabilities::new(
            [
                Mode::MotionTransfer,
                Mode::Restyle,
                Mode::ObjectSwap,
                Mode::Edit,
                Mode::Extend,
            ]
            .into(),
            BTreeSet::from([Resolution::P480, Resolution::P720, Resolution::P1080]),
            CostUnit::Usd,
            None,
        );
        capabilities.models = VideoModel::all().into_iter().collect();
        provider.capabilities = Ok(capabilities);
        Arc::new(Priced(provider))
    }
}

struct NoWindow;

impl AgentWindow for NoWindow {
    fn is_open(&self) -> bool {
        false
    }
    fn runs_in_tray(&self) -> bool {
        false
    }
    fn show(&self, _target: &AgentShowTarget) -> bool {
        false
    }
}

async fn send(client: &mut AgentClient, command: AgentCommand) -> AgentResponse {
    let mut request = AgentRequest::new(command);
    request.client = Some("test-agent".to_owned());
    client.send(&request).await.unwrap()
}

/// The app with an API key connected, in `folder`.
async fn connected_app(folder: &std::path::Path) -> Arc<AppEnvironment> {
    let environment = AppEnvironment::open(
        AppDirectories {
            data_dir: folder.join("data"),
            default_output_dir: folder.join("Videos"),
        },
        PlatformServices {
            vault: Arc::new(MemoryVault::new()),
            connection: Arc::new(FakeServices::default()),
            providers: Arc::new(Factory),
            connectivity: Arc::new(AlwaysOnline),
            media: Some(Arc::new(MissingFfmpeg)),
            utc_offset: UtcOffset::UTC,
        },
    )
    .await
    .unwrap();
    environment.start().await;
    environment
        .connection
        .connect_api_key(&ApiKeyForm::new("kid", "secret"))
        .await;
    environment
}

#[tokio::test]
async fn an_agent_quotes_queues_and_follows_a_video_through_the_socket() {
    let folder = tempfile::tempdir().unwrap();
    let environment = connected_app(folder.path()).await;
    let endpoint = AgentEndpoint::at(folder.path().join("a.sock"));
    let runtime = AgentRuntime::new(
        environment.clone(),
        Arc::new(NoWindow),
        "0.0.0-test",
        Some(endpoint.clone()),
    );
    runtime.start();
    assert!(matches!(runtime.status(), AgentStatus::Listening(_)));
    let mut client = AgentClient::connect(&endpoint, Duration::from_secs(5))
        .await
        .unwrap();

    // Off until the person allows agents.
    let off = send(
        &mut client,
        AgentCommand::Capabilities(CapabilitiesCommand::default()),
    )
    .await;
    assert_eq!(off.error.map(|error| error.code), Some(ErrorCode::AGENTS_OFF));
    let preferences = AgentPreferences::new(environment.preferences.clone());
    preferences.set_allowed(true).unwrap();
    preferences.set_daily_budget(500, ProviderKind::ApiKey).unwrap();

    let capabilities = send(
        &mut client,
        AgentCommand::Capabilities(CapabilitiesCommand {
            models: true,
            ..CapabilitiesCommand::default()
        }),
    )
    .await
    .capabilities
    .unwrap();
    let budget = capabilities.budget.unwrap();
    assert_eq!(
        (budget.unit, budget.daily_limit, budget.remaining),
        (AgentBudgetUnit::UsdCents, 500, 500)
    );
    let models = capabilities.models.unwrap();
    assert!(models.iter().all(|model| model.available));

    // A clip made from a prompt alone, on a model that makes one without a source.
    let model = VideoModel::all()
        .into_iter()
        .find(|model| model.makes_clips_without_source() && model.modes().contains(&Mode::MotionTransfer))
        .unwrap();
    let mut video = AgentVideoRequest::new("motion_transfer");
    video.model = Some(model.raw_value().to_owned());
    video.prompt = Some("A paper boat drifts down a rainy street".to_owned());
    let quote = send(
        &mut client,
        AgentCommand::Quote(QuoteCommand { video: video.clone() }),
    )
    .await;
    let quote = quote
        .quote
        .unwrap_or_else(|| panic!("no quote: {:?}", quote.error));
    assert_eq!(quote.model, model.raw_value());
    assert_eq!(
        quote.price,
        Some(AgentAmount::new(130, AgentBudgetUnit::UsdCents))
    );
    assert!(quote.is_exact);
    assert_eq!((quote.remaining_today, quote.blocker), (Some(500), None));

    let create = CreateVideoCommand {
        request_id: "r1".to_owned(),
        fingerprint: "a".repeat(64),
        parameters: Some("b".repeat(64)),
        video,
        max_cost: AgentAmount::new(300, AgentBudgetUnit::UsdCents),
        wait_seconds: 5,
    };
    let created = send(&mut client, AgentCommand::CreateVideo(create.clone())).await;
    let created = created
        .created
        .unwrap_or_else(|| panic!("not queued: {:?}", created.error));
    assert!(!created.known);
    assert_eq!(created.jobs.len(), 1);
    let job = &created.jobs[0];
    assert_eq!(job.price, Some(AgentAmount::new(130, AgentBudgetUnit::UsdCents)));
    assert_eq!(job.request_id.as_deref(), Some("r1"));

    // The same request again answers with the same job and queues nothing.
    let again_command = create.clone();
    let again = send(&mut client, AgentCommand::CreateVideo(create))
        .await
        .created
        .unwrap();
    assert!(again.known);
    assert_eq!(again.jobs[0].id, job.id);

    let jobs = send(&mut client, AgentCommand::Jobs(JobsCommand::default()))
        .await
        .jobs
        .unwrap();
    assert_eq!(jobs.jobs.len(), 1);
    assert_eq!(jobs.jobs[0].id, job.id);
    assert_eq!(jobs.jobs[0].request_id.as_deref(), Some("r1"));
    assert_eq!(jobs.jobs[0].client.as_deref(), Some("test-agent"));
    held_over_max_cost_goes_on_when_raised(&mut client, again_command).await;
    runtime.stop();
}

/// Over its `max_cost` the guard holds a job; the same request with a `max_cost` that covers the price lifts the hold
/// and answers with the same job.
async fn held_over_max_cost_goes_on_when_raised(client: &mut AgentClient, again_command: CreateVideoCommand) {
    let mut held = CreateVideoCommand {
        request_id: "r2".to_owned(),
        fingerprint: "c".repeat(64),
        parameters: Some("d".repeat(64)),
        max_cost: AgentAmount::new(100, AgentBudgetUnit::UsdCents),
        ..again_command
    };
    let first = send(client, AgentCommand::CreateVideo(held.clone()))
        .await
        .created
        .unwrap();
    // The price is known at once; the guard holds the job at its submit, which `jobs` waits for.
    let waited = JobsCommand {
        ids: vec![first.jobs[0].id],
        wait_seconds: 10,
        ..JobsCommand::default()
    };
    let followed = send(client, AgentCommand::Jobs(waited)).await.jobs.unwrap();
    let job = &followed.jobs[0];
    assert!(!followed.timed_out);
    assert_eq!(
        (job.state.as_str(), job.hold.as_deref()),
        ("paused", Some("over_max_cost"))
    );
    assert!(job.needs_person);
    held.fingerprint = "e".repeat(64);
    held.max_cost = AgentAmount::new(300, AgentBudgetUnit::UsdCents);
    let raised = send(client, AgentCommand::CreateVideo(held)).await;
    let raised = raised
        .created
        .unwrap_or_else(|| panic!("not raised: {:?}", raised.error));
    assert!(raised.known);
    assert_eq!(raised.jobs[0].id, first.jobs[0].id);
    assert_ne!(raised.jobs[0].hold.as_deref(), Some("over_max_cost"));
}

/// `ImageFake` with one model, Soul 2, for the agents' image requests.
struct SoulImages(Arc<ImageFake>);

#[async_trait::async_trait]
impl ImageProvider for SoulImages {
    fn kind(&self) -> ProviderKind {
        self.0.kind()
    }
    fn owner(&self) -> Option<String> {
        Some("kid".to_owned())
    }
    async fn image_models(&self) -> Result<Vec<ImageModel>, PortError> {
        let mut model =
            ImageModel::new("soul", "Soul 2", vec!["1:1".to_owned()]).with_variant_counts(vec![1, 4]);
        model.input = ImageModelInput::Optional;
        Ok(vec![model])
    }
    async fn image_estimate(&self, request: &ImageRequest) -> Result<Cost, PortError> {
        self.0.image_estimate(request).await
    }
    async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError> {
        self.0.upload(file).await
    }
    async fn submit_images(&self, request: &ImageRequest, key: Uuid) -> Result<Vec<RemoteJob>, PortError> {
        self.0.submit_images(request, key).await
    }
    async fn image_status(&self, job: &RemoteJob) -> Result<ImageJobStatus, PortError> {
        self.0.image_status(job).await
    }
    async fn cancel_images(&self, job: &RemoteJob) -> Result<(), PortError> {
        self.0.cancel_images(job).await
    }
}

fn image_command(request_id: &str, fingerprint: char, max_cents: i64) -> AgentCommand {
    AgentCommand::CreateImage(CreateImageCommand {
        request_id: request_id.to_owned(),
        fingerprint: fingerprint.to_string().repeat(64),
        image: AgentImageRequest {
            prompt: "A lighthouse at dusk".to_owned(),
            ..AgentImageRequest::default()
        },
        max_cost: AgentAmount::new(max_cents, AgentBudgetUnit::UsdCents),
    })
}

/// `create_image` sends a request within its `max_cost` and the budget, holds one over it for the person, and the
/// Library is searched, read and marked, all through the socket.
#[tokio::test]
async fn an_agent_makes_images_and_works_the_library_through_the_socket() {
    let folder = tempfile::tempdir().unwrap();
    let environment = connected_app(folder.path()).await;
    let fake = ImageFake::new();
    environment
        .images
        .set_provider(Some(Arc::new(SoulImages(fake.clone()))));
    let image = environment
        .store
        .save_material(catalog_image(
            "A fox portrait",
            None,
            1.0,
            Culling::Unreviewed,
            false,
        ))
        .await
        .unwrap();
    let endpoint = AgentEndpoint::at(folder.path().join("a.sock"));
    let runtime = AgentRuntime::new(
        environment.clone(),
        Arc::new(NoWindow),
        "0.0.0-test",
        Some(endpoint.clone()),
    );
    runtime.start();
    let mut client = AgentClient::connect(&endpoint, Duration::from_secs(5))
        .await
        .unwrap();
    let preferences = AgentPreferences::new(environment.preferences.clone());
    preferences.set_allowed(true).unwrap();
    preferences.set_daily_budget(500, ProviderKind::ApiKey).unwrap();

    // Within max_cost: sent, at the provider's quote.
    let sent = send(&mut client, image_command("img-1", 'a', 50)).await;
    let sent = sent
        .created
        .unwrap_or_else(|| panic!("not sent: {:?}", sent.error));
    assert!(!sent.known);
    assert_eq!(sent.jobs.len(), 1);
    assert_eq!(sent.jobs[0].request_id.as_deref(), Some("img-1"));
    let again = send(&mut client, image_command("img-1", 'a', 50))
        .await
        .created
        .unwrap();
    assert!(again.known);
    assert_eq!(again.jobs[0].id, sent.jobs[0].id);

    // Over max_cost: held for the person, never sent.
    let held = send(&mut client, image_command("img-2", 'b', 5))
        .await
        .created
        .unwrap();
    let waited = JobsCommand {
        ids: vec![held.jobs[0].id],
        wait_seconds: 10,
        ..JobsCommand::default()
    };
    let followed = send(&mut client, AgentCommand::Jobs(waited)).await.jobs.unwrap();
    assert!(followed.jobs[0].needs_person, "{:?}", followed.jobs[0]);
    assert_eq!(fake.submitted().len(), 1);

    works_the_library(&mut client, image.id).await;
    let clip = environment.store.save_material(video(1.0)).await.unwrap();
    refuses_a_sheet_over_the_persons(&mut client, &runtime.service, clip.id.into()).await;
    runtime.stop();
}

/// `library_search` → `library_get` → `library_mark` on the one item of the Library.
async fn works_the_library(client: &mut AgentClient, image: Uuid) {
    let found = send(
        client,
        AgentCommand::LibrarySearch(LibrarySearchCommand {
            query: Some("fox".to_owned()),
            ..LibrarySearchCommand::default()
        }),
    )
    .await;
    let found = found
        .library
        .unwrap_or_else(|| panic!("no page: {:?}", found.error));
    assert_eq!(found.items.len(), 1);
    let id = found.items[0].id;
    assert_eq!(app_id(id), image);
    let details = send(
        client,
        AgentCommand::LibraryGet(LibraryGetCommand { ids: vec![id] }),
    )
    .await
    .details
    .unwrap();
    assert_eq!(details.items[0].item.id, id);
    assert!(details.not_found.is_none());
    let marked = send(
        client,
        AgentCommand::LibraryMark(LibraryMarkCommand {
            ids: vec![id],
            verdict: Some(AgentCulling::Keep),
            star: None,
        }),
    )
    .await;
    let marked = marked
        .marked
        .unwrap_or_else(|| panic!("not marked: {:?}", marked.error));
    assert_eq!(marked.changed, 1);
    assert_eq!(marked.items[0].verdict, AgentVerdict::Kept);
    let kept = send(
        client,
        AgentCommand::LibrarySearch(LibrarySearchCommand {
            verdict: Some(AgentVerdict::Kept),
            ..LibrarySearchCommand::default()
        }),
    )
    .await
    .library
    .unwrap();
    assert_eq!(kept.items.len(), 1);
    refuses_subtitles(client, id).await;
    exports(client, id).await;
}

/// `subtitles` refuses an image as one, and an ID with no item as not found (`AgentService+Subtitles.swift`).
async fn refuses_subtitles(client: &mut AgentClient, image: jutsu_agent::Uuid) {
    let get = |item: jutsu_agent::Uuid| {
        AgentCommand::Subtitles(SubtitlesCommand {
            action: AgentSubtitlesAction::Get,
            item,
            request_id: None,
            fingerprint: None,
            language: None,
            engine: None,
            format: None,
        })
    };
    let refused = send(client, get(image)).await.error.unwrap();
    assert_eq!(
        (refused.code, refused.message.as_str()),
        (
            ErrorCode::INVALID_REQUEST,
            "Subtitles are made of a video's speech; this item is an image."
        )
    );
    let missing = send(client, get(Uuid::new_v4().into())).await.error.unwrap();
    assert_eq!(missing.code, ErrorCode::NOT_FOUND);
    assert!(missing.message.starts_with("There's no video or image "));
    refuses_sheets(client, image).await;
}

/// `show` checks the item before it opens a sheet (`showSheet`, `showSubtitles`): the Subtitles sheet takes a video,
/// and either sheet an item with a file here; `enhance` on this computer refuses a missing item the same way.
async fn refuses_sheets(client: &mut AgentClient, image: jutsu_agent::Uuid) {
    let subtitles = send(
        client,
        AgentCommand::Show(ShowCommand::sheet(AgentSheet::Subtitles, image)),
    )
    .await
    .error
    .unwrap();
    assert_eq!(
        (
            subtitles.code,
            subtitles.message.as_str(),
            subtitles.next.as_deref()
        ),
        (
            ErrorCode::INVALID_REQUEST,
            "The Subtitles sheet is for videos; this item is an image.",
            Some("Pick a video's Library ID.")
        )
    );
    let nowhere: jutsu_agent::Uuid = Uuid::new_v4().into();
    for sheet in [AgentSheet::UpscaleOnMac, AgentSheet::Subtitles] {
        let missing = send(client, AgentCommand::Show(ShowCommand::sheet(sheet, nowhere)))
            .await
            .error
            .unwrap();
        assert_eq!(
            (missing.code, missing.message),
            (
                ErrorCode::NOT_FOUND,
                format!("There's no video or image {nowhere} with a file on this computer.")
            )
        );
    }
    let enhance = send(
        client,
        AgentCommand::Enhance(EnhanceCommand {
            request_id: "up-1".to_owned(),
            fingerprint: "f".repeat(64),
            item: nowhere,
            operation: AgentEnhancement::Upscale,
            target: None,
            frame_rate: None,
            preset: None,
            engine: Some(EnhanceCommand::MAC_ENGINE.to_owned()),
            mode: None,
            aspect_ratio: None,
            language: None,
            voice: None,
            max_cost: None,
        }),
    )
    .await
    .error
    .unwrap();
    assert_eq!(
        (enhance.code, enhance.next.as_deref()),
        (
            ErrorCode::NOT_FOUND,
            Some("Call library_search for the item IDs; library_download brings one from Higgsfield.")
        )
    );
}

/// `export` of an image: the operation answers at once, skips it, and `jobs` follows it.
async fn exports(client: &mut AgentClient, id: jutsu_agent::Uuid) {
    let exported = send(
        client,
        AgentCommand::Export(ExportCommand {
            items: vec![id],
            preset: AgentExportPreset::Premiere,
            sidecar: None,
            wait_seconds: Some(10),
        }),
    )
    .await;
    let exported = exported
        .exported
        .unwrap_or_else(|| panic!("not exported: {:?}", exported.error));
    assert!(!exported.timed_out);
    assert_eq!(exported.job.state, "done");
    assert_eq!(exported.job.kind, "export");
    let files = exported.job.files.clone().unwrap_or_default();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status, AgentExportStatus::Skipped);
    let followed = send(
        client,
        AgentCommand::Jobs(JobsCommand {
            ids: vec![exported.job.id],
            ..JobsCommand::default()
        }),
    )
    .await
    .jobs
    .unwrap();
    assert_eq!(followed.jobs[0].state, "done");
    assert_eq!(followed.jobs[0].kind, "export");
}

/// `show` doesn't open a sheet over one the person has open (`showSubtitles`), but replaces its own; once the person
/// closes theirs, it goes on to the window.
async fn refuses_a_sheet_over_the_persons(
    client: &mut AgentClient,
    service: &AgentService,
    video: jutsu_agent::Uuid,
) {
    let show = AgentCommand::Show(ShowCommand::sheet(AgentSheet::Subtitles, video));
    service.set_window_sheet(Some(WindowSheet::Other));
    let refused = send(client, show.clone()).await.error.unwrap();
    assert_eq!(
        (
            refused.code,
            refused.message.as_str(),
            refused.retryable,
            refused.next.as_deref()
        ),
        (
            ErrorCode::NEEDS_PERSON,
            "The person has a sheet open in Jutsu Studio, so the sheet isn't shown over it.",
            true,
            Some("Ask the person to finish or close it, then call show again.")
        )
    );
    // Past the refusal, `show` reaches the window, which this test has none of.
    for open in [Some(WindowSheet::Subtitles), Some(WindowSheet::OnDevice), None] {
        service.set_window_sheet(open);
        let shown = send(client, show.clone()).await.error.unwrap();
        assert_eq!(shown.message, "Jutsu Studio couldn't open its window.");
    }
}
