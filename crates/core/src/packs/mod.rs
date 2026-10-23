//! Pack system: built-in and user-defined domain packs.
//!
//! A pack is a TOML manifest (`pack.toml`) plus EDI template files. It defines
//! a domain (retail, healthcare, supply chain), its documents, EDI delimiters
//! and ISA version, and trace-value generation rules.
//!
//! Built-in packs are compiled into the binary via [`include_str!`]. A user
//! packs directory can add or override packs at runtime.
//!
//! # Examples
//!
//! ```rust
//! use trace_core::packs::Catalog;
//!
//! let catalog = Catalog::built_in();
//! let retail = catalog.get("retail").expect("retail pack always present");
//! assert_eq!(retail.manifest.pack.label, "Retail");
//! assert_eq!(retail.documents().len(), 9);
//! ```

pub mod catalog;
pub mod loader;
pub mod manifest;
pub mod render;
pub mod trace_values;

#[cfg(test)]
mod tests;

pub use catalog::Catalog;
pub use loader::LoadError;
pub use manifest::{DocumentDef, PackManifest, Sender};
pub use render::{RenderContext, RenderError, render_template};
pub use trace_values::{TraceValues, generate_trace_values};
