//! Error shape returned to clients.
//!
//! Bodies are `{"error": "...", "fields": [...]}`. Internal detail (driver
//! messages, upstream bodies, secrets) is logged server-side only.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use trace_core::Error as CoreError;
use tracing::error;

#[derive(Debug, Clone, Serialize)]
pub struct ApiError {
    pub error: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldErr>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldErr {
    pub field: String,
    pub message: String,
}

impl ApiError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            error: message.into(),
            fields: vec![],
        }
    }

    fn with_fields(fields: Vec<FieldErr>) -> Self {
        Self {
            error: "Validation failed".into(),
            fields,
        }
    }
}

#[derive(Debug)]
pub struct AppError(pub StatusCode, pub ApiError);

impl AppError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self(status, ApiError::new(message))
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn internal() -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

/// Converts a core error into a response.
///
/// `Validation`, `NotFound`, `Conflict` and `Cancelled` carry user-safe text.
/// `Storage`, `Secret` and `Http` carry internal detail: it is logged and
/// the client gets a generic message.
pub fn core_err(err: CoreError) -> AppError {
    match err {
        CoreError::Validation(fields) => AppError(
            StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::with_fields(
                fields
                    .into_iter()
                    .map(|f| FieldErr {
                        field: f.field,
                        message: f.message,
                    })
                    .collect(),
            ),
        ),
        CoreError::NotFound(entity) => AppError::not_found(format!("{entity} not found")),
        CoreError::Conflict(msg) => AppError::new(StatusCode::CONFLICT, msg),
        CoreError::Cancelled => AppError::new(StatusCode::CONFLICT, "operation cancelled"),
        CoreError::Storage(detail) => {
            error!(detail, "storage error");
            AppError::internal()
        }
        CoreError::Secret(detail) => {
            error!(detail, "secret store error");
            AppError::internal()
        }
        CoreError::Http(detail) => {
            error!(detail, "upstream http error");
            AppError::new(StatusCode::BAD_GATEWAY, "upstream request failed")
        }
    }
}

pub type HandlerResult<T> = std::result::Result<T, AppError>;

/// Runs blocking core code (SQLite, OS keychain) off the async workers.
pub async fn blocking<T, F>(f: F) -> HandlerResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> trace_core::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result.map_err(core_err),
        Err(_) => {
            error!("blocking task failed");
            Err(AppError::internal())
        }
    }
}
