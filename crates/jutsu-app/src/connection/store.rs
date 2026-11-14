//! Holds the one connection or none (`ConnectionStore.swift`). The credential goes only to the vault; the metadata
//! store keeps what the UI shows. Every change of credential is reported through the credential-change hook, which
//! the environment uses to build the provider.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::future::BoxFuture;
use jutsu_core::{Balance, Credential, GenerationProvider, OAuthTokens, ProviderKind};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::messages::UserMessage;
use super::ports::{ConnectionMetadataStore, ConnectionServices, CredentialVault, VaultError};
use super::state::{AccountInfo, ApiKeyForm, ConnectionState, credential_matches};

/// Called with the new credential after every connect and with `None` after sign-out.
pub type CredentialChange = Arc<dyn Fn(Option<Credential>) -> BoxFuture<'static, ()> + Send + Sync>;

/// What the store is busy with.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum Activity {
    #[default]
    Idle,
    ValidatingKey,
    /// Sign-in is open in the browser; the page's URL once it is known.
    SigningIn(Option<Url>),
}

impl Activity {
    /// Whether the sign-in sheet is up.
    pub fn is_signing_in(&self) -> bool {
        matches!(self, Self::SigningIn(_))
    }
}

struct Inner {
    state: ConnectionState,
    activity: Activity,
    needs_attention: bool,
    alert: Option<UserMessage>,
    /// The vault hasn't been read yet (`restoring`).
    is_restoring: bool,
    /// Bumped on every start and cancel, so a sign-in that finishes late can't apply its result.
    attempt: u64,
    /// Bumped whenever the credential changes, so a balance read or token refresh for the previous one can't land.
    generation: u64,
    sign_in: Option<CancellationToken>,
    /// The revocation started by the last sign-out, for tests to wait on.
    revocation: Option<JoinHandle<()>>,
}

/// Holds the one connection or none.
pub struct ConnectionStore {
    vault: Arc<dyn CredentialVault>,
    metadata: Arc<dyn ConnectionMetadataStore>,
    services: Arc<dyn ConnectionServices>,
    inner: Mutex<Inner>,
    on_credential_change: Mutex<CredentialChange>,
    changes: watch::Sender<u64>,
}

impl std::fmt::Debug for ConnectionStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.locked();
        formatter
            .debug_struct("ConnectionStore")
            .field("state", &inner.state)
            .field("activity", &inner.activity)
            .finish_non_exhaustive()
    }
}

impl ConnectionStore {
    /// Restores the connection from the metadata, provided the vault still holds the credential it describes.
    pub fn new(
        vault: Arc<dyn CredentialVault>,
        metadata: Arc<dyn ConnectionMetadataStore>,
        services: Arc<dyn ConnectionServices>,
    ) -> Self {
        let (state, alert) = restored(metadata.as_ref(), vault.load_credential());
        Self::with_state(vault, metadata, services, state, alert, false)
    }

    /// Leaves the vault unread until `activate`, which reads it off the caller's task: the OS credential store can
    /// block for seconds (a locked keyring asks for its password). Until then `is_restoring` is set and the state is
    /// `None`, so the window shows neither onboarding nor the connection.
    pub fn restoring(
        vault: Arc<dyn CredentialVault>,
        metadata: Arc<dyn ConnectionMetadataStore>,
        services: Arc<dyn ConnectionServices>,
    ) -> Self {
        Self::with_state(vault, metadata, services, ConnectionState::None, None, true)
    }

    fn with_state(
        vault: Arc<dyn CredentialVault>,
        metadata: Arc<dyn ConnectionMetadataStore>,
        services: Arc<dyn ConnectionServices>,
        state: ConnectionState,
        alert: Option<UserMessage>,
        is_restoring: bool,
    ) -> Self {
        Self {
            vault,
            metadata,
            services,
            inner: Mutex::new(Inner {
                state,
                activity: Activity::Idle,
                needs_attention: false,
                alert,
                is_restoring,
                attempt: 0,
                generation: 0,
                sign_in: None,
                revocation: None,
            }),
            on_credential_change: Mutex::new(Arc::new(|_| Box::pin(async {}))),
            changes: watch::channel(0).0,
        }
    }

    /// Sets the hook called after every change of credential.
    pub fn set_on_credential_change(&self, hook: CredentialChange) {
        *self
            .on_credential_change
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = hook;
    }

    /// A revision that goes up whenever anything the UI shows changes.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub fn state(&self) -> ConnectionState {
        self.locked().state.clone()
    }

    pub fn activity(&self) -> Activity {
        self.locked().activity.clone()
    }

    /// The provider refused the stored credential (401, refresh refused); the user has to sign in again or replace
    /// the key. The connection stays until they do.
    pub fn needs_attention(&self) -> bool {
        self.locked().needs_attention
    }

    /// The alert to show.
    pub fn alert(&self) -> Option<UserMessage> {
        self.locked().alert.clone()
    }

    /// Whether the stored connection is still being read (`restoring`).
    pub fn is_restoring(&self) -> bool {
        self.locked().is_restoring
    }

    /// Shows `message` as the alert, as a failed queue action does.
    pub fn show_alert(&self, message: UserMessage) {
        self.update(|inner| inner.alert = Some(message));
    }

    /// The alert was dismissed.
    pub fn dismiss_alert(&self) {
        self.update(|inner| inner.alert = None);
    }

    /// Reports the restored credential once on launch. A store that fails now ends like one failing at `new`:
    /// disconnected, with the metadata kept and an alert. A store made with `restoring` reads the vault here, on a
    /// blocking task.
    pub async fn activate(&self) {
        if self.locked().is_restoring {
            let vault = Arc::clone(&self.vault);
            let loaded = tokio::task::spawn_blocking(move || vault.load_credential())
                .await
                .unwrap_or_else(|error| Err(VaultError::Store(error.to_string())));
            let credential = loaded.as_ref().ok().cloned().flatten();
            let (state, alert) = restored(self.metadata.as_ref(), loaded);
            let is_connected = state.is_connected();
            self.update(|inner| {
                inner.state = state;
                inner.alert = alert;
                inner.is_restoring = false;
            });
            return self.credential_changed(credential.filter(|_| is_connected)).await;
        }
        if !self.locked().state.is_connected() {
            return self.credential_changed(None).await;
        }
        match self.vault.load_credential() {
            Ok(credential) => self.credential_changed(credential).await,
            Err(error) => {
                self.update(|inner| {
                    inner.state = ConnectionState::None;
                    inner.alert = Some(UserMessage::failure(error.to_string()));
                });
                self.credential_changed(None).await;
            }
        }
    }

    // MARK: API key

    pub async fn connect_api_key(&self, form: &ApiKeyForm) {
        {
            let mut inner = self.locked();
            if inner.activity != Activity::Idle {
                return;
            }
            let Some(Credential::ApiKey { .. }) = form.credential() else {
                inner.alert = Some(UserMessage::incomplete_key());
                drop(inner);
                return self.notify();
            };
            inner.activity = Activity::ValidatingKey;
        }
        self.notify();
        let Some(Credential::ApiKey { id, secret }) = form.credential() else {
            return;
        };
        match self.services.validate_api_key(&id, &secret).await {
            Ok(false) => self.update(|inner| inner.alert = Some(UserMessage::invalid_key())),
            Ok(true) => {
                let key_id = id.clone();
                if let Err(error) = self
                    .connect(
                        Credential::ApiKey { id, secret },
                        ConnectionState::ApiKey { key_id },
                    )
                    .await
                {
                    self.update(|inner| inner.alert = Some(UserMessage::failure(error.to_string())));
                }
            }
            Err(error) => {
                let alert = UserMessage::from_error(&error, Some(ProviderKind::ApiKey));
                self.update(|inner| inner.alert = alert);
            }
        }
        self.update(|inner| inner.activity = Activity::Idle);
    }

    // MARK: Account

    /// Starts signing in, replacing any sign-in already running. Also used to sign in again while connected.
    pub fn start_sign_in(self: &Arc<Self>) -> JoinHandle<()> {
        let (attempt, cancel) = {
            let mut inner = self.locked();
            if let Some(running) = inner.sign_in.take() {
                running.cancel();
            }
            inner.attempt += 1;
            inner.activity = Activity::SigningIn(None);
            let cancel = CancellationToken::new();
            inner.sign_in = Some(cancel.clone());
            (inner.attempt, cancel)
        };
        self.notify();
        let store = Arc::clone(self);
        tokio::spawn(async move { store.sign_in(attempt, cancel).await })
    }

    pub fn cancel_sign_in(&self) {
        {
            let mut inner = self.locked();
            if !inner.activity.is_signing_in() {
                return;
            }
            inner.attempt += 1;
            if let Some(running) = inner.sign_in.take() {
                running.cancel();
            }
            inner.activity = Activity::Idle;
        }
        self.notify();
    }

    /// Reuses the client registered by the stored sign-in, if any. A store that can't be read stops before the
    /// browser opens: the new sign-in couldn't be saved either, and registering again would leave another client.
    /// A stored value that doesn't decode is replaced by the new sign-in.
    async fn sign_in(self: Arc<Self>, attempt: u64, cancel: CancellationToken) {
        let previous = match self.vault.load_credential() {
            Ok(Some(Credential::Oauth(tokens))) => Some(tokens),
            Ok(_) | Err(VaultError::Decoding(_)) => None,
            Err(error) => {
                self.finish_sign_in(attempt, |inner| {
                    inner.alert = Some(UserMessage::failure(error.to_string()));
                });
                return;
            }
        };
        let weak = Arc::downgrade(&self);
        let show = Arc::new(move |url: Url| {
            if let Some(store) = weak.upgrade() {
                store.show(url, attempt);
            }
        });
        let result = self.services.sign_in_with_browser(previous, show, cancel).await;
        match result {
            Ok(tokens) => {
                let mut account = None;
                self.finish_sign_in(attempt, |inner| {
                    account = Some(account_signed_in_with(&inner.state, &tokens));
                });
                let Some(account) = account else { return };
                if let Err(error) = self
                    .connect(Credential::Oauth(tokens), ConnectionState::Account(account))
                    .await
                {
                    self.update(|inner| inner.alert = Some(UserMessage::failure(error.to_string())));
                }
            }
            Err(error) => {
                let alert = UserMessage::from_error(&error, Some(ProviderKind::Account));
                self.finish_sign_in(attempt, |inner| inner.alert = alert);
            }
        }
    }

    /// Ends sign-in `attempt` with `change`, unless it was cancelled or replaced meanwhile.
    fn finish_sign_in(&self, attempt: u64, change: impl FnOnce(&mut Inner)) {
        {
            let mut inner = self.locked();
            if inner.attempt != attempt {
                return;
            }
            inner.activity = Activity::Idle;
            inner.sign_in = None;
            change(&mut inner);
        }
        self.notify();
    }

    fn show(&self, url: Url, attempt: u64) {
        {
            let mut inner = self.locked();
            if inner.attempt != attempt {
                return;
            }
            inner.activity = Activity::SigningIn(Some(url));
        }
        self.notify();
    }

    /// Fills in what a provider call learned about the account; plan and e-mail are persisted.
    pub fn update_account(&self, info: AccountInfo) {
        let metadata = {
            let mut inner = self.locked();
            if !matches!(inner.state, ConnectionState::Account(_)) {
                return;
            }
            inner.state = ConnectionState::Account(info);
            inner.state.metadata()
        };
        if let Some(metadata) = metadata {
            self.metadata.save(&metadata);
        }
        self.notify();
    }

    /// `ultra` → "Ultra", `creator_plus` → "Creator Plus".
    pub fn plan_title(name: &str) -> String {
        name.replace('_', " ")
            .split(' ')
            .map(|word| {
                let mut characters = word.chars();
                characters.next().map_or_else(String::new, |first| {
                    first
                        .to_uppercase()
                        .chain(characters.flat_map(char::to_lowercase))
                        .collect()
                })
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Shows the credits `provider` reports for the signed-in account, and its plan when it names one. A provider
    /// that reports no credits, or fails, hides them rather than leave an old number up; the plan stays, since it
    /// rarely changes. An answer that arrives after a sign-out or another sign-in is dropped.
    pub async fn refresh_balance(&self, provider: &dyn GenerationProvider) {
        let generation = {
            let inner = self.locked();
            if !matches!(inner.state, ConnectionState::Account(_)) {
                return;
            }
            inner.generation
        };
        let (credits, plan) = match provider.balance().await {
            Ok(Some(Balance::Credits { credits, plan })) => {
                (Some(credits), plan.as_deref().map(Self::plan_title))
            }
            _ => (None, None),
        };
        let info = {
            let inner = self.locked();
            let ConnectionState::Account(info) = &inner.state else {
                return;
            };
            if inner.generation != generation {
                return;
            }
            let mut info = info.clone();
            info.credits = credits;
            if let Some(plan) = plan {
                info.plan = Some(plan);
            }
            info
        };
        self.update_account(info);
    }

    // MARK: Lifecycle

    /// The provider refused the credential. Ignored when not connected.
    pub fn mark_needs_attention(&self) {
        self.update(|inner| {
            if inner.state.is_connected() {
                inner.needs_attention = true;
            }
        });
    }

    /// Identifies the current credential for `save_refreshed_tokens`. Read it when building a provider.
    pub fn credential_generation(&self) -> u64 {
        self.locked().generation
    }

    /// Saves tokens refreshed for the credential of `generation`. Writes nothing and returns `false` when the user
    /// has signed out, signed in again or connected otherwise since: a late refresh must not bring old tokens back.
    pub fn save_refreshed_tokens(&self, tokens: OAuthTokens, generation: u64) -> Result<bool, VaultError> {
        let inner = self.locked();
        if inner.generation != generation || !matches!(inner.state, ConnectionState::Account(_)) {
            return Ok(false);
        }
        self.vault.save_credential(&Credential::Oauth(tokens))?;
        Ok(true)
    }

    /// Deletes the credential and goes back to onboarding. Stays connected if the store refuses the delete. An
    /// account's tokens are then revoked in the background; sign-out doesn't wait for it or need the network.
    /// Tokens that can't be read aren't revoked, and the alert says so.
    pub async fn sign_out(&self) {
        self.cancel_sign_in();
        let (signed_out, unreadable) = match self.vault.load_credential() {
            Ok(credential) => (credential, false),
            Err(_) => (None, true),
        };
        if let Err(error) = self.vault.delete() {
            return self.update(|inner| inner.alert = Some(UserMessage::failure(error.to_string())));
        }
        self.metadata.clear();
        let services = Arc::clone(&self.services);
        self.update(|inner| {
            if unreadable && matches!(inner.state, ConnectionState::Account(_)) {
                inner.alert = Some(UserMessage::not_revoked());
            }
            inner.state = ConnectionState::None;
            inner.generation += 1;
            inner.needs_attention = false;
            if let Some(Credential::Oauth(tokens)) = signed_out {
                inner.revocation = Some(tokio::spawn(async move { services.revoke(tokens).await }));
            }
        });
        self.credential_changed(None).await;
    }

    /// The revocation the last sign-out started, if any; for tests to wait on.
    pub fn take_revocation(&self) -> Option<JoinHandle<()>> {
        self.locked().revocation.take()
    }

    async fn connect(&self, credential: Credential, state: ConnectionState) -> Result<(), VaultError> {
        self.vault.save_credential(&credential)?;
        if let Some(metadata) = state.metadata() {
            self.metadata.save(&metadata);
        }
        self.update(|inner| {
            inner.state = state;
            inner.generation += 1;
            inner.needs_attention = false;
        });
        self.credential_changed(Some(credential)).await;
        Ok(())
    }

    async fn credential_changed(&self, credential: Option<Credential>) {
        let hook = self
            .on_credential_change
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        hook(credential).await;
    }

    fn update(&self, change: impl FnOnce(&mut Inner)) {
        change(&mut self.locked());
        self.notify();
    }

    fn notify(&self) {
        self.changes.send_modify(|revision| *revision += 1);
    }

    fn locked(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The connection the metadata describes, provided the vault holds its credential. A locked or failing store says
/// nothing about whether the connection exists, so the metadata is kept and an alert says why.
fn restored(
    metadata: &dyn ConnectionMetadataStore,
    loaded: Result<Option<Credential>, VaultError>,
) -> (ConnectionState, Option<UserMessage>) {
    let saved = metadata.load();
    match loaded {
        Ok(credential) => match (saved, credential) {
            (Some(saved), Some(credential)) if credential_matches(&credential, &saved) => {
                (ConnectionState::from_metadata(Some(&saved)), None)
            }
            _ => {
                metadata.clear();
                (ConnectionState::None, None)
            }
        },
        Err(error) => (
            ConnectionState::None,
            Some(UserMessage::failure(error.to_string())),
        ),
    }
}

/// The account the new tokens name. The plan carries over only when it is the same account; credits never do,
/// since the tokens may name another account without saying so. They are read again on connect.
fn account_signed_in_with(state: &ConnectionState, tokens: &OAuthTokens) -> AccountInfo {
    let mut account = state.account_info().cloned().unwrap_or_default();
    account.credits = None;
    if tokens.email.is_none() && tokens.name.is_none() {
        return account;
    }
    if tokens.email != account.email {
        account = AccountInfo::default();
    }
    account.email.clone_from(&tokens.email);
    account.name.clone_from(&tokens.name);
    account
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
