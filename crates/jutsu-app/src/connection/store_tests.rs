//! `ConnectionStoreTests.swift`, case for case.

use std::sync::Arc;

use jutsu_core::{Balance, Credential, OAuthTokens, PortError, ProviderError, ProviderKind, SignInError};

use super::*;
use crate::connection::{ConnectionMetadata, MemoryVault};
use crate::testing::{BalanceProvider, FakeMetadataStore, FakeServices, Gate, Shared, eventually, fixture};

struct Harness {
    store: Arc<ConnectionStore>,
    vault: Arc<MemoryVault>,
    metadata: Arc<FakeMetadataStore>,
    credentials: Arc<Shared<Vec<Option<Credential>>>>,
}

fn key(id: &str, secret: &str) -> Credential {
    Credential::ApiKey {
        id: id.to_owned(),
        secret: secret.to_owned(),
    }
}

fn key_metadata(id: &str) -> ConnectionMetadata {
    ConnectionMetadata {
        kind: ProviderKind::ApiKey,
        display_name: Some(id.to_owned()),
        plan: None,
        name: None,
    }
}

fn account_metadata(email: &str, plan: Option<&str>) -> ConnectionMetadata {
    ConnectionMetadata {
        kind: ProviderKind::Account,
        display_name: Some(email.to_owned()),
        plan: plan.map(str::to_owned),
        name: None,
    }
}

fn harness_with(
    services: FakeServices,
    vault: Option<Credential>,
    metadata: Option<ConnectionMetadata>,
) -> Harness {
    let vault = Arc::new(match vault {
        Some(credential) => MemoryVault::with_credential(&credential),
        None => MemoryVault::new(),
    });
    let metadata = FakeMetadataStore::new(metadata);
    harness_over(services, vault, metadata)
}

fn harness_over(
    services: FakeServices,
    vault: Arc<MemoryVault>,
    metadata: Arc<FakeMetadataStore>,
) -> Harness {
    let store = Arc::new(ConnectionStore::new(
        vault.clone(),
        metadata.clone(),
        Arc::new(services),
    ));
    let credentials = Shared::new(Vec::new());
    let recorded = credentials.clone();
    store.set_on_credential_change(Arc::new(move |credential| {
        recorded.update(|all| all.push(credential));
        Box::pin(async {})
    }));
    Harness {
        store,
        vault,
        metadata,
        credentials,
    }
}

fn harness() -> Harness {
    harness_with(FakeServices::default(), None, None)
}

fn account(email: Option<&str>, name: Option<&str>, plan: Option<&str>) -> ConnectionState {
    ConnectionState::Account(AccountInfo {
        email: email.map(str::to_owned),
        name: name.map(str::to_owned),
        plan: plan.map(str::to_owned),
        credits: None,
    })
}

fn has_row(store: &ConnectionStore, label: &str) -> bool {
    store.state().card_rows(None).iter().any(|row| row.label == label)
}

async fn sign_in(store: &Arc<ConnectionStore>) {
    store.start_sign_in().await.unwrap();
}

// MARK: Restore

#[tokio::test]
async fn fresh_install_starts_disconnected() {
    let h = harness();
    assert_eq!(h.store.state(), ConnectionState::None);
    h.store.activate().await;
    assert_eq!(h.credentials.value(), [None]);
}

#[tokio::test]
async fn persisted_api_key_is_restored_and_reported_on_activate() {
    let h = harness_with(
        FakeServices::default(),
        Some(key("kid", "s")),
        Some(key_metadata("kid")),
    );
    assert_eq!(h.store.state(), ConnectionState::ApiKey { key_id: "kid".into() });
    h.store.activate().await;
    assert_eq!(h.credentials.value(), [Some(key("kid", "s"))]);
}

#[test]
fn persisted_account_is_restored() {
    let h = harness_with(
        FakeServices::default(),
        Some(Credential::Oauth(fixture::tokens("access"))),
        Some(account_metadata("me@example.com", Some("Pro"))),
    );
    assert_eq!(
        h.store.state(),
        account(Some("me@example.com"), None, Some("Pro"))
    );
}

#[test]
fn metadata_without_credential_is_dropped() {
    let h = harness_with(FakeServices::default(), None, Some(key_metadata("kid")));
    assert_eq!(h.store.state(), ConnectionState::None);
    assert_eq!(h.metadata.stored(), None);
}

#[test]
fn credential_of_another_kind_is_not_restored() {
    let h = harness_with(
        FakeServices::default(),
        Some(Credential::Oauth(fixture::tokens("access"))),
        Some(key_metadata("kid")),
    );
    assert_eq!(h.store.state(), ConnectionState::None);
}

// MARK: API key

#[tokio::test]
async fn valid_key_is_trimmed_saved_and_reported() {
    let checked = Shared::new(Vec::<String>::new());
    let seen = checked.clone();
    let services = FakeServices::validating(move |id, secret| {
        seen.update(|all| all.push(format!("{id}:{secret}")));
        async { Ok(true) }
    });
    let h = harness_with(services, None, None);
    let form = ApiKeyForm::new("  kid\t", "\nsecret\n");
    assert!(form.is_complete());
    h.store.connect_api_key(&form).await;
    assert_eq!(checked.value(), ["kid:secret"]);
    assert_eq!(h.store.state(), ConnectionState::ApiKey { key_id: "kid".into() });
    assert_eq!(h.store.activity(), Activity::Idle);
    assert_eq!(h.store.alert(), None);
    assert_eq!(h.vault.credential(), Some(key("kid", "secret")));
    assert_eq!(h.metadata.stored(), Some(key_metadata("kid")));
    assert_eq!(h.credentials.value(), [Some(key("kid", "secret"))]);
}

#[tokio::test]
async fn refused_key_shows_key_is_invalid_and_saves_nothing() {
    let h = harness_with(FakeServices::validating(|_, _| async { Ok(false) }), None, None);
    h.store.connect_api_key(&ApiKeyForm::new("kid", "bad")).await;
    assert_eq!(h.store.state(), ConnectionState::None);
    assert_eq!(h.store.alert(), Some(UserMessage::invalid_key()));
    assert_eq!(h.vault.credential(), None);
    assert_eq!(h.metadata.stored(), None);
    assert_eq!(h.credentials.value(), []);
}

#[tokio::test]
async fn network_failure_while_validating_says_so_and_saves_nothing() {
    let h = harness_with(
        FakeServices::validating(|_, _| async { Err(ProviderError::Network.into()) }),
        None,
        None,
    );
    h.store.connect_api_key(&ApiKeyForm::new("kid", "s")).await;
    assert_eq!(h.store.state(), ConnectionState::None);
    assert_eq!(h.store.alert().unwrap().title, "Can't reach Higgsfield");
    assert_eq!(h.store.activity(), Activity::Idle);
    assert_eq!(h.vault.credential(), None);
}

#[tokio::test]
async fn incomplete_form_is_not_sent() {
    let h = harness_with(
        FakeServices::validating(|_, _| async { panic!("validated") }),
        None,
        None,
    );
    h.store.connect_api_key(&ApiKeyForm::new("kid", "   ")).await;
    assert_eq!(h.store.alert(), Some(UserMessage::incomplete_key()));
    assert_eq!(h.store.state(), ConnectionState::None);
}

// MARK: Account

#[tokio::test]
async fn browser_sign_in_connects_the_account() {
    let h = harness();
    sign_in(&h.store).await;
    assert_eq!(h.store.state(), ConnectionState::Account(AccountInfo::default()));
    assert_eq!(h.store.activity(), Activity::Idle);
    assert_eq!(
        h.vault.credential(),
        Some(Credential::Oauth(fixture::tokens("access")))
    );
    assert_eq!(h.metadata.stored().unwrap().kind, ProviderKind::Account);
    assert_eq!(
        h.credentials.value(),
        [Some(Credential::Oauth(fixture::tokens("access")))]
    );
}

#[tokio::test]
async fn closing_the_browser_sheet_is_silent() {
    let h = harness_with(
        FakeServices::browsing(|_| async { Err(PortError::Cancelled) }),
        None,
        None,
    );
    sign_in(&h.store).await;
    assert_eq!(h.store.state(), ConnectionState::None);
    assert_eq!(h.store.alert(), None);
    assert_eq!(h.store.activity(), Activity::Idle);
}

#[tokio::test]
async fn denied_access_shows_a_message() {
    let h = harness_with(
        FakeServices::browsing(|_| async { Err(SignInError::Denied.into()) }),
        None,
        None,
    );
    sign_in(&h.store).await;
    assert_eq!(h.store.state(), ConnectionState::None);
    assert_eq!(h.store.alert().unwrap().title, "Access was denied");
}

fn gated(gate: &Arc<Gate<OAuthTokens>>) -> FakeServices {
    let gate = gate.clone();
    FakeServices::browsing(move |_| {
        let gate = gate.clone();
        async move { gate.wait().await.ok_or(PortError::Cancelled) }
    })
}

#[tokio::test]
async fn canceled_sign_in_ignores_a_late_result() {
    let gate = Gate::new();
    let h = harness_with(gated(&gate), None, None);
    let task = h.store.start_sign_in();
    assert!(h.store.activity().is_signing_in());
    eventually(|| gate.is_waiting()).await;
    h.store.cancel_sign_in();
    assert_eq!(h.store.activity(), Activity::Idle);
    gate.open(fixture::tokens("access"));
    task.await.unwrap();
    assert_eq!(h.store.state(), ConnectionState::None);
    assert_eq!(h.vault.credential(), None);
    assert_eq!(h.credentials.value(), []);
}

#[tokio::test]
async fn the_sign_in_page_is_shown_while_the_browser_is_open() {
    let gate = Gate::new();
    let h = harness_with(gated(&gate), None, None);
    let task = h.store.start_sign_in();
    assert!(h.store.activity().is_signing_in());
    eventually(|| h.store.activity() == Activity::SigningIn(Some(fixture::sign_in_page()))).await;
    gate.open(fixture::tokens("access"));
    task.await.unwrap();
    assert_eq!(h.store.activity(), Activity::Idle);
    assert_eq!(h.store.state().kind(), Some(ProviderKind::Account));
}

#[tokio::test]
async fn signing_in_again_replaces_the_open_sign_in() {
    let cancelled = Shared::new(false);
    let first = Shared::new(true);
    let (was_cancelled, is_first) = (cancelled.clone(), first.clone());
    let services = FakeServices::browsing(move |cancel| {
        let (was_cancelled, is_first) = (was_cancelled.clone(), is_first.clone());
        async move {
            if !is_first.value() {
                return Ok(fixture::tokens("second"));
            }
            is_first.update(|first| *first = false);
            cancel.cancelled().await;
            was_cancelled.update(|cancelled| *cancelled = true);
            Err(PortError::Cancelled)
        }
    });
    let h = harness_with(services, None, None);
    let browser = h.store.start_sign_in();
    eventually(|| !first.value()).await;
    sign_in(&h.store).await;
    browser.await.unwrap();
    assert!(cancelled.value());
    assert_eq!(
        h.vault.credential(),
        Some(Credential::Oauth(fixture::tokens("second")))
    );
    assert_eq!(h.store.alert(), None);
}

#[tokio::test]
async fn signing_in_again_clears_needs_attention_and_keeps_account_info() {
    let h = harness_with(
        FakeServices::default(),
        Some(Credential::Oauth(fixture::tokens("old"))),
        Some(account_metadata("me@example.com", Some("Pro"))),
    );
    h.store.mark_needs_attention();
    assert!(h.store.needs_attention());
    sign_in(&h.store).await;
    assert!(!h.store.needs_attention());
    assert_eq!(
        h.store.state(),
        account(Some("me@example.com"), None, Some("Pro"))
    );
    assert_eq!(
        h.vault.credential(),
        Some(Credential::Oauth(fixture::tokens("access")))
    );
    assert_eq!(
        h.credentials.value(),
        [Some(Credential::Oauth(fixture::tokens("access")))]
    );
}

#[tokio::test]
async fn id_token_account_is_shown_and_persisted() {
    let tokens = fixture::tokens_for("access", Some("me@example.com"), Some("Me"));
    let signed = tokens.clone();
    let h = harness_with(
        FakeServices::browsing(move |_| {
            let tokens = signed.clone();
            async move { Ok(tokens) }
        }),
        None,
        None,
    );
    sign_in(&h.store).await;
    assert_eq!(h.store.state(), account(Some("me@example.com"), Some("Me"), None));
    let saved = ConnectionMetadata {
        kind: ProviderKind::Account,
        display_name: Some("me@example.com".into()),
        plan: None,
        name: Some("Me".into()),
    };
    assert_eq!(h.metadata.stored(), Some(saved));
    let restarted = harness_over(FakeServices::default(), h.vault.clone(), h.metadata.clone());
    assert_eq!(
        restarted.store.state(),
        account(Some("me@example.com"), Some("Me"), None)
    );
}

fn signing_in_as(tokens: OAuthTokens) -> FakeServices {
    FakeServices::browsing(move |_| {
        let tokens = tokens.clone();
        async move { Ok(tokens) }
    })
}

#[tokio::test]
async fn signing_in_as_another_account_drops_the_plan() {
    let h = harness_with(
        signing_in_as(fixture::tokens_for("access", Some("other@example.com"), None)),
        Some(Credential::Oauth(fixture::tokens("old"))),
        Some(account_metadata("me@example.com", Some("Pro"))),
    );
    sign_in(&h.store).await;
    assert_eq!(h.store.state(), account(Some("other@example.com"), None, None));
}

#[tokio::test]
async fn signing_in_as_the_same_account_keeps_the_plan() {
    let h = harness_with(
        signing_in_as(fixture::tokens_for("access", Some("me@example.com"), Some("Me"))),
        Some(Credential::Oauth(fixture::tokens("old"))),
        Some(account_metadata("me@example.com", Some("Pro"))),
    );
    sign_in(&h.store).await;
    assert_eq!(
        h.store.state(),
        account(Some("me@example.com"), Some("Me"), Some("Pro"))
    );
}

#[tokio::test]
async fn stored_tokens_are_offered_for_client_reuse() {
    let services = FakeServices::default();
    let previous = services.previous.clone();
    let h = harness_with(services, Some(Credential::Oauth(fixture::tokens("old"))), None);
    sign_in(&h.store).await;
    sign_in(&h.store).await;
    assert_eq!(
        previous.value(),
        [Some(fixture::tokens("old")), Some(fixture::tokens("access"))]
    );
}

#[tokio::test]
async fn fresh_sign_in_has_no_previous_tokens() {
    let services = FakeServices::default();
    let previous = services.previous.clone();
    let h = harness_with(services, Some(key("kid", "s")), None);
    sign_in(&h.store).await;
    assert_eq!(previous.value(), [None]);
}

#[tokio::test]
async fn canceling_the_sheet_cancels_the_browser_sign_in() {
    let cancelled = Shared::new(false);
    let was_cancelled = cancelled.clone();
    let h = harness_with(
        FakeServices::browsing(move |cancel| {
            let was_cancelled = was_cancelled.clone();
            async move {
                cancel.cancelled().await;
                was_cancelled.update(|cancelled| *cancelled = true);
                Err(PortError::Cancelled)
            }
        }),
        None,
        None,
    );
    let task = h.store.start_sign_in();
    h.store.cancel_sign_in();
    task.await.unwrap();
    assert!(cancelled.value());
    assert_eq!(h.store.alert(), None);
    assert_eq!(h.store.activity(), Activity::Idle);
}

#[tokio::test]
async fn refused_refresh_asks_to_sign_in_again() {
    let h = harness_with(
        FakeServices::browsing(|_| async { Err(SignInError::ReauthenticationRequired.into()) }),
        None,
        None,
    );
    sign_in(&h.store).await;
    assert_eq!(h.store.alert().unwrap().title, "Sign in again");
}

#[test]
fn needs_attention_is_ignored_when_disconnected() {
    let h = harness();
    h.store.mark_needs_attention();
    assert!(!h.store.needs_attention());
}

#[tokio::test]
async fn update_account_persists_plan_but_not_credits() {
    let h = harness();
    sign_in(&h.store).await;
    h.store.update_account(AccountInfo {
        email: Some("me@example.com".into()),
        plan: Some("Creator".into()),
        credits: Some(1200),
        name: None,
    });
    assert_eq!(
        h.store.state(),
        ConnectionState::Account(AccountInfo {
            email: Some("me@example.com".into()),
            plan: Some("Creator".into()),
            credits: Some(1200),
            name: None,
        })
    );
    assert_eq!(
        h.metadata.stored(),
        Some(account_metadata("me@example.com", Some("Creator")))
    );
}

// MARK: Balance

#[tokio::test]
async fn balance_fills_in_credits_and_a_missing_one_hides_them() {
    let h = harness();
    sign_in(&h.store).await;

    h.store
        .refresh_balance(&BalanceProvider::credits(1_250, None))
        .await;
    assert!(has_row(&h.store, "Credits"));
    assert_eq!(h.store.state().account_info().unwrap().credits, Some(1_250));

    h.store
        .refresh_balance(&BalanceProvider::new(|| async { Ok(None) }))
        .await;
    assert!(!has_row(&h.store, "Credits"));

    h.store.refresh_balance(&BalanceProvider::credits(7, None)).await;
    h.store
        .refresh_balance(&BalanceProvider::new(|| async {
            Err(ProviderError::Network.into())
        }))
        .await;
    assert!(!has_row(&h.store, "Credits"));
}

#[tokio::test]
async fn balance_names_the_plan_and_an_answer_without_one_keeps_it() {
    let h = harness();
    sign_in(&h.store).await;

    h.store
        .refresh_balance(&BalanceProvider::credits(10, Some("creator_plus")))
        .await;
    assert_eq!(
        h.store.state().account_info().unwrap().plan.as_deref(),
        Some("Creator Plus")
    );
    assert_eq!(h.metadata.stored().unwrap().plan.as_deref(), Some("Creator Plus"));

    h.store.refresh_balance(&BalanceProvider::credits(9, None)).await;
    assert_eq!(
        h.store.state().account_info().unwrap().plan.as_deref(),
        Some("Creator Plus")
    );
    assert!(
        h.store
            .state()
            .card_rows(None)
            .iter()
            .any(|row| row.label == "Plan" && row.value == "Creator Plus")
    );
}

#[tokio::test]
async fn api_key_has_no_balance_row() {
    let h = harness();
    h.store.connect_api_key(&ApiKeyForm::new("kid", "secret")).await;
    let mut provider = BalanceProvider::new(|| async { Ok(Some(Balance::Usd(12.into()))) });
    provider.kind = ProviderKind::ApiKey;
    provider.owner = Some("kid".into());

    h.store.refresh_balance(&provider).await;

    assert_eq!(h.store.state(), ConnectionState::ApiKey { key_id: "kid".into() });
    assert!(!has_row(&h.store, "Credits"));
}

#[tokio::test]
async fn balance_read_for_the_previous_sign_in_never_lands() {
    let h = harness();
    sign_in(&h.store).await;
    let gate = Gate::<Option<Balance>>::new();
    let waiting = gate.clone();
    let provider = BalanceProvider::new(move || {
        let waiting = waiting.clone();
        async move { Ok(waiting.wait().await.flatten()) }
    });
    let store = h.store.clone();
    let refresh = tokio::spawn(async move { store.refresh_balance(&provider).await });
    eventually(|| gate.is_waiting()).await;

    sign_in(&h.store).await;
    gate.open(Some(Balance::Credits {
        credits: 99,
        plan: None,
    }));
    refresh.await.unwrap();

    assert_eq!(h.store.state().account_info().unwrap().credits, None);
}

#[tokio::test]
async fn signing_in_again_drops_the_old_credits() {
    let h = harness();
    sign_in(&h.store).await;
    h.store
        .refresh_balance(&BalanceProvider::credits(500, None))
        .await;

    sign_in(&h.store).await;

    assert_eq!(h.store.state().account_info().unwrap().credits, None);
}

// MARK: Sign out

#[tokio::test]
async fn sign_out_deletes_the_credential_and_reports_none() {
    let h = harness_with(
        FakeServices::default(),
        Some(key("kid", "s")),
        Some(key_metadata("kid")),
    );
    h.store.mark_needs_attention();
    h.store.sign_out().await;
    assert_eq!(h.store.state(), ConnectionState::None);
    assert!(!h.store.needs_attention());
    assert_eq!(h.vault.credential(), None);
    assert_eq!(h.metadata.stored(), None);
    assert_eq!(h.credentials.value(), [None]);
    let restarted = harness_over(FakeServices::default(), h.vault.clone(), h.metadata.clone());
    assert_eq!(restarted.store.state(), ConnectionState::None);
}

#[tokio::test]
async fn sign_out_revokes_the_account_in_the_background() {
    let services = FakeServices::default();
    let revoked = services.revoked.clone();
    let h = harness_with(services, None, None);
    sign_in(&h.store).await;

    h.store.sign_out().await;
    h.store.take_revocation().unwrap().await.unwrap();

    assert_eq!(revoked.value(), [fixture::tokens("access")]);
}

#[tokio::test]
async fn removing_an_api_key_revokes_nothing() {
    let services = FakeServices::default();
    let revoked = services.revoked.clone();
    let h = harness_with(services, Some(key("kid", "s")), Some(key_metadata("kid")));
    h.store.sign_out().await;
    assert!(h.store.take_revocation().is_none());
    assert_eq!(revoked.value(), []);
}

// MARK: Refreshed tokens

#[tokio::test]
async fn refresh_for_the_current_sign_in_is_saved() {
    let h = harness();
    sign_in(&h.store).await;
    let refreshed = fixture::tokens("access-2");

    let saved = h
        .store
        .save_refreshed_tokens(refreshed.clone(), h.store.credential_generation())
        .unwrap();
    assert!(saved);
    assert_eq!(h.vault.credential(), Some(Credential::Oauth(refreshed)));
}

#[tokio::test]
async fn refresh_finishing_after_sign_out_is_dropped() {
    let h = harness();
    sign_in(&h.store).await;
    let generation = h.store.credential_generation();
    h.store.sign_out().await;

    let saved = h
        .store
        .save_refreshed_tokens(fixture::tokens("access-2"), generation)
        .unwrap();
    assert!(!saved);
    assert_eq!(h.vault.credential(), None);
}

#[tokio::test]
async fn refresh_of_the_previous_account_is_dropped_after_a_switch() {
    let sign_ins = Shared::new(0);
    let counted = sign_ins.clone();
    let services = FakeServices::browsing(move |_| {
        counted.update(|count| *count += 1);
        let count = counted.value();
        async move {
            Ok(if count == 1 {
                fixture::tokens_for("a", Some("a@example.com"), None)
            } else {
                fixture::tokens_for("b", Some("b@example.com"), None)
            })
        }
    });
    let h = harness_with(services, None, None);
    sign_in(&h.store).await;
    let first = h.store.credential_generation();
    sign_in(&h.store).await;

    let saved = h
        .store
        .save_refreshed_tokens(fixture::tokens_for("a-2", Some("a@example.com"), None), first)
        .unwrap();
    assert!(!saved);
    assert_eq!(
        h.vault.credential(),
        Some(Credential::Oauth(fixture::tokens_for(
            "b",
            Some("b@example.com"),
            None
        )))
    );
}

#[test]
fn account_tokens_under_another_accounts_name_are_not_restored() {
    let h = harness_with(
        FakeServices::default(),
        Some(Credential::Oauth(fixture::tokens_for(
            "access",
            Some("a@example.com"),
            None,
        ))),
        Some(account_metadata("b@example.com", None)),
    );
    assert_eq!(h.store.state(), ConnectionState::None);
}

#[test]
fn a_key_under_another_keys_name_is_not_restored() {
    let h = harness_with(
        FakeServices::default(),
        Some(key("other", "s")),
        Some(key_metadata("kid")),
    );
    assert_eq!(h.store.state(), ConnectionState::None);
}

#[tokio::test]
async fn failed_delete_keeps_the_connection() {
    let h = harness_with(
        FakeServices::default(),
        Some(key("kid", "s")),
        Some(key_metadata("kid")),
    );
    h.vault.set_fails_delete(true);
    h.store.sign_out().await;
    assert_eq!(h.store.state(), ConnectionState::ApiKey { key_id: "kid".into() });
    assert!(h.store.alert().is_some());
    assert_eq!(h.credentials.value(), []);
}

// MARK: Credential store read failures

#[tokio::test]
async fn store_failing_at_activate_disconnects_with_an_alert() {
    let h = harness_with(
        FakeServices::default(),
        Some(key("kid", "s")),
        Some(key_metadata("kid")),
    );
    h.vault.set_fails_load(true);

    h.store.activate().await;

    assert_eq!(h.store.state(), ConnectionState::None);
    assert!(h.store.alert().is_some());
    assert_eq!(h.credentials.value(), [None]);
    assert!(h.metadata.stored().is_some());
}

#[tokio::test]
async fn store_failing_at_sign_in_stops_before_the_browser() {
    let services = FakeServices::default();
    let previous = services.previous.clone();
    let h = harness_with(services, None, None);
    h.vault.set_fails_load(true);

    sign_in(&h.store).await;

    assert!(h.store.alert().is_some());
    assert_eq!(h.store.activity(), Activity::Idle);
    assert_eq!(previous.value(), []);
    assert_eq!(h.credentials.value(), []);
}

#[tokio::test]
async fn sign_out_says_when_the_tokens_could_not_be_revoked() {
    let h = harness();
    sign_in(&h.store).await;
    h.vault.set_fails_load(true);

    h.store.sign_out().await;

    assert_eq!(h.store.state(), ConnectionState::None);
    assert_eq!(h.store.alert(), Some(UserMessage::not_revoked()));
    assert!(h.store.take_revocation().is_none());
    assert_eq!(h.credentials.value().last(), Some(&None));
}

// MARK: Change broadcast

#[tokio::test]
async fn every_change_is_announced() {
    let h = harness();
    let mut changes = h.store.subscribe();
    h.store.connect_api_key(&ApiKeyForm::new("kid", "s")).await;
    assert!(changes.has_changed().unwrap());
    changes.mark_unchanged();
    h.store.dismiss_alert();
    assert!(changes.has_changed().unwrap());
}
