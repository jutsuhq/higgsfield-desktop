//! The network side of connecting (`HiggsfieldConnection.swift`): key validation over REST, account sign-in over
//! OAuth with a loopback redirect, refresh and revocation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use higgsfield::api::HiggsfieldApiClient;
use higgsfield::mcp::McpError;
use higgsfield::mcp::OAuthError;
use higgsfield::mcp::oauth::{
    CallbackPage, ClientRegistration, LoopbackAuthorizationPresenter, OAuthClient, OAuthDiscovery,
    OpenBrowser, PkceAuthorizer, TokenSet,
};
use jutsu_core::{OAuthTokens, PortError, SignInError};
use jutsu_providers::{account_sign_in_error, oauth_tokens, provider_error, token_set};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::ports::{ConnectionServices, ShowUrl};

/// Where the app reaches Higgsfield.
pub mod higgsfield_urls {
    use super::{LazyLock, Url};

    #[allow(clippy::expect_used, reason = "constants that parse")]
    fn url(text: &str) -> Url {
        Url::parse(text).expect("a constant URL parses")
    }

    pub static MCP_ENDPOINT: LazyLock<Url> = LazyLock::new(|| url("https://mcp.higgsfield.ai/mcp"));
    pub static CONSOLE: LazyLock<Url> = LazyLock::new(|| url("https://console.higgsfield.ai"));
    /// `docs/oauth-client.json` as GitHub Pages serves it: the sign-in's `client_id`, the same Client ID Metadata
    /// Document the macOS app uses. The authorization server downloads it, so moving or removing it breaks every
    /// new sign-in.
    pub static CLIENT_METADATA_DOCUMENT: LazyLock<Url> =
        LazyLock::new(|| url("https://jutsuhq.github.io/jutsu-studio/oauth-client.json"));
}

/// Opens a URL in the user's default browser (the shell plugs in `tauri-plugin-opener`).
pub type OpenUrl = Arc<dyn Fn(&Url) -> Result<(), String> + Send + Sync>;
/// Brings the app's window back to the front.
pub type BringToFront = Arc<dyn Fn() + Send + Sync>;

/// `ConnectionServices` against Higgsfield.
#[derive(Clone)]
pub struct HiggsfieldConnectionServices {
    open_url: OpenUrl,
    bring_to_front: BringToFront,
    page: CallbackPage,
    http: reqwest::Client,
}

impl std::fmt::Debug for HiggsfieldConnectionServices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HiggsfieldConnectionServices")
            .finish_non_exhaustive()
    }
}

impl HiggsfieldConnectionServices {
    /// `page` is what the browser tab shows after the redirect.
    pub fn new(open_url: OpenUrl, bring_to_front: BringToFront, page: CallbackPage) -> Self {
        Self {
            open_url,
            bring_to_front,
            page,
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl ConnectionServices for HiggsfieldConnectionServices {
    async fn validate_api_key(&self, id: &str, secret: &str) -> Result<bool, PortError> {
        HiggsfieldApiClient::new(id, secret)
            .validate_key()
            .await
            .map_err(|error| PortError::Provider(provider_error(error)))
    }

    async fn sign_in_with_browser(
        &self,
        previous: Option<OAuthTokens>,
        show: ShowUrl,
        cancel: CancellationToken,
    ) -> Result<OAuthTokens, PortError> {
        // A browser that doesn't open doesn't end the sign-in: the sheet offers the link to open again or copy.
        let insecure = Arc::new(AtomicBool::new(false));
        let open: OpenBrowser = {
            let (open_url, insecure) = (Arc::clone(&self.open_url), Arc::clone(&insecure));
            Arc::new(move |url: &Url| {
                // The opener would hand any scheme to its app; discovery already demands https.
                if url.scheme() != "https" {
                    insecure.store(true, Ordering::SeqCst);
                    return Err("The sign-in page isn't on a secure (https) address.".to_owned());
                }
                show(url.clone());
                if let Err(error) = open_url(url) {
                    tracing::error!(%error, "the default browser didn't open the sign-in page");
                }
                Ok(())
            })
        };
        let presenter = LoopbackAuthorizationPresenter::new(open)
            .with_page(self.page.clone())
            .with_cancellation(cancel);
        let result = async {
            let client = AccountAuthorization::client(&self.http, previous.map(token_set).as_ref()).await?;
            PkceAuthorizer::new(client, Arc::new(presenter)).authorize().await
        }
        .await;
        // The browser keeps focus after the redirect; bring the app back for the result.
        (self.bring_to_front)();
        match result {
            Ok(tokens) => Ok(oauth_tokens(tokens)),
            Err(_) if insecure.load(Ordering::SeqCst) => Err(SignInError::InvalidResponse.into()),
            Err(error) => Err(account_sign_in_error(error)),
        }
    }

    async fn revoke(&self, tokens: OAuthTokens) {
        if let Err(error) = AccountAuthorization::revoke(&self.http, &token_set(tokens)).await {
            tracing::info!(%error, "can't revoke the signed-out tokens");
        }
    }
}

/// OAuth against the authorization servers the MCP endpoint advertises. Endpoints are discovered on every sign-in
/// and refresh, never hard-coded.
pub struct AccountAuthorization;

impl AccountAuthorization {
    /// The OAuth client at the first server offering the authorization code flow, identified by the Client ID
    /// Metadata Document where the server accepts one; `previous` lets registration reuse the client ID stored with
    /// earlier tokens.
    pub async fn client(
        http: &reqwest::Client,
        previous: Option<&TokenSet>,
    ) -> Result<OAuthClient, McpError> {
        let discovered = OAuthDiscovery::new(http.clone())
            .discover(&higgsfield_urls::MCP_ENDPOINT, None)
            .await?;
        let server = discovered
            .authorization_code_server()
            .ok_or(OAuthError::UnsupportedFlow)?;
        let client_id = ClientRegistration::new(
            http.clone(),
            Some(higgsfield_urls::CLIENT_METADATA_DOCUMENT.clone()),
        )
        .client_id(server, previous)
        .await?;
        Ok(
            OAuthClient::new(server.clone(), client_id, discovered.resource.resource.clone())
                .with_http_client(http.clone()),
        )
    }

    /// Refreshes at the server that issued `tokens` only; fails with `ReauthenticationRequired` when discovery no
    /// longer lists it.
    pub async fn refresh(http: &reqwest::Client, tokens: &TokenSet) -> Result<TokenSet, McpError> {
        let discovered = OAuthDiscovery::new(http.clone())
            .discover(&higgsfield_urls::MCP_ENDPOINT, None)
            .await?;
        discovered
            .client_issuing(tokens, http.clone())?
            .refresh(tokens)
            .await
    }

    pub async fn revoke(http: &reqwest::Client, tokens: &TokenSet) -> Result<(), McpError> {
        let discovered = OAuthDiscovery::new(http.clone())
            .discover(&higgsfield_urls::MCP_ENDPOINT, None)
            .await?;
        discovered
            .client_issuing(tokens, http.clone())?
            .revoke(tokens)
            .await
    }
}
