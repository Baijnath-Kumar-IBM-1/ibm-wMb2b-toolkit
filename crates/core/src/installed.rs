//! Installed document types (per environment).
//!
//! Document types are keyed by transaction code (for example `820`), so a
//! code shared by several domains is installed once per environment.

use serde::Serialize;
use time::OffsetDateTime;

use crate::error::{Error, Result};
use crate::ids::EnvironmentId;
use crate::validation;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstalledDocument {
    pub transaction_code: String,
    /// Identifier returned by the B2B API for the document type.
    pub document_type_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub installed_at: OffsetDateTime,
}

/// Transaction codes are 3 digits (X12).
pub fn validate_transaction_code(code: &str) -> Result<String> {
    validation::digits(code, 3, 3).map_err(|m| Error::validation("transaction_code", m))
}

/// Storage port for installed document types.
pub trait InstalledDocumentRepository: Send + Sync {
    fn list(&self, environment_id: &EnvironmentId) -> Result<Vec<InstalledDocument>>;
    /// Inserts or replaces by `(environment, transaction_code)`.
    fn save(&self, environment_id: &EnvironmentId, doc: &InstalledDocument) -> Result<()>;
    fn remove(&self, environment_id: &EnvironmentId, transaction_code: &str) -> Result<()>;
}
