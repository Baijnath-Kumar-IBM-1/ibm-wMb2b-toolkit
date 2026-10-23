//! Server configuration.

use std::path::PathBuf;

use secrecy::SecretString;
use trace_storage::SecretBackendPolicy;

/// Where the UI comes from.
#[derive(Debug, Clone, Default)]
pub enum UiSource {
    /// No UI: only `/api` is served (the Vite dev server serves the UI).
    #[default]
    None,
    /// A built UI directory (`ui/dist`), with `index.html` as SPA fallback.
    Dir(PathBuf),
    /// The UI compiled into the binary (`embed-ui` feature).
    #[cfg(feature = "embed-ui")]
    Embedded,
}

#[derive(Debug)]
pub struct ServerConfig {
    /// Holds the database and, with the file backend, the encrypted secrets.
    pub data_dir: PathBuf,
    /// Loopback port; 0 picks a free one.
    pub port: u16,
    /// API token. A random 256-bit token is generated when `None`. A given
    /// token must be at least 32 characters (development use).
    pub token: Option<SecretString>,
    pub ui: UiSource,
    pub secret_policy: SecretBackendPolicy,
    /// Passphrase for the encrypted-file secret backend.
    pub passphrase: Option<SecretString>,
    /// Extra browser origins allowed besides the server's own (for example
    /// the Vite dev server). Must be exact `http(s)://host[:port]` values.
    pub extra_origins: Vec<String>,
    /// Directory with user packs; defaults to `<data_dir>/packs`.
    pub user_packs_dir: Option<PathBuf>,
}

impl ServerConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            port: 0,
            token: None,
            ui: UiSource::None,
            secret_policy: SecretBackendPolicy::Auto,
            passphrase: None,
            extra_origins: Vec::new(),
            user_packs_dir: None,
        }
    }
}

/// Per-user data directory without extra dependencies.
pub fn default_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Trace"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Trace"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|d| d.join("trace"))
    }
}
