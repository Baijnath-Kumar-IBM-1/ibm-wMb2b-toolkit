//! Secret store implementations.
//!
//! - [`KeychainSecretStore`]: the OS keychain (macOS Keychain, Windows
//!   Credential Manager, Linux Secret Service). Preferred.
//! - [`EncryptedFileSecretStore`]: AES-256-GCM file protected by a
//!   passphrase. Fallback for machines without a keychain service
//!   (for example a headless Linux box running `trace-server`).
//! - [`open_secret_store`] picks one according to a [`SecretBackendPolicy`].

mod file;
mod keychain;

use std::path::Path;
use std::sync::Arc;

use secrecy::SecretString;
use trace_core::secrets::SecretStore;
use trace_core::{Error, Result};

pub use file::{EncryptedFileSecretStore, KdfParams};
pub use keychain::KeychainSecretStore;

/// File name of the encrypted fallback store inside the data directory.
pub const ENCRYPTED_FILE_NAME: &str = "secrets.enc";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretBackend {
    Keychain,
    EncryptedFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecretBackendPolicy {
    /// Keychain when it works, otherwise the encrypted file if a passphrase
    /// was supplied, otherwise an error that explains how to fix it.
    #[default]
    Auto,
    /// Keychain only.
    Keychain,
    /// Encrypted file only (needs a passphrase).
    EncryptedFile,
}

/// Opens the secret store for the application data directory `data_dir`.
///
/// `passphrase` is only used by the encrypted-file backend. It must come
/// from the user at start-up (prompt or environment variable); never store
/// it in the database or in a file.
pub fn open_secret_store(
    data_dir: &Path,
    policy: SecretBackendPolicy,
    passphrase: Option<&SecretString>,
) -> Result<(Arc<dyn SecretStore>, SecretBackend)> {
    let file_store = || -> Result<(Arc<dyn SecretStore>, SecretBackend)> {
        let pass = passphrase.ok_or_else(|| {
            Error::Secret(
                "no OS keychain is available; provide a passphrase \
                 (TRACE_SECRETS_PASSPHRASE) to use the encrypted secrets file"
                    .into(),
            )
        })?;
        let store = EncryptedFileSecretStore::open(&data_dir.join(ENCRYPTED_FILE_NAME), pass)?;
        Ok((Arc::new(store), SecretBackend::EncryptedFile))
    };
    match policy {
        SecretBackendPolicy::EncryptedFile => file_store(),
        SecretBackendPolicy::Keychain => {
            let store = KeychainSecretStore::native()?;
            store.probe()?;
            Ok((Arc::new(store), SecretBackend::Keychain))
        }
        SecretBackendPolicy::Auto => {
            let native = KeychainSecretStore::native().and_then(|s| s.probe().map(|()| s));
            match native {
                Ok(store) => Ok((Arc::new(store), SecretBackend::Keychain)),
                Err(_) => file_store(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(s: &str) -> SecretString {
        SecretString::from(s.to_string())
    }

    #[test]
    fn file_policy_requires_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let err = open_secret_store(dir.path(), SecretBackendPolicy::EncryptedFile, None)
            .err()
            .unwrap();
        assert!(err.to_string().contains("TRACE_SECRETS_PASSPHRASE"));
    }

    #[test]
    fn file_policy_creates_store_in_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let (store, backend) = open_secret_store(
            dir.path(),
            SecretBackendPolicy::EncryptedFile,
            Some(&pass("<your-passphrase>")),
        )
        .unwrap();
        assert_eq!(backend, SecretBackend::EncryptedFile);
        let id = trace_core::secrets::SecretId::generate();
        store.put(&id, &pass("v")).unwrap();
        assert!(dir.path().join(ENCRYPTED_FILE_NAME).exists());
    }
}
