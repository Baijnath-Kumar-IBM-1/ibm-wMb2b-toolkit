use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};

use super::Shared;
use crate::error::{AppError, HandlerResult};
use crate::state::RunView;

pub fn router() -> Router<Shared> {
    Router::new()
        .route("/runs/{id}", get(get_run))
        .route("/runs/{id}/cancel", post(cancel_run))
}

async fn get_run(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> HandlerResult<Json<RunView>> {
    state
        .runs
        .get(&id)
        .map(Json)
        .ok_or_else(|| AppError::not_found("run not found"))
}

async fn cancel_run(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> HandlerResult<StatusCode> {
    if state.runs.cancel(&id) {
        Ok(StatusCode::ACCEPTED)
    } else {
        Err(AppError::not_found("no active run with this id"))
    }
}
