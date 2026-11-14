use base64::Engine;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde_json::Value;

/// The account an `OpenID` Connect `id_token` names, for display and to tell local jobs of accounts apart. The
/// signature is not verified, so nothing may be authorized on these claims. The token comes straight from the
/// token endpoint over TLS.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct IdTokenClaims {
    pub email: Option<String>,
    pub name: Option<String>,
    pub subject: Option<String>,
}

impl IdTokenClaims {
    /// `None` unless `jwt` is a compact JWS whose payload is a JSON object. Missing or empty claims stay `None`.
    #[must_use]
    pub fn from_jwt(jwt: &str) -> Option<Self> {
        let parts: Vec<&str> = jwt.split('.').collect();
        let [_, payload, _] = parts.as_slice() else {
            return None;
        };
        let data = base64url_decoded(payload)?;
        let Value::Object(payload) = serde_json::from_slice::<Value>(&data).ok()? else {
            return None;
        };
        let text = |claim: &str| {
            payload
                .get(claim)?
                .as_str()
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        Some(Self {
            email: text("email"),
            name: text("name"),
            subject: text("sub"),
        })
    }
}

/// Base64url (RFC 4648 §5), with or without `=` padding.
pub(crate) fn base64url_decoded(text: &str) -> Option<Vec<u8>> {
    const ENGINE: GeneralPurpose = GeneralPurpose::new(
        &STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::RequireNone)
            .with_decode_allow_trailing_bits(true),
    );
    let base64 = text.replace('-', "+").replace('_', "/");
    ENGINE.decode(base64.trim_matches('=')).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "eyJhbGciOiJSUzI1NiJ9";
    /// `{"email":"me@example.com","name":"Me"}`
    const PAYLOAD: &str = "eyJlbWFpbCI6Im1lQGV4YW1wbGUuY29tIiwibmFtZSI6Ik1lIn0";

    fn claims(email: Option<&str>, name: Option<&str>, subject: Option<&str>) -> IdTokenClaims {
        IdTokenClaims {
            email: email.map(str::to_owned),
            name: name.map(str::to_owned),
            subject: subject.map(str::to_owned),
        }
    }

    #[test]
    fn base64url_padding_is_optional() {
        // `{"name":"??>"}` has a `-` and one pad; `{"name":"?~?~~"}` has a `_` and two.
        for (encoded, name) in [
            ("eyJuYW1lIjoiPz8-In0", "??>"),
            ("eyJuYW1lIjoiP34_fn4ifQ", "?~?~~"),
        ] {
            let padding = "=".repeat((4 - encoded.len() % 4) % 4);
            for padded in [encoded.to_owned(), format!("{encoded}{padding}")] {
                let decoded = IdTokenClaims::from_jwt(&format!("{HEADER}.{padded}.")).unwrap();
                assert_eq!(decoded.name.as_deref(), Some(name));
                assert_eq!(decoded.email, None);
            }
        }
    }

    #[test]
    fn missing_or_empty_claims_stay_none() {
        // `{"sub":"1"}` and `{"email":"","name":7}`
        let subject = IdTokenClaims::from_jwt(&format!("{HEADER}.eyJzdWIiOiIxIn0.sig"));
        assert_eq!(subject, Some(claims(None, None, Some("1"))));
        let empty = IdTokenClaims::from_jwt(&format!("{HEADER}.eyJlbWFpbCI6IiIsIm5hbWUiOjd9.sig"));
        assert_eq!(empty, Some(IdTokenClaims::default()));
    }

    #[test]
    fn garbage_is_not_an_id_token() {
        for jwt in [
            String::new(),
            "garbage".into(),
            format!("{HEADER}.{PAYLOAD}"),
            format!("{HEADER}.{PAYLOAD}.sig.extra.parts"),
            format!("{HEADER}.!!!.sig"),
            format!("{HEADER}..sig"),
            format!("{HEADER}.WzFd.sig"),        // `[1]`: JSON, not an object
            format!("{HEADER}.bm90IGpzb24.sig"), // `not json`
            format!("{HEADER}.a.sig"),           // one base64 character can't decode
        ] {
            assert_eq!(IdTokenClaims::from_jwt(&jwt), None, "{jwt}");
        }
    }
}
