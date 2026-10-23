//! Toolkit v2 desktop app.
//!
//! The window loads the same UI the standalone server hosts. The server runs
//! inside this process on `127.0.0.1` with a random port and a random
//! per-launch token; the token is handed to the page as a script variable
//! and never written to disk or logged.

use std::path::PathBuf;
use std::sync::Mutex;

use secrecy::ExposeSecret;
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use trace_server::{ServerConfig, ServerHandle, UiSource, serve};

/// Holds the running server so it can be stopped on exit.
struct Server(Mutex<Option<ServerHandle>>);

fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("cannot find the app data directory: {e}"))
}

/// A script that sets the token before any page script runs. The token is
/// URL-safe base64, but is still encoded as a JSON string so no value can
/// break out of the literal.
fn token_script(token: &str) -> String {
    let literal = serde_json::to_string(token).unwrap_or_else(|_| "\"\"".to_string());
    format!("window.__TRACE_TOKEN__ = {literal};")
}

/// True only for the local server itself. The scheme, host and port are
/// compared as parsed values: a string prefix check would accept
/// `http://127.0.0.1:8799@other.example/`.
fn is_local(url: &tauri::Url, origin: &tauri::Url) -> bool {
    url.scheme() == origin.scheme()
        && url.host_str() == origin.host_str()
        && url.port_or_known_default() == origin.port_or_known_default()
        && url.username().is_empty()
        && url.password().is_none()
}

fn start(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(data_dir(app)?);
    config.ui = UiSource::Embedded;

    let handle = tauri::async_runtime::block_on(serve(config))?;
    let base = handle.base_url();
    let script = token_script(handle.token().expose_secret());
    let origin: tauri::Url = base.parse()?;

    WebviewWindowBuilder::new(app, "main", WebviewUrl::External(base.parse()?))
        .title("Trace")
        .inner_size(1280.0, 800.0)
        .min_inner_size(900.0, 600.0)
        .initialization_script(script)
        // The UI has no external links. Anything that is not the local server
        // is refused, so a stray link can never load remote content into a
        // window that holds the API token.
        .on_navigation(move |url| is_local(url, &origin))
        .build()?;

    app.manage(Server(Mutex::new(Some(handle))));
    Ok(())
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let built = tauri::Builder::default()
        .setup(|app| {
            start(app.handle()).map_err(|e| {
                // Details go to the log only.
                tracing::error!(error = %e, "could not start the local server");
                e
            })
        })
        .build(tauri::generate_context!());
    let app = match built {
        Ok(app) => app,
        Err(_) => {
            eprintln!(
                "Trace could not start. Run it from a terminal to see the reason in the log."
            );
            std::process::exit(1);
        }
    };

    app.run(|handle, event| {
        if let RunEvent::Exit = event
            && let Some(server) = handle.try_state::<Server>()
            && let Ok(mut slot) = server.0.lock()
            && let Some(running) = slot.take()
        {
            tauri::async_runtime::block_on(running.shutdown());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> tauri::Url {
        s.parse().unwrap()
    }

    #[test]
    fn only_the_local_server_is_navigable() {
        let origin = url("http://127.0.0.1:8799");
        assert!(is_local(&url("http://127.0.0.1:8799/"), &origin));
        assert!(is_local(&url("http://127.0.0.1:8799/#token=x"), &origin));
        assert!(!is_local(&url("http://127.0.0.1:8798/"), &origin));
        assert!(!is_local(&url("https://127.0.0.1:8799/"), &origin));
        assert!(!is_local(&url("http://localhost:8799/"), &origin));
        assert!(!is_local(
            &url("http://127.0.0.1:8799@evil.example/"),
            &origin
        ));
        assert!(!is_local(&url("http://evil.example/"), &origin));
    }

    #[test]
    fn token_script_is_a_single_string_assignment() {
        assert_eq!(
            token_script("abc-_9"),
            "window.__TRACE_TOKEN__ = \"abc-_9\";"
        );
        assert!(token_script("a\"; alert(1); \"").contains("\\\""));
    }
}
