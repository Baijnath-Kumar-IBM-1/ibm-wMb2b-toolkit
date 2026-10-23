//! Document type installation (script: `install_document_type`).
//!
//! Document types are keyed by transaction code per environment, so a code
//! shared by several packs (820) is installed once.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use crate::b2b::api::{B2bApi, json_text, path_segment, status_error};
use crate::clock::Clock;
use crate::environments::EnvironmentRepository;
use crate::error::{Error, Result};
use crate::events::{Event, EventLevel, EventPublisher};
use crate::http::Method;
use crate::ids::EnvironmentId;
use crate::installed::{InstalledDocument, InstalledDocumentRepository, validate_transaction_code};
use crate::packs::Catalog;
use crate::steps::CancelToken;

/// Pause after each document that was actually created on the tenant (the
/// script sleeps 0.5 s between installs).
pub const DEFAULT_PAUSE: Duration = Duration::from_millis(500);

/// Document type key used by the B2B API, for example
/// `WMX12-4010--850---------` for X12 004010 transaction 850.
///
/// Byte-for-byte equal to the script's
/// `printf "WMX12-4010--%s---------" "$txn"` for 004010. The `4010` part is
/// the pack version without its leading zeros; `WMX12` is `WM` + standard.
/// There are nine dashes after the transaction code.
pub fn doctype_key(standard: &str, version: &str, txn: &str) -> Result<String> {
    Ok(format!(
        "WM{standard}-{}--{txn}---------",
        api_version(version)?
    ))
}

/// Version as the B2B API writes it: `004010` becomes `4010`.
fn api_version(version: &str) -> Result<&str> {
    let v = version.trim_start_matches('0');
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::validation(
            "version",
            "pack version must be digits such as 004010",
        ));
    }
    Ok(v)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentOutcome {
    /// Created on the tenant by this run.
    Installed,
    /// Already on the tenant; its key was confirmed with a GET.
    AlreadyExisted,
    /// Recorded as installed in an earlier run; no tenant call was made.
    AlreadyInstalled,
    Failed,
    /// Not reached because an earlier document failed or the run was cancelled.
    NotRun,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentReport {
    pub transaction_code: String,
    pub outcome: DocumentOutcome,
    pub document_type_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallReport {
    pub documents: Vec<DocumentReport>,
    pub cancelled: bool,
}

impl InstallReport {
    pub fn is_complete(&self) -> bool {
        self.documents.iter().all(|d| {
            matches!(
                d.outcome,
                DocumentOutcome::Installed
                    | DocumentOutcome::AlreadyExisted
                    | DocumentOutcome::AlreadyInstalled
            )
        })
    }
}

pub struct DocumentService {
    api: Arc<B2bApi>,
    catalog: Arc<Catalog>,
    envs: Arc<dyn EnvironmentRepository>,
    installed: Arc<dyn InstalledDocumentRepository>,
    clock: Arc<dyn Clock>,
    events: Arc<dyn EventPublisher>,
    pause: Duration,
}

impl DocumentService {
    pub fn new(
        api: Arc<B2bApi>,
        catalog: Arc<Catalog>,
        envs: Arc<dyn EnvironmentRepository>,
        installed: Arc<dyn InstalledDocumentRepository>,
        clock: Arc<dyn Clock>,
        events: Arc<dyn EventPublisher>,
    ) -> Self {
        Self {
            api,
            catalog,
            envs,
            installed,
            clock,
            events,
            pause: DEFAULT_PAUSE,
        }
    }

    #[must_use]
    pub fn with_pause(mut self, pause: Duration) -> Self {
        self.pause = pause;
        self
    }

    pub fn list_installed(&self, environment_id: &EnvironmentId) -> Result<Vec<InstalledDocument>> {
        self.installed.list(environment_id)
    }

    /// Installs documents of one pack on the tenant. `codes` selects a subset
    /// (in the given order); `None` installs the whole pack in its sequence.
    ///
    /// Like the script, the run stops at the first failure; run it again to
    /// continue, because installed documents are skipped.
    pub async fn install(
        &self,
        environment_id: &EnvironmentId,
        pack_id: &str,
        codes: Option<&[String]>,
        cancel: &CancelToken,
    ) -> Result<InstallReport> {
        let env = self
            .envs
            .get(environment_id)?
            .ok_or(Error::NotFound("environment"))?;
        let pack = self
            .catalog
            .get(pack_id)
            .ok_or(Error::NotFound("domain pack"))?;
        let standard = pack.manifest.pack.standard.clone();
        let version = pack.manifest.pack.version.clone();
        api_version(&version)?;

        let mut wanted: Vec<String> = match codes {
            None => pack.documents().iter().map(|d| d.code.clone()).collect(),
            Some(list) => list
                .iter()
                .map(|c| validate_transaction_code(c))
                .collect::<Result<_>>()?,
        };
        let mut seen = std::collections::HashSet::new();
        wanted.retain(|c| seen.insert(c.clone()));
        if let Some(unknown) = wanted
            .iter()
            .find(|c| !pack.documents().iter().any(|d| &d.code == *c))
        {
            return Err(Error::validation(
                "codes",
                format!("{unknown} is not a document of the '{pack_id}' pack"),
            ));
        }

        let known: Vec<InstalledDocument> = self.installed.list(environment_id)?;
        let mut documents: Vec<DocumentReport> = wanted
            .iter()
            .map(|c| DocumentReport {
                transaction_code: c.clone(),
                outcome: DocumentOutcome::NotRun,
                document_type_id: None,
                error: None,
            })
            .collect();
        let mut cancelled = false;

        for report in &mut documents {
            let txn = report.transaction_code.clone();
            if let Some(done) = known.iter().find(|d| d.transaction_code == txn) {
                report.outcome = DocumentOutcome::AlreadyInstalled;
                report.document_type_id = Some(done.document_type_id.clone());
                continue;
            }
            if cancel.is_cancelled() {
                cancelled = true;
                break;
            }
            match self.install_one(&env, &standard, &version, &txn).await {
                Ok((outcome, id)) => {
                    self.installed.save(
                        environment_id,
                        &InstalledDocument {
                            transaction_code: txn.clone(),
                            document_type_id: id.clone(),
                            installed_at: self.clock.now(),
                        },
                    )?;
                    report.outcome = outcome;
                    report.document_type_id = Some(id);
                    self.emit(EventLevel::Info, "document.installed", &txn, &env.id, None);
                    if matches!(outcome, DocumentOutcome::Installed) && !self.pause.is_zero() {
                        tokio::time::sleep(self.pause).await;
                    }
                }
                Err(e) if matches!(e, Error::Http(_) | Error::Conflict(_)) => {
                    let message = e.to_string();
                    report.outcome = DocumentOutcome::Failed;
                    report.error = Some(message.clone());
                    self.emit(
                        EventLevel::Error,
                        "document.install_failed",
                        &txn,
                        &env.id,
                        Some(&message),
                    );
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(InstallReport {
            documents,
            cancelled,
        })
    }

    async fn install_one(
        &self,
        env: &crate::environments::Environment,
        standard: &str,
        version: &str,
        txn: &str,
    ) -> Result<(DocumentOutcome, String)> {
        let key = doctype_key(standard, version, txn)?;
        let body = json!({
            "customTransaction": "",
            "referenceDocument": Value::Null,
            "standard": standard,
            "status": "Active",
            "transaction": txn,
            "type": { "id": "edi", "displayValue": "EDI" },
            "version": api_version(version)?,
            "referenceType": "WMINT",
        });
        let response = self
            .api
            .call(env, Method::Post, "/b2b/api/v1/documents", Some(&body))
            .await?;

        if response.is_success() {
            let value: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
            let id = json_text(&value, "/id").ok_or_else(|| {
                Error::Http("document type was created but the response has no id".into())
            })?;
            return Ok((DocumentOutcome::Installed, id));
        }

        // Same rule as the script: HTTP 409, or a body that says the document
        // already exists / is installed / is a duplicate.
        let lower = String::from_utf8_lossy(&response.body).to_lowercase();
        let conflict = response.status == 409
            || lower.contains("already exists")
            || lower.contains("already installed")
            || lower.contains("duplicate");
        if !conflict {
            return Err(status_error(
                &response,
                &format!("installing document type {txn}"),
                &[],
            ));
        }

        let get = self
            .api
            .call(
                env,
                Method::Get,
                &format!("/b2b/api/v1/documents/{}", path_segment(&key)?),
                None,
            )
            .await?;
        if get.is_success() {
            // The script stores the key itself as the document type id.
            Ok((DocumentOutcome::AlreadyExisted, key))
        } else {
            Err(status_error(
                &get,
                &format!("looking up existing document type {txn}"),
                &[],
            ))
        }
    }

    fn emit(
        &self,
        level: EventLevel,
        kind: &str,
        txn: &str,
        environment_id: &EnvironmentId,
        error: Option<&str>,
    ) {
        let mut data = json!({ "environment_id": environment_id, "transaction_code": txn });
        if let Some(e) = error {
            data["error"] = json!(e);
        }
        self.events.publish(
            Event::new(
                self.clock.now(),
                level,
                kind,
                format!("Document type {txn}"),
            )
            .with_data(data),
        );
    }
}
