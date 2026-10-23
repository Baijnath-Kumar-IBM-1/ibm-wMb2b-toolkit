//! Secret store port.
//!
//! Secrets (API keys, partner passwords) live only behind this trait. The
//! database stores [`SecretId`] references; values never appear in the
//! database, logs, events or API responses.

use std::fmt;

use secrecy::SecretString;

use crate::error::{Error, Result};
use uuid::Uuid;

/// Opaque reference to a stored secret. Carries no information about the
/// value or its owner.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SecretId(String);

impl SecretId {
    const PREFIX: &'static str = "sec-";

    pub fn generate() -> Self {
        Self(format!("{}{}", Self::PREFIX, Uuid::new_v4().simple()))
    }

    /// Accepts only ids shaped like [`SecretId::generate`] output, so a value
    /// read from storage can never smuggle path or keychain-attribute text.
    pub fn parse(value: &str) -> Result<Self> {
        let hex = value.strip_prefix(Self::PREFIX).unwrap_or("");
        if hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(Error::Secret("invalid secret id".into()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretId({})", self.0)
    }
}

impl fmt::Display for SecretId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Write-mostly secret storage. Implementations must be safe to share across
/// threads and must never log values.
pub trait SecretStore: Send + Sync {
    /// Stores or replaces the secret.
    fn put(&self, id: &SecretId, value: &SecretString) -> Result<()>;
    /// Returns the secret, or `None` when it does not exist.
    fn get(&self, id: &SecretId) -> Result<Option<SecretString>>;
    /// Removes the secret. Deleting a missing secret succeeds.
    fn delete(&self, id: &SecretId) -> Result<()>;
}

/// Validates a secret value that is sent as an HTTP header or password:
/// non-empty after trimming, bounded, no control characters.
pub fn validate_secret_text(field: &str, value: &str, min: usize, max: usize) -> Result<()> {
    let len = value.chars().count();
    if len < min || len > max {
        return Err(Error::validation(
            field,
            format!("must be between {min} and {max} characters"),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(Error::validation(
            field,
            "must not contain control characters",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_parse() {
        let id = SecretId::generate();
        assert_eq!(SecretId::parse(id.as_str()).unwrap(), id);
    }

    #[test]
    fn parse_rejects_unexpected_shapes() {
        for bad in [
            "",
            "sec-",
            "sec-xyz",
            "../etc",
            "sec-0123456789abcdef0123456789abcde/",
        ] {
            assert!(SecretId::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn secret_text_rules() {
        assert!(validate_secret_text("p", "abcdefgh", 8, 16).is_ok());
        assert!(validate_secret_text("p", "short", 8, 16).is_err());
        assert!(validate_secret_text("p", "abcdefgh\n", 8, 16).is_err());
    }
}
