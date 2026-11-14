//! `CredentialVault` on the system's credential store through `keyring` (`Secrets` on macOS), and in memory for
//! tests.

use std::sync::{Mutex, PoisonError};

use super::ports::{CredentialVault, VaultError};

/// The credential store, as a sentence names it: "The key is deleted from …".
pub fn credential_store_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows Credential Manager"
    } else if cfg!(target_os = "macos") {
        "macOS Keychain"
    } else {
        "the system keyring"
    }
}

/// The credential store, as the connection card's "Stored in" row names it.
pub fn credential_store_title() -> &'static str {
    if cfg!(target_os = "windows") || cfg!(target_os = "macos") {
        credential_store_name()
    } else {
        "System keyring"
    }
}

/// The active connection's credential in the system's credential store (Windows Credential Manager, the Secret
/// Service on Linux, the login keychain on macOS): one generic item under `service`, account `active`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyringVault {
    service: String,
}

impl KeyringVault {
    pub const ACCOUNT: &'static str = "active";
    pub const SERVICE: &'static str = "com.jutsustudio.desktop.connection";
    /// A debug build keeps its own item, so it never meets the one a release build wrote.
    pub const DEBUG_SERVICE: &'static str = "com.jutsustudio.desktop.connection.debug";

    /// The item of this build: `DEBUG_SERVICE` in debug builds, `SERVICE` otherwise.
    pub fn for_this_build() -> Self {
        Self::new(if cfg!(debug_assertions) {
            Self::DEBUG_SERVICE
        } else {
            Self::SERVICE
        })
    }

    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    pub fn service(&self) -> &str {
        &self.service
    }

    fn entry(&self) -> Result<keyring::Entry, VaultError> {
        keyring::Entry::new(&self.service, Self::ACCOUNT).map_err(|error| store_error(&error))
    }
}

fn store_error(error: &keyring::Error) -> VaultError {
    VaultError::Store(error.to_string())
}

// note: `keyring` calls block for as long as the system store takes, usually milliseconds; they run where the
// connection store runs, as `KeychainStore` does on the main actor in the macOS app.
impl CredentialVault for KeyringVault {
    fn save_data(&self, data: &[u8]) -> Result<(), VaultError> {
        self.entry()?
            .set_secret(data)
            .map_err(|error| store_error(&error))
    }

    fn load_data(&self) -> Result<Option<Vec<u8>>, VaultError> {
        match self.entry()?.get_secret() {
            Ok(data) => Ok(Some(data)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(store_error(&error)),
        }
    }

    fn delete(&self) -> Result<(), VaultError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(store_error(&error)),
        }
    }
}

/// A vault in memory, for tests and previews. `fails_load` and `fails_delete` make the next calls fail as a locked
/// store would.
#[derive(Debug, Default)]
pub struct MemoryVault {
    state: Mutex<MemoryVaultState>,
}

#[derive(Debug, Default)]
struct MemoryVaultState {
    data: Option<Vec<u8>>,
    fails_load: bool,
    fails_delete: bool,
}

impl MemoryVault {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_credential(credential: &jutsu_core::Credential) -> Self {
        let vault = Self::new();
        let _ = vault.save_credential(credential);
        vault
    }

    pub fn set_fails_load(&self, fails: bool) {
        self.locked().fails_load = fails;
    }

    pub fn set_fails_delete(&self, fails: bool) {
        self.locked().fails_delete = fails;
    }

    /// What is stored, read past `fails_load`.
    pub fn credential(&self) -> Option<jutsu_core::Credential> {
        let data = self.locked().data.clone()?;
        serde_json::from_slice(&data).ok()
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, MemoryVaultState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl CredentialVault for MemoryVault {
    fn save_data(&self, data: &[u8]) -> Result<(), VaultError> {
        self.locked().data = Some(data.to_vec());
        Ok(())
    }

    fn load_data(&self) -> Result<Option<Vec<u8>>, VaultError> {
        let state = self.locked();
        if state.fails_load {
            return Err(VaultError::Store("The credential store is locked.".into()));
        }
        Ok(state.data.clone())
    }

    fn delete(&self) -> Result<(), VaultError> {
        let mut state = self.locked();
        if state.fails_delete {
            return Err(VaultError::Store(
                "The credential store refused the delete.".into(),
            ));
        }
        state.data = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use jutsu_core::Credential;

    use super::*;

    #[test]
    fn the_debug_build_has_its_own_item() {
        assert_eq!(
            KeyringVault::for_this_build().service(),
            "com.jutsustudio.desktop.connection.debug"
        );
    }

    #[test]
    fn the_credential_is_stored_as_the_macos_app_codes_it() {
        let vault = MemoryVault::new();
        vault
            .save_credential(&Credential::ApiKey {
                id: "kid".into(),
                secret: "s".into(),
            })
            .unwrap();
        let data = vault.load_data().unwrap().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&data).unwrap(),
            serde_json::json!({ "apiKey": { "id": "kid", "secret": "s" } })
        );
    }

    #[test]
    fn a_stored_value_that_is_not_a_credential_is_a_decoding_error() {
        let vault = MemoryVault::new();
        vault.save_data(b"not json").unwrap();
        assert!(matches!(vault.load_credential(), Err(VaultError::Decoding(_))));
    }

    /// The real OS store, on a throwaway service: run with `cargo test -p jutsu-app -- --ignored` on a desktop
    /// session with an unlocked credential store.
    #[test]
    #[ignore = "uses the system credential store"]
    fn the_system_store_round_trips() {
        let vault = KeyringVault::new(format!(
            "com.jutsustudio.desktop.connection.test-{}",
            std::process::id()
        ));
        let credential = Credential::ApiKey {
            id: "kid".into(),
            secret: "secret".into(),
        };
        vault.delete().unwrap();
        assert_eq!(vault.load_credential().unwrap(), None);
        vault.save_credential(&credential).unwrap();
        assert_eq!(vault.load_credential().unwrap(), Some(credential));
        vault.delete().unwrap();
        vault.delete().unwrap();
        assert_eq!(vault.load_credential().unwrap(), None);
    }
}
