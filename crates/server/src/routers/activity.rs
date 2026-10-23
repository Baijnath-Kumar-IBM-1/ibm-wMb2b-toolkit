use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use trace_core::counters::{CounterRepository, Counters};
use trace_core::ids::{EnvironmentId, PartnerId, SubmissionId};
use trace_core::submissions::{Submission, SubmissionRepository};

use super::Shared;
use crate::error::{AppError, HandlerResult, blocking};
use crate::extract::{ApiQuery, PathId};

pub fn router() -> Router<Shared> {
    Router::new()
        .route("/environments/{id}/submissions", get(list))
        .route("/environments/{id}/counters", get(counters))
        .route("/submissions/{id}", get(show))
}

const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 1000;

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<u32>,
}

/// A submission without its (large) payload; `has_payload` says whether
/// `GET /api/submissions/{id}` has one.
#[derive(Serialize)]
struct SubmissionSummary {
    id: SubmissionId,
    environment_id: EnvironmentId,
    #[serde(with = "time::serde::rfc3339")]
    submitted_at: OffsetDateTime,
    domain: String,
    transaction_code: String,
    sender_partner_id: Option<PartnerId>,
    receiver_partner_id: Option<PartnerId>,
    isa_control: u32,
    gs_control: u32,
    http_status: Option<u16>,
    result: String,
    trace_values: Value,
    response_excerpt: Option<String>,
    duration_ms: Option<u64>,
    has_payload: bool,
}

impl From<Submission> for SubmissionSummary {
    fn from(s: Submission) -> Self {
        Self {
            has_payload: s.payload.is_some(),
            id: s.id,
            environment_id: s.environment_id,
            submitted_at: s.submitted_at,
            domain: s.domain,
            transaction_code: s.transaction_code,
            sender_partner_id: s.sender_partner_id,
            receiver_partner_id: s.receiver_partner_id,
            isa_control: s.isa_control,
            gs_control: s.gs_control,
            http_status: s.http_status,
            result: s.result,
            trace_values: s.trace_values,
            response_excerpt: s.response_excerpt,
            duration_ms: s.duration_ms,
        }
    }
}

async fn list(
    State(state): State<Shared>,
    PathId(id): PathId<EnvironmentId>,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> HandlerResult<Json<Vec<SubmissionSummary>>> {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let rows = blocking(move || {
        state.envs.get(&id)?;
        SubmissionRepository::list_recent(state.db.as_ref(), &id, limit)
    })
    .await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

async fn show(
    State(state): State<Shared>,
    PathId(id): PathId<SubmissionId>,
) -> HandlerResult<Json<Submission>> {
    blocking(move || SubmissionRepository::get(state.db.as_ref(), &id))
        .await?
        .map(Json)
        .ok_or_else(|| AppError::not_found("submission not found"))
}

async fn counters(
    State(state): State<Shared>,
    PathId(id): PathId<EnvironmentId>,
) -> HandlerResult<Json<Counters>> {
    blocking(move || {
        state.envs.get(&id)?;
        CounterRepository::peek(state.db.as_ref(), &id)
    })
    .await
    .map(Json)
}
