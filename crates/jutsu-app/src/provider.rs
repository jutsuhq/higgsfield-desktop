//! The provider of the active connection, behind a handle the features and the engine read, and the factory that
//! builds it from a credential (`ProviderFactory` in `HiggsfieldConnection.swift`).

use std::sync::Arc;

use futures::future::BoxFuture;
use higgsfield::api::HiggsfieldApiClient;
use higgsfield::mcp::McpClient;
use higgsfield::mcp::account::HiggsfieldAccount;
use higgsfield::mcp::oauth::TokenProvider;
use jutsu_core::{
    AccountElements, AccountUploads, CharacterSheetMaker, Credential, GenerationHistory, GenerationLikes,
    GenerationProvider, ImageProvider, ModelConstraintTable, ModelConstraintsSource, MotionLibrary,
    OAuthTokens, ViralityScoring,
};
use jutsu_providers::{HiggsfieldApiProvider, HiggsfieldMcpProvider, oauth_tokens, token_set};
use tokio::sync::watch;

use crate::connection::{AccountAuthorization, higgsfield_urls};

/// The provider of the active connection; `None` while disconnected.
pub type ActiveProvider = Option<Arc<dyn GenerationProvider>>;

/// The active provider and a generation that goes up whenever it is replaced, a new sign-in to the same account
/// included. Clones share it. Readers take `current()`; the engine can `subscribe()` and follow every change.
#[derive(Clone)]
pub struct ProviderHandle {
    sender: Arc<watch::Sender<(u64, ActiveProvider)>>,
}

impl Default for ProviderHandle {
    fn default() -> Self {
        Self {
            sender: Arc::new(watch::channel((0, None)).0),
        }
    }
}

impl std::fmt::Debug for ProviderHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (generation, provider) = &*self.sender.borrow();
        formatter
            .debug_struct("ProviderHandle")
            .field("generation", generation)
            .field("kind", &provider.as_ref().map(|provider| provider.kind()))
            .finish()
    }
}

impl ProviderHandle {
    pub fn current(&self) -> ActiveProvider {
        self.sender.borrow().1.clone()
    }

    /// Goes up whenever the provider is replaced. Provider-backed loads key on it rather than on the connection's
    /// state, which also changes with every balance refresh.
    pub fn generation(&self) -> u64 {
        self.sender.borrow().0
    }

    pub fn set(&self, provider: ActiveProvider) {
        self.sender.send_modify(|(generation, current)| {
            *generation += 1;
            *current = provider;
        });
    }

    pub fn subscribe(&self) -> watch::Receiver<(u64, ActiveProvider)> {
        self.sender.subscribe()
    }
}

/// Saves tokens refreshed for the connection the provider was built for, and drops them once it is no longer the
/// connection's.
pub type PersistTokens = Arc<dyn Fn(OAuthTokens) -> BoxFuture<'static, ()> + Send + Sync>;
/// The account's Unlimited generations setting, read when a job is queued and sent.
pub type UnlimitedOptIn = Arc<dyn Fn() -> bool + Send + Sync>;
/// The model limits Create cached for the connected account (`ModelConstraintStore`), or the built-in table; read
/// when a request is checked and sent.
pub type ConstraintsReader = Arc<dyn Fn() -> ModelConstraintTable + Send + Sync>;

/// Builds the provider for a credential.
pub trait ProviderFactory: Send + Sync {
    fn provider(
        &self,
        credential: Credential,
        persist: PersistTokens,
        unlimited_opt_in: UnlimitedOptIn,
    ) -> Arc<dyn GenerationProvider>;

    /// The provider with the account's history and likes, which only an account connection has.
    fn build(
        &self,
        credential: Credential,
        persist: PersistTokens,
        unlimited_opt_in: UnlimitedOptIn,
    ) -> BuiltProvider {
        BuiltProvider {
            provider: self.provider(credential, persist, unlimited_opt_in),
            history: None,
            likes: None,
            virality: None,
            voices: None,
            credits: None,
        }
    }

    /// `build`, and the image side of the same object; `None` when the connection makes no images.
    fn build_with_images(
        &self,
        credential: Credential,
        persist: PersistTokens,
        unlimited_opt_in: UnlimitedOptIn,
    ) -> (BuiltProvider, Option<Arc<dyn ImageProvider>>) {
        (self.build(credential, persist, unlimited_opt_in), None)
    }

    /// `build_with_images`, and the account services Create offers beside the provider; none for an API key. An
    /// account's requests keep to the limits `constraints` reads.
    fn build_for_create(
        &self,
        credential: Credential,
        persist: PersistTokens,
        unlimited_opt_in: UnlimitedOptIn,
        _constraints: ConstraintsReader,
    ) -> (BuiltProvider, Option<Arc<dyn ImageProvider>>, CreateAccount) {
        let (built, images) = self.build_with_images(credential, persist, unlimited_opt_in);
        (built, images, CreateAccount::default())
    }
}

/// The account services Create uses beside the provider: the Motion Library, the uploads and elements of
/// References › From Higgsfield, character sheets, and the account's model limits. All `None` for an API key.
#[derive(Clone, Default)]
pub struct CreateAccount {
    pub motion: Option<Arc<dyn MotionLibrary>>,
    pub uploads: Option<Arc<dyn AccountUploads>>,
    pub elements: Option<Arc<dyn AccountElements>>,
    pub characters: Option<Arc<dyn CharacterSheetMaker>>,
    pub constraints: Option<Arc<dyn ModelConstraintsSource>>,
}

impl std::fmt::Debug for CreateAccount {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CreateAccount")
            .field("motion", &self.motion.is_some())
            .field("elements", &self.elements.is_some())
            .finish_non_exhaustive()
    }
}

/// A provider and the account services the same object offers.
#[derive(Clone)]
pub struct BuiltProvider {
    pub provider: Arc<dyn GenerationProvider>,
    pub history: Option<Arc<dyn GenerationHistory>>,
    pub likes: Option<Arc<dyn GenerationLikes>>,
    /// Virality scores of the account's results.
    pub virality: Option<Arc<dyn ViralityScoring>>,
    /// The voices Change Voice offers.
    pub voices: Option<Arc<dyn jutsu_core::VoiceLibrary>>,
    /// What the account was charged and refunded.
    pub credits: Option<Arc<dyn jutsu_core::CreditHistory>>,
}

/// `HiggsfieldApiProvider` for an API key; `HiggsfieldMcpProvider` with a `TokenProvider` that refreshes at the
/// issuing server and writes refreshed tokens back through `persist` for an account.
#[derive(Debug, Clone, Default)]
pub struct HiggsfieldProviderFactory {
    http: reqwest::Client,
}

impl ProviderFactory for HiggsfieldProviderFactory {
    fn provider(
        &self,
        credential: Credential,
        persist: PersistTokens,
        unlimited_opt_in: UnlimitedOptIn,
    ) -> Arc<dyn GenerationProvider> {
        match credential {
            Credential::ApiKey { id, secret } => Arc::new(HiggsfieldApiProvider::new(
                HiggsfieldApiClient::new(id, &secret),
                None,
            )),
            Credential::Oauth(tokens) => Arc::new(self.account(tokens, persist, unlimited_opt_in)),
        }
    }

    fn build(
        &self,
        credential: Credential,
        persist: PersistTokens,
        unlimited_opt_in: UnlimitedOptIn,
    ) -> BuiltProvider {
        self.build_with_images(credential, persist, unlimited_opt_in).0
    }

    fn build_with_images(
        &self,
        credential: Credential,
        persist: PersistTokens,
        unlimited_opt_in: UnlimitedOptIn,
    ) -> (BuiltProvider, Option<Arc<dyn ImageProvider>>) {
        let built_in: ConstraintsReader = Arc::new(ModelConstraintTable::built_in);
        let (built, images, _) = self.build_for_create(credential, persist, unlimited_opt_in, built_in);
        (built, images)
    }

    fn build_for_create(
        &self,
        credential: Credential,
        persist: PersistTokens,
        unlimited_opt_in: UnlimitedOptIn,
        constraints: ConstraintsReader,
    ) -> (BuiltProvider, Option<Arc<dyn ImageProvider>>, CreateAccount) {
        match credential {
            Credential::Oauth(tokens) => {
                let provider = Arc::new(self.account(tokens, persist, unlimited_opt_in).with_constraints(
                    move || {
                        let table = constraints();
                        async move { table }
                    },
                ));
                let built = BuiltProvider {
                    provider: provider.clone(),
                    history: Some(provider.clone()),
                    likes: Some(provider.clone()),
                    virality: Some(provider.clone()),
                    voices: Some(provider.clone()),
                    credits: Some(provider.clone()),
                };
                let account = CreateAccount {
                    motion: Some(provider.clone()),
                    uploads: Some(provider.clone()),
                    elements: Some(provider.clone()),
                    characters: Some(provider.clone()),
                    constraints: Some(provider.clone()),
                };
                (built, Some(provider), account)
            }
            Credential::ApiKey { id, secret } => {
                let provider = Arc::new(HiggsfieldApiProvider::new(
                    HiggsfieldApiClient::new(id, &secret),
                    None,
                ));
                let built = BuiltProvider {
                    provider: provider.clone(),
                    history: None,
                    likes: None,
                    virality: None,
                    voices: None,
                    credits: None,
                };
                (built, Some(provider), CreateAccount::default())
            }
        }
    }
}

impl HiggsfieldProviderFactory {
    fn account(
        &self,
        tokens: OAuthTokens,
        persist: PersistTokens,
        unlimited_opt_in: UnlimitedOptIn,
    ) -> HiggsfieldMcpProvider {
        let owner = tokens.owner().map(str::to_owned);
        let http = self.http.clone();
        let tokens = TokenProvider::new(
            token_set(tokens),
            move |tokens| {
                let http = http.clone();
                async move { AccountAuthorization::refresh(&http, &tokens).await }
            },
            move |tokens| persist(oauth_tokens(tokens)),
        );
        let client = McpClient::new(higgsfield_urls::MCP_ENDPOINT.clone(), tokens);
        HiggsfieldMcpProvider::new(HiggsfieldAccount::new(Arc::new(client)), owner).with_unlimited_opt_in(
            move || {
                let opted_in = unlimited_opt_in();
                async move { opted_in }
            },
        )
    }
}
