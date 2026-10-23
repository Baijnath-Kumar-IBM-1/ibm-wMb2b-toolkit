//! Partners feature: trading partners that live in an environment.
//!
//! Each partner owns one channel and one partner user on the tenant. The
//! password is a secret (see [`crate::secrets`]). Provisioning progress is
//! kept as step records in [`crate::steps`] under [`partner_scope`].

use std::sync::Arc;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::environments::EnvironmentRepository;
use crate::error::{Error, Result};
use crate::events::{Event, EventLevel, EventPublisher};
use crate::ids::{EnvironmentId, PartnerId};
use crate::secrets::{SecretId, SecretStore, validate_secret_text};
use crate::steps::{StepStatus, StepStore};
use crate::validation::{self, Collector};

/// External identity type. Only DUNS is verified against the B2B API (the v1
/// script sends `externalIdType: DUNS` and ISA qualifier `01`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IdType {
    #[default]
    Duns,
}

impl IdType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Duns => "DUNS",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "DUNS" => Ok(Self::Duns),
            _ => Err(Error::validation("id_type", "only DUNS is supported")),
        }
    }
}

/// Step-store scope under which a partner's provisioning steps are saved.
pub fn partner_scope(id: &PartnerId) -> String {
    format!("partner:{id}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partner {
    pub id: PartnerId,
    pub environment_id: EnvironmentId,
    pub name: String,
    pub org_unit: String,
    pub id_type: IdType,
    pub id_value: String,
    pub username: String,
    pub full_name: String,
    pub channel_name: String,
    pub password_secret: Option<SecretId>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl Partner {
    pub fn view(&self) -> PartnerView {
        PartnerView {
            id: self.id.clone(),
            environment_id: self.environment_id.clone(),
            name: self.name.clone(),
            org_unit: self.org_unit.clone(),
            id_type: self.id_type,
            id_value: self.id_value.clone(),
            username: self.username.clone(),
            full_name: self.full_name.clone(),
            channel_name: self.channel_name.clone(),
            has_password: self.password_secret.is_some(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// What the API returns. The password appears only as `has_password`.
#[derive(Debug, Clone, Serialize)]
pub struct PartnerView {
    pub id: PartnerId,
    pub environment_id: EnvironmentId,
    pub name: String,
    pub org_unit: String,
    pub id_type: IdType,
    pub id_value: String,
    pub username: String,
    pub full_name: String,
    pub channel_name: String,
    pub has_password: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PartnerInput {
    pub name: String,
    pub org_unit: String,
    #[serde(default)]
    pub id_type: IdType,
    pub id_value: String,
    pub username: String,
    pub full_name: String,
    pub channel_name: String,
}

fn name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '&')
}

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
}

/// Password length bounds enforced before anything is stored.
pub const PASSWORD_MIN: usize = 8;
pub const PASSWORD_MAX: usize = 128;

impl PartnerInput {
    /// Validates every field and returns the normalised input.
    ///
    /// - `id_value` (DUNS): 9 to 15 digits (DUNS is 9; DUNS+4 is 13; the ISA
    ///   identifier is at most 15 characters).
    /// - `username`: 3 to 64 of letters, digits, `.`, `_`, `-`, `@`.
    /// - `channel_name`: 1 to 64 of letters, digits, `.`, `_`, `-`.
    pub fn validated(&self) -> Result<Self> {
        let mut c = Collector::new();
        let name = c.field(
            "name",
            validation::restricted_text(
                &self.name,
                1,
                64,
                name_char,
                "letters, digits, spaces and - _ . &",
            ),
        );
        let org_unit = c.field(
            "org_unit",
            validation::restricted_text(
                &self.org_unit,
                1,
                64,
                name_char,
                "letters, digits, spaces and - _ . &",
            ),
        );
        let id_value = c.field("id_value", validation::digits(&self.id_value, 9, 15));
        let username = c.field(
            "username",
            validation::restricted_text(
                &self.username,
                3,
                64,
                |ch| token_char(ch) || ch == '@',
                "letters, digits and . _ - @",
            ),
        );
        let full_name = c.field("full_name", validation::text(&self.full_name, 1, 100));
        let channel_name = c.field(
            "channel_name",
            validation::restricted_text(
                &self.channel_name,
                1,
                64,
                token_char,
                "letters, digits and . _ -",
            ),
        );
        c.finish()?;
        match (name, org_unit, id_value, username, full_name, channel_name) {
            (
                Some(name),
                Some(org_unit),
                Some(id_value),
                Some(username),
                Some(full_name),
                Some(channel_name),
            ) => Ok(Self {
                name,
                org_unit,
                id_type: self.id_type,
                id_value,
                username,
                full_name,
                channel_name,
            }),
            _ => Err(Error::validation("partner", "invalid")),
        }
    }
}

/// Storage port for partners.
pub trait PartnerRepository: Send + Sync {
    fn list(&self, environment_id: &EnvironmentId) -> Result<Vec<Partner>>;
    fn get(&self, id: &PartnerId) -> Result<Option<Partner>>;
    /// Inserts or updates by id. Within one environment the name, ID value,
    /// username and channel name must each be unique (names compare
    /// case-insensitively); a duplicate is `Error::Conflict`.
    fn save(&self, partner: &Partner) -> Result<()>;
    /// Deletes the partner and its step records.
    fn delete(&self, id: &PartnerId) -> Result<()>;
}

pub struct PartnerService {
    repo: Arc<dyn PartnerRepository>,
    environments: Arc<dyn EnvironmentRepository>,
    steps: Arc<dyn StepStore>,
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
    events: Arc<dyn EventPublisher>,
}

impl PartnerService {
    pub fn new(
        repo: Arc<dyn PartnerRepository>,
        environments: Arc<dyn EnvironmentRepository>,
        steps: Arc<dyn StepStore>,
        secrets: Arc<dyn SecretStore>,
        clock: Arc<dyn Clock>,
        events: Arc<dyn EventPublisher>,
    ) -> Self {
        Self {
            repo,
            environments,
            steps,
            secrets,
            clock,
            events,
        }
    }

    pub fn list(&self, environment_id: &EnvironmentId) -> Result<Vec<Partner>> {
        self.require_environment(environment_id)?;
        self.repo.list(environment_id)
    }

    pub fn get(&self, id: &PartnerId) -> Result<Partner> {
        self.repo.get(id)?.ok_or(Error::NotFound("partner"))
    }

    pub fn create(
        &self,
        environment_id: &EnvironmentId,
        input: &PartnerInput,
        password: &SecretString,
    ) -> Result<Partner> {
        self.require_environment(environment_id)?;
        let input = input.validated()?;
        validate_password(password)?;
        let now = self.clock.now();
        let secret_id = SecretId::generate();
        let partner = Partner {
            id: PartnerId::generate(),
            environment_id: environment_id.clone(),
            name: input.name,
            org_unit: input.org_unit,
            id_type: input.id_type,
            id_value: input.id_value,
            username: input.username,
            full_name: input.full_name,
            channel_name: input.channel_name,
            password_secret: Some(secret_id.clone()),
            created_at: now,
            updated_at: now,
        };
        self.secrets.put(&secret_id, password)?;
        if let Err(e) = self.repo.save(&partner) {
            let _ = self.secrets.delete(&secret_id);
            return Err(e);
        }
        self.emit("partner.created", &partner);
        Ok(partner)
    }

    /// Updates a partner that has not been provisioned yet. After any
    /// provisioning step has succeeded the values already exist on the tenant,
    /// so editing is rejected with `Error::Conflict`.
    pub fn update(
        &self,
        id: &PartnerId,
        input: &PartnerInput,
        password: Option<&SecretString>,
    ) -> Result<Partner> {
        let input = input.validated()?;
        if let Some(p) = password {
            validate_password(p)?;
        }
        let mut partner = self.get(id)?;
        let provisioned = self
            .steps
            .list(&partner_scope(id))?
            .iter()
            .any(|r| r.status == StepStatus::Succeeded);
        if provisioned {
            return Err(Error::Conflict(
                "partner is already provisioned on the tenant; delete and recreate it to change it"
                    .into(),
            ));
        }
        partner.name = input.name;
        partner.org_unit = input.org_unit;
        partner.id_type = input.id_type;
        partner.id_value = input.id_value;
        partner.username = input.username;
        partner.full_name = input.full_name;
        partner.channel_name = input.channel_name;
        partner.updated_at = self.clock.now();

        let mut previous = None;
        let mut fresh = None;
        if let Some(p) = password {
            let sid = SecretId::generate();
            self.secrets.put(&sid, p)?;
            previous = partner.password_secret.replace(sid.clone());
            fresh = Some(sid);
        }
        if let Err(e) = self.repo.save(&partner) {
            if let Some(sid) = fresh {
                let _ = self.secrets.delete(&sid);
            }
            return Err(e);
        }
        if let Some(old) = previous {
            let _ = self.secrets.delete(&old);
        }
        self.emit("partner.updated", &partner);
        Ok(partner)
    }

    /// Deletes the local record, its step records and its password. This does
    /// not remove anything from the tenant.
    pub fn delete(&self, id: &PartnerId) -> Result<()> {
        let partner = self.get(id)?;
        self.repo.delete(id)?;
        if let Some(sid) = &partner.password_secret {
            let _ = self.secrets.delete(sid);
        }
        self.emit("partner.deleted", &partner);
        Ok(())
    }

    fn require_environment(&self, id: &EnvironmentId) -> Result<()> {
        self.environments
            .get(id)?
            .map(|_| ())
            .ok_or(Error::NotFound("environment"))
    }

    fn emit(&self, kind: &str, partner: &Partner) {
        self.events.publish(
            Event::new(
                self.clock.now(),
                EventLevel::Info,
                kind,
                format!("Partner '{}'", partner.name),
            )
            .with_data(json!({
                "environment_id": partner.environment_id,
                "partner_id": partner.id,
            })),
        );
    }
}

fn validate_password(password: &SecretString) -> Result<()> {
    use secrecy::ExposeSecret;
    validate_secret_text(
        "password",
        password.expose_secret(),
        PASSWORD_MIN,
        PASSWORD_MAX,
    )
}
