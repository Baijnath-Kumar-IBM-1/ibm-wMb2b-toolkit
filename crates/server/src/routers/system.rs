use axum::routing::get;
use axum::{Json, Router, extract::State};
use serde::Serialize;
use trace_storage::SecretBackend;

use super::Shared;

pub fn router() -> Router<Shared> {
    Router::new().route("/health", get(health))
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
    /// Where secrets are kept: `keychain` or `encrypted_file`.
    secret_backend: &'static str,
}

async fn health(State(state): State<Shared>) -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        secret_backend: match state.secret_backend {
            SecretBackend::Keychain => "keychain",
            SecretBackend::EncryptedFile => "encrypted_file",
        },
    })
}
