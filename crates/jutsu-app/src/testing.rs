//! Fakes for the crate's tests (`Fakes.swift`, `Fixture`).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use futures::future::BoxFuture;
use jutsu_core::{
    Balance, Capabilities, Cost, GenerationProvider, JobSpec, OAuthTokens, PortError, PreparedFile,
    ProviderError, ProviderKind, RemoteAsset, RemoteInputs, RemoteJob, RemoteStatus, StylePreset,
};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use crate::connection::{ConnectionMetadata, ConnectionMetadataStore, ConnectionServices, ShowUrl};

/// A shared value tests read and change.
#[derive(Debug, Default)]
pub struct Shared<T>(Mutex<T>);

impl<T: Clone> Shared<T> {
    pub fn new(value: T) -> Arc<Self> {
        Arc::new(Self(Mutex::new(value)))
    }

    pub fn value(&self) -> T {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn update(&self, change: impl FnOnce(&mut T)) {
        change(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner));
    }
}

pub mod fixture {
    use super::{OAuthTokens, Url};

    pub fn sign_in_page() -> Url {
        Url::parse("https://auth.example/authorize?state=s").unwrap()
    }

    pub fn tokens(access: &str) -> OAuthTokens {
        tokens_for(access, None, None)
    }

    pub fn tokens_for(access: &str, email: Option<&str>, name: Option<&str>) -> OAuthTokens {
        OAuthTokens {
            access_token: access.to_owned(),
            refresh_token: Some("refresh".to_owned()),
            expires_at: time::OffsetDateTime::from_unix_timestamp(2_000_000_000).unwrap(),
            scope: "openid email offline_access".to_owned(),
            client_id: "client".to_owned(),
            issuer: Some(Url::parse("https://clerk.higgsfield.ai").unwrap()),
            email: email.map(str::to_owned),
            name: name.map(str::to_owned),
            subject: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct FakeMetadataStore {
    stored: Mutex<Option<ConnectionMetadata>>,
}

impl FakeMetadataStore {
    pub fn new(stored: Option<ConnectionMetadata>) -> Arc<Self> {
        Arc::new(Self {
            stored: Mutex::new(stored),
        })
    }

    pub fn stored(&self) -> Option<ConnectionMetadata> {
        self.stored.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl ConnectionMetadataStore for FakeMetadataStore {
    fn load(&self) -> Option<ConnectionMetadata> {
        self.stored()
    }

    fn save(&self, metadata: &ConnectionMetadata) {
        *self.stored.lock().unwrap_or_else(PoisonError::into_inner) = Some(metadata.clone());
    }

    fn clear(&self) {
        *self.stored.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

pub type Validate = Arc<dyn Fn(String, String) -> BoxFuture<'static, Result<bool, PortError>> + Send + Sync>;
pub type Browser =
    Arc<dyn Fn(CancellationToken) -> BoxFuture<'static, Result<OAuthTokens, PortError>> + Send + Sync>;

/// `FakeServices`: accepts every key and signs in with `fixture::tokens("access")` unless told otherwise; shows
/// `fixture::sign_in_page()` before the browser answers.
pub struct FakeServices {
    pub validate: Validate,
    pub browser: Browser,
    /// The `previous` tokens of every sign-in, in order.
    pub previous: Arc<Shared<Vec<Option<OAuthTokens>>>>,
    pub revoked: Arc<Shared<Vec<OAuthTokens>>>,
}

impl Default for FakeServices {
    fn default() -> Self {
        Self {
            validate: Arc::new(|_, _| Box::pin(async { Ok(true) })),
            browser: Arc::new(|_| Box::pin(async { Ok(fixture::tokens("access")) })),
            previous: Shared::new(Vec::new()),
            revoked: Shared::new(Vec::new()),
        }
    }
}

impl FakeServices {
    pub fn validating<F, Fut>(validate: F) -> Self
    where
        F: Fn(String, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<bool, PortError>> + Send + 'static,
    {
        Self {
            validate: Arc::new(move |id, secret| Box::pin(validate(id, secret))),
            ..Self::default()
        }
    }

    pub fn browsing<F, Fut>(browser: F) -> Self
    where
        F: Fn(CancellationToken) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<OAuthTokens, PortError>> + Send + 'static,
    {
        Self {
            browser: Arc::new(move |cancel| Box::pin(browser(cancel))),
            ..Self::default()
        }
    }
}

#[async_trait]
impl ConnectionServices for FakeServices {
    async fn validate_api_key(&self, id: &str, secret: &str) -> Result<bool, PortError> {
        (self.validate)(id.to_owned(), secret.to_owned()).await
    }

    async fn sign_in_with_browser(
        &self,
        previous: Option<OAuthTokens>,
        show: ShowUrl,
        cancel: CancellationToken,
    ) -> Result<OAuthTokens, PortError> {
        self.previous.update(|all| all.push(previous));
        show(fixture::sign_in_page());
        (self.browser)(cancel).await
    }

    async fn revoke(&self, tokens: OAuthTokens) {
        self.revoked.update(|all| all.push(tokens));
    }
}

/// A one-shot gate a fake waits on until the test opens it.
#[derive(Debug, Default)]
pub struct Gate<T> {
    waiting: std::sync::atomic::AtomicBool,
    value: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<T>>>,
    sender: Mutex<Option<tokio::sync::oneshot::Sender<T>>>,
}

impl<T: Send + 'static> Gate<T> {
    pub fn new() -> Arc<Self> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        Arc::new(Self {
            waiting: std::sync::atomic::AtomicBool::new(false),
            value: tokio::sync::Mutex::new(Some(receiver)),
            sender: Mutex::new(Some(sender)),
        })
    }

    pub async fn wait(&self) -> Option<T> {
        self.waiting.store(true, std::sync::atomic::Ordering::SeqCst);
        let receiver = self.value.lock().await.take()?;
        receiver.await.ok()
    }

    pub fn is_waiting(&self) -> bool {
        self.waiting.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn open(&self, value: T) {
        if let Some(sender) = self.sender.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = sender.send(value);
        }
    }
}

/// Polls `condition` until it holds, for at most five seconds.
pub async fn eventually(condition: impl Fn() -> bool) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the condition never held");
}

pub type Answer = Arc<dyn Fn() -> BoxFuture<'static, Result<Option<Balance>, PortError>> + Send + Sync>;

/// A provider that only answers `balance()` and `capabilities()`; everything else is refused.
pub struct BalanceProvider {
    pub kind: ProviderKind,
    pub owner: Option<String>,
    pub answer: Answer,
    pub capabilities: Result<Capabilities, PortError>,
}

impl BalanceProvider {
    pub fn new<F, Fut>(answer: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<Balance>, PortError>> + Send + 'static,
    {
        Self {
            kind: ProviderKind::Account,
            owner: Some("user_1".to_owned()),
            answer: Arc::new(move || Box::pin(answer())),
            capabilities: Err(ProviderError::ModelUnavailable.into()),
        }
    }

    pub fn credits(credits: i64, plan: Option<&str>) -> Self {
        let plan = plan.map(str::to_owned);
        Self::new(move || {
            let plan = plan.clone();
            async move { Ok(Some(Balance::Credits { credits, plan })) }
        })
    }
}

fn refused<T>() -> Result<T, PortError> {
    Err(ProviderError::ModelUnavailable.into())
}

#[async_trait]
impl GenerationProvider for BalanceProvider {
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    fn owner(&self) -> Option<String> {
        self.owner.clone()
    }
    async fn capabilities(&self) -> Result<Capabilities, PortError> {
        self.capabilities.clone()
    }
    async fn presets(&self) -> Result<Vec<StylePreset>, PortError> {
        Ok(Vec::new())
    }
    async fn estimate(&self, _spec: &JobSpec) -> Result<Cost, PortError> {
        refused()
    }
    async fn upload(&self, _file: PreparedFile) -> Result<RemoteAsset, PortError> {
        refused()
    }
    async fn submit(
        &self,
        _spec: &JobSpec,
        _inputs: &RemoteInputs,
        _key: Uuid,
    ) -> Result<RemoteJob, PortError> {
        refused()
    }
    async fn submit_is_idempotent(&self, _spec: &JobSpec) -> Result<bool, PortError> {
        Ok(true)
    }
    async fn status(&self, _job: &RemoteJob) -> Result<RemoteStatus, PortError> {
        refused()
    }
    async fn cancel(&self, _job: &RemoteJob) -> Result<(), PortError> {
        refused()
    }
    async fn balance(&self) -> Result<Option<Balance>, PortError> {
        (self.answer)().await
    }
}

/// `path` made absolute on every system: the tests' Unix paths (`/tmp/clip.mp4`) get a drive on Windows.
pub fn absolute(path: impl AsRef<std::path::Path>) -> std::path::PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::path::PathBuf::from(format!("C:{}", path.to_string_lossy().replace('/', "\\")))
    }
}

/// `path` as the system writes it: `absolute`, as text.
pub fn native(path: &str) -> String {
    absolute(path).to_string_lossy().into_owned()
}

/// The `file:` URL of `path`, made absolute on every system (`absolute`).
pub fn file_url(path: impl AsRef<std::path::Path>) -> Url {
    Url::from_file_path(absolute(path)).expect("an absolute path")
}

/// An agent socket in `folder`; on Windows a pipe of its own, its inbox in the folder.
pub fn agent_endpoint(folder: &std::path::Path) -> jutsu_agent::AgentEndpoint {
    let mut endpoint = jutsu_agent::AgentEndpoint::at(folder.join("a.sock"));
    if cfg!(windows) {
        let unique = folder.file_name().expect("folder name").to_string_lossy();
        endpoint.name = std::path::PathBuf::from(format!(
            "{}jutsu-test-{unique}",
            jutsu_agent::socket::PIPE_NAMESPACE
        ));
    }
    endpoint
}
