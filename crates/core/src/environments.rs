//! Environments feature: a B2B tenant the toolkit talks to.
//!
//! Model, storage port and service live together. The API key is stored in
//! the [`SecretStore`]; the database holds only an opaque [`SecretId`].

use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::error::{Error, Result};
use crate::events::{Event, EventLevel, EventPublisher};
use crate::ids::EnvironmentId;
use crate::partners::PartnerRepository;
use crate::secrets::{SecretId, SecretStore};
use crate::validation::{self, Collector};

/// MCSP IAM host presets offered by the UI. Any valid host is also accepted.
pub const IAM_HOST_TEST: &str = "account-iam.platform.test.saas.ibm.com";
pub const IAM_HOST_PROD: &str = "account-iam.platform.saas.ibm.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub id: EnvironmentId,
    pub name: String,
    /// Host name without scheme, for example `dev1-m-abc-c1.b2b.ipaas.ibm.com`.
    pub b2b_host: String,
    /// MCSP instance id (UUID).
    pub instance_id: String,
    pub iam_host: String,
    /// Optional PEM bundle with extra trusted CAs for private certificates.
    pub ca_bundle_path: Option<String>,
    pub is_default: bool,
    pub api_key_secret: Option<SecretId>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl Environment {
    pub fn view(&self) -> EnvironmentView {
        EnvironmentView {
            id: self.id.clone(),
            name: self.name.clone(),
            b2b_host: self.b2b_host.clone(),
            instance_id: self.instance_id.clone(),
            iam_host: self.iam_host.clone(),
            ca_bundle_path: self.ca_bundle_path.clone(),
            is_default: self.is_default,
            has_api_key: self.api_key_secret.is_some(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// What the API returns. Secrets appear only as the `has_api_key` flag.
#[derive(Debug, Clone, Serialize)]
pub struct EnvironmentView {
    pub id: EnvironmentId,
    pub name: String,
    pub b2b_host: String,
    pub instance_id: String,
    pub iam_host: String,
    pub ca_bundle_path: Option<String>,
    pub is_default: bool,
    pub has_api_key: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// User-editable, non-secret fields.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct EnvironmentInput {
    pub name: String,
    pub b2b_host: String,
    pub instance_id: String,
    #[serde(default = "default_iam_host")]
    pub iam_host: String,
    #[serde(default)]
    pub ca_bundle_path: Option<String>,
}

fn default_iam_host() -> String {
    IAM_HOST_TEST.to_string()
}

impl EnvironmentInput {
    /// Validates every field and returns the normalised input (trimmed,
    /// lower-cased hosts, canonical UUID).
    pub fn validated(&self) -> Result<Self> {
        let mut c = Collector::new();
        let name = c.field("name", validation::text(&self.name, 1, 64));
        let b2b_host = c.field("b2b_host", validation::host(&self.b2b_host, true));
        let instance_id = c.field("instance_id", validation::uuid(&self.instance_id));
        let iam_host = c.field("iam_host", validation::host(&self.iam_host, false));
        let ca_bundle_path = match self.ca_bundle_path.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(p) => c.field("ca_bundle_path", validation::absolute_path(p)),
        };
        c.finish()?;
        match (name, b2b_host, instance_id, iam_host) {
            (Some(name), Some(b2b_host), Some(instance_id), Some(iam_host)) => Ok(Self {
                name,
                b2b_host,
                instance_id,
                iam_host,
                ca_bundle_path,
            }),
            _ => Err(Error::validation("environment", "invalid")),
        }
    }
}

/// The API key is passed separately so it is never part of a `Debug`/serde
/// struct. It is sent as an HTTP header, so only printable ASCII is allowed.
pub fn validate_api_key(key: &SecretString) -> Result<()> {
    let k = key.expose_secret();
    let ok = (8..=4096).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_graphic());
    if ok {
        Ok(())
    } else {
        Err(Error::validation(
            "api_key",
            "must be 8 to 4096 printable characters without spaces",
        ))
    }
}

/// Storage port for environments.
pub trait EnvironmentRepository: Send + Sync {
    fn list(&self) -> Result<Vec<Environment>>;
    fn get(&self, id: &EnvironmentId) -> Result<Option<Environment>>;
    /// Inserts or updates by id. A name that is already used by another
    /// environment (case-insensitive) is `Error::Conflict`.
    fn save(&self, env: &Environment) -> Result<()>;
    /// Deletes the environment and, by cascade, everything that belongs to it.
    fn delete(&self, id: &EnvironmentId) -> Result<()>;
    /// Makes `id` the only default environment, atomically.
    fn set_default(&self, id: &EnvironmentId) -> Result<()>;
}

pub struct EnvironmentService {
    repo: Arc<dyn EnvironmentRepository>,
    partners: Arc<dyn PartnerRepository>,
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
    events: Arc<dyn EventPublisher>,
}

impl EnvironmentService {
    pub fn new(
        repo: Arc<dyn EnvironmentRepository>,
        partners: Arc<dyn PartnerRepository>,
        secrets: Arc<dyn SecretStore>,
        clock: Arc<dyn Clock>,
        events: Arc<dyn EventPublisher>,
    ) -> Self {
        Self {
            repo,
            partners,
            secrets,
            clock,
            events,
        }
    }

    pub fn list(&self) -> Result<Vec<Environment>> {
        self.repo.list()
    }

    pub fn get(&self, id: &EnvironmentId) -> Result<Environment> {
        self.repo.get(id)?.ok_or(Error::NotFound("environment"))
    }

    /// Creates an environment. The first one becomes the default.
    pub fn create(&self, input: &EnvironmentInput, api_key: &SecretString) -> Result<Environment> {
        let input = input.validated()?;
        validate_api_key(api_key)?;
        let now = self.clock.now();
        let secret_id = SecretId::generate();
        let is_default = self.repo.list()?.is_empty();
        let env = Environment {
            id: EnvironmentId::generate(),
            name: input.name,
            b2b_host: input.b2b_host,
            instance_id: input.instance_id,
            iam_host: input.iam_host,
            ca_bundle_path: input.ca_bundle_path,
            is_default,
            api_key_secret: Some(secret_id.clone()),
            created_at: now,
            updated_at: now,
        };
        self.secrets.put(&secret_id, api_key)?;
        if let Err(e) = self.repo.save(&env) {
            self.discard_secret(&secret_id);
            return Err(e);
        }
        self.emit("environment.created", &env);
        Ok(env)
    }

    /// Updates the non-secret fields; replaces the API key only when given.
    pub fn update(
        &self,
        id: &EnvironmentId,
        input: &EnvironmentInput,
        api_key: Option<&SecretString>,
    ) -> Result<Environment> {
        let input = input.validated()?;
        if let Some(k) = api_key {
            validate_api_key(k)?;
        }
        let mut env = self.get(id)?;
        env.name = input.name;
        env.b2b_host = input.b2b_host;
        env.instance_id = input.instance_id;
        env.iam_host = input.iam_host;
        env.ca_bundle_path = input.ca_bundle_path;
        env.updated_at = self.clock.now();

        let mut previous_secret = None;
        let mut new_secret = None;
        if let Some(k) = api_key {
            // New id per rotation: if saving fails, the old key is untouched.
            let sid = SecretId::generate();
            self.secrets.put(&sid, k)?;
            previous_secret = env.api_key_secret.replace(sid.clone());
            new_secret = Some(sid);
        }
        if let Err(e) = self.repo.save(&env) {
            if let Some(sid) = new_secret {
                self.discard_secret(&sid);
            }
            return Err(e);
        }
        if let Some(old) = previous_secret {
            self.discard_secret(&old);
        }
        self.emit("environment.updated", &env);
        Ok(env)
    }

    pub fn set_default(&self, id: &EnvironmentId) -> Result<()> {
        self.get(id)?;
        self.repo.set_default(id)
    }

    /// Deletes the environment, its partners and all their secrets.
    /// Secrets are removed after the database rows; a failure there is
    /// reported as a warning event and does not fail the delete.
    pub fn delete(&self, id: &EnvironmentId) -> Result<()> {
        let env = self.get(id)?;
        let mut secret_ids: Vec<SecretId> = env.api_key_secret.iter().cloned().collect();
        for p in self.partners.list(id)? {
            secret_ids.extend(p.password_secret);
        }
        self.repo.delete(id)?;
        for sid in &secret_ids {
            self.discard_secret(sid);
        }
        if env.is_default
            && let Some(next) = self.repo.list()?.into_iter().next()
        {
            self.repo.set_default(&next.id)?;
        }
        self.emit("environment.deleted", &env);
        Ok(())
    }

    fn discard_secret(&self, id: &SecretId) {
        if self.secrets.delete(id).is_err() {
            self.events.publish(
                Event::new(
                    self.clock.now(),
                    EventLevel::Warn,
                    "secret.cleanup_failed",
                    "A stored secret could not be removed",
                )
                .with_data(json!({ "secret_id": id.as_str() })),
            );
        }
    }

    fn emit(&self, kind: &str, env: &Environment) {
        self.events.publish(
            Event::new(
                self.clock.now(),
                EventLevel::Info,
                kind,
                format!("Environment '{}'", env.name),
            )
            .with_data(json!({ "environment_id": env.id })),
        );
    }
}
