//! Pack catalog: an ordered collection of packs with lookup by id.
//!
//! Built-in packs are loaded first; user packs (from a directory) are appended
//! and may override built-in packs with the same id.

use std::path::Path;

use crate::packs::loader::{LoadError, LoadedPack, load_built_in, load_from_dir};

/// An ordered list of packs with fast lookup by domain id.
///
/// The catalog preserves the insertion order so the UI can iterate packs in a
/// consistent sequence.
#[derive(Debug, Default, Clone)]
pub struct Catalog {
    packs: Vec<LoadedPack>,
}

impl Catalog {
    /// Catalog containing only the three built-in packs (retail, healthcare,
    /// supply chain).
    pub fn built_in() -> Self {
        Self {
            packs: load_built_in(),
        }
    }

    /// Catalog built from built-in packs, then augmented or overridden by
    /// packs found in `user_dir`. Errors for individual user packs are
    /// returned alongside the catalog so callers can surface warnings.
    ///
    /// If `user_dir` does not exist the built-in catalog is returned without
    /// any errors.
    pub fn with_user_dir(user_dir: &Path) -> (Self, Vec<LoadError>) {
        let mut packs = load_built_in();
        let mut errors = Vec::new();

        if user_dir.exists() {
            let (user_packs, user_errors) = load_from_dir(user_dir);
            errors.extend(user_errors);
            for up in user_packs {
                // Replace a built-in pack with the same id if it exists.
                if let Some(pos) = packs
                    .iter()
                    .position(|p| p.manifest.pack.id == up.manifest.pack.id)
                {
                    packs[pos] = up;
                } else {
                    packs.push(up);
                }
            }
        }

        (Self { packs }, errors)
    }

    /// Look up a pack by its domain id.
    pub fn get(&self, id: &str) -> Option<&LoadedPack> {
        self.packs.iter().find(|p| p.manifest.pack.id == id)
    }

    /// All packs in insertion order.
    pub fn all(&self) -> &[LoadedPack] {
        &self.packs
    }

    /// Number of packs in the catalog.
    pub fn len(&self) -> usize {
        self.packs.len()
    }

    /// True if the catalog has no packs.
    pub fn is_empty(&self) -> bool {
        self.packs.is_empty()
    }
}
