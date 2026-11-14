use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// SHA-256 for the S256 code challenge; tests inject their own.
pub trait PkceHasher: Send + Sync {
    fn sha256(&self, data: &[u8]) -> Vec<u8>;
}

/// The `sha2` hasher.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sha256Hasher;

impl PkceHasher for Sha256Hasher {
    fn sha256(&self, data: &[u8]) -> Vec<u8> {
        Sha256::digest(data).to_vec()
    }
}

/// A PKCE verifier and its S256 challenge (RFC 7636).
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new(hasher: &dyn PkceHasher) -> Result<Self, getrandom::Error> {
        Ok(Self::with_verifier(random_url_safe_string(64)?, hasher))
    }

    pub fn with_verifier(verifier: String, hasher: &dyn PkceHasher) -> Self {
        let challenge = base64url_encoded(&hasher.sha256(verifier.as_bytes()));
        Self { verifier, challenge }
    }
}

impl std::fmt::Debug for Pkce {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pkce")
            .field("challenge", &self.challenge)
            .finish_non_exhaustive()
    }
}

/// Base64url without padding (RFC 7636 Appendix A).
pub(crate) fn base64url_encoded(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

const ALPHABET: &[u8; 66] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

/// Random string over the RFC 3986 unreserved characters; PKCE verifiers allow exactly these. Bytes at or above
/// the largest multiple of the alphabet's size are drawn again, so every character is equally likely.
pub(crate) fn random_url_safe_string(length: usize) -> Result<String, getrandom::Error> {
    #[allow(clippy::cast_possible_truncation)]
    const LIMIT: u8 = (256 / ALPHABET.len() * ALPHABET.len()) as u8;
    let mut text = String::with_capacity(length);
    let mut buffer = [0_u8; 64];
    while text.len() < length {
        getrandom::fill(&mut buffer)?;
        for &byte in buffer.iter().filter(|&&byte| byte < LIMIT) {
            if text.len() == length {
                break;
            }
            text.push(char::from(ALPHABET[usize::from(byte) % ALPHABET.len()]));
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// Returns fixed bytes whose base64 contains `+`, `/` and `=`, so the base64url rewrite is visible.
    struct FakeHasher;

    impl PkceHasher for FakeHasher {
        fn sha256(&self, _: &[u8]) -> Vec<u8> {
            vec![0xfb, 0xff, 0xfe, 0xfb, 0xff]
        }
    }

    #[test]
    fn verifier_uses_unreserved_characters_and_valid_length() {
        let verifiers: Vec<String> = (0..20)
            .map(|_| Pkce::new(&FakeHasher).unwrap().verifier)
            .collect();
        for verifier in &verifiers {
            assert!((43..=128).contains(&verifier.len()));
            assert!(verifier.bytes().all(|byte| ALPHABET.contains(&byte)));
        }
        assert_eq!(verifiers.iter().collect::<HashSet<_>>().len(), verifiers.len());
        assert_eq!(Pkce::with_verifier("v".into(), &FakeHasher).challenge, "-__--_8");
    }

    #[test]
    fn sha256_hasher_matches_rfc_7636_vector() {
        let pkce = Pkce::with_verifier(
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
            &Sha256Hasher,
        );
        assert_eq!(pkce.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }
}
