//! The network half of `PKCETests`: the authorization code flow and client registration.

use std::sync::Arc;

use async_trait::async_trait;
use higgsfield::mcp::oauth::{
    AuthorizationPresenter, AuthorizationRedirect, ClientRegistration, MakeUrl, OAuthClient, PkceAuthorizer,
    PkceHasher, TokenSet,
};
use higgsfield::mcp::{McpError, OAuthError};
use serde_json::json;
use url::Url;

use crate::support::mcp::{Locked, Reply, fixture};
use crate::support::stub::{Stub, body_json, form, header, target};

/// Returns fixed bytes whose base64 contains `+`, `/` and `=`, so the base64url rewrite is visible.
struct FakeHasher;

impl PkceHasher for FakeHasher {
    fn sha256(&self, _: &[u8]) -> Vec<u8> {
        vec![0xfb, 0xff, 0xfe, 0xfb, 0xff]
    }
}

const REDIRECT_URI: &str = "http://127.0.0.1:47613/callback";

/// Opens nothing: records the authorization URL and comes back with `redirect(state)`.
struct FakePresenter {
    opened: Arc<Locked<Vec<Url>>>,
    redirect: Box<dyn Fn(&str) -> String + Send + Sync>,
}

impl FakePresenter {
    fn new(redirect: impl Fn(&str) -> String + Send + Sync + 'static) -> Self {
        Self {
            opened: Locked::new(Vec::new()),
            redirect: Box::new(redirect),
        }
    }
}

#[async_trait]
impl AuthorizationPresenter for FakePresenter {
    async fn authorize(&self, make_url: MakeUrl<'_>) -> Result<AuthorizationRedirect, McpError> {
        let url = make_url(REDIRECT_URI)?;
        self.opened.update(|opened| opened.push(url.clone()));
        let state = url
            .query_pairs()
            .find(|(name, _)| name == "state")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default();
        Ok(AuthorizationRedirect {
            callback: Url::parse(&(self.redirect)(&state)).unwrap(),
            redirect_uri: REDIRECT_URI.to_owned(),
        })
    }
}

fn authorizer(stub: &Stub, presenter: FakePresenter) -> (PkceAuthorizer, Arc<Locked<Vec<Url>>>) {
    let opened = presenter.opened.clone();
    let client = fixture::client(
        fixture::server(fixture::CLERK_METADATA),
        stub.client(),
        fixture::distant_past(),
    );
    (
        PkceAuthorizer::with_hasher(client, Arc::new(presenter), Arc::new(FakeHasher)),
        opened,
    )
}

#[tokio::test]
async fn authorization_code_flow_sends_pkce_state_and_resource() {
    let stub = Stub::replying(|request| {
        if request.url.path() == "/oauth/token" {
            Reply::json_text(fixture::TOKEN_RESPONSE)
        } else {
            Reply::not_found()
        }
    })
    .await;
    let presenter = FakePresenter::new(|state| {
        format!(
            "http://127.0.0.1:47613/callback?code=the-code&state={state}&{}",
            fixture::CLERK_ISS
        )
    });
    let (authorizer, opened) = authorizer(&stub, presenter);

    let tokens = authorizer.authorize().await.unwrap();

    assert_eq!(tokens.access_token, "new-access");
    assert_eq!(tokens.refresh_token.as_deref(), Some("new-refresh"));
    let opened = opened.value()[0].clone();
    let query: std::collections::HashMap<_, _> = opened.query_pairs().into_owned().collect();
    assert_eq!(opened.path(), "/oauth/authorize");
    assert_eq!(query["response_type"], "code");
    assert_eq!(query["client_id"], "client-1");
    assert_eq!(query["redirect_uri"], "http://127.0.0.1:47613/callback");
    assert_eq!(query["scope"], "openid email offline_access");
    assert_eq!(query["code_challenge"], "-__--_8");
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["resource"], "https://mcp.higgsfield.ai/mcp");
    assert_eq!(query["state"].len(), 32);

    let exchange = &stub.requests().await[0];
    assert_eq!(exchange.method.as_str(), "POST");
    assert_eq!(
        header(exchange, "Content-Type"),
        Some("application/x-www-form-urlencoded")
    );
    let form = form(exchange);
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], "the-code");
    assert_eq!(form["client_id"], "client-1");
    assert_eq!(form["redirect_uri"], "http://127.0.0.1:47613/callback");
    assert_eq!(form["resource"], "https://mcp.higgsfield.ai/mcp");
    assert!((43..=128).contains(&form["code_verifier"].len()));
}

#[tokio::test]
async fn state_mismatch_never_reaches_the_token_endpoint() {
    let stub = Stub::replying(|_| Reply::json_text(fixture::TOKEN_RESPONSE)).await;
    let presenter =
        FakePresenter::new(|_| "http://127.0.0.1:47613/callback?code=the-code&state=forged".into());
    let (authorizer, _) = authorizer(&stub, presenter);
    assert_eq!(
        authorizer.authorize().await,
        Err(OAuthError::StateMismatch.into())
    );
    assert!(stub.requests().await.is_empty());
}

#[tokio::test]
async fn token_endpoint_errors_are_mapped() {
    let stub = Stub::replying(|_| {
        Reply::json_text(r#"{"error":"invalid_grant","error_description":"code expired"}"#).with_status(400)
    })
    .await;
    let presenter = FakePresenter::new(|state| {
        format!(
            "http://127.0.0.1:47613/callback?code=old&state={state}&{}",
            fixture::CLERK_ISS
        )
    });
    let (authorizer, _) = authorizer(&stub, presenter);
    let expected = OAuthError::Server {
        code: "invalid_grant".into(),
        description: Some("code expired".into()),
    };
    assert_eq!(authorizer.authorize().await, Err(expected.into()));
}

#[tokio::test]
async fn missing_issuer_never_reaches_the_token_endpoint_of_a_server_that_sends_it() {
    let stub = Stub::replying(|_| Reply::json_text(fixture::TOKEN_RESPONSE)).await;
    let presenter =
        FakePresenter::new(|state| format!("http://127.0.0.1:47613/callback?code=the-code&state={state}"));
    let (authorizer, _) = authorizer(&stub, presenter);
    assert_eq!(
        authorizer.authorize().await,
        Err(OAuthError::IssuerMismatch.into())
    );
    assert!(stub.requests().await.is_empty());
}

#[tokio::test]
async fn configured_metadata_document_url_is_the_client_id_when_supported() {
    let stub = Stub::replying(|_| Reply::not_found()).await;
    let document = Url::parse(fixture::METADATA_DOCUMENT_URL).unwrap();
    let registration = ClientRegistration::new(stub.client(), Some(document));
    let client_id = registration
        .client_id(&fixture::server(fixture::CLERK_METADATA), None)
        .await
        .unwrap();
    assert_eq!(
        client_id,
        "https://jutsuhq.github.io/jutsu-studio/oauth-client.json"
    );
    assert!(stub.requests().await.is_empty());
}

#[tokio::test]
async fn dynamic_registration_sends_public_client_metadata() {
    let stub = Stub::replying(|_| Reply::json_text(r#"{"client_id":"registered-1"}"#).with_status(201)).await;
    let client_id = ClientRegistration::new(stub.client(), None)
        .client_id(&fixture::server(&fixture::clerk_without_cimd()), None)
        .await
        .unwrap();
    assert_eq!(client_id, "registered-1");

    let request = &stub.requests().await[0];
    assert_eq!(
        target(request).as_str(),
        format!("{}/oauth/register", fixture::CLERK)
    );
    assert_eq!(header(request, "Content-Type"), Some("application/json"));
    let body = body_json(request);
    assert_eq!(body["token_endpoint_auth_method"], "none");
    assert_eq!(
        body["redirect_uris"],
        json!([
            "http://127.0.0.1:47613/callback",
            "http://127.0.0.1:51874/callback",
            "http://127.0.0.1:58329/callback"
        ])
    );
    assert_eq!(
        body["grant_types"],
        json!(["authorization_code", "refresh_token"])
    );
    assert_eq!(body["response_types"], json!(["code"]));
    assert_eq!(body["client_name"], "Jutsu Studio");
}

// The network half of `IDTokenTests`: the account a sign-in names, and reusing a registered client.

const HEADER: &str = "eyJhbGciOiJSUzI1NiJ9";
/// `{"email":"me@example.com","name":"Me"}`
const PAYLOAD: &str = "eyJlbWFpbCI6Im1lQGV4YW1wbGUuY29tIiwibmFtZSI6Ik1lIn0";

fn clerk_iss_redirect(state: &str) -> String {
    format!(
        "http://127.0.0.1:47613/callback?code=c&state={state}&{}",
        fixture::CLERK_ISS
    )
}

async fn sign_in(token_response: &str) -> TokenSet {
    let response = token_response.to_owned();
    let stub = Stub::replying(move |_| Reply::json_text(&response)).await;
    let (authorizer, _) = authorizer(&stub, FakePresenter::new(clerk_iss_redirect));
    authorizer.authorize().await.unwrap()
}

fn clerk() -> Url {
    Url::parse(fixture::CLERK).unwrap()
}

#[tokio::test]
async fn token_response_with_id_token_names_and_identifies_the_account() {
    let tokens = sign_in(&format!(
        r#"{{"access_token":"a","expires_in":60,"id_token":"{HEADER}.{PAYLOAD}.signature"}}"#
    ))
    .await;
    assert_eq!(tokens.email.as_deref(), Some("me@example.com"));
    assert_eq!(tokens.name.as_deref(), Some("Me"));
    assert_eq!(tokens.issuer, Some(clerk()));
    assert_eq!(tokens.client_id, "client-1");

    // `{"sub":"user_1","email":"me@example.com"}`
    let id_token = format!("{HEADER}.eyJzdWIiOiJ1c2VyXzEiLCJlbWFpbCI6Im1lQGV4YW1wbGUuY29tIn0.signature");
    let tokens = sign_in(&format!(
        r#"{{"access_token":"a","expires_in":60,"id_token":"{id_token}"}}"#
    ))
    .await;
    assert_eq!(tokens.subject.as_deref(), Some("user_1"));
}

#[tokio::test]
async fn a_missing_or_unreadable_id_token_still_signs_in() {
    let tokens = sign_in(fixture::TOKEN_RESPONSE).await;
    assert_eq!(tokens.access_token, "new-access");
    assert_eq!(tokens.email, None);
    assert_eq!(tokens.name, None);
    assert_eq!(tokens.issuer, Some(clerk()));

    let tokens = sign_in(r#"{"access_token":"a","id_token":"garbage"}"#).await;
    assert_eq!(tokens.access_token, "a");
    assert_eq!(tokens.email, None);
}

#[tokio::test]
async fn refresh_without_id_token_keeps_the_account() {
    let stub = Stub::replying(|_| Reply::json_text(fixture::TOKEN_RESPONSE)).await;
    let client = fixture::client(
        fixture::server(fixture::CLERK_METADATA),
        stub.client(),
        fixture::distant_past(),
    );
    let mut old = fixture::tokens("access-1", Some("refresh-1"), fixture::distant_past());
    old.email = Some("me@example.com".into());
    old.name = Some("Me".into());
    old.subject = Some("user_1".into());
    let refreshed = client.refresh(&old).await.unwrap();
    assert_eq!(refreshed.access_token, "new-access");
    assert_eq!(refreshed.email.as_deref(), Some("me@example.com"));
    assert_eq!(refreshed.name.as_deref(), Some("Me"));
    assert_eq!(refreshed.subject.as_deref(), Some("user_1"));
}

fn registered_2() -> Reply {
    Reply::json_text(r#"{"client_id":"registered-2"}"#).with_status(201)
}

fn previous(issuer: Option<&str>, client_id: &str) -> TokenSet {
    let mut tokens = fixture::tokens("access-1", Some("refresh-1"), fixture::distant_past());
    tokens.issuer = issuer.map(|issuer| Url::parse(issuer).unwrap());
    client_id.clone_into(&mut tokens.client_id);
    tokens
}

#[tokio::test]
async fn configured_metadata_document_is_preferred_over_a_stored_registration() {
    let stub = Stub::replying(|_| registered_2()).await;
    let document = Url::parse(fixture::METADATA_DOCUMENT_URL).unwrap();
    let registration = ClientRegistration::new(stub.client(), Some(document));
    let previous = previous(Some(fixture::CLERK), "client-1");
    let client_id = registration
        .client_id(&fixture::server(fixture::CLERK_METADATA), Some(&previous))
        .await
        .unwrap();
    assert_eq!(
        client_id,
        "https://jutsuhq.github.io/jutsu-studio/oauth-client.json"
    );
    assert!(stub.requests().await.is_empty());
}

#[tokio::test]
async fn registered_client_id_is_reused_on_the_next_sign_in() {
    let stub = Stub::replying(|request| {
        if request.url.path() == "/oauth/register" {
            Reply::json_text(r#"{"client_id":"registered-1"}"#).with_status(201)
        } else {
            Reply::json_text(fixture::TOKEN_RESPONSE)
        }
    })
    .await;
    // The real clerk document: it accepts metadata documents, but none is configured by default.
    let server = fixture::server(fixture::CLERK_METADATA);
    let registration = ClientRegistration::new(stub.client(), None);

    let first = registration.client_id(&server, None).await.unwrap();
    let client =
        OAuthClient::new(server.clone(), first.clone(), fixture::mcp_url()).with_http_client(stub.client());
    let presenter = FakePresenter::new(clerk_iss_redirect);
    let tokens = PkceAuthorizer::with_hasher(client, Arc::new(presenter), Arc::new(FakeHasher))
        .authorize()
        .await
        .unwrap();
    let second = registration.client_id(&server, Some(&tokens)).await.unwrap();

    assert_eq!(first, "registered-1");
    assert_eq!(second, "registered-1");
    assert_eq!(
        stub.requests_to(&format!("{}/oauth/register", fixture::CLERK))
            .await
            .len(),
        1
    );
}

#[tokio::test]
async fn registration_is_reused_when_the_issuer_differs_only_by_a_trailing_slash() {
    let stub = Stub::replying(|_| registered_2()).await;
    let previous = previous(Some(&format!("{}/", fixture::CLERK)), "registered-1");
    let client_id = ClientRegistration::new(stub.client(), None)
        .client_id(&fixture::server(&fixture::clerk_without_cimd()), Some(&previous))
        .await
        .unwrap();
    assert_eq!(client_id, "registered-1");
    assert!(stub.requests().await.is_empty());
}

#[tokio::test]
async fn registration_at_another_issuer_is_not_reused() {
    let stub = Stub::replying(|_| registered_2()).await;
    for previous in [
        previous(Some(fixture::DEVICE_AUTH), "client-1"),
        previous(None, "client-1"),
    ] {
        let client_id = ClientRegistration::new(stub.client(), None)
            .client_id(&fixture::server(&fixture::clerk_without_cimd()), Some(&previous))
            .await
            .unwrap();
        assert_eq!(client_id, "registered-2");
    }
    assert_eq!(stub.requests().await.len(), 2);
}

#[tokio::test]
async fn metadata_document_client_id_is_never_reused_for_registration() {
    let stub = Stub::replying(|_| registered_2()).await;
    let previous = previous(
        Some(fixture::CLERK),
        "https://jutsuhq.github.io/jutsu-studio/oauth-client.json",
    );
    let client_id = ClientRegistration::new(stub.client(), None)
        .client_id(&fixture::server(&fixture::clerk_without_cimd()), Some(&previous))
        .await
        .unwrap();
    assert_eq!(client_id, "registered-2");
}
