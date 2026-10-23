//! One router per feature, composed in [`api`]. To add a feature, add a
//! module here and one `.merge(...)` line below.

use std::future::Future;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::http::StatusCode;
use serde::Serialize;
use serde_json::Value;
use tracing::error;

use crate::error::{AppError, HandlerResult, core_err};
use crate::state::{AppState, StartError};

pub mod activity;
pub mod documents;
pub mod environments;
pub mod events;
pub mod packs;
pub mod partners;
pub mod preferences;
pub mod runs;
pub mod submit;
pub mod system;

pub type Shared = Arc<AppState>;

/// All `/api` routes (without the guards).
pub fn api() -> Router<Shared> {
    Router::new()
        .merge(system::router())
        .merge(events::router())
        .merge(runs::router())
        .merge(environments::router())
        .merge(partners::router())
        .merge(documents::router())
        .merge(packs::router())
        .merge(submit::router())
        .merge(activity::router())
        .merge(preferences::router())
}

#[derive(Debug, Serialize)]
pub struct RunStarted {
    pub run_id: String,
}

/// Starts a background run and answers `202 Accepted` with its id.
///
/// `scope` allows one active run per scope (for example per partner); a
/// second start answers `409`. The run's outcome is read from
/// `GET /api/runs/{id}` and from the event stream.
pub fn spawn_run<F, Fut>(
    state: &Shared,
    kind: &'static str,
    scope: &str,
    work: F,
) -> HandlerResult<(StatusCode, Json<RunStarted>)>
where
    F: FnOnce(trace_core::steps::CancelToken) -> Fut,
    Fut: Future<Output = trace_core::Result<Value>> + Send + 'static,
{
    let (id, cancel) = state
        .runs
        .start(kind, scope, state.clock.now())
        .map_err(|e| match e {
            StartError::Busy => AppError::new(
                StatusCode::CONFLICT,
                "this operation is already running; wait for it to finish",
            ),
            StartError::NoRandom => AppError::internal(),
        })?;
    let task = tokio::spawn(work(cancel));
    let state = state.clone();
    let run_id = id.clone();
    tokio::spawn(async move {
        let outcome = match task.await {
            Ok(Ok(report)) => Ok(report),
            Ok(Err(e)) => Err(core_err(e).1),
            Err(_) => {
                error!(run = %run_id, "run task failed");
                Err(AppError::internal().1)
            }
        };
        state.runs.finish(&run_id, outcome);
    });
    Ok((StatusCode::ACCEPTED, Json(RunStarted { run_id: id })))
}

/// Serialises a report; reports are plain data so this cannot fail in
/// practice, but a failure is mapped instead of unwrapped.
pub fn to_value<T: Serialize>(report: &T) -> trace_core::Result<Value> {
    serde_json::to_value(report)
        .map_err(|e| trace_core::Error::Storage(format!("report serialisation: {e}")))
}
