//! OAuth 2.1 sign-in for the MCP server: discovery (RFC 9728, RFC 8414), client registration (RFC 7591 or a Client
//! ID Metadata Document), the authorization code flow with PKCE and a loopback redirect, and token refresh.

mod authorizer;
mod callback_page;
mod client;
mod discovery;
mod id_token;
mod loopback;
mod metadata;
mod pkce;
mod redirect_waiter;
mod registration;
mod token_provider;
mod token_set;

pub use authorizer::{AuthorizationPresenter, AuthorizationRedirect, MakeUrl, PkceAuthorizer};
pub use callback_page::CallbackPage;
pub use client::{Clock, OAuthClient};
pub use discovery::{DiscoveredAuthorization, OAuthDiscovery};
pub use id_token::IdTokenClaims;
pub use loopback::{LoopbackAuthorizationPresenter, OpenBrowser};
pub use metadata::{AuthorizationServerMetadata, ProtectedResourceMetadata};
pub use pkce::{PkceHasher, Sha256Hasher};
pub use registration::ClientRegistration;
pub use token_provider::TokenProvider;
pub use token_set::TokenSet;
