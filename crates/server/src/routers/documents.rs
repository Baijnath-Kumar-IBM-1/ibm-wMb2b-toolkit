use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use trace_core::ids::EnvironmentId;
use trace_core::installed::InstalledDocument;

use super::{RunStarted, Shared, spawn_run, to_value};
use crate::error::{HandlerResult, blocking};
use crate::extract::{ApiJson, PathId};

pub fn router() -> Router<Shared> {
    Router::new()
        .route("/environments/{id}/documents", get(installed))
        .route("/environments/{id}/documents/install", post(install))
}

#[derive(Deserialize)]
struct InstallBody {
    pack_id: String,
    /// Transaction codes to install; omit for the whole pack.
    #[serde(default)]
    codes: Option<Vec<String>>,
}

async fn installed(
    State(state): State<Shared>,
    PathId(id): PathId<EnvironmentId>,
) -> HandlerResult<Json<Vec<InstalledDocument>>> {
    blocking(move || state.documents.list_installed(&id))
        .await
        .map(Json)
}

async fn install(
    State(state): State<Shared>,
    PathId(id): PathId<EnvironmentId>,
    ApiJson(body): ApiJson<InstallBody>,
) -> HandlerResult<(StatusCode, Json<RunStarted>)> {
    let scope = format!("documents:{}", id.as_str());
    let service = state.documents.clone();
    spawn_run(
        &state,
        "documents.install",
        &scope,
        move |cancel| async move {
            let report = service
                .install(&id, &body.pack_id, body.codes.as_deref(), &cancel)
                .await?;
            to_value(&report)
        },
    )
}
