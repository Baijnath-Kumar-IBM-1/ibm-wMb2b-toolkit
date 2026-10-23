use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use trace_core::submit::{PreviewReport, PreviewRequest, SubmitRequest};

use super::{RunStarted, Shared, spawn_run, to_value};
use crate::error::{HandlerResult, blocking};
use crate::extract::ApiJson;

pub fn router() -> Router<Shared> {
    Router::new()
        .route("/preview", post(preview))
        .route("/submit", post(submit))
}

/// Renders documents without sending them.
async fn preview(
    State(state): State<Shared>,
    ApiJson(request): ApiJson<PreviewRequest>,
) -> HandlerResult<Json<PreviewReport>> {
    blocking(move || state.submit.preview(&request))
        .await
        .map(Json)
}

/// Starts a submission in the background. One submission per environment at a
/// time, so control numbers and run counts stay in sequence.
async fn submit(
    State(state): State<Shared>,
    ApiJson(request): ApiJson<SubmitRequest>,
) -> HandlerResult<(StatusCode, Json<RunStarted>)> {
    let scope = format!("submit:{}", request.environment_id.as_str());
    let service = state.submit.clone();
    spawn_run(&state, "submission", &scope, move |cancel| async move {
        let report = service.submit(&request, &cancel).await?;
        to_value(&report)
    })
}
