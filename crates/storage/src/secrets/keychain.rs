//! OS keychain secret store.

use std::fmt;
use std::sync::Arc;

use keyring_core::api::CredentialStore;
use secrecy::{ExposeSecret, SecretString};
use trace_core::secrets::{SecretId, SecretStore};
use trace_core::{Error, Result};

/// Service name under which all entries are grouped in the keychain.
const SERVICE: &str = "ibm-b2b-toolkit-trace";

fn secret_err(e: &keyring_core::Error) -> Error {
    // Keychain errors can name the OS component or entry; they stay in the
    // internal `Secret` variant, which is never shown to users or logged
    // with values.
    Error::Secret(format!("keychain: {e}"))
}

pub struct KeychainSecretStore {
    store: Arc<CredentialStore>,
}

impl fmt::Debug for KeychainSecretStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeychainSecretStore")
    }
}

impl KeychainSecretStore {
    /// The platform credential store (initialised on first use).
    pub fn native() -> Result<Self> {
        if let Err(e) = keyring::Entry::store_status() {
            return Err(secret_err(e));
        }
        keyring_core::get_default_store()
            .map(|store| Self { store })
            .ok_or_else(|| Error::Secret("keychain: no credential store".into()))
    }

    /// Uses a specific credential store (tests use `keyring_core::mock`).
    pub fn with_store(store: Arc<CredentialStore>) -> Self {
        Self { store }
    }

    fn entry(&self, user: &str) -> Result<keyring_core::Entry> {
        self.store
            .build(SERVICE, user, None)
            .map_err(|e| secret_err(&e))
    }

    /// Writes, reads back and deletes a throw-away entry to prove the
    /// keychain is reachable (a Secret Service may be installed but locked,
    /// or absent on a headless machine).
    pub fn probe(&self) -> Result<()> {
        let entry = self.entry("probe")?;
        entry.set_password("probe").map_err(|e| secret_err(&e))?;
        let read = entry.get_password().map_err(|e| secret_err(&e));
        let _ = entry.delete_credential();
        if read? == "probe" {
            Ok(())
        } else {
            Err(Error::Secret(
                "keychain: probe read back a different value".into(),
            ))
        }
    }
}

impl SecretStore for KeychainSecretStore {
    fn put(&self, id: &SecretId, value: &SecretString) -> Result<()> {
        self.entry(id.as_str())?
            .set_password(value.expose_secret())
            .map_err(|e| secret_err(&e))
    }

    fn get(&self, id: &SecretId) -> Result<Option<SecretString>> {
        match self.entry(id.as_str())?.get_password() {
            Ok(v) => Ok(Some(SecretString::from(v))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(secret_err(&e)),
        }
    }

    fn delete(&self, id: &SecretId) -> Result<()> {
        match self.entry(id.as_str())?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(secret_err(&e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> KeychainSecretStore {
        KeychainSecretStore::with_store(keyring_core::mock::Store::new().unwrap())
    }

    fn val(s: &str) -> SecretString {
        SecretString::from(s.to_string())
    }

    #[test]
    fn put_get_replace_delete() {
        let s = store();
        let id = SecretId::generate();
        assert!(s.get(&id).unwrap().is_none());
        s.put(&id, &val("one")).unwrap();
        assert_eq!(s.get(&id).unwrap().unwrap().expose_secret(), "one");
        s.put(&id, &val("two")).unwrap();
        assert_eq!(s.get(&id).unwrap().unwrap().expose_secret(), "two");
        s.delete(&id).unwrap();
        assert!(s.get(&id).unwrap().is_none());
        s.delete(&id).unwrap();
    }

    #[test]
    fn ids_are_independent() {
        let s = store();
        let (a, b) = (SecretId::generate(), SecretId::generate());
        s.put(&a, &val("A")).unwrap();
        s.put(&b, &val("B")).unwrap();
        s.delete(&a).unwrap();
        assert_eq!(s.get(&b).unwrap().unwrap().expose_secret(), "B");
    }

    #[test]
    fn probe_succeeds_and_leaves_nothing_behind() {
        let s = store();
        s.probe().unwrap();
        // The probe entry is deleted afterwards.
        assert!(matches!(
            s.entry("probe").unwrap().get_password(),
            Err(keyring_core::Error::NoEntry)
        ));
    }
}
