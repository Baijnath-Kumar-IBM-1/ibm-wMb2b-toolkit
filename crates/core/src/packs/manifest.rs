//! Pack manifest deserialization types.
//!
//! The manifest schema is the TOML `pack.toml` in each pack directory.

use std::collections::BTreeMap;

use serde::Deserialize;

/// Top-level manifest. Deserialized from `pack.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct PackManifest {
    pub pack: PackMeta,
    /// Template variable → pattern string.
    /// Keys match `{{PLACEHOLDER}}` names in EDI templates exactly.
    #[serde(default)]
    pub trace_rules: BTreeMap<String, String>,
    #[serde(default)]
    pub documents: Vec<DocumentDef>,
}

/// Core pack metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct PackMeta {
    /// Machine identifier, e.g. `"retail"`. Must be unique in the catalog.
    pub id: String,
    /// Human-readable name, e.g. `"Retail"`.
    pub label: String,
    /// EDI standard, e.g. `"X12"`.
    pub standard: String,
    /// Standard version, e.g. `"004010"`.
    pub version: String,
    /// ISA element terminator character, e.g. `~` or `!`.
    pub terminator: String,
    /// ISA version string (ISA12), e.g. `"00401"`.
    pub isa_version: String,
}

/// One document type within a pack.
#[derive(Debug, Clone, Deserialize)]
pub struct DocumentDef {
    /// X12 transaction set code, e.g. `"850"`.
    pub code: String,
    /// Human-readable name, e.g. `"Purchase Order"`.
    pub name: String,
    /// Optional explanatory note shown in the UI.
    #[serde(default)]
    pub note: String,
    /// Path to the EDI template, relative to the pack directory.
    pub template: String,
    /// Which partner is the sender (determines ISA06/ISA08 ordering).
    pub sender: Sender,
}

/// Which partner sends the document.
///
/// Each template fixes its direction in the ISA06/ISA08 fields:
/// - `P1` → ISA06 = P1_ISA, ISA08 = P2_ISA  
/// - `P2` → ISA06 = P2_ISA, ISA08 = P1_ISA
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sender {
    P1,
    P2,
}
