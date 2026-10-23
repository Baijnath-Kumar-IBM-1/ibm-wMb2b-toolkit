//! Resumable step runner.
//!
//! A flow (for example partner provisioning) is an ordered list of named
//! [`Step`]s. Each step's output (typically remote IDs) is saved in a
//! [`StepStore`] as soon as the step succeeds. Running the flow again skips
//! succeeded steps and re-runs failed or missing ones, so a retry resumes
//! where the last run stopped. Later steps read earlier outputs through
//! [`StepContext`].
//!
//! Records are grouped by a `scope` string chosen by the feature (see
//! [`crate::partners::partner_scope`]).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::error::{Error, Result};
use crate::events::{Event, EventLevel, EventPublisher};
use crate::http::BoxFuture;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Succeeded,
    Failed,
}

impl StepStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Saved result of one step. `output` holds IDs only, never secrets.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepRecord {
    pub name: String,
    pub status: StepStatus,
    pub output: Value,
    /// User-safe failure text for `Failed` records.
    pub error: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Storage port for step records.
pub trait StepStore: Send + Sync {
    /// Records of a scope, in the order their steps first ran.
    fn list(&self, scope: &str) -> Result<Vec<StepRecord>>;
    /// Inserts or replaces the record with the same `name` in the scope.
    fn save(&self, scope: &str, record: &StepRecord) -> Result<()>;
    /// Removes every record of the scope (used to start a flow over).
    fn clear(&self, scope: &str) -> Result<()>;
}

/// Outputs of the steps that have succeeded so far in this flow.
#[derive(Debug, Clone, Default)]
pub struct StepContext {
    outputs: BTreeMap<String, Value>,
}

impl StepContext {
    pub fn output(&self, step: &str) -> Option<&Value> {
        self.outputs.get(step)
    }

    /// A string field of an earlier step's output.
    pub fn output_str(&self, step: &str, field: &str) -> Option<&str> {
        self.outputs.get(step)?.get(field)?.as_str()
    }
}

/// One unit of work in a flow.
pub trait Step: Send + Sync {
    fn name(&self) -> &str;
    fn run(&self, ctx: StepContext) -> BoxFuture<'static, Result<Value>>;
}

/// Step built from a closure.
pub struct FnStep<F> {
    name: String,
    f: F,
}

impl<F> FnStep<F>
where
    F: Fn(StepContext) -> BoxFuture<'static, Result<Value>> + Send + Sync,
{
    pub fn new(name: impl Into<String>, f: F) -> Self {
        Self {
            name: name.into(),
            f,
        }
    }
}

impl<F> Step for FnStep<F>
where
    F: Fn(StepContext) -> BoxFuture<'static, Result<Value>> + Send + Sync,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&self, ctx: StepContext) -> BoxFuture<'static, Result<Value>> {
        (self.f)(ctx)
    }
}

/// Cooperative cancellation, checked between steps.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepOutcome {
    /// Already succeeded in an earlier run; not executed again.
    Skipped,
    Succeeded,
    Failed,
    /// Not reached because an earlier step failed or the run was cancelled.
    NotRun,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    pub name: String,
    pub outcome: StepOutcome,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    pub steps: Vec<StepReport>,
    pub cancelled: bool,
}

impl RunReport {
    /// True when every step succeeded or was already done.
    pub fn is_complete(&self) -> bool {
        self.steps
            .iter()
            .all(|s| matches!(s.outcome, StepOutcome::Skipped | StepOutcome::Succeeded))
    }
}

pub struct StepRunner {
    store: Arc<dyn StepStore>,
    events: Arc<dyn EventPublisher>,
    clock: Arc<dyn Clock>,
}

impl StepRunner {
    pub fn new(
        store: Arc<dyn StepStore>,
        events: Arc<dyn EventPublisher>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            store,
            events,
            clock,
        }
    }

    /// Runs `steps` in order, stopping at the first failure.
    ///
    /// `kind_prefix` names the events (`{prefix}.step.started`, `.succeeded`,
    /// `.skipped`, `.failed`). Storage errors while saving a record abort the
    /// run with `Err`; a failing step is reported in the [`RunReport`], not as
    /// `Err`.
    pub async fn run(
        &self,
        scope: &str,
        kind_prefix: &str,
        steps: &[Box<dyn Step>],
        cancel: &CancelToken,
    ) -> Result<RunReport> {
        let saved = self.store.list(scope)?;
        let mut ctx = StepContext::default();
        let mut reports: Vec<StepReport> = steps
            .iter()
            .map(|s| StepReport {
                name: s.name().to_string(),
                outcome: StepOutcome::NotRun,
                error: None,
            })
            .collect();
        let mut cancelled = false;

        for (i, step) in steps.iter().enumerate() {
            let name = step.name();
            if let Some(done) = saved
                .iter()
                .find(|r| r.name == name && r.status == StepStatus::Succeeded)
            {
                ctx.outputs.insert(name.to_string(), done.output.clone());
                reports[i].outcome = StepOutcome::Skipped;
                self.emit(EventLevel::Info, kind_prefix, "skipped", name, None);
                continue;
            }
            if cancel.is_cancelled() {
                cancelled = true;
                break;
            }
            self.emit(EventLevel::Info, kind_prefix, "started", name, None);
            match step.run(ctx.clone()).await {
                Ok(output) => {
                    self.store.save(
                        scope,
                        &StepRecord {
                            name: name.to_string(),
                            status: StepStatus::Succeeded,
                            output: output.clone(),
                            error: None,
                            updated_at: self.clock.now(),
                        },
                    )?;
                    ctx.outputs.insert(name.to_string(), output);
                    reports[i].outcome = StepOutcome::Succeeded;
                    self.emit(EventLevel::Info, kind_prefix, "succeeded", name, None);
                }
                Err(e) => {
                    let message = public_message(&e);
                    self.store.save(
                        scope,
                        &StepRecord {
                            name: name.to_string(),
                            status: StepStatus::Failed,
                            output: Value::Null,
                            error: Some(message.clone()),
                            updated_at: self.clock.now(),
                        },
                    )?;
                    reports[i].outcome = StepOutcome::Failed;
                    reports[i].error = Some(message.clone());
                    self.emit(
                        EventLevel::Error,
                        kind_prefix,
                        "failed",
                        name,
                        Some(&message),
                    );
                    break;
                }
            }
        }
        Ok(RunReport {
            steps: reports,
            cancelled,
        })
    }

    fn emit(&self, level: EventLevel, prefix: &str, what: &str, step: &str, error: Option<&str>) {
        let mut data = json!({ "step": step });
        if let Some(e) = error {
            data["error"] = json!(e);
        }
        self.events.publish(
            Event::new(
                self.clock.now(),
                level,
                format!("{prefix}.step.{what}"),
                format!("Step '{step}' {what}"),
            )
            .with_data(data),
        );
    }
}

/// Text that may be stored and shown to the user. Storage and secret-store
/// errors can include driver or OS detail, so they are replaced. HTTP error
/// text is kept because it explains connectivity problems (for example a TLS
/// failure); HTTP client code must keep secrets out of it.
fn public_message(e: &Error) -> String {
    match e {
        Error::Storage(_) | Error::Secret(_) => "internal error; see the application log".into(),
        other => other.to_string(),
    }
}
