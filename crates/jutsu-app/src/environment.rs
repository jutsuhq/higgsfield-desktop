//! The composition root (`AppEnvironment.swift`): built once at launch, it owns the store, the file layout, the
//! preferences, the connection, the provider of the active connection and the job engine, and hands the engine a new
//! provider on every change of credential. No Tauri types: the shell hands it its directories and platform services.

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use futures::future::BoxFuture;
use jutsu_core::{
    BudgetError, Capabilities, Connectivity, Credential, Job, MediaPreparing, OAuthTokens, OutputSubfolder,
    PortError, ProviderError, SpendStatus,
};
use jutsu_engine::{AgentSpendFile, AgentSpendPolicy, EngineDeps, EngineSettings, JobEngine, SpendStatusFn};
use jutsu_media::{FfmpegTools, MediaPreparer};
use jutsu_store::{FileLayout, Store, StoreError};
use time::{OffsetDateTime, UtcOffset};
use tokio::sync::broadcast::error::RecvError;

use crate::connection::{
    ConnectionDetails, ConnectionServices, ConnectionSnapshot, ConnectionStore, CredentialVault,
    unlimited_text,
};
use crate::create::{CreateDeps, CreateService, FfmpegSourceMedia};
use crate::images::ImageSpendLog;
use crate::images::service::{ImageDeps, ImageService};
use crate::library::{AccountServices, LibraryService};
use crate::preferences::PreferencesStore;
use crate::provider::{
    ConstraintsReader, CreateAccount, PersistTokens, ProviderFactory, ProviderHandle, UnlimitedOptIn,
};
use crate::queue::{MissingFfmpeg, QueueService};

/// How long quitting waits for submits already sent to be answered.
pub const QUIT_GRACE: time::Duration = time::Duration::seconds(5);
/// When a failed engine start is tried again, after the first attempt (`AppEnvironment.swift`).
const ENGINE_START_RETRIES: [u64; 3] = [2, 10, 60];

/// Where the app keeps its files: the app data dir, and the default output folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirectories {
    /// The app data dir (`%APPDATA%\…`, `$XDG_DATA_HOME/…`); the store, the settings and the caches.
    pub data_dir: PathBuf,
    /// `<Videos>/Jutsu Studio`, or `Jutsu Studio Desktop Debug` for a debug build.
    pub default_output_dir: PathBuf,
}

/// What the environment reaches the system and the network through; tests pass fakes.
#[derive(Clone)]
pub struct PlatformServices {
    pub vault: Arc<dyn CredentialVault>,
    pub connection: Arc<dyn ConnectionServices>,
    pub providers: Arc<dyn ProviderFactory>,
    /// Whether the computer is online, for the engine.
    pub connectivity: Arc<dyn Connectivity>,
    /// `None` prepares media with ffmpeg found on the `PATH` (`FfmpegTools::from_environment`), or fails every
    /// preparing call with a clear message when it isn't there.
    pub media: Option<Arc<dyn MediaPreparing>>,
    /// The local time zone's offset, read once at launch: it decides where "today" starts for the spend cap.
    pub utc_offset: UtcOffset,
}

/// Why the environment couldn't be built.
#[derive(Debug, thiserror::Error)]
pub enum EnvironmentError {
    #[error("Jutsu Studio can't open its library: {0}")]
    Store(#[from] StoreError),
    #[error("Jutsu Studio can't create its folders: {0}")]
    Io(#[from] std::io::Error),
    #[error("Jutsu Studio couldn't start: {0}")]
    Join(#[from] tokio::task::JoinError),
}

/// The composition root.
pub struct AppEnvironment {
    pub layout: FileLayout,
    pub store: Arc<Store>,
    pub preferences: Arc<PreferencesStore>,
    pub connection: Arc<ConnectionStore>,
    /// The provider of the active connection, for the features and the engine.
    pub provider: ProviderHandle,
    /// Runs the jobs.
    pub engine: JobEngine,
    /// The Queue's live jobs and commands.
    pub queue: Arc<QueueService>,
    pub utc_offset: UtcOffset,
    /// The Create screen's video draft, its prices and its submit.
    pub create: Arc<CreateService>,
    /// The Library's grid, result pages and actions.
    pub library: Arc<LibraryService>,
    /// Enhance on this computer: Smooth Motion and Upscale 4×, one run at a time.
    pub on_device: Arc<crate::on_device::service::OnDeviceService>,
    /// The Image side of Create: its form, runs and results.
    pub images: Arc<ImageService>,
    /// The Templates menu and sheet.
    pub templates: Arc<crate::templates::TemplateService>,
    /// The prompt history and its stars.
    pub prompts: Arc<crate::prompts::PromptService>,
    /// What agents spent and hold today: the engine's spending guard and `capabilities` read the same file.
    pub agent_spend: Arc<AgentSpendFile>,
    /// Subtitles made on this computer, their model and what agents ask of them.
    pub subtitles: Arc<crate::subtitles::service::SubtitleService>,
    providers: Arc<dyn ProviderFactory>,
}

impl std::fmt::Debug for AppEnvironment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppEnvironment")
            .field("layout", &self.layout)
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

impl AppEnvironment {
    /// Creates the folders, opens the store and the settings, and builds the engine. The connection is read from
    /// the vault in `start`, off this task: until then it is restoring. Call `start` once the shell is ready for the
    /// first credential change.
    #[allow(clippy::too_many_lines, reason = "builds every service in one place")]
    pub async fn open(
        directories: AppDirectories,
        platform: PlatformServices,
    ) -> Result<Arc<Self>, EnvironmentError> {
        let layout = FileLayout::new(directories.data_dir, directories.default_output_dir);
        let prepared = layout.clone();
        tokio::task::spawn_blocking(move || prepared.prepare_directories()).await??;
        let store = Arc::new(Store::open(&layout.store_path()).await?);
        let root = layout.root.clone();
        let preferences = Arc::new(tokio::task::spawn_blocking(move || PreferencesStore::open(&root)).await?);
        let connection = Arc::new(ConnectionStore::restoring(
            platform.vault,
            preferences.clone(),
            platform.connection,
        ));
        let offset = platform.utc_offset;
        let media = platform.media.unwrap_or_else(|| system_media(&layout));
        let spend_root = layout.root.clone();
        let image_spend = Arc::new(
            tokio::task::spawn_blocking(move || {
                ImageSpendLog::open(Some(&spend_root), OffsetDateTime::now_utc(), offset)
            })
            .await?,
        );
        let mut deps = EngineDeps::new(store.clone(), store.clone(), media.clone(), platform.connectivity);
        deps.spend_status = Some(spend_status(&store, &preferences, &image_spend, offset));
        let agent_spend = Arc::new(AgentSpendFile::new(
            layout.root.join(AgentSpendFile::FILE_NAME),
            offset,
        ));
        deps.agent_policy = Some(agent_policy(&agent_spend, &preferences));
        let current = preferences.get();
        let mut settings = EngineSettings::new(output_directory(&layout, &current));
        settings.utc_offset = offset;
        settings.output_name_pattern = current.output_name_pattern();
        settings.max_concurrency = engine_limit(current.max_concurrency(None));
        let engine = JobEngine::new(deps, settings);
        let provider = ProviderHandle::default();
        let queue = Arc::new(QueueService::new(
            engine.clone(),
            store.clone(),
            preferences.clone(),
            connection.clone(),
            provider.clone(),
            offset,
            image_spend.clone(),
        ));
        let create = CreateService::new(CreateDeps {
            layout: layout.clone(),
            store: store.clone(),
            preferences: preferences.clone(),
            connection: connection.clone(),
            provider: provider.clone(),
            engine: engine.clone(),
            media: Arc::new(FfmpegSourceMedia::from_environment()),
            utc_offset: offset,
            image_spend: image_spend.clone(),
            preparer: media.clone(),
        });
        let library = Arc::new(LibraryService::new(
            store.clone(),
            layout.clone(),
            preferences.clone(),
            connection.clone(),
            provider.clone(),
            engine.clone(),
            offset,
            image_spend.clone(),
        ));
        let on_device = crate::on_device::service::OnDeviceService::new(
            store.clone(),
            layout.clone(),
            preferences.clone(),
        );
        library.use_on_device(on_device.clone());
        let image_deps = ImageDeps {
            layout: layout.clone(),
            store: store.clone(),
            preferences: preferences.clone(),
            connection: connection.clone(),
            media,
            spend: image_spend,
            utc_offset: offset,
        };
        let images = tokio::task::spawn_blocking(move || ImageService::new(image_deps)).await?;
        create.use_characters(images.characters().clone());
        images.attach_agent_images(agent_spend.clone());
        let templates = Arc::new(crate::templates::TemplateService::new(
            store.clone(),
            layout.clone(),
            create.clone(),
            images.clone(),
        ));
        let prompts = Arc::new(crate::prompts::PromptService::new(
            store.clone(),
            preferences.clone(),
        ));
        let subtitles = crate::subtitles::service::SubtitleService::open(
            &layout,
            store.clone(),
            preferences.clone(),
            connection.clone(),
        );
        let environment = Arc::new(Self {
            subtitles,
            layout,
            store,
            preferences,
            connection,
            provider,
            engine,
            queue,
            utc_offset: offset,
            create,
            library,
            on_device,
            images,
            templates,
            prompts,
            agent_spend,
            providers: platform.providers,
        });
        let weak = Arc::downgrade(&environment);
        environment
            .connection
            .set_on_credential_change(Arc::new(move |credential| {
                let weak = weak.clone();
                Box::pin(async move {
                    if let Some(environment) = weak.upgrade() {
                        environment.use_credential(credential).await;
                    }
                })
            }));
        Ok(environment)
    }

    /// Runs once at launch: follows the engine's events, loads the stored jobs, reads the connection from the vault
    /// (on a blocking task) and hands it to the engine, then starts the engine, trying again after 2, 10 and 60 s
    /// when it fails.
    pub async fn start(self: &Arc<Self>) {
        self.follow_engine_events();
        self.library.follow_store();
        self.subtitles.refresh().await;
        self.on_device.follow_failures(self.connection.clone());
        self.images.restore_agent_images().await;
        self.images.start();
        self.queue.seed().await;
        let connection = Arc::downgrade(&self.connection);
        self.engine.on_connection_needs_attention(move || {
            let connection = connection.clone();
            Box::pin(async move {
                if let Some(connection) = connection.upgrade() {
                    connection.mark_needs_attention();
                }
            })
        });
        self.connection.activate().await;
        self.follow_preferences();
        if let Err(error) = self.engine.start().await {
            tracing::error!(%error, "can't start the engine");
            let engine = self.engine.clone();
            tokio::spawn(async move {
                for delay in ENGINE_START_RETRIES {
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                    match engine.start().await {
                        Ok(()) => return,
                        Err(error) => tracing::error!(%error, delay, "can't start the engine"),
                    }
                }
            });
        }
    }

    /// Stops all work as on quit; submits already sent get `QUIT_GRACE` to be answered. The Create draft is saved.
    pub async fn stop(&self) {
        self.create.save_now().await;
        self.images.stop().await;
        self.engine.stop(QUIT_GRACE).await;
    }

    /// Applies the engine's events to the Queue, and reads the balance again whenever a job finishes.
    fn follow_engine_events(self: &Arc<Self>) {
        let mut events = self.engine.subscribe();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "the queue missed engine events");
                        continue;
                    }
                    Err(RecvError::Closed) => return,
                };
                let Some(environment) = weak.upgrade() else {
                    return;
                };
                if environment
                    .queue
                    .apply(event, OffsetDateTime::now_utc())
                    .is_some()
                {
                    environment.schedule_balance_refresh();
                }
            }
        });
    }

    /// Hands changed settings to the engine: the concurrency, the output folder and the name pattern.
    fn follow_preferences(self: &Arc<Self>) {
        let mut changes = self.preferences.subscribe();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                let Some(environment) = weak.upgrade() else {
                    return;
                };
                environment.apply_preferences();
            }
        });
    }

    fn apply_preferences(&self) {
        let preferences = self.preferences.get();
        let kind = self.provider.current().map(|provider| provider.kind());
        self.engine
            .set_max_concurrency(engine_limit(preferences.max_concurrency(kind)));
        self.engine
            .set_output_directory(output_directory(&self.layout, &preferences));
        self.engine
            .set_output_name_pattern(preferences.output_name_pattern());
    }

    /// The connection as the window shows it.
    pub fn connection_snapshot(&self) -> ConnectionSnapshot {
        ConnectionSnapshot::new(&self.connection, self.preferences.get().low_credit_threshold())
    }

    /// The Settings › Connection card: the rows with what the connection runs, and the unlimited allowance.
    pub async fn connection_details(&self) -> ConnectionDetails {
        let capabilities = self.capabilities().await;
        let state = self.connection.state();
        ConnectionDetails {
            rows: state.card_rows(capabilities.as_ref()),
            unlimited_text: capabilities
                .as_ref()
                .filter(|_| state.account_info().is_some())
                .and_then(unlimited_text),
            uses_unlimited: self.preferences.get().uses_unlimited(),
        }
    }

    /// What the active connection can run. Flags the connection on 401. `None` while disconnected, after a
    /// failure, and for a provider replaced meanwhile, so another account's never answer.
    pub async fn capabilities(&self) -> Option<Capabilities> {
        let provider = self.provider.current()?;
        let generation = self.provider.generation();
        match provider.capabilities().await {
            Ok(capabilities) => (self.provider.generation() == generation).then_some(capabilities),
            Err(error) => {
                if error == PortError::Provider(ProviderError::Unauthorized) {
                    self.connection.mark_needs_attention();
                }
                tracing::error!(%error, "can't load capabilities");
                None
            }
        }
    }

    /// Reads the connection's balance again, off the caller's task; on connect, and later whenever a job finishes.
    pub fn schedule_balance_refresh(&self) {
        let Some(provider) = self.provider.current() else {
            return;
        };
        let connection = self.connection.clone();
        tokio::spawn(async move { connection.refresh_balance(provider.as_ref()).await });
    }

    /// The folders the media server shows files from: the output folder in use, the earlier ones, and the app's
    /// inputs and caches.
    pub fn media_roots(&self) -> Vec<PathBuf> {
        let preferences = self.preferences.get();
        let mut roots = vec![self.layout.default_output_folder.clone()];
        roots.extend(preferences.output_folder.map(PathBuf::from));
        roots.extend(
            preferences
                .previous_output_folders
                .unwrap_or_default()
                .into_iter()
                .map(PathBuf::from),
        );
        roots.extend([self.layout.sources_folder(), self.layout.cache_folder()]);
        roots.extend(self.layout.downloaded_input_folders());
        roots
    }

    async fn use_credential(self: Arc<Self>, credential: Option<Credential>) {
        let generation = self.connection.credential_generation();
        let built = credential.map(|credential| {
            let weak = Arc::downgrade(&self);
            let persist: PersistTokens = Arc::new(move |tokens| {
                let weak = weak.clone();
                Box::pin(async move { save_refreshed(&weak, tokens, generation) })
            });
            let preferences = Arc::downgrade(&self.preferences);
            let unlimited: UnlimitedOptIn = Arc::new(move || {
                preferences
                    .upgrade()
                    .is_some_and(|preferences| preferences.get().uses_unlimited())
            });
            let create = Arc::downgrade(&self.create);
            let constraints: ConstraintsReader = Arc::new(move || {
                create
                    .upgrade()
                    .map_or_else(jutsu_core::ModelConstraintTable::built_in, |create| {
                        create.model_constraints()
                    })
            });
            self.providers
                .build_for_create(credential, persist, unlimited, constraints)
        });
        let (built, image_provider, account) = built.map_or(
            (None, None, CreateAccount::default()),
            |(built, images, account)| (Some(built), images, account),
        );
        self.images.set_provider(image_provider);
        let provider = built.as_ref().map(|built| built.provider.clone());
        let kind = provider.as_ref().map(|provider| provider.kind());
        self.provider.set(provider.clone());
        self.images.set_character_maker(account.characters.clone());
        self.create.set_account(account).await;
        self.library.set_account(AccountServices {
            generation: self.provider.generation(),
            history: built.as_ref().and_then(|built| built.history.clone()),
            likes: built.as_ref().and_then(|built| built.likes.clone()),
            voices: built.as_ref().and_then(|built| built.voices.clone()),
            credits: built.as_ref().and_then(|built| built.credits.clone()),
            virality: built.and_then(|built| built.virality),
        });
        self.engine
            .set_max_concurrency(engine_limit(self.preferences.get().max_concurrency(kind)));
        self.engine.set_provider(provider).await;
        self.schedule_balance_refresh();
    }
}

/// ffmpeg from the `PATH`, or a stand-in that fails every preparing call with a clear message.
fn system_media(layout: &FileLayout) -> Arc<dyn MediaPreparing> {
    if let Some(tools) = FfmpegTools::from_environment() {
        Arc::new(MediaPreparer::new(tools, layout.cache_folder()))
    } else {
        tracing::warn!("ffmpeg isn't on the PATH; jobs that need media prepared fail");
        Arc::new(MissingFfmpeg)
    }
}

/// `<output folder>/Videos`: the chosen output folder, or the default one.
fn output_directory(layout: &FileLayout, preferences: &crate::Preferences) -> PathBuf {
    let folder = preferences
        .output_folder
        .as_ref()
        .map_or_else(|| layout.default_output_folder.clone(), PathBuf::from);
    OutputSubfolder::Videos.path(&folder)
}

/// The engine's spending guard for agents: the ledger, and Settings › Agents' daily budget and unlimited allowance,
/// read at each check.
fn agent_policy(ledger: &Arc<AgentSpendFile>, preferences: &Arc<PreferencesStore>) -> AgentSpendPolicy {
    let budget = crate::agents::preferences::AgentPreferences::new(preferences.clone());
    let unlimited = budget.clone();
    AgentSpendPolicy::new(ledger.clone(), move |kind| budget.daily_budget(kind))
        .allowing_unlimited(move || unlimited.allows_unlimited())
}

fn engine_limit(limit: i64) -> usize {
    usize::try_from(limit.max(1)).unwrap_or(1)
}

/// Today's spend without the job, the image charges included, and the cap of its connection; `None` without a cap.
fn spend_status(
    store: &Arc<Store>,
    preferences: &Arc<PreferencesStore>,
    image_spend: &Arc<ImageSpendLog>,
    offset: UtcOffset,
) -> SpendStatusFn {
    let store = Arc::downgrade(store);
    let image_spend = image_spend.clone();
    let preferences = Arc::downgrade(preferences);
    Arc::new(
        move |job: Job| -> BoxFuture<'static, Result<Option<SpendStatus>, BudgetError>> {
            let store = store.clone();
            let preferences = preferences.clone();
            let image_spend = image_spend.clone();
            Box::pin(async move {
                let (Some(store), Some(preferences)) = (store.upgrade(), preferences.upgrade()) else {
                    return Err(BudgetError::SpendingUnavailable);
                };
                let Some(cap) = preferences.get().spend_cap(Some(job.provider_kind)) else {
                    return Ok(None);
                };
                let committed = crate::settings::spending::committed_today_excluding(
                    &store,
                    &image_spend,
                    cap.unit,
                    OffsetDateTime::now_utc(),
                    offset,
                    Some(job.id),
                )
                .await?;
                Ok(Some(SpendStatus {
                    committed_today: committed,
                    cap: Some(cap),
                }))
            })
        },
    )
}

fn save_refreshed(environment: &Weak<AppEnvironment>, tokens: OAuthTokens, generation: u64) {
    let Some(environment) = environment.upgrade() else {
        return;
    };
    match environment.connection.save_refreshed_tokens(tokens, generation) {
        Ok(true) => {}
        Ok(false) => tracing::info!("dropped tokens refreshed for a connection that has since changed"),
        Err(error) => tracing::error!(%error, "can't save the refreshed sign-in"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use futures::StreamExt;
    use futures::stream::BoxStream;
    use jutsu_core::{
        Balance, ClosedRange, Cost, CostUnit, GenerationProvider, JobSpec, Mode, PixelSize, PreparedFile,
        ProviderKind, RemoteAsset, RemoteInputs, RemoteJob, RemoteStatus, Resolution, SourceClip,
        StylePreset, VideoModel,
    };
    use jutsu_engine::EnqueueOptions;
    use rust_decimal::Decimal;
    use url::Url;
    use uuid::Uuid;

    use super::*;
    use crate::connection::{ApiKeyForm, ConnectionState, MemoryVault};
    use crate::preferences::Preferences;
    use crate::queue::{DuplicateOutcome, QueueEntryModel};
    use crate::testing::{BalanceProvider, FakeServices, Shared, eventually, fixture};

    /// Online from the start, and never offline.
    struct AlwaysOnline;

    impl Connectivity for AlwaysOnline {
        fn updates(&self) -> BoxStream<'static, bool> {
            futures::stream::once(async { true })
                .chain(futures::stream::pending())
                .boxed()
        }
    }

    /// `BalanceProvider` that quotes $1.00 for every job.
    struct PricedProvider(BalanceProvider);

    #[async_trait]
    impl GenerationProvider for PricedProvider {
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
            Ok(Cost::usd(Decimal::ONE, false))
        }
        async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError> {
            self.0.upload(file).await
        }
        async fn submit(
            &self,
            spec: &JobSpec,
            inputs: &RemoteInputs,
            key: Uuid,
        ) -> Result<RemoteJob, PortError> {
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

    fn spec() -> JobSpec {
        JobSpec::new(
            Mode::MotionTransfer,
            VideoModel::GENJUTSU,
            Some(SourceClip::new(
                Url::parse("file:///tmp/clips/dance.mov").unwrap(),
                30.0,
                PixelSize {
                    width: 1280,
                    height: 720,
                },
                ClosedRange::new(0.0, 12.0),
            )),
            Resolution::P720,
        )
    }

    /// Builds `BalanceProvider`s and keeps what each build was given.
    #[derive(Default)]
    struct FakeFactory {
        persist: Mutex<Vec<PersistTokens>>,
        unlimited: Mutex<Vec<UnlimitedOptIn>>,
        constraints: Mutex<Vec<ConstraintsReader>>,
        credentials: Arc<Shared<Vec<Credential>>>,
    }

    impl ProviderFactory for FakeFactory {
        fn provider(
            &self,
            credential: Credential,
            persist: PersistTokens,
            unlimited_opt_in: UnlimitedOptIn,
        ) -> Arc<dyn GenerationProvider> {
            self.persist.lock().unwrap().push(persist);
            self.unlimited.lock().unwrap().push(unlimited_opt_in);
            self.credentials.update(|all| all.push(credential.clone()));
            let mut provider = BalanceProvider::credits(1_250, Some("creator_plus"));
            if let Credential::ApiKey { id, .. } = credential {
                provider.kind = ProviderKind::ApiKey;
                provider.owner = Some(id);
            }
            provider.capabilities = Ok(Capabilities::new(
                BTreeSet::from([Mode::MotionTransfer]),
                BTreeSet::from([Resolution::P480]),
                CostUnit::Credits,
                None,
            ));
            Arc::new(PricedProvider(provider))
        }

        fn build_for_create(
            &self,
            credential: Credential,
            persist: PersistTokens,
            unlimited_opt_in: UnlimitedOptIn,
            constraints: ConstraintsReader,
        ) -> (
            crate::provider::BuiltProvider,
            Option<Arc<dyn jutsu_core::ImageProvider>>,
            CreateAccount,
        ) {
            self.constraints.lock().unwrap().push(constraints);
            let (built, images) = self.build_with_images(credential, persist, unlimited_opt_in);
            (built, images, CreateAccount::default())
        }
    }

    fn platform(vault: Arc<MemoryVault>, factory: Arc<FakeFactory>) -> PlatformServices {
        PlatformServices {
            vault,
            connection: Arc::new(FakeServices::default()),
            providers: factory,
            connectivity: Arc::new(AlwaysOnline),
            media: Some(Arc::new(MissingFfmpeg)),
            utc_offset: UtcOffset::UTC,
        }
    }

    struct Fixture {
        folder: tempfile::TempDir,
        environment: Arc<AppEnvironment>,
        vault: Arc<MemoryVault>,
        factory: Arc<FakeFactory>,
    }

    async fn fixture() -> Fixture {
        let folder = tempfile::tempdir().unwrap();
        let vault = Arc::new(MemoryVault::new());
        let factory = Arc::new(FakeFactory::default());
        let environment = AppEnvironment::open(
            AppDirectories {
                data_dir: folder.path().join("data"),
                default_output_dir: folder.path().join("Videos").join("Jutsu Studio"),
            },
            platform(vault.clone(), factory.clone()),
        )
        .await
        .unwrap();
        Fixture {
            folder,
            environment,
            vault,
            factory,
        }
    }

    #[tokio::test]
    async fn a_fresh_profile_opens_in_a_temp_dir_disconnected() {
        let fixture = fixture().await;
        let environment = &fixture.environment;
        assert!(environment.layout.store_path().exists());
        environment.start().await;
        assert!(environment.provider.current().is_none());
        let snapshot = environment.connection_snapshot();
        assert_eq!(snapshot.kind, None);
        assert_eq!(snapshot.plate, None);
        assert_eq!(environment.connection_details().await.rows, []);
    }

    #[tokio::test]
    async fn media_roots_follow_the_output_folders() {
        let fixture = fixture().await;
        let environment = &fixture.environment;
        let roots = environment.media_roots();
        assert_eq!(roots[0], environment.layout.default_output_folder);
        assert!(roots.contains(&environment.layout.cache_folder()));

        environment
            .preferences
            .update(crate::Preferences {
                output_folder: Some("/tmp/picked".to_owned()),
                previous_output_folders: Some(vec!["/tmp/earlier".to_owned()]),
                ..Default::default()
            })
            .unwrap();
        let roots = environment.media_roots();
        assert!(roots.contains(&PathBuf::from("/tmp/picked")));
        assert!(roots.contains(&PathBuf::from("/tmp/earlier")));
    }

    #[tokio::test]
    async fn connecting_builds_the_provider_and_reads_the_balance() {
        let fixture = fixture().await;
        let environment = &fixture.environment;
        environment.start().await;
        let before = environment.provider.generation();

        environment.connection.start_sign_in().await.unwrap();

        let provider = environment.provider.current().unwrap();
        assert_eq!(provider.kind(), ProviderKind::Account);
        assert!(environment.provider.generation() > before);
        eventually(|| {
            environment
                .connection
                .state()
                .account_info()
                .is_some_and(|info| info.credits == Some(1_250))
        })
        .await;
        let plate = environment.connection_snapshot().plate.unwrap();
        assert_eq!(plate.badge, "Creator Plus");
        assert_eq!(plate.amount, "1,250");
        let details = environment.connection_details().await;
        assert_eq!(details.rows.last().unwrap().value, "Motion transfer");
    }

    #[tokio::test]
    async fn signing_out_drops_the_provider() {
        let fixture = fixture().await;
        let environment = &fixture.environment;
        environment
            .connection
            .connect_api_key(&ApiKeyForm::new("kid", "secret"))
            .await;
        assert_eq!(
            environment.provider.current().unwrap().kind(),
            ProviderKind::ApiKey
        );
        environment.connection.sign_out().await;
        assert!(environment.provider.current().is_none());
        assert_eq!(environment.connection.state(), ConnectionState::None);
    }

    #[tokio::test]
    async fn the_restored_connection_is_handed_over_on_start() {
        let fixture = fixture().await;
        fixture
            .environment
            .connection
            .connect_api_key(&ApiKeyForm::new("kid", "secret"))
            .await;
        drop(fixture.environment);
        let folder = fixture.folder;
        let reopened = AppEnvironment::open(
            AppDirectories {
                data_dir: folder.path().join("data"),
                default_output_dir: folder.path().join("Videos").join("Jutsu Studio"),
            },
            platform(fixture.vault.clone(), fixture.factory.clone()),
        )
        .await
        .unwrap();
        // The vault isn't read until `start`: the window shows neither onboarding nor the connection meanwhile.
        assert_eq!(reopened.connection.state(), ConnectionState::None);
        assert!(reopened.connection_snapshot().is_restoring);
        reopened.start().await;
        assert!(!reopened.connection_snapshot().is_restoring);
        assert_eq!(
            reopened.connection.state(),
            ConnectionState::ApiKey { key_id: "kid".into() }
        );
        assert_eq!(
            fixture.factory.credentials.value().last(),
            Some(&Credential::ApiKey {
                id: "kid".into(),
                secret: "secret".into()
            })
        );
        assert!(reopened.provider.current().is_some());
    }

    #[tokio::test]
    async fn refreshed_tokens_are_written_back_until_the_connection_changes() {
        let fixture = fixture().await;
        let environment = &fixture.environment;
        environment.connection.start_sign_in().await.unwrap();
        let persist = fixture.factory.persist.lock().unwrap().last().unwrap().clone();

        persist(fixture::tokens("access-2")).await;
        assert_eq!(
            fixture.vault.credential(),
            Some(Credential::Oauth(fixture::tokens("access-2")))
        );

        environment.connection.sign_out().await;
        persist(fixture::tokens("access-3")).await;
        assert_eq!(fixture.vault.credential(), None);
    }

    #[tokio::test]
    async fn requests_keep_to_the_limits_create_shows() {
        let fixture = fixture().await;
        fixture.environment.connection.start_sign_in().await.unwrap();
        let constraints = fixture
            .factory
            .constraints
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        // Nothing cached for the account: the built-in table, as the form shows.
        assert_eq!(constraints(), jutsu_core::ModelConstraintTable::built_in());
        assert_eq!(constraints(), fixture.environment.create.model_constraints());
    }

    #[tokio::test]
    async fn the_unlimited_opt_in_reads_the_current_setting() {
        let fixture = fixture().await;
        let environment = &fixture.environment;
        environment.connection.start_sign_in().await.unwrap();
        let opt_in = fixture.factory.unlimited.lock().unwrap().last().unwrap().clone();
        assert!(!opt_in());
        environment
            .preferences
            .update(Preferences {
                use_unlimited: Some(true),
                ..Preferences::default()
            })
            .unwrap();
        assert!(opt_in());
    }

    #[tokio::test]
    async fn a_refused_credential_needs_attention() {
        let fixture = fixture().await;
        let environment = &fixture.environment;
        environment.connection.start_sign_in().await.unwrap();
        let mut refusing = BalanceProvider::credits(1, None);
        refusing.capabilities = Err(ProviderError::Unauthorized.into());
        environment.provider.set(Some(Arc::new(refusing)));
        assert_eq!(environment.capabilities().await, None);
        assert!(environment.connection.needs_attention());
        assert!(environment.connection_snapshot().needs_attention);
    }

    #[tokio::test]
    async fn a_job_without_ffmpeg_fails_in_the_queue_with_a_clear_message() {
        let fixture = fixture().await;
        let environment = &fixture.environment;
        environment.start().await;
        environment
            .connection
            .connect_api_key(&ApiKeyForm::new("kid", "secret"))
            .await;
        let mut changes = environment.queue.subscribe();
        changes.mark_unchanged();

        let job = environment
            .engine
            .enqueue(spec(), EnqueueOptions::default())
            .await
            .unwrap();

        let now = OffsetDateTime::now_utc();
        let mut problems = Vec::new();
        for _ in 0..500 {
            problems = environment.queue.snapshot(now).await.problems;
            if !problems.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(changes.has_changed().unwrap());
        let [QueueEntryModel::Job { row }] = problems.as_slice() else {
            panic!("expected one failed job, got {problems:?}");
        };
        assert_eq!(row.model.id, job.id.to_string());
        assert_eq!(row.model.status_label, "Failed");
        assert!(row.model.note.as_ref().unwrap().text.contains("ffmpeg"));
        assert_eq!(environment.queue.active_count(), 0);
        assert!(matches!(environment.queue.snapshot(now).await.problems.len(), 1));
    }

    #[tokio::test]
    async fn duplicating_queues_the_same_request_again() {
        let fixture = fixture().await;
        let environment = &fixture.environment;
        environment.start().await;
        environment
            .connection
            .connect_api_key(&ApiKeyForm::new("kid", "secret"))
            .await;
        let job = environment
            .engine
            .enqueue(spec(), EnqueueOptions::default())
            .await
            .unwrap();
        eventually(|| environment.queue.active_count() == 0).await;

        let outcome = environment
            .queue
            .duplicate(job.id, false, OffsetDateTime::now_utc())
            .await;
        assert_eq!(
            outcome,
            DuplicateOutcome::Queued {
                notice: "Final queued · $1.00".to_owned()
            }
        );

        environment
            .preferences
            .update(Preferences {
                spend_cap_enabled: Some(true),
                spend_cap_usd: Some(0.5),
                ..Preferences::default()
            })
            .unwrap();
        let DuplicateOutcome::Confirm { confirmation } = environment
            .queue
            .duplicate(job.id, false, OffsetDateTime::now_utc())
            .await
        else {
            panic!("over the cap asks first");
        };
        assert_eq!(confirmation.title, "Over today's spend cap");
        assert_eq!(confirmation.confirm_title, "Start anyway");
    }

    #[tokio::test]
    async fn a_refused_command_becomes_the_connection_alert() {
        let fixture = fixture().await;
        let environment = &fixture.environment;
        environment.start().await;
        environment
            .queue
            .perform(Uuid::new_v4(), crate::queue::JobCommand::Retry)
            .await;
        assert_eq!(
            environment.connection.alert().unwrap().message,
            "This job is no longer in the queue."
        );
    }

    #[tokio::test]
    async fn stopping_returns_without_running_jobs() {
        let fixture = fixture().await;
        fixture.environment.start().await;
        tokio::time::timeout(std::time::Duration::from_secs(2), fixture.environment.stop())
            .await
            .unwrap();
    }

    #[test]
    fn results_land_in_videos_under_the_output_folder() {
        let layout = FileLayout::new("/tmp/data", "/tmp/Videos/Jutsu Studio");
        assert_eq!(
            output_directory(&layout, &Preferences::default()),
            PathBuf::from("/tmp/Videos/Jutsu Studio/Videos")
        );
        let picked = Preferences {
            output_folder: Some("/tmp/picked".to_owned()),
            ..Preferences::default()
        };
        assert_eq!(
            output_directory(&layout, &picked),
            PathBuf::from("/tmp/picked/Videos")
        );
    }
}
