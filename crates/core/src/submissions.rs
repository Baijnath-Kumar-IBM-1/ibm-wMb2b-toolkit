//! Submission records (written by the submission engine in [`crate::submit`],
//! read by Activity).

use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;

use crate::error::Result;
use crate::ids::{EnvironmentId, PartnerId, SubmissionId};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Submission {
    pub id: SubmissionId,
    pub environment_id: EnvironmentId,
    #[serde(with = "time::serde::rfc3339")]
    pub submitted_at: OffsetDateTime,
    pub domain: String,
    pub transaction_code: String,
    pub sender_partner_id: Option<PartnerId>,
    pub receiver_partner_id: Option<PartnerId>,
    pub isa_control: u32,
    pub gs_control: u32,
    /// HTTP status of the post; `None` when the document was only generated
    /// or the request failed before a response.
    pub http_status: Option<u16>,
    /// `success`, `failed` or `generated`.
    pub result: String,
    /// Linked trace values (names to values). Never secrets.
    pub trace_values: Value,
    /// The EDI document that was rendered and posted. Contains no secrets.
    pub payload: Option<String>,
    /// Short, single-line excerpt of the channel response (or of the
    /// transport error). Known secrets are redacted before it is stored.
    pub response_excerpt: Option<String>,
    /// How long the post took, in milliseconds.
    pub duration_ms: Option<u64>,
}

/// Storage port for submissions.
pub trait SubmissionRepository: Send + Sync {
    fn save(&self, submission: &Submission) -> Result<()>;
    /// Newest first.
    fn list_recent(&self, environment_id: &EnvironmentId, limit: u32) -> Result<Vec<Submission>>;
    fn get(&self, id: &SubmissionId) -> Result<Option<Submission>>;
}
