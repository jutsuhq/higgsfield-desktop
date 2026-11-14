use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::future::{BoxFuture, FutureExt, Shared};
use time::OffsetDateTime;

use super::client::Clock;
use super::token_set::TokenSet;
use crate::core::HiggsfieldError;
use crate::mcp::error::{McpError, OAuthError};

type Refresh = Arc<dyn Fn(TokenSet) -> BoxFuture<'static, Result<TokenSet, McpError>> + Send + Sync>;
type Persist = Arc<dyn Fn(TokenSet) -> BoxFuture<'static, ()> + Send + Sync>;
type Refreshing = Shared<BoxFuture<'static, Result<TokenSet, McpError>>>;

/// Hands out a valid access token: refreshes ahead of expiry, runs one refresh at a time for all callers, and
/// asks for a new sign-in only when the refresh itself is refused. Clones share the tokens.
#[derive(Clone)]
pub struct TokenProvider {
    state: Arc<Mutex<State>>,
    refresh: Refresh,
    persist: Persist,
    now: Clock,
    /// How long before `expires_at` a token already counts as expired.
    margin: time::Duration,
}

struct State {
    tokens: TokenSet,
    /// The refresh under way, which all callers join. It stores and persists its tokens itself, so a caller that
    /// stops waiting doesn't lose them.
    refreshing: Option<Refreshing>,
}

impl std::fmt::Debug for TokenProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TokenProvider")
            .field("margin", &self.margin)
            .finish_non_exhaustive()
    }
}

impl TokenProvider {
    pub const DEFAULT_MARGIN: time::Duration = time::Duration::seconds(60);

    /// `refresh` exchanges the refresh token; `persist` saves refreshed tokens (the app writes them to the
    /// keychain), and may drop the write when the user signed out or switched accounts meanwhile.
    pub fn new<R, RF, P, PF>(tokens: TokenSet, refresh: R, persist: P) -> Self
    where
        R: Fn(TokenSet) -> RF + Send + Sync + 'static,
        RF: Future<Output = Result<TokenSet, McpError>> + Send + 'static,
        P: Fn(TokenSet) -> PF + Send + Sync + 'static,
        PF: Future<Output = ()> + Send + 'static,
    {
        Self {
            state: Arc::new(Mutex::new(State {
                tokens,
                refreshing: None,
            })),
            refresh: Arc::new(move |tokens| refresh(tokens).boxed()),
            persist: Arc::new(move |tokens| persist(tokens).boxed()),
            now: Arc::new(OffsetDateTime::now_utc),
            margin: Self::DEFAULT_MARGIN,
        }
    }

    #[must_use]
    pub fn with_clock(mut self, now: Clock) -> Self {
        self.now = now;
        self
    }

    #[must_use]
    pub fn with_margin(mut self, margin: time::Duration) -> Self {
        self.margin = margin;
        self
    }

    /// The tokens as they are now.
    #[must_use]
    pub fn tokens(&self) -> TokenSet {
        self.locked().tokens.clone()
    }

    pub async fn access_token(&self) -> Result<String, McpError> {
        {
            let state = self.locked();
            if state.refreshing.is_none()
                && (self.now)() < state.tokens.expires_at.saturating_sub(self.margin)
            {
                return Ok(state.tokens.access_token.clone());
            }
        }
        Ok(self.refreshed().await?.access_token)
    }

    /// After a `401` for `rejected`: refreshes, unless another caller already replaced that token.
    pub async fn access_token_replacing(&self, rejected: &str) -> Result<String, McpError> {
        {
            let state = self.locked();
            if state.refreshing.is_none() && state.tokens.access_token != rejected {
                return Ok(state.tokens.access_token.clone());
            }
        }
        Ok(self.refreshed().await?.access_token)
    }

    fn locked(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn refreshed(&self) -> Result<TokenSet, McpError> {
        let refreshing = {
            let mut state = self.locked();
            if let Some(refreshing) = &state.refreshing {
                refreshing.clone()
            } else {
                if state.tokens.refresh_token.is_none() {
                    return Err(OAuthError::ReauthenticationRequired.into());
                }
                let refreshing = self.start_refresh(state.tokens.clone());
                state.refreshing = Some(refreshing.clone());
                refreshing
            }
        };
        refreshing.await
    }

    /// Runs the refresh as its own task, which stores and persists the new tokens before it ends.
    fn start_refresh(&self, tokens: TokenSet) -> Refreshing {
        let (refresh, persist, state) = (self.refresh.clone(), self.persist.clone(), self.state.clone());
        let task = tokio::spawn(async move {
            let result = run(&refresh, tokens).await;
            {
                let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                if let Ok(new) = &result {
                    state.tokens = new.clone();
                }
                state.refreshing = None;
            }
            if let Ok(new) = &result {
                persist(new.clone()).await;
            }
            result
        });
        async move { task.await.unwrap_or(Err(HiggsfieldError::Network.into())) }
            .boxed()
            .shared()
    }
}

/// Only a refusal of the refresh token asks for a new sign-in. Anything else (no answer, a 5xx, an unreadable
/// reply, metadata that can't be read) keeps the tokens for the next call to refresh again, and fails as a
/// `HiggsfieldError`: as it is, or as `Network` when it isn't one.
async fn run(refresh: &Refresh, tokens: TokenSet) -> Result<TokenSet, McpError> {
    refresh(tokens).await.map_err(|error| match error {
        McpError::Higgsfield(error) => error.into(),
        McpError::OAuth(error) if error.refuses_refresh() => OAuthError::ReauthenticationRequired.into(),
        _ => HiggsfieldError::Network.into(),
    })
}
