//! Pack loader: built-in (embedded) and user directory packs.
//!
//! Built-in packs are embedded at compile time via [`include_str!`] so no
//! external files are needed at runtime. User packs are loaded from a directory
//! on the file system and can add new domains or override built-in ones (the
//! last pack with a given `id` wins).

use std::collections::BTreeMap;
use std::path::Path;

use crate::packs::manifest::PackManifest;

/// A loaded pack: its manifest and the raw EDI template strings.
#[derive(Debug, Clone)]
pub struct LoadedPack {
    pub manifest: PackManifest,
    /// Map from `DocumentDef::code` → raw template text.
    pub(crate) templates: BTreeMap<String, String>,
}

impl LoadedPack {
    /// Returns the [`crate::packs::manifest::DocumentDef`] slice in pack order.
    pub fn documents(&self) -> &[crate::packs::manifest::DocumentDef] {
        &self.manifest.documents
    }

    /// Raw EDI template text for a given transaction code, or `None` if
    /// the template is missing.
    pub fn template(&self, code: &str) -> Option<&str> {
        self.templates.get(code).map(String::as_str)
    }
}

/// Error returned when a pack cannot be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("failed to parse manifest for pack '{id}': {source}")]
    ManifestParse {
        id: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("missing template '{path}' for pack '{id}' document '{code}'")]
    MissingTemplate {
        id: String,
        code: String,
        path: String,
    },
    #[error("I/O error loading pack '{id}': {source}")]
    Io {
        id: String,
        #[source]
        source: std::io::Error,
    },
    #[error("pack manifest for '{id}' has empty id")]
    EmptyId { id: String },
}

/// Parse a manifest TOML string and resolve its templates from a `lookup`
/// function.
///
/// `lookup(relative_path) -> Option<String>` returns the raw template text.
/// This abstraction works for both the embedded lookup (static `&str` map)
/// and the filesystem lookup.
pub(crate) fn parse_pack(
    id_hint: &str,
    manifest_toml: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<LoadedPack, LoadError> {
    let manifest: PackManifest =
        toml::from_str(manifest_toml).map_err(|e| LoadError::ManifestParse {
            id: id_hint.to_string(),
            source: e,
        })?;

    if manifest.pack.id.is_empty() {
        return Err(LoadError::EmptyId {
            id: id_hint.to_string(),
        });
    }

    let mut templates = BTreeMap::new();
    for doc in &manifest.documents {
        let text = lookup(&doc.template).ok_or_else(|| LoadError::MissingTemplate {
            id: manifest.pack.id.clone(),
            code: doc.code.clone(),
            path: doc.template.clone(),
        })?;
        templates.insert(doc.code.clone(), text);
    }

    Ok(LoadedPack {
        manifest,
        templates,
    })
}

/// Built-in packs (embedded at compile time).
macro_rules! embed_pack {
    ($dir:literal, $($code:literal),+ $(,)?) => {{
        let manifest_toml = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packs/",
            $dir,
            "/pack.toml"
        ));
        let mut map: BTreeMap<&'static str, &'static str> = BTreeMap::new();
        $(
            map.insert(
                concat!("templates/", $code, ".edi"),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../packs/",
                    $dir,
                    "/templates/",
                    $code,
                    ".edi"
                )),
            );
        )+
        (manifest_toml, map)
    }};
}

/// Load all three built-in packs.
pub fn load_built_in() -> Vec<LoadedPack> {
    // Each tuple is (manifest_toml, template_map).
    let retail = embed_pack!(
        "retail", "840", "843", "850", "855", "860", "865", "856", "810", "820"
    );
    let healthcare = embed_pack!("healthcare", "837", "275", "277", "835", "820");
    let supply_chain = embed_pack!("supply_chain", "204", "990", "211", "214", "210", "820");

    [retail, healthcare, supply_chain]
        .into_iter()
        .map(|(toml_str, map): (&str, BTreeMap<&str, &str>)| {
            parse_pack("built-in", toml_str, |path| {
                map.get(path).map(|s| s.to_string())
            })
            // Built-in packs are validated in tests; a panic here means a
            // broken build, not a user error.
            .expect("built-in pack is malformed")
        })
        .collect()
}

/// Load any packs found in `dir`. Each sub-directory containing a `pack.toml`
/// is treated as a pack. Template paths inside the manifest are resolved
/// relative to the pack sub-directory.
///
/// Errors for individual packs are collected and returned; other packs are
/// still loaded. An empty directory returns an empty vec.
pub fn load_from_dir(dir: &Path) -> (Vec<LoadedPack>, Vec<LoadError>) {
    let mut packs = Vec::new();
    let mut errors = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(source) => {
            errors.push(LoadError::Io {
                id: dir.display().to_string(),
                source,
            });
            return (packs, errors);
        }
    };

    for entry in entries.flatten() {
        let pack_dir = entry.path();
        if !pack_dir.is_dir() {
            continue;
        }
        let manifest_path = pack_dir.join("pack.toml");
        if !manifest_path.exists() {
            continue;
        }
        let id_hint = pack_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let manifest_text = match std::fs::read_to_string(&manifest_path) {
            Ok(t) => t,
            Err(source) => {
                errors.push(LoadError::Io {
                    id: id_hint,
                    source,
                });
                continue;
            }
        };

        let pack_dir_clone = pack_dir.clone();
        match parse_pack(&id_hint, &manifest_text, |rel| {
            let abs = pack_dir_clone.join(rel);
            std::fs::read_to_string(&abs).ok()
        }) {
            Ok(p) => packs.push(p),
            Err(e) => errors.push(e),
        }
    }

    (packs, errors)
}
