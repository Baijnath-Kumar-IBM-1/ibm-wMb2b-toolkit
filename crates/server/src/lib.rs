//! Toolkit v2 local HTTP API and UI hosting.
//!
//! The server binds `127.0.0.1` only, requires a per-launch bearer token on
//! every `/api` route, checks `Host` and `Origin`, and returns generic error
//! bodies. The same [`serve`] function backs the standalone `trace-server`
//! binary and the Tauri desktop shell.

pub mod auth;
pub mod config;
pub mod error;
pub mod extract;
pub mod routers;
pub mod state;
pub mod ui;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::http::{Method, header};
use axum::middleware;
use secrecy::{ExposeSecret, SecretString};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tower_http::cors::{AllowOrigin, CorsLayer};
use trace_core::packs::Catalog;
use trace_http::ReqwestClient;
use trace_storage::{Database, open_secret_store};
use tracing::{info, warn};

pub use config::{ServerConfig, UiSource, default_data_dir};
pub use state::{AppState, Parts};

const DB_FILE: &str = "trace.db";
const MIN_TOKEN_LEN: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("cannot open the data store: {0}")]
    Storage(String),
    #[error("cannot listen on 127.0.0.1: {0}")]
    Bind(#[source] std::io::Error),
    #[error("no secure random source: {0}")]
    Random(String),
}

/// A running server.
pub struct ServerHandle {
    addr: SocketAddr,
    token: SecretString,
    state: Arc<AppState>,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl ServerHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:<port>`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The API token. Treat as a secret: hand it to the UI, never log it.
    pub fn token(&self) -> &SecretString {
        &self.token
    }

    pub fn state(&self) -> &Arc<AppState> {
        &self.state
    }

    /// Stops accepting requests, ends event streams and waits for the server.
    pub async fn shutdown(mut self) {
        let _ = self.state.shutdown.send(true);
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = self.task.await;
    }

    /// Resolves when the server task ends.
    pub async fn wait(&mut self) {
        let _ = (&mut self.task).await;
    }
}

/// Opens the stores and starts serving on `127.0.0.1:<port>`.
pub async fn serve(config: ServerConfig) -> Result<ServerHandle, ServeError> {
    check_config(&config)?;
    let parts = open_parts(&config).await?;
    serve_with(config, parts).await
}

fn check_config(config: &ServerConfig) -> Result<(), ServeError> {
    for origin in &config.extra_origins {
        if !auth::valid_origin(origin) {
            return Err(ServeError::Config(
                "extra origins must look like http://host:port".into(),
            ));
        }
    }
    match &config.token {
        Some(t) if t.expose_secret().len() < MIN_TOKEN_LEN => Err(ServeError::Config(format!(
            "the token must be at least {MIN_TOKEN_LEN} characters"
        ))),
        _ => Ok(()),
    }
}

/// Starts serving with ready-made collaborators (tests, embedding).
pub async fn serve_with(config: ServerConfig, parts: Parts) -> Result<ServerHandle, ServeError> {
    check_config(&config)?;
    let token = match &config.token {
        Some(t) => t.clone(),
        None => auth::generate_token().map_err(|e| ServeError::Random(e.to_string()))?,
    };
    let state = AppState::build(parts);

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.port))
        .await
        .map_err(ServeError::Bind)?;
    let addr = listener.local_addr().map_err(ServeError::Bind)?;

    let guard = Arc::new(auth::Guard::new(
        token.clone(),
        addr.port(),
        &config.extra_origins,
    ));
    let app = build_app(&state, guard, &config);

    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await;
        if let Err(e) = result {
            warn!(error = %e, "server stopped with an error");
        }
    });
    info!(%addr, "listening");
    Ok(ServerHandle {
        addr,
        token,
        state,
        stop: Some(stop),
        task,
    })
}

async fn open_parts(config: &ServerConfig) -> Result<Parts, ServeError> {
    let data_dir = config.data_dir.clone();
    let policy = config.secret_policy;
    let passphrase = config.passphrase.clone();
    let packs_dir = config
        .user_packs_dir
        .clone()
        .unwrap_or_else(|| data_dir.join("packs"));

    tokio::task::spawn_blocking(move || {
        let db = Database::open(&data_dir.join(DB_FILE))
            .map_err(|e| ServeError::Storage(e.to_string()))?;
        let (secrets, backend) = open_secret_store(&data_dir, policy, passphrase.as_ref())
            .map_err(|e| ServeError::Storage(e.to_string()))?;
        let (catalog, load_errors) = Catalog::with_user_dir(&packs_dir);
        for err in load_errors {
            warn!(error = %err, "a user pack was skipped");
        }
        Ok(Parts::new(
            Arc::new(db),
            secrets,
            backend,
            Arc::new(ReqwestClient::new()),
            catalog,
        ))
    })
    .await
    .map_err(|e| ServeError::Storage(e.to_string()))?
}

/// Builds the full router. Public so tests can drive it without a socket.
pub fn build_app(state: &Arc<AppState>, guard: Arc<auth::Guard>, config: &ServerConfig) -> Router {
    let mut api = routers::api()
        .fallback(ui::api_not_found)
        .layer(middleware::from_fn_with_state(
            guard.clone(),
            auth::api_guard,
        ));

    if !config.extra_origins.is_empty() {
        // Only for dev servers on another origin; the same-origin UI needs no CORS.
        let allowed: Vec<_> = config
            .extra_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        api = api.layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(allowed))
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
                .max_age(std::time::Duration::from_secs(600)),
        );
    }

    let app = Router::new().nest("/api", api);
    ui::attach(app, &config.ui)
        .layer(middleware::from_fn(auth::security_headers))
        .layer(middleware::from_fn_with_state(guard, auth::host_guard))
        .with_state(state.clone())
}
