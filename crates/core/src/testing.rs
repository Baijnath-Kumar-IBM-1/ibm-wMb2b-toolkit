//! In-memory fakes for the ports. Available to this crate's tests and, with
//! the `testing` feature, to other crates' tests.
//!
//! [`MemoryDb`] implements every repository trait over one shared state so
//! cascades (deleting an environment removes its partners) behave like the
//! SQLite implementation.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use secrecy::SecretString;
use time::{Duration, OffsetDateTime};

use crate::clock::Clock;
use crate::counters::{self, ControlNumbers, CounterRepository, Counters, MAX_ALLOCATION};
use crate::environments::{Environment, EnvironmentRepository};
use crate::error::{Error, Result};
use crate::events::{Event, EventPublisher};
use crate::http::{BoxFuture, HttpClient, HttpRequest, HttpResponse};
use crate::ids::{EnvironmentId, PartnerId, SubmissionId};
use crate::installed::{InstalledDocument, InstalledDocumentRepository};
use crate::partners::{Partner, PartnerRepository, partner_scope};
use crate::preferences::{PreferenceRepository, Preferences};
use crate::secrets::{SecretId, SecretStore};
use crate::steps::{StepRecord, StepStore};
use crate::submissions::{Submission, SubmissionRepository};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Clock that only moves when told to.
#[derive(Debug)]
pub struct FixedClock(Mutex<OffsetDateTime>);

impl FixedClock {
    pub fn new(at: OffsetDateTime) -> Self {
        Self(Mutex::new(at))
    }

    pub fn advance(&self, by: Duration) {
        *lock(&self.0) += by;
    }
}

impl Default for FixedClock {
    /// 2025-01-01T00:00:00Z.
    fn default() -> Self {
        Self::new(OffsetDateTime::UNIX_EPOCH + Duration::days(20_089))
    }
}

impl Clock for FixedClock {
    fn now(&self) -> OffsetDateTime {
        *lock(&self.0)
    }
}

/// Publisher that keeps everything it receives.
#[derive(Debug, Default)]
pub struct RecordingPublisher(Mutex<Vec<Event>>);

impl RecordingPublisher {
    pub fn events(&self) -> Vec<Event> {
        lock(&self.0).clone()
    }

    pub fn kinds(&self) -> Vec<String> {
        lock(&self.0).iter().map(|e| e.kind.clone()).collect()
    }
}

impl EventPublisher for RecordingPublisher {
    fn publish(&self, event: Event) {
        lock(&self.0).push(event);
    }
}

#[derive(Debug, Default)]
pub struct MemorySecretStore {
    values: Mutex<BTreeMap<String, String>>,
}

impl MemorySecretStore {
    pub fn len(&self) -> usize {
        lock(&self.values).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SecretStore for MemorySecretStore {
    fn put(&self, id: &SecretId, value: &SecretString) -> Result<()> {
        use secrecy::ExposeSecret;
        lock(&self.values).insert(id.as_str().into(), value.expose_secret().into());
        Ok(())
    }

    fn get(&self, id: &SecretId) -> Result<Option<SecretString>> {
        Ok(lock(&self.values)
            .get(id.as_str())
            .map(|v| SecretString::from(v.clone())))
    }

    fn delete(&self, id: &SecretId) -> Result<()> {
        lock(&self.values).remove(id.as_str());
        Ok(())
    }
}

#[derive(Default)]
struct State {
    environments: Vec<Environment>,
    partners: Vec<Partner>,
    steps: BTreeMap<String, Vec<StepRecord>>,
    counters: BTreeMap<EnvironmentId, Counters>,
    installed: BTreeMap<EnvironmentId, Vec<InstalledDocument>>,
    submissions: Vec<Submission>,
    preferences: Option<Preferences>,
}

/// One in-memory database implementing every repository port.
#[derive(Default)]
pub struct MemoryDb {
    state: Mutex<State>,
}

impl MemoryDb {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl EnvironmentRepository for MemoryDb {
    fn list(&self) -> Result<Vec<Environment>> {
        let mut v = lock(&self.state).environments.clone();
        v.sort_by_key(|e| e.name.to_lowercase());
        Ok(v)
    }

    fn get(&self, id: &EnvironmentId) -> Result<Option<Environment>> {
        Ok(lock(&self.state)
            .environments
            .iter()
            .find(|e| &e.id == id)
            .cloned())
    }

    fn save(&self, env: &Environment) -> Result<()> {
        let mut s = lock(&self.state);
        if s.environments
            .iter()
            .any(|e| e.id != env.id && e.name.eq_ignore_ascii_case(&env.name))
        {
            return Err(Error::Conflict(
                "an environment with this name already exists".into(),
            ));
        }
        match s.environments.iter_mut().find(|e| e.id == env.id) {
            Some(slot) => *slot = env.clone(),
            None => s.environments.push(env.clone()),
        }
        Ok(())
    }

    fn delete(&self, id: &EnvironmentId) -> Result<()> {
        let mut s = lock(&self.state);
        let gone: Vec<PartnerId> = s
            .partners
            .iter()
            .filter(|p| &p.environment_id == id)
            .map(|p| p.id.clone())
            .collect();
        for p in &gone {
            s.steps.remove(&partner_scope(p));
        }
        s.partners.retain(|p| &p.environment_id != id);
        s.environments.retain(|e| &e.id != id);
        s.counters.remove(id);
        s.installed.remove(id);
        s.submissions.retain(|x| &x.environment_id != id);
        Ok(())
    }

    fn set_default(&self, id: &EnvironmentId) -> Result<()> {
        let mut s = lock(&self.state);
        for e in &mut s.environments {
            e.is_default = &e.id == id;
        }
        Ok(())
    }
}

impl PartnerRepository for MemoryDb {
    fn list(&self, environment_id: &EnvironmentId) -> Result<Vec<Partner>> {
        let mut v: Vec<Partner> = lock(&self.state)
            .partners
            .iter()
            .filter(|p| &p.environment_id == environment_id)
            .cloned()
            .collect();
        v.sort_by_key(|p| p.name.to_lowercase());
        Ok(v)
    }

    fn get(&self, id: &PartnerId) -> Result<Option<Partner>> {
        Ok(lock(&self.state)
            .partners
            .iter()
            .find(|p| &p.id == id)
            .cloned())
    }

    fn save(&self, partner: &Partner) -> Result<()> {
        let mut s = lock(&self.state);
        let clash = s.partners.iter().any(|p| {
            p.id != partner.id
                && p.environment_id == partner.environment_id
                && (p.name.eq_ignore_ascii_case(&partner.name)
                    || p.id_value == partner.id_value
                    || p.username.eq_ignore_ascii_case(&partner.username)
                    || p.channel_name.eq_ignore_ascii_case(&partner.channel_name))
        });
        if clash {
            return Err(Error::Conflict(
                "name, ID value, username and channel name must be unique within the environment"
                    .into(),
            ));
        }
        match s.partners.iter_mut().find(|p| p.id == partner.id) {
            Some(slot) => *slot = partner.clone(),
            None => s.partners.push(partner.clone()),
        }
        Ok(())
    }

    fn delete(&self, id: &PartnerId) -> Result<()> {
        let mut s = lock(&self.state);
        s.partners.retain(|p| &p.id != id);
        s.steps.remove(&partner_scope(id));
        Ok(())
    }
}

impl StepStore for MemoryDb {
    fn list(&self, scope: &str) -> Result<Vec<StepRecord>> {
        Ok(lock(&self.state)
            .steps
            .get(scope)
            .cloned()
            .unwrap_or_default())
    }

    fn save(&self, scope: &str, record: &StepRecord) -> Result<()> {
        let mut s = lock(&self.state);
        let list = s.steps.entry(scope.to_string()).or_default();
        match list.iter_mut().find(|r| r.name == record.name) {
            Some(slot) => *slot = record.clone(),
            None => list.push(record.clone()),
        }
        Ok(())
    }

    fn clear(&self, scope: &str) -> Result<()> {
        lock(&self.state).steps.remove(scope);
        Ok(())
    }
}

impl CounterRepository for MemoryDb {
    fn peek(&self, environment_id: &EnvironmentId) -> Result<Counters> {
        Ok(lock(&self.state)
            .counters
            .get(environment_id)
            .copied()
            .unwrap_or_default())
    }

    fn allocate(&self, environment_id: &EnvironmentId, count: u32) -> Result<Vec<ControlNumbers>> {
        if !(1..=MAX_ALLOCATION).contains(&count) {
            return Err(Error::validation("count", "must be between 1 and 1000"));
        }
        let mut s = lock(&self.state);
        let c = s.counters.entry(environment_id.clone()).or_default();
        let out = (0..count)
            .map(|i| ControlNumbers {
                isa: counters::advance(c.isa_next, i),
                gs: counters::advance(c.gs_next, i),
            })
            .collect();
        c.isa_next = counters::advance(c.isa_next, count);
        c.gs_next = counters::advance(c.gs_next, count);
        Ok(out)
    }

    fn increment_run_count(&self, environment_id: &EnvironmentId) -> Result<u64> {
        let mut s = lock(&self.state);
        let c = s.counters.entry(environment_id.clone()).or_default();
        c.run_count += 1;
        Ok(c.run_count)
    }
}

impl InstalledDocumentRepository for MemoryDb {
    fn list(&self, environment_id: &EnvironmentId) -> Result<Vec<InstalledDocument>> {
        let mut v = lock(&self.state)
            .installed
            .get(environment_id)
            .cloned()
            .unwrap_or_default();
        v.sort_by(|a, b| a.transaction_code.cmp(&b.transaction_code));
        Ok(v)
    }

    fn save(&self, environment_id: &EnvironmentId, doc: &InstalledDocument) -> Result<()> {
        let mut s = lock(&self.state);
        let list = s.installed.entry(environment_id.clone()).or_default();
        match list
            .iter_mut()
            .find(|d| d.transaction_code == doc.transaction_code)
        {
            Some(slot) => *slot = doc.clone(),
            None => list.push(doc.clone()),
        }
        Ok(())
    }

    fn remove(&self, environment_id: &EnvironmentId, transaction_code: &str) -> Result<()> {
        if let Some(list) = lock(&self.state).installed.get_mut(environment_id) {
            list.retain(|d| d.transaction_code != transaction_code);
        }
        Ok(())
    }
}

impl PreferenceRepository for MemoryDb {
    fn get(&self) -> Result<Preferences> {
        Ok(lock(&self.state).preferences.unwrap_or_default())
    }

    fn set(&self, preferences: &Preferences) -> Result<()> {
        lock(&self.state).preferences = Some(*preferences);
        Ok(())
    }
}

impl SubmissionRepository for MemoryDb {
    fn save(&self, submission: &Submission) -> Result<()> {
        lock(&self.state).submissions.push(submission.clone());
        Ok(())
    }

    fn list_recent(&self, environment_id: &EnvironmentId, limit: u32) -> Result<Vec<Submission>> {
        let mut v: Vec<Submission> = lock(&self.state)
            .submissions
            .iter()
            .filter(|s| &s.environment_id == environment_id)
            .cloned()
            .collect();
        v.sort_by(|a, b| b.submitted_at.cmp(&a.submitted_at));
        v.truncate(limit as usize);
        Ok(v)
    }

    fn get(&self, id: &SubmissionId) -> Result<Option<Submission>> {
        Ok(lock(&self.state)
            .submissions
            .iter()
            .find(|s| &s.id == id)
            .cloned())
    }
}

/// Scripted HTTP client: returns queued responses in order and records the
/// requests it saw.
#[derive(Default)]
pub struct FakeHttp {
    responses: Mutex<VecDeque<Result<HttpResponse>>>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl FakeHttp {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn push(&self, status: u16, body: &str) {
        lock(&self.responses).push_back(Ok(HttpResponse {
            status,
            body: body.as_bytes().to_vec(),
        }));
    }

    pub fn push_error(&self, message: &str) {
        lock(&self.responses).push_back(Err(Error::Http(message.into())));
    }

    pub fn requests(&self) -> Vec<HttpRequest> {
        lock(&self.seen).clone()
    }

    /// Forgets the recorded requests (queued responses are kept).
    pub fn clear_requests(&self) {
        lock(&self.seen).clear();
    }
}

impl HttpClient for FakeHttp {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse>> {
        lock(&self.seen).push(request);
        let next = lock(&self.responses)
            .pop_front()
            .unwrap_or_else(|| Err(Error::Http("no scripted response".into())));
        Box::pin(async move { next })
    }
}
