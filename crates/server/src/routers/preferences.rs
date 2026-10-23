use axum::routing::get;
use axum::{Json, Router, extract::State};
use trace_core::preferences::{PreferenceRepository, Preferences};

use super::Shared;
use crate::error::{HandlerResult, blocking};
use crate::extract::ApiJson;

pub fn router() -> Router<Shared> {
    Router::new().route("/preferences", get(show).put(update))
}

async fn show(State(state): State<Shared>) -> HandlerResult<Json<Preferences>> {
    blocking(move || PreferenceRepository::get(state.db.as_ref()))
        .await
        .map(Json)
}

async fn update(
    State(state): State<Shared>,
    ApiJson(preferences): ApiJson<Preferences>,
) -> HandlerResult<Json<Preferences>> {
    blocking(move || {
        preferences.validate()?;
        PreferenceRepository::set(state.db.as_ref(), &preferences)?;
        Ok(preferences)
    })
    .await
    .map(Json)
}
