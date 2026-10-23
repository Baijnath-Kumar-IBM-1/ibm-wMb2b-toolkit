//! Partner provisioning: the five tenant calls from `provision_assets`, run
//! as saved, resumable steps.
//!
//! Step names are persisted keys (see [`crate::steps`]); do not rename them.
//! Outputs are remote ids only. The partner password is read from the secret
//! store inside the `create_user` step and never leaves it.

use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

use secrecy::ExposeSecret;
use serde::Serialize;
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::b2b::api::{B2bApi, expect_json, json_text, path_segment};
use crate::clock::Clock;
use crate::environments::{Environment, EnvironmentRepository};
use crate::error::{Error, Result};
use crate::events::{Event, EventLevel, EventPublisher};
use crate::http::Method;
use crate::ids::PartnerId;
use crate::partners::{Partner, PartnerRepository, partner_scope};
use crate::secrets::SecretStore;
use crate::steps::{
    CancelToken, FnStep, RunReport, Step, StepContext, StepRunner, StepStatus, StepStore,
};

pub const STEP_CREATE_PARTNER: &str = "create_partner";
pub const STEP_CREATE_CHANNEL: &str = "create_channel";
pub const STEP_ASSOCIATE_CHANNEL: &str = "associate_channel";
pub const STEP_CREATE_USER: &str = "create_user";
pub const STEP_ASSOCIATE_USER: &str = "associate_user";

/// Step names in run order (same order as the script's `provision_assets`).
pub const PROVISION_STEPS: [&str; 5] = [
    STEP_CREATE_PARTNER,
    STEP_CREATE_CHANNEL,
    STEP_ASSOCIATE_CHANNEL,
    STEP_CREATE_USER,
    STEP_ASSOCIATE_USER,
];

const EVENT_PREFIX: &str = "partner.provision";

/// State of one step as shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepState {
    pub name: &'static str,
    /// `pending`, `succeeded` or `failed`.
    pub status: &'static str,
    pub error: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub updated_at: Option<OffsetDateTime>,
}

/// Provisioning state of one partner, built from the saved step records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartnerProvisioning {
    pub partner_id: PartnerId,
    pub steps: Vec<StepState>,
    /// True when every step has succeeded.
    pub complete: bool,
    /// Tenant ids (not secrets), available once the step has succeeded.
    pub partner_internal_id: Option<String>,
    pub channel_id: Option<String>,
    /// URL documents are posted to (used by the submission engine).
    pub channel_endpoint_uri: Option<String>,
    pub user_id: Option<String>,
}

struct Shared {
    api: Arc<B2bApi>,
    env: Environment,
    partner: Partner,
    secrets: Arc<dyn SecretStore>,
}

pub struct ProvisioningService {
    api: Arc<B2bApi>,
    partners: Arc<dyn PartnerRepository>,
    envs: Arc<dyn EnvironmentRepository>,
    step_store: Arc<dyn StepStore>,
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
    events: Arc<dyn EventPublisher>,
    running: Mutex<HashSet<PartnerId>>,
}

/// Releases the per-partner run lock, also when the future is dropped.
struct RunGuard<'a> {
    set: &'a Mutex<HashSet<PartnerId>>,
    id: PartnerId,
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        lock(self.set).remove(&self.id);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl ProvisioningService {
    pub fn new(
        api: Arc<B2bApi>,
        partners: Arc<dyn PartnerRepository>,
        envs: Arc<dyn EnvironmentRepository>,
        step_store: Arc<dyn StepStore>,
        secrets: Arc<dyn SecretStore>,
        clock: Arc<dyn Clock>,
        events: Arc<dyn EventPublisher>,
    ) -> Self {
        Self {
            api,
            partners,
            envs,
            step_store,
            secrets,
            clock,
            events,
            running: Mutex::new(HashSet::new()),
        }
    }

    /// Saved state of a partner's provisioning, without contacting the tenant.
    pub fn status(&self, partner_id: &PartnerId) -> Result<PartnerProvisioning> {
        self.partners
            .get(partner_id)?
            .ok_or(Error::NotFound("partner"))?;
        let records = self.step_store.list(&partner_scope(partner_id))?;
        let find = |name: &str| records.iter().find(|r| r.name == name);
        let steps: Vec<StepState> = PROVISION_STEPS
            .iter()
            .map(|&name| match find(name) {
                Some(r) => StepState {
                    name,
                    status: r.status.as_str(),
                    error: r.error.clone(),
                    updated_at: Some(r.updated_at),
                },
                None => StepState {
                    name,
                    status: "pending",
                    error: None,
                    updated_at: None,
                },
            })
            .collect();
        let out = |step: &str, field: &str| {
            find(step)
                .filter(|r| r.status == StepStatus::Succeeded)
                .and_then(|r| json_text(&r.output, &format!("/{field}")))
        };
        Ok(PartnerProvisioning {
            partner_id: partner_id.clone(),
            complete: steps.iter().all(|s| s.status == "succeeded"),
            partner_internal_id: out(STEP_CREATE_PARTNER, "partner_id"),
            channel_id: out(STEP_CREATE_CHANNEL, "channel_id"),
            channel_endpoint_uri: out(STEP_CREATE_CHANNEL, "endpoint_uri"),
            user_id: out(STEP_CREATE_USER, "user_id"),
            steps,
        })
    }

    /// Runs the provisioning steps that have not succeeded yet. Calling it
    /// again after a failure resumes at the failed step; calling it for a
    /// complete partner makes no tenant calls.
    ///
    /// A failing step is reported in the returned [`RunReport`], not as
    /// `Err`. Only one run per partner can be active at a time.
    pub async fn provision(
        &self,
        partner_id: &PartnerId,
        cancel: &CancelToken,
    ) -> Result<RunReport> {
        let partner = self
            .partners
            .get(partner_id)?
            .ok_or(Error::NotFound("partner"))?;
        let env = self
            .envs
            .get(&partner.environment_id)?
            .ok_or(Error::NotFound("environment"))?;
        if env.api_key_secret.is_none() {
            return Err(Error::Conflict(
                "this environment has no API key; add one in its settings".into(),
            ));
        }
        if partner.password_secret.is_none() {
            return Err(Error::Conflict(
                "this partner has no password; set one before provisioning".into(),
            ));
        }
        let _guard = self.acquire(partner_id)?;

        let shared = Arc::new(Shared {
            api: self.api.clone(),
            env,
            partner,
            secrets: self.secrets.clone(),
        });
        let steps = build_steps(&shared);
        let runner = StepRunner::new(
            self.step_store.clone(),
            self.events.clone(),
            self.clock.clone(),
        );
        let report = runner
            .run(&partner_scope(partner_id), EVENT_PREFIX, &steps, cancel)
            .await?;

        let (level, kind, message) = if report.is_complete() {
            (EventLevel::Info, "partner.provisioned", "provisioned")
        } else if report.cancelled {
            (EventLevel::Warn, "partner.provision_cancelled", "cancelled")
        } else {
            (EventLevel::Error, "partner.provision_failed", "failed")
        };
        self.events.publish(
            Event::new(
                self.clock.now(),
                level,
                kind,
                format!("Partner '{}' {message}", shared.partner.name),
            )
            .with_data(json!({
                "environment_id": shared.partner.environment_id,
                "partner_id": partner_id,
            })),
        );
        Ok(report)
    }

    fn acquire(&self, id: &PartnerId) -> Result<RunGuard<'_>> {
        if !lock(&self.running).insert(id.clone()) {
            return Err(Error::Conflict(
                "provisioning is already running for this partner".into(),
            ));
        }
        Ok(RunGuard {
            set: &self.running,
            id: id.clone(),
        })
    }
}

fn make_step<F, Fut>(name: &str, f: F) -> Box<dyn Step>
where
    F: Fn(StepContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value>> + Send + 'static,
{
    Box::new(FnStep::new(name, move |ctx| Box::pin(f(ctx))))
}

fn build_steps(shared: &Arc<Shared>) -> Vec<Box<dyn Step>> {
    let s = shared.clone();
    let create_partner = make_step(STEP_CREATE_PARTNER, move |_| create_partner(s.clone()));
    let s = shared.clone();
    let create_channel = make_step(STEP_CREATE_CHANNEL, move |_| create_channel(s.clone()));
    let s = shared.clone();
    let associate_channel = make_step(STEP_ASSOCIATE_CHANNEL, move |c| {
        associate_channel(s.clone(), c)
    });
    let s = shared.clone();
    let create_user = make_step(STEP_CREATE_USER, move |_| create_user(s.clone()));
    let s = shared.clone();
    let associate_user = make_step(STEP_ASSOCIATE_USER, move |c| associate_user(s.clone(), c));
    vec![
        create_partner,
        create_channel,
        associate_channel,
        create_user,
        associate_user,
    ]
}

/// `POST /b2b/api/v1/partners` (script: `create_partner`).
async fn create_partner(s: Arc<Shared>) -> Result<Value> {
    let p = &s.partner;
    let body = json!({
        "corporation": {
            "corporationName": p.name,
            "orgUnitName": p.org_unit,
            "routingOff": false,
            "status": "Active",
        },
        "identities": [
            { "externalIdType": p.id_type.as_str(), "externalIdValue": p.id_value }
        ],
    });
    let response = s
        .api
        .call(&s.env, Method::Post, "/b2b/api/v1/partners", Some(&body))
        .await?;
    let value = expect_json(&response, "creating the partner", &[])?;
    let id = json_text(&value, "/internalId").ok_or_else(|| {
        Error::Http("partner was created but the response has no internalId".into())
    })?;
    Ok(json!({ "partner_id": path_segment(&id)? }))
}

/// `POST /b2b/api/v1/channels` (script: `create_channel`).
async fn create_channel(s: Arc<Shared>) -> Result<Value> {
    let body = json!({
        "channeltype": "HTTP-IN",
        "name": s.partner.channel_name,
        "description": "",
        "partnerSpecific": "false",
        "state": "Active",
        "groups": [{
            "name": "flatFile",
            "fields": [
                { "name": "keyValuePairs", "value": [] },
                { "name": "overwrite", "value": "false" },
            ],
            "passwordChange": false,
        }],
    });
    let response = s
        .api
        .call(&s.env, Method::Post, "/b2b/api/v1/channels", Some(&body))
        .await?;
    let value = expect_json(&response, "creating the channel", &[])?;
    let id = json_text(&value, "/id")
        .ok_or_else(|| Error::Http("channel was created but the response has no id".into()))?;
    let uri = json_text(&value, "/endpoint/uri").ok_or_else(|| {
        Error::Http("channel was created but the response has no endpoint.uri".into())
    })?;
    Ok(json!({ "channel_id": path_segment(&id)?, "endpoint_uri": uri }))
}

fn required<'a>(ctx: &'a StepContext, step: &str, field: &str) -> Result<&'a str> {
    ctx.output_str(step, field)
        .ok_or_else(|| Error::Http(format!("missing '{field}' from step '{step}'")))
}

/// `PUT /b2b/api/v1/partners/{id}/b2b-gateways` (script: `associate_channel_to_partner`).
async fn associate_channel(s: Arc<Shared>, ctx: StepContext) -> Result<Value> {
    let partner_id = path_segment(required(&ctx, STEP_CREATE_PARTNER, "partner_id")?)?;
    let channel_id = required(&ctx, STEP_CREATE_CHANNEL, "channel_id")?;
    let body = json!({ "b2bGateways": [{ "id": channel_id }] });
    let response = s
        .api
        .call(
            &s.env,
            Method::Put,
            &format!("/b2b/api/v1/partners/{partner_id}/b2b-gateways"),
            Some(&body),
        )
        .await?;
    expect_json(&response, "associating the channel", &[])?;
    Ok(json!({}))
}

/// `POST /b2b/api/v1/partner-users` (script: `create_partner_user`).
async fn create_user(s: Arc<Shared>) -> Result<Value> {
    let secret_id = s.partner.password_secret.as_ref().ok_or_else(|| {
        Error::Conflict("this partner has no password; set one before provisioning".into())
    })?;
    let password = s.secrets.get(secret_id)?.ok_or_else(|| {
        Error::Conflict("the stored partner password is missing; set it again".into())
    })?;
    let body = json!({
        "user_name": s.partner.username,
        "password": password.expose_secret(),
        "full_name": s.partner.full_name,
    });
    let response = s
        .api
        .call(
            &s.env,
            Method::Post,
            "/b2b/api/v1/partner-users",
            Some(&body),
        )
        .await?;
    let value = expect_json(
        &response,
        "creating the partner user",
        &[password.expose_secret()],
    )?;
    let id = json_text(&value, "/id")
        .ok_or_else(|| Error::Http("user was created but the response has no id".into()))?;
    Ok(json!({ "user_id": path_segment(&id)? }))
}

/// `PUT /b2b/api/v1/partners/{id}/users` (script: `associate_user_to_partner`).
async fn associate_user(s: Arc<Shared>, ctx: StepContext) -> Result<Value> {
    let partner_id = path_segment(required(&ctx, STEP_CREATE_PARTNER, "partner_id")?)?;
    let user_id = required(&ctx, STEP_CREATE_USER, "user_id")?;
    let body = json!({ "users": [{ "id": user_id, "username": s.partner.username }] });
    let response = s
        .api
        .call(
            &s.env,
            Method::Put,
            &format!("/b2b/api/v1/partners/{partner_id}/users"),
            Some(&body),
        )
        .await?;
    expect_json(&response, "associating the user", &[])?;
    Ok(json!({}))
}
