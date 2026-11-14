//! The two providers on stub hosts, as the Swift tests' `Support.swift` builds them.

use std::sync::Arc;

use higgsfield::api::HiggsfieldApiClient;

use super::account::account_on;
use super::stub::Stub;
use crate::{HiggsfieldApiProvider, HiggsfieldMcpProvider};

/// An account provider whose MCP calls and uploads all go to `stub`, signed in with a token that never expires
/// (`access-1`), owned by `user_1`. `unlimited_opt_in` is the user's Unlimited generations setting.
pub fn make_provider(stub: &Stub, unlimited_opt_in: bool) -> HiggsfieldMcpProvider {
    HiggsfieldMcpProvider::new(Arc::new(account_on(stub, None)), Some("user_1".to_owned()))
        .with_unlimited_opt_in(move || async move { unlimited_opt_in })
}

/// An API key provider whose calls go to `stub` (`StubServer.makeAPIProvider`), with key ID `key_id` and secret
/// `key-secret`.
pub fn make_api_provider(stub: &Stub, key_id: &str) -> HiggsfieldApiProvider {
    let client = HiggsfieldApiClient::new(key_id, "key-secret")
        .with_base_url(stub.base_url.clone())
        .with_http_client(stub.client());
    HiggsfieldApiProvider::new(client, None)
}
