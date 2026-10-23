use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use secrecy::SecretString;
use serde::Deserialize;
use trace_core::b2b::PartnerProvisioning;
use trace_core::ids::{EnvironmentId, PartnerId};
use trace_core::partners::{PartnerInput, PartnerView};

use super::{RunStarted, Shared, spawn_run, to_value};
use crate::error::{HandlerResult, blocking};
use crate::extract::{ApiJson, PathId};

pub fn router() -> Router<Shared> {
    Router::new()
        .route("/environments/{id}/partners", get(list).post(create))
        .route("/partners/{id}", get(show).put(update).delete(remove))
        .route("/partners/{id}/provisioning", get(provisioning))
        .route("/partners/{id}/provision", post(provision))
}

/// The password is write-only: it is accepted here and never returned.
#[derive(Deserialize)]
struct CreateBody {
    #[serde(flatten)]
    input: PartnerInput,
    password: SecretString,
}

#[derive(Deserialize)]
struct UpdateBody {
    #[serde(flatten)]
    input: PartnerInput,
    /// Omit to keep the stored password.
    #[serde(default)]
    password: Option<SecretString>,
}

async fn list(
    State(state): State<Shared>,
    PathId(env_id): PathId<EnvironmentId>,
) -> HandlerResult<Json<Vec<PartnerView>>> {
    let partners = blocking(move || state.partners.list(&env_id)).await?;
    Ok(Json(partners.iter().map(|p| p.view()).collect()))
}

async fn show(
    State(state): State<Shared>,
    PathId(id): PathId<PartnerId>,
) -> HandlerResult<Json<PartnerView>> {
    let partner = blocking(move || state.partners.get(&id)).await?;
    Ok(Json(partner.view()))
}

async fn create(
    State(state): State<Shared>,
    PathId(env_id): PathId<EnvironmentId>,
    ApiJson(body): ApiJson<CreateBody>,
) -> HandlerResult<(StatusCode, Json<PartnerView>)> {
    let partner =
        blocking(move || state.partners.create(&env_id, &body.input, &body.password)).await?;
    Ok((StatusCode::CREATED, Json(partner.view())))
}

async fn update(
    State(state): State<Shared>,
    PathId(id): PathId<PartnerId>,
    ApiJson(body): ApiJson<UpdateBody>,
) -> HandlerResult<Json<PartnerView>> {
    let partner = blocking(move || {
        state
            .partners
            .update(&id, &body.input, body.password.as_ref())
    })
    .await?;
    Ok(Json(partner.view()))
}

async fn remove(
    State(state): State<Shared>,
    PathId(id): PathId<PartnerId>,
) -> HandlerResult<StatusCode> {
    blocking(move || state.partners.delete(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn provisioning(
    State(state): State<Shared>,
    PathId(id): PathId<PartnerId>,
) -> HandlerResult<Json<PartnerProvisioning>> {
    blocking(move || state.provisioning.status(&id))
        .await
        .map(Json)
}

/// Starts (or resumes) provisioning in the background.
async fn provision(
    State(state): State<Shared>,
    PathId(id): PathId<PartnerId>,
) -> HandlerResult<(StatusCode, Json<RunStarted>)> {
    // Fail fast with 404 for an unknown partner instead of a failed run.
    let check = state.clone();
    let probe = id.clone();
    blocking(move || check.partners.get(&probe).map(|_| ())).await?;
    let scope = format!("partner:{}", id.as_str());
    let service = state.provisioning.clone();
    spawn_run(
        &state,
        "partner.provision",
        &scope,
        move |cancel| async move {
            let report = service.provision(&id, &cancel).await?;
            to_value(&report)
        },
    )
}
