//! Toolkit v2 core.
//!
//! Feature services and storage ports live here. This crate has no HTTP
//! server, no UI and no database driver; `trace-storage` implements the
//! ports. See `docs/adding-a-feature.md` for how features are structured.

pub mod b2b;
pub mod clock;
pub mod counters;
pub mod environments;
pub mod error;
pub mod events;
pub mod http;
pub mod ids;
pub mod installed;
pub mod packs;
pub mod partners;
pub mod preferences;
pub mod secrets;
pub mod steps;
pub mod submissions;
pub mod submit;
pub mod validation;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod tests;

pub use error::{Error, Result};
