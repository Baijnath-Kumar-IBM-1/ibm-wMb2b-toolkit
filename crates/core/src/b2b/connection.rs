//! Connection test: can the saved API key obtain an MCSP token?

use std::sync::Arc;

use serde::Serialize;
use serde_json::json;

use crate::b2b::api::B2bApi;
use crate::clock::Clock;
use crate::environments::EnvironmentRepository;
use crate::error::{Error, Result};
use crate::events::{Event, EventLevel, EventPublisher};
use crate::ids::EnvironmentId;

/// Result shown to the user. `message` never contains secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectionReport {
    pub ok: bool,
    pub message: String,
}

pub struct ConnectionService {
    api: Arc<B2bApi>,
    envs: Arc<dyn EnvironmentRepository>,
    clock: Arc<dyn Clock>,
    events: Arc<dyn EventPublisher>,
}

impl ConnectionService {
    pub fn new(
        api: Arc<B2bApi>,
        envs: Arc<dyn EnvironmentRepository>,
        clock: Arc<dyn Clock>,
        events: Arc<dyn EventPublisher>,
    ) -> Self {
        Self {
            api,
            envs,
            clock,
            events,
        }
    }

    /// Always requests a fresh token, so a changed or revoked key is noticed.
    ///
    /// This checks the IAM host, instance id, API key and TLS trust. It does
    /// not call the B2B host (the script has no read-only endpoint to use for
    /// that); a wrong B2B host shows up on the first provisioning step.
    ///
    /// Failures the user can act on are returned as `ok: false`; storage and
    /// secret-store failures are returned as `Err`.
    pub async fn test(&self, environment_id: &EnvironmentId) -> Result<ConnectionReport> {
        let env = self
            .envs
            .get(environment_id)?
            .ok_or(Error::NotFound("environment"))?;
        self.api.forget_token(environment_id);
        let report = match self.api.token(&env, true).await {
            Ok(_) => ConnectionReport {
                ok: true,
                message: "Connected: the API key was accepted and a token was issued.".into(),
            },
            Err(Error::Http(detail)) => ConnectionReport {
                ok: false,
                message: detail,
            },
            Err(Error::Conflict(message)) => ConnectionReport { ok: false, message },
            Err(other) => return Err(other),
        };
        self.events.publish(
            Event::new(
                self.clock.now(),
                if report.ok {
                    EventLevel::Info
                } else {
                    EventLevel::Warn
                },
                "environment.connection_tested",
                format!("Connection test for '{}'", env.name),
            )
            .with_data(json!({ "environment_id": env.id, "ok": report.ok })),
        );
        Ok(report)
    }
}
