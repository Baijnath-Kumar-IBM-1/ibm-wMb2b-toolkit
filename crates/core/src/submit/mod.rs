//! Submission engine: renders EDI documents and posts them to the sender
//! partner's channel.
//!
//! Ported from `post_edi_document`, `submit_document_sequence`, the trace
//! value generation and the render flow of the v1 script (branch `v1-bash`).
//!
//! - Each document is posted to the channel endpoint of its sender (the
//!   pack's `sender` is `p1` or `p2`) with HTTP Basic Auth (partner user and
//!   the password from the [`SecretStore`]) and `Content-Type: application/EDI`.
//!   The MCSP bearer token is not used for channel posts.
//! - ISA and GS control numbers are reserved atomically in one call to
//!   [`CounterRepository::allocate`] before the first post, so concurrent runs
//!   never share a number. A run that stops early leaves gaps, which X12
//!   permits.
//! - A sequence stops at the first failed document (the script carried on);
//!   documents after it are reported as `not_run`.
//! - [`SubmissionService::preview`] renders without allocating numbers,
//!   incrementing the run count, posting or saving anything.
//!
//! Passwords are read just before use and never stored, logged, returned or
//! put into events. Response excerpts and transport errors are redacted
//! before they are saved or emitted. TLS validation stays on in the
//! [`HttpClient`]; plain `http://` endpoints are refused here.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::b2b::ProvisioningService;
use crate::b2b::api::excerpt;
use crate::clock::Clock;
use crate::counters::{self, ControlNumbers, CounterRepository, MAX_ALLOCATION};
use crate::environments::{Environment, EnvironmentRepository};
use crate::error::{Error, Result};
use crate::events::{Event, EventLevel, EventPublisher};
use crate::http::{HttpClient, HttpRequest, Method};
use crate::ids::{EnvironmentId, PartnerId, SubmissionId};
use crate::installed::validate_transaction_code;
use crate::packs::loader::LoadedPack;
use crate::packs::manifest::DocumentDef;
use crate::packs::{
    Catalog, RenderContext, Sender, TraceValues, generate_trace_values, render_template,
};
use crate::partners::{Partner, PartnerRepository};
use crate::preferences::PreferenceRepository;
use crate::secrets::SecretStore;
use crate::steps::CancelToken;
use crate::submissions::{Submission, SubmissionRepository};
use crate::validation::{self, Collector};

#[cfg(test)]
mod tests;

/// How often a waiting run checks for cancellation.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// What to submit.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SubmitRequest {
    pub environment_id: EnvironmentId,
    pub pack_id: String,
    /// Transaction codes in the order to send them; `None` sends the whole
    /// pack in its sequence. One code submits a single document.
    #[serde(default)]
    pub codes: Option<Vec<String>>,
    /// Saved partners that play P1 and P2 of the pack.
    pub p1_partner_id: PartnerId,
    pub p2_partner_id: PartnerId,
    /// Replacements for the pack's generated trace values (key to value).
    #[serde(default)]
    pub trace_overrides: BTreeMap<String, String>,
}

/// A party of a preview: a saved partner or a manually entered ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Party {
    Saved {
        partner_id: PartnerId,
    },
    /// A DUNS number (9 to 15 digits). Works for generation only.
    Manual {
        id_value: String,
    },
}

/// What to render without sending.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PreviewRequest {
    /// Environment whose counters supply the control numbers (read only).
    pub environment_id: EnvironmentId,
    pub pack_id: String,
    #[serde(default)]
    pub codes: Option<Vec<String>>,
    pub p1: Party,
    pub p2: Party,
    #[serde(default)]
    pub trace_overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionOutcome {
    /// The channel answered with a 2xx status.
    Submitted,
    /// Non-2xx answer or transport failure.
    Failed,
    /// Not reached because an earlier document failed or the run was cancelled.
    NotRun,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentSubmission {
    pub transaction_code: String,
    pub outcome: SubmissionOutcome,
    pub isa_control: Option<u32>,
    pub gs_control: Option<u32>,
    pub http_status: Option<u16>,
    /// User-safe failure text (redacted).
    pub error: Option<String>,
    pub submission_id: Option<SubmissionId>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubmitReport {
    /// Run number used for the `{seq}` part of the trace values.
    pub run_count: u64,
    pub trace_values: BTreeMap<String, String>,
    pub documents: Vec<DocumentSubmission>,
    pub cancelled: bool,
}

impl SubmitReport {
    /// True when every document was submitted.
    pub fn is_complete(&self) -> bool {
        self.documents
            .iter()
            .all(|d| d.outcome == SubmissionOutcome::Submitted)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewDocument {
    pub transaction_code: String,
    pub name: String,
    /// `p1` or `p2`.
    pub sender: &'static str,
    pub isa_control: u32,
    pub gs_control: u32,
    pub payload: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewReport {
    /// Run number a submission started now would use.
    pub run_count: u64,
    pub trace_values: BTreeMap<String, String>,
    pub documents: Vec<PreviewDocument>,
}

/// Collaborators of [`SubmissionService`].
pub struct SubmissionDeps {
    pub http: Arc<dyn HttpClient>,
    pub catalog: Arc<Catalog>,
    pub envs: Arc<dyn EnvironmentRepository>,
    pub partners: Arc<dyn PartnerRepository>,
    pub counters: Arc<dyn CounterRepository>,
    pub submissions: Arc<dyn SubmissionRepository>,
    pub prefs: Arc<dyn PreferenceRepository>,
    pub secrets: Arc<dyn SecretStore>,
    pub provisioning: Arc<ProvisioningService>,
    pub clock: Arc<dyn Clock>,
    pub events: Arc<dyn EventPublisher>,
}

pub struct SubmissionService {
    deps: SubmissionDeps,
    delay: Option<Duration>,
}

/// Where and how to post for one sender.
struct Route {
    url: String,
    username: String,
    password: SecretString,
}

impl Route {
    fn authorization(&self) -> String {
        let pair = format!("{}:{}", self.username, self.password.expose_secret());
        format!("Basic {}", STANDARD.encode(pair))
    }

    /// Values that must never appear in stored or emitted text.
    fn secrets(&self) -> Vec<String> {
        let pair = format!("{}:{}", self.username, self.password.expose_secret());
        vec![
            self.password.expose_secret().to_string(),
            STANDARD.encode(pair),
        ]
    }
}

/// Everything fixed for one run.
struct Run<'a> {
    env: &'a Environment,
    pack: &'a LoadedPack,
    p1: &'a Partner,
    p2: &'a Partner,
    p1_route: Option<Route>,
    p2_route: Option<Route>,
    trace: &'a TraceValues,
    timeout: Duration,
}

fn sender_label(sender: Sender) -> &'static str {
    match sender {
        Sender::P1 => "p1",
        Sender::P2 => "p2",
    }
}

impl SubmissionService {
    pub fn new(deps: SubmissionDeps) -> Self {
        Self { deps, delay: None }
    }

    /// Replaces the delay between documents, which otherwise comes from the
    /// preferences (`inter_document_delay_secs`). Mainly for tests.
    #[must_use]
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Submits one document: a sequence of exactly `code`.
    pub async fn submit_document(
        &self,
        request: &SubmitRequest,
        code: &str,
        cancel: &CancelToken,
    ) -> Result<SubmitReport> {
        let mut one = request.clone();
        one.codes = Some(vec![code.to_string()]);
        self.submit(&one, cancel).await
    }

    /// Renders and posts documents in order. Stops at the first document the
    /// channel does not accept; the report says which documents were not run.
    ///
    /// Invalid input, a missing channel or password and similar problems are
    /// returned as `Err` before anything is allocated or posted.
    pub async fn submit(
        &self,
        request: &SubmitRequest,
        cancel: &CancelToken,
    ) -> Result<SubmitReport> {
        let d = &self.deps;
        let env = d
            .envs
            .get(&request.environment_id)?
            .ok_or(Error::NotFound("environment"))?;
        let pack = self.pack(&request.pack_id)?;
        let docs = select_documents(pack, &request.pack_id, request.codes.as_deref())?;
        check_overrides(pack, &request.trace_overrides)?;
        let p1 = self.saved_partner(&env.id, &request.p1_partner_id, "p1_partner_id")?;
        let p2 = self.saved_partner(&env.id, &request.p2_partner_id, "p2_partner_id")?;
        let prefs = d.prefs.get()?;

        let senders = |s: Sender| docs.iter().any(|doc| doc.sender == s);
        let p1_route = if senders(Sender::P1) {
            Some(self.route(&p1)?)
        } else {
            None
        };
        let p2_route = if senders(Sender::P2) {
            Some(self.route(&p2)?)
        } else {
            None
        };

        // Dry render with placeholder numbers so a broken template is found
        // before any control number is used.
        let peek = d.counters.peek(&env.id)?;
        let probe = self.trace(pack, peek.run_count + 1, &request.trace_overrides);
        for doc in &docs {
            self.render(
                pack,
                doc,
                &p1.id_value,
                &p2.id_value,
                ControlNumbers { isa: 1, gs: 1 },
                &probe,
            )?;
        }

        let mut documents: Vec<DocumentSubmission> = docs
            .iter()
            .map(|doc| DocumentSubmission {
                transaction_code: doc.code.clone(),
                outcome: SubmissionOutcome::NotRun,
                isa_control: None,
                gs_control: None,
                http_status: None,
                error: None,
                submission_id: None,
                duration_ms: None,
            })
            .collect();

        if cancel.is_cancelled() {
            self.emit_cancelled(&env.id);
            return Ok(SubmitReport {
                run_count: peek.run_count,
                trace_values: BTreeMap::new(),
                documents,
                cancelled: true,
            });
        }

        // Same order as the script: count the run first, then derive the
        // trace values from the new count.
        let run_count = d.counters.increment_run_count(&env.id)?;
        let trace = self.trace(pack, run_count, &request.trace_overrides);
        let count = u32::try_from(docs.len())
            .map_err(|_| Error::validation("codes", "too many documents"))?;
        let numbers = d.counters.allocate(&env.id, count)?;

        let run = Run {
            env: &env,
            pack,
            p1: &p1,
            p2: &p2,
            p1_route,
            p2_route,
            trace: &trace,
            timeout: Duration::from_secs(u64::from(prefs.payload_timeout_secs)),
        };
        let delay = self
            .delay
            .unwrap_or_else(|| Duration::from_secs(u64::from(prefs.inter_document_delay_secs)));

        let mut cancelled = false;
        for (i, doc) in docs.iter().enumerate() {
            if i > 0 {
                pause(delay, cancel).await;
            }
            if cancel.is_cancelled() {
                cancelled = true;
                self.emit_cancelled(&env.id);
                break;
            }
            let done = self.post_document(&run, doc, numbers[i]).await?;
            let failed = done.outcome == SubmissionOutcome::Failed;
            documents[i] = done;
            if failed {
                break;
            }
        }

        Ok(SubmitReport {
            run_count,
            trace_values: trace.0.clone(),
            documents,
            cancelled,
        })
    }

    /// Renders documents with the numbers the next submission would use,
    /// without allocating them, counting a run, posting or saving anything.
    pub fn preview(&self, request: &PreviewRequest) -> Result<PreviewReport> {
        let d = &self.deps;
        let env = d
            .envs
            .get(&request.environment_id)?
            .ok_or(Error::NotFound("environment"))?;
        let pack = self.pack(&request.pack_id)?;
        let docs = select_documents(pack, &request.pack_id, request.codes.as_deref())?;
        check_overrides(pack, &request.trace_overrides)?;
        let p1 = self.party_duns(&env.id, &request.p1, "p1")?;
        let p2 = self.party_duns(&env.id, &request.p2, "p2")?;

        let peek = d.counters.peek(&env.id)?;
        let run_count = peek.run_count + 1;
        let trace = self.trace(pack, run_count, &request.trace_overrides);

        let mut out = Vec::with_capacity(docs.len());
        for (i, doc) in docs.iter().enumerate() {
            let offset = u32::try_from(i).unwrap_or(u32::MAX);
            let nums = ControlNumbers {
                isa: counters::advance(peek.isa_next, offset),
                gs: counters::advance(peek.gs_next, offset),
            };
            out.push(PreviewDocument {
                transaction_code: doc.code.clone(),
                name: doc.name.clone(),
                sender: sender_label(doc.sender),
                isa_control: nums.isa,
                gs_control: nums.gs,
                payload: self.render(pack, doc, &p1, &p2, nums, &trace)?,
            });
        }
        Ok(PreviewReport {
            run_count,
            trace_values: trace.0.clone(),
            documents: out,
        })
    }

    async fn post_document(
        &self,
        run: &Run<'_>,
        doc: &DocumentDef,
        nums: ControlNumbers,
    ) -> Result<DocumentSubmission> {
        let d = &self.deps;
        let (sender, receiver, route) = match doc.sender {
            Sender::P1 => (run.p1, run.p2, run.p1_route.as_ref()),
            Sender::P2 => (run.p2, run.p1, run.p2_route.as_ref()),
        };
        let route =
            route.ok_or_else(|| Error::Conflict("no channel route for the sender".into()))?;
        let payload = self.render(
            run.pack,
            doc,
            &run.p1.id_value,
            &run.p2.id_value,
            nums,
            run.trace,
        )?;
        let base = json!({
            "environment_id": run.env.id,
            "transaction_code": doc.code,
            "sender": sender_label(doc.sender),
            "isa_control": nums.isa,
            "gs_control": nums.gs,
        });
        self.emit(
            EventLevel::Info,
            "submission.rendered",
            format!("Rendered {}", doc.code),
            base.clone(),
        );

        let request = HttpRequest::new(Method::Post, route.url.clone(), run.timeout)
            .header("Authorization", route.authorization())
            .header("Content-Type", "application/EDI")
            .body(payload.clone().into_bytes())
            .ca_bundle(run.env.ca_bundle_path.clone());
        self.emit(
            EventLevel::Info,
            "submission.posting",
            format!("Posting {}", doc.code),
            base.clone(),
        );

        let redact_values = route.secrets();
        let redact: Vec<&str> = redact_values.iter().map(String::as_str).collect();
        let started = Instant::now();
        let sent = d.http.send(request).await;
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let (http_status, success, text, error) = match sent {
            Ok(response) => {
                let text = excerpt(&response.body, &redact);
                let error =
                    (!response.is_success()).then(|| format!("HTTP {}: {text}", response.status));
                (Some(response.status), response.is_success(), text, error)
            }
            Err(Error::Http(message)) => {
                let text = excerpt(message.as_bytes(), &redact);
                (
                    None,
                    false,
                    text.clone(),
                    Some(format!("request failed: {text}")),
                )
            }
            Err(other) => return Err(other),
        };

        let submission_id = SubmissionId::generate();
        let linked = linked_trace(run.pack.template(&doc.code).unwrap_or_default(), run.trace);
        d.submissions.save(&Submission {
            id: submission_id.clone(),
            environment_id: run.env.id.clone(),
            submitted_at: d.clock.now(),
            domain: run.pack.manifest.pack.id.clone(),
            transaction_code: doc.code.clone(),
            sender_partner_id: Some(sender.id.clone()),
            receiver_partner_id: Some(receiver.id.clone()),
            isa_control: nums.isa,
            gs_control: nums.gs,
            http_status,
            result: if success { "success" } else { "failed" }.into(),
            trace_values: json!(linked),
            payload: Some(payload),
            response_excerpt: (!text.is_empty()).then_some(text),
            duration_ms: Some(duration_ms),
        })?;

        let mut data = base;
        data["submission_id"] = json!(submission_id);
        data["http_status"] = json!(http_status);
        data["duration_ms"] = json!(duration_ms);
        data["result"] = json!(if success { "success" } else { "failed" });
        if let Some(e) = &error {
            data["error"] = json!(e);
        }
        self.emit(
            if success {
                EventLevel::Info
            } else {
                EventLevel::Error
            },
            "submission.result",
            if success {
                format!("Document {} submitted", doc.code)
            } else {
                format!("Document {} failed", doc.code)
            },
            data,
        );

        Ok(DocumentSubmission {
            transaction_code: doc.code.clone(),
            outcome: if success {
                SubmissionOutcome::Submitted
            } else {
                SubmissionOutcome::Failed
            },
            isa_control: Some(nums.isa),
            gs_control: Some(nums.gs),
            http_status,
            error,
            submission_id: Some(submission_id),
            duration_ms: Some(duration_ms),
        })
    }

    fn pack(&self, pack_id: &str) -> Result<&LoadedPack> {
        self.deps
            .catalog
            .get(pack_id)
            .ok_or(Error::NotFound("domain pack"))
    }

    fn saved_partner(
        &self,
        environment_id: &EnvironmentId,
        id: &PartnerId,
        field: &str,
    ) -> Result<Partner> {
        let partner = self
            .deps
            .partners
            .get(id)?
            .ok_or(Error::NotFound("partner"))?;
        if &partner.environment_id != environment_id {
            return Err(Error::validation(
                field,
                "this partner belongs to another environment",
            ));
        }
        Ok(partner)
    }

    fn party_duns(
        &self,
        environment_id: &EnvironmentId,
        party: &Party,
        field: &str,
    ) -> Result<String> {
        match party {
            Party::Saved { partner_id } => Ok(self
                .saved_partner(environment_id, partner_id, field)?
                .id_value),
            Party::Manual { id_value } => validation::digits(id_value, 9, 15)
                .map_err(|m| Error::validation(format!("{field}.id_value"), m)),
        }
    }

    /// Channel URL and credentials of a provisioned partner.
    fn route(&self, partner: &Partner) -> Result<Route> {
        let name = &partner.name;
        let status = self.deps.provisioning.status(&partner.id)?;
        if !status.complete {
            return Err(Error::Conflict(format!(
                "partner '{name}' is not fully provisioned; provision it first"
            )));
        }
        let uri = status.channel_endpoint_uri.ok_or_else(|| {
            Error::Conflict(format!("partner '{name}' has no channel endpoint yet"))
        })?;
        let url = channel_url(&uri)
            .map_err(|message| Error::Conflict(format!("partner '{name}': {message}")))?;
        let secret_id = partner
            .password_secret
            .as_ref()
            .ok_or_else(|| Error::Conflict(format!("partner '{name}' has no password")))?;
        let password = self.deps.secrets.get(secret_id)?.ok_or_else(|| {
            Error::Conflict(format!("the password of partner '{name}' is not available"))
        })?;
        Ok(Route {
            url,
            username: partner.username.clone(),
            password,
        })
    }

    fn trace(
        &self,
        pack: &LoadedPack,
        run_count: u64,
        overrides: &BTreeMap<String, String>,
    ) -> TraceValues {
        let mut trace = generate_trace_values(
            &pack.manifest.trace_rules,
            self.deps.clock.as_ref(),
            run_count,
        );
        for (key, value) in overrides {
            trace.set(key.clone(), value.clone());
        }
        trace
    }

    fn render(
        &self,
        pack: &LoadedPack,
        doc: &DocumentDef,
        p1_duns: &str,
        p2_duns: &str,
        nums: ControlNumbers,
        trace: &TraceValues,
    ) -> Result<String> {
        let template = pack.template(&doc.code).ok_or_else(|| {
            Error::validation(
                "codes",
                format!("the pack has no template for {}", doc.code),
            )
        })?;
        render_template(
            template,
            &RenderContext {
                p1_duns,
                p2_duns,
                isa_ctrl: nums.isa,
                gs_ctrl: nums.gs,
                trace,
                clock: self.deps.clock.as_ref(),
            },
        )
        .map_err(|e| Error::validation("codes", format!("{} cannot be rendered: {e}", doc.code)))
    }

    fn emit(&self, level: EventLevel, kind: &str, message: String, data: Value) {
        self.deps
            .events
            .publish(Event::new(self.deps.clock.now(), level, kind, message).with_data(data));
    }

    fn emit_cancelled(&self, environment_id: &EnvironmentId) {
        self.emit(
            EventLevel::Warn,
            "submission.cancelled",
            "Submission cancelled".into(),
            json!({ "environment_id": environment_id }),
        );
    }
}

/// The requested documents in order, or the whole pack sequence.
fn select_documents(
    pack: &LoadedPack,
    pack_id: &str,
    codes: Option<&[String]>,
) -> Result<Vec<DocumentDef>> {
    let docs: Vec<DocumentDef> = match codes {
        None => pack.documents().to_vec(),
        Some(list) => list
            .iter()
            .map(|code| {
                let code = validate_transaction_code(code)?;
                pack.documents()
                    .iter()
                    .find(|d| d.code == code)
                    .cloned()
                    .ok_or_else(|| {
                        Error::validation(
                            "codes",
                            format!("{code} is not a document of the '{pack_id}' pack"),
                        )
                    })
            })
            .collect::<Result<_>>()?,
    };
    if docs.is_empty() {
        return Err(Error::validation("codes", "select at least one document"));
    }
    if docs.len() > MAX_ALLOCATION as usize {
        return Err(Error::validation(
            "codes",
            format!("at most {MAX_ALLOCATION} documents per run"),
        ));
    }
    Ok(docs)
}

/// Overrides may only replace trace values the pack defines, and their text
/// ends up inside EDI, so it is limited to characters that cannot break
/// segments or elements.
fn check_overrides(pack: &LoadedPack, overrides: &BTreeMap<String, String>) -> Result<()> {
    let mut c = Collector::new();
    for (key, value) in overrides {
        if !pack.manifest.trace_rules.contains_key(key) {
            c.add(
                "trace_overrides",
                format!("{key} is not a trace value of this pack"),
            );
            continue;
        }
        if let Err(message) = validation::restricted_text(
            value,
            1,
            50,
            |ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':'),
            "letters, digits and - _ . / :",
        ) {
            c.add(&format!("trace_overrides.{key}"), message);
        }
    }
    c.finish()
}

/// The trace values a template actually uses (for linking submissions).
fn linked_trace(template: &str, trace: &TraceValues) -> BTreeMap<String, String> {
    trace
        .0
        .iter()
        .filter(|(key, _)| template.contains(&format!("{{{{{key}}}}}")))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Channel endpoints are stored without scheme sometimes; like the script,
/// `https://` is added then. Plain `http://` is refused.
fn channel_url(uri: &str) -> std::result::Result<String, &'static str> {
    let uri = uri.trim();
    if uri.is_empty() || uri.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("the channel endpoint is not a valid URL");
    }
    let has_prefix = |p: &str| {
        uri.get(..p.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(p))
    };
    if has_prefix("http://") {
        return Err("the channel endpoint uses plain http://; only https:// is allowed");
    }
    if has_prefix("https://") {
        Ok(uri.to_string())
    } else {
        Ok(format!("https://{uri}"))
    }
}

/// Sleeps for `total` but returns early once `cancel` is set.
async fn pause(total: Duration, cancel: &CancelToken) {
    if total.is_zero() {
        return;
    }
    let deadline = tokio::time::Instant::now() + total;
    while !cancel.is_cancelled() {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        tokio::time::sleep(left.min(CANCEL_POLL)).await;
    }
}
