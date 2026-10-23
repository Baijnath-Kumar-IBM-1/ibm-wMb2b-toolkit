//! Service-level tests over the in-memory fakes.

mod environments;
mod partners;
mod steps;

use std::sync::Arc;

use secrecy::SecretString;

use crate::environments::{EnvironmentInput, EnvironmentService, IAM_HOST_TEST};
use crate::partners::{PartnerInput, PartnerService};
use crate::steps::StepStore;
use crate::testing::{FixedClock, MemoryDb, MemorySecretStore, RecordingPublisher};

pub struct Fixture {
    pub db: Arc<MemoryDb>,
    pub secrets: Arc<MemorySecretStore>,
    pub events: Arc<RecordingPublisher>,
    pub clock: Arc<FixedClock>,
    pub envs: EnvironmentService,
    pub partners: PartnerService,
}

impl Fixture {
    pub fn new() -> Self {
        let db = MemoryDb::new();
        let secrets = Arc::new(MemorySecretStore::default());
        let events = Arc::new(RecordingPublisher::default());
        let clock = Arc::new(FixedClock::default());
        let envs = EnvironmentService::new(
            db.clone(),
            db.clone(),
            secrets.clone(),
            clock.clone(),
            events.clone(),
        );
        let steps: Arc<dyn StepStore> = db.clone();
        let partners = PartnerService::new(
            db.clone(),
            db.clone(),
            steps,
            secrets.clone(),
            clock.clone(),
            events.clone(),
        );
        Self {
            db,
            secrets,
            events,
            clock,
            envs,
            partners,
        }
    }
}

pub fn env_input(name: &str) -> EnvironmentInput {
    EnvironmentInput {
        name: name.into(),
        b2b_host: "dev2205134-m-yqb-c1.b2b.ipaas.ibm.com".into(),
        instance_id: "123e4567-e89b-12d3-a456-426614174000".into(),
        iam_host: IAM_HOST_TEST.into(),
        ca_bundle_path: None,
    }
}

pub fn partner_input(name: &str, duns: &str, user: &str, channel: &str) -> PartnerInput {
    PartnerInput {
        name: name.into(),
        org_unit: "Retail Procurement".into(),
        id_type: Default::default(),
        id_value: duns.into(),
        username: user.into(),
        full_name: "Demo User".into(),
        channel_name: channel.into(),
    }
}

pub fn secret(v: &str) -> SecretString {
    SecretString::from(v.to_string())
}
