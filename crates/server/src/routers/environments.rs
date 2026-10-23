use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use secrecy::SecretString;
use serde::Deserialize;
use trace_core::b2b::ConnectionReport;
use trace_core::environments::{EnvironmentInput, EnvironmentView};
use trace_core::ids::EnvironmentId;

use super::Shared;
use crate::error::{HandlerResult, blocking, core_err};
use crate::extract::{ApiJson, PathId};

pub fn router() -> Router<Shared> {
    Router::new()
        .route("/environments", get(list).post(create))
        .route("/environments/{id}", get(show).put(update).delete(remove))
        .route("/environments/{id}/default", post(set_default))
        .route("/environments/{id}/test", post(test_connection))
}

/// The API key is write-only: it is accepted here and never returned.
#[derive(Deserialize)]
struct CreateBody {
    #[serde(flatten)]
    input: EnvironmentInput,
    api_key: SecretString,
}

#[derive(Deserialize)]
struct UpdateBody {
    #[serde(flatten)]
    input: EnvironmentInput,
    /// Omit to keep the stored key.
    #[serde(default)]
    api_key: Option<SecretString>,
}

async fn list(State(state): State<Shared>) -> HandlerResult<Json<Vec<EnvironmentView>>> {
    let envs = blocking(move || state.envs.list()).await?;
    Ok(Json(envs.iter().map(|e| e.view()).collect()))
}

async fn show(
    State(state): State<Shared>,
    PathId(id): PathId<EnvironmentId>,
) -> HandlerResult<Json<EnvironmentView>> {
    let env = blocking(move || state.envs.get(&id)).await?;
    Ok(Json(env.view()))
}

async fn create(
    State(state): State<Shared>,
    ApiJson(body): ApiJson<CreateBody>,
) -> HandlerResult<(StatusCode, Json<EnvironmentView>)> {
    let env = blocking(move || state.envs.create(&body.input, &body.api_key)).await?;
    Ok((StatusCode::CREATED, Json(env.view())))
}

async fn update(
    State(state): State<Shared>,
    PathId(id): PathId<EnvironmentId>,
    ApiJson(body): ApiJson<UpdateBody>,
) -> HandlerResult<Json<EnvironmentView>> {
    let api = state.api.clone();
    let key = id.clone();
    let env = blocking(move || state.envs.update(&key, &body.input, body.api_key.as_ref())).await?;
    // Host, instance or key may have changed: drop the cached token.
    api.forget_token(&id);
    Ok(Json(env.view()))
}

async fn remove(
    State(state): State<Shared>,
    PathId(id): PathId<EnvironmentId>,
) -> HandlerResult<StatusCode> {
    let api = state.api.clone();
    let key = id.clone();
    blocking(move || state.envs.delete(&key)).await?;
    api.forget_token(&id);
    Ok(StatusCode::NO_CONTENT)
}

async fn set_default(
    State(state): State<Shared>,
    PathId(id): PathId<EnvironmentId>,
) -> HandlerResult<StatusCode> {
    blocking(move || state.envs.set_default(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn test_connection(
    State(state): State<Shared>,
    PathId(id): PathId<EnvironmentId>,
) -> HandlerResult<Json<ConnectionReport>> {
    state.connection.test(&id).await.map(Json).map_err(core_err)
}
