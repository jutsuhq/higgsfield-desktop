//! `CredentialTests.swift`.

use jutsu_core::*;
use time::OffsetDateTime;

fn tokens() -> OAuthTokens {
    OAuthTokens {
        access_token: "ACCESS-SECRET".into(),
        refresh_token: Some("REFRESH-SECRET".into()),
        expires_at: OffsetDateTime::UNIX_EPOCH,
        scope: "openid offline_access".into(),
        client_id: "https://example.com/client.json".into(),
        issuer: None,
        email: None,
        name: None,
        subject: None,
    }
}

#[test]
fn api_key_description_redacts_the_secret() {
    let credential = Credential::ApiKey {
        id: "key-id".into(),
        secret: "KEY-SECRET".into(),
    };
    for text in [
        credential.to_string(),
        format!("{credential:?}"),
        format!("{credential}"),
    ] {
        assert!(!text.contains("KEY-SECRET"));
        assert!(text.contains("key-id"));
    }
}

#[test]
fn oauth_description_redacts_both_tokens() {
    let tokens = tokens();
    for text in [
        Credential::Oauth(tokens.clone()).to_string(),
        format!("{tokens:?}"),
        format!("{tokens}"),
    ] {
        assert!(!text.contains("ACCESS-SECRET"));
        assert!(!text.contains("REFRESH-SECRET"));
        assert!(text.contains("openid offline_access"));
    }
}

#[test]
fn credential_round_trips_through_json() {
    for credential in [
        Credential::ApiKey {
            id: "id".into(),
            secret: "secret".into(),
        },
        Credential::Oauth(tokens()),
    ] {
        let data = serde_json::to_string(&credential).unwrap();
        assert_eq!(serde_json::from_str::<Credential>(&data).unwrap(), credential);
    }
}

#[test]
fn tokens_stored_before_subjects_decode_and_fall_back_to_the_email() {
    let mut with_subject = tokens();
    with_subject.email = Some("me@example.com".into());
    with_subject.subject = Some("user_1".into());
    assert_eq!(with_subject.owner(), Some("user_1"));
    let mut object = serde_json::to_value(&with_subject).unwrap();
    assert!(object.as_object_mut().unwrap().remove("subject").is_some());

    let legacy: OAuthTokens = serde_json::from_value(object).unwrap();

    assert_eq!(legacy.subject, None);
    assert_eq!(legacy.owner(), Some("me@example.com"));
    assert_eq!(tokens().owner(), None);
}
