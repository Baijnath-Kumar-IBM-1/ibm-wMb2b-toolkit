//! B2B tenant access: authentication, connection test, partner provisioning
//! and document type installation.
//!
//! Ported from the v1 script `b2b-toolkit.sh` (branch `v1-bash`) (`acquire_mcsp_token`, `create_partner`,
//! `create_channel`, `associate_channel_to_partner`, `create_partner_user`,
//! `associate_user_to_partner`, `install_document_type`, `provision_assets`)
//! with the same API paths and payloads. Differences from the script:
//! certificates are always validated, tokens are cached and refreshed on
//! 401, and provisioning is resumable through the step runner.

pub mod api;
pub mod connection;
pub mod documents;
pub mod provisioning;

pub use api::B2bApi;
pub use connection::{ConnectionReport, ConnectionService};
pub use documents::{DocumentOutcome, DocumentReport, DocumentService, InstallReport, doctype_key};
pub use provisioning::{PROVISION_STEPS, PartnerProvisioning, ProvisioningService, StepState};

#[cfg(test)]
mod tests;
