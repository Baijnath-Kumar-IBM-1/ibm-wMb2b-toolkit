//! Shared application state: the wired services and the run registry.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;
use tokio::sync::watch;
use trace_core::b2b::{B2bApi, ConnectionService, DocumentService, ProvisioningService};
use trace_core::clock::{Clock, SystemClock};
use trace_core::environments::{EnvironmentRepository, EnvironmentService};
use trace_core::events::{EventBus, EventPublisher};
use trace_core::http::HttpClient;
use trace_core::packs::Catalog;
use trace_core::partners::{PartnerRepository, PartnerService};
use trace_core::preferences::PreferenceRepository;
use trace_core::secrets::SecretStore;
use trace_core::steps::{CancelToken, StepStore};
use trace_core::submissions::SubmissionRepository;
use trace_core::submit::{SubmissionDeps, SubmissionService};
use trace_storage::{Database, SecretBackend};

use crate::error::ApiError;

/// Everything the server needs from the outside.
pub struct Parts {
    pub db: Arc<Database>,
    pub secrets: Arc<dyn SecretStore>,
    pub secret_backend: SecretBackend,
    pub http: Arc<dyn HttpClient>,
    pub catalog: Arc<Catalog>,
    pub clock: Arc<dyn Clock>,
    pub events: EventBus,
}

impl Parts {
    pub fn new(
        db: Arc<Database>,
        secrets: Arc<dyn SecretStore>,
        secret_backend: SecretBackend,
        http: Arc<dyn HttpClient>,
        catalog: Catalog,
    ) -> Self {
        Self {
            db,
            secrets,
            secret_backend,
            http,
            catalog: Arc::new(catalog),
            clock: Arc::new(SystemClock),
            events: EventBus::default(),
        }
    }
}

pub struct AppState {
    pub db: Arc<Database>,
    pub secret_backend: SecretBackend,
    pub catalog: Arc<Catalog>,
    pub events: EventBus,
    pub clock: Arc<dyn Clock>,
    pub envs: Arc<EnvironmentService>,
    pub partners: Arc<PartnerService>,
    pub api: Arc<B2bApi>,
    pub connection: Arc<ConnectionService>,
    pub provisioning: Arc<ProvisioningService>,
    pub documents: Arc<DocumentService>,
    pub submit: Arc<SubmissionService>,
    pub runs: Runs,
    /// Flips to `true` on shutdown so open event streams end.
    pub shutdown: watch::Sender<bool>,
}

impl AppState {
    pub fn build(parts: Parts) -> Arc<Self> {
        let Parts {
            db,
            secrets,
            secret_backend,
            http,
            catalog,
            clock,
            events,
        } = parts;
        let clock_for_state = clock.clone();
        let publisher: Arc<dyn EventPublisher> = events.clone().into_publisher();
        let env_repo: Arc<dyn EnvironmentRepository> = db.clone();
        let partner_repo: Arc<dyn PartnerRepository> = db.clone();
        let step_store: Arc<dyn StepStore> = db.clone();
        let prefs: Arc<dyn PreferenceRepository> = db.clone();
        let submissions: Arc<dyn SubmissionRepository> = db.clone();

        let envs = Arc::new(EnvironmentService::new(
            env_repo.clone(),
            partner_repo.clone(),
            secrets.clone(),
            clock.clone(),
            publisher.clone(),
        ));
        let partners = Arc::new(PartnerService::new(
            partner_repo.clone(),
            env_repo.clone(),
            step_store.clone(),
            secrets.clone(),
            clock.clone(),
            publisher.clone(),
        ));
        let api = B2bApi::new(http.clone(), secrets.clone(), prefs.clone(), clock.clone());
        let connection = Arc::new(ConnectionService::new(
            api.clone(),
            env_repo.clone(),
            clock.clone(),
            publisher.clone(),
        ));
        let provisioning = Arc::new(ProvisioningService::new(
            api.clone(),
            partner_repo.clone(),
            env_repo.clone(),
            step_store,
            secrets.clone(),
            clock.clone(),
            publisher.clone(),
        ));
        let documents = Arc::new(DocumentService::new(
            api.clone(),
            catalog.clone(),
            env_repo.clone(),
            db.clone(),
            clock.clone(),
            publisher.clone(),
        ));
        let submit = Arc::new(SubmissionService::new(SubmissionDeps {
            http,
            catalog: catalog.clone(),
            envs: env_repo,
            partners: partner_repo,
            counters: db.clone(),
            submissions,
            prefs,
            secrets,
            provisioning: provisioning.clone(),
            clock,
            events: publisher,
        }));
        let (shutdown, _) = watch::channel(false);
        Arc::new(Self {
            db,
            secret_backend,
            catalog,
            events,
            clock: clock_for_state,
            envs,
            partners,
            api,
            connection,
            provisioning,
            documents,
            submit,
            runs: Runs::default(),
            shutdown,
        })
    }
}

// ---- runs ---------------------------------------------------------------

/// Long operations (provisioning, document install, submission) run in the
/// background; the client follows them through events and `GET /runs/{id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunView {
    pub id: String,
    pub kind: &'static str,
    pub status: RunStatus,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    /// The service's report when the run finished (secret-free).
    pub report: Option<Value>,
    /// User-safe failure when the run could not start work or errored.
    pub error: Option<ApiError>,
}

struct RunEntry {
    view: RunView,
    cancel: CancelToken,
    /// Scope lock key, released when the run ends.
    key: String,
}

#[derive(Default)]
pub struct Runs {
    inner: Mutex<RunsInner>,
}

#[derive(Default)]
struct RunsInner {
    entries: HashMap<String, RunEntry>,
    order: Vec<String>,
}

/// How many finished runs are kept for lookups.
const KEEP_FINISHED: usize = 50;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub enum StartError {
    /// Another run holds the scope key.
    Busy,
    NoRandom,
}

impl Runs {
    /// Registers a run unless one with the same scope `key` is active.
    pub fn start(
        &self,
        kind: &'static str,
        key: &str,
        now: OffsetDateTime,
    ) -> Result<(String, CancelToken), StartError> {
        let mut inner = lock(&self.inner);
        if inner
            .entries
            .values()
            .any(|e| e.key == key && e.view.status == RunStatus::Running)
        {
            return Err(StartError::Busy);
        }
        let mut raw = [0_u8; 16];
        getrandom::fill(&mut raw).map_err(|_| StartError::NoRandom)?;
        let id: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let cancel = CancelToken::new();
        inner.entries.insert(
            id.clone(),
            RunEntry {
                view: RunView {
                    id: id.clone(),
                    kind,
                    status: RunStatus::Running,
                    started_at: now,
                    report: None,
                    error: None,
                },
                cancel: cancel.clone(),
                key: key.to_string(),
            },
        );
        inner.order.push(id.clone());
        // Evict the oldest finished runs beyond the cap.
        let finished: Vec<String> = inner
            .order
            .iter()
            .filter(|i| {
                inner
                    .entries
                    .get(*i)
                    .is_some_and(|e| e.view.status != RunStatus::Running)
            })
            .cloned()
            .collect();
        if finished.len() > KEEP_FINISHED {
            for old in &finished[..finished.len() - KEEP_FINISHED] {
                inner.entries.remove(old);
                inner.order.retain(|i| i != old);
            }
        }
        Ok((id, cancel))
    }

    pub fn finish(&self, id: &str, outcome: Result<Value, ApiError>) {
        if let Some(entry) = lock(&self.inner).entries.get_mut(id) {
            match outcome {
                Ok(report) => {
                    entry.view.status = RunStatus::Finished;
                    entry.view.report = Some(report);
                }
                Err(error) => {
                    entry.view.status = RunStatus::Failed;
                    entry.view.error = Some(error);
                }
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<RunView> {
        lock(&self.inner).entries.get(id).map(|e| e.view.clone())
    }

    /// Returns false when the run is unknown or already over.
    pub fn cancel(&self, id: &str) -> bool {
        match lock(&self.inner).entries.get(id) {
            Some(e) if e.view.status == RunStatus::Running => {
                e.cancel.cancel();
                true
            }
            _ => false,
        }
    }
}
