//! Error type shared by all core services and ports.

use serde::Serialize;

/// One invalid input field, safe to show to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

impl FieldError {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// Core error.
///
/// `Validation`, `NotFound`, `Conflict` and `Cancelled` carry user-safe text.
/// `Storage`, `Secret` and `Http` carry internal detail (driver messages and
/// similar) and MUST be logged server-side only; the HTTP layer returns a
/// generic message for them.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("secret store error: {0}")]
    Secret(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("operation cancelled")]
    Cancelled,
}

impl Error {
    pub fn validation(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Validation(vec![FieldError::new(field, message)])
    }

    /// True when the message may be shown to the end user as is.
    pub fn is_user_facing(&self) -> bool {
        matches!(
            self,
            Self::Validation(_) | Self::NotFound(_) | Self::Conflict(_) | Self::Cancelled
        )
    }
}

pub type Result<T> = std::result::Result<T, Error>;
