//! Toolkit v2 storage.
//!
//! - [`Database`]: SQLite (versioned migrations) implementing the repository
//!   ports from `trace-core`.
//! - [`secrets`]: OS-keychain secret store with an encrypted-file fallback.

mod db;
mod fsutil;
pub mod migrations;
pub mod secrets;
mod sqlite;

pub use db::Database;
pub use secrets::{
    EncryptedFileSecretStore, KeychainSecretStore, SecretBackend, SecretBackendPolicy,
    open_secret_store,
};

#[cfg(test)]
mod tests;
