//! Standalone server: serves the API and the UI on 127.0.0.1.
//!
//! The URL with the access token in the fragment (`#token=...`) is printed
//! to the terminal, so the token never travels in a request line or a log.

use std::path::PathBuf;
use std::process::ExitCode;

use secrecy::{ExposeSecret, SecretString};
use trace_server::{ServerConfig, UiSource, default_data_dir, serve};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "\
Usage: trace-server [options]

  --data-dir <dir>      Where the database and secrets live (default: per-user dir)
  --port <n>            Port on 127.0.0.1 (default: a free port)
  --ui-dir <dir>        Serve a built UI from this directory (default: ui/dist if present)
  --allow-origin <url>  Also accept this browser origin (dev server), repeatable
  --token <value>       Use this API token (at least 32 characters; development only)

Environment:
  TRACE_SECRETS_PASSPHRASE  Passphrase for the encrypted secrets file when no
                            OS keychain is available
  TRACE_API_TOKEN           Same as --token
  RUST_LOG                  Log filter, for example info
";

fn parse_args() -> Result<Option<ServerConfig>, String> {
    let mut data_dir: Option<PathBuf> = None;
    let mut port = 0_u16;
    let mut ui_dir: Option<PathBuf> = None;
    let mut origins = Vec::new();
    let mut token = std::env::var("TRACE_API_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--data-dir" => data_dir = Some(PathBuf::from(value("--data-dir")?)),
            "--port" => {
                port = value("--port")?
                    .parse()
                    .map_err(|_| "--port must be a number from 0 to 65535".to_string())?;
            }
            "--ui-dir" => ui_dir = Some(PathBuf::from(value("--ui-dir")?)),
            "--allow-origin" => origins.push(value("--allow-origin")?),
            "--token" => token = Some(value("--token")?),
            other => return Err(format!("unknown option: {other}")),
        }
    }

    let data_dir = data_dir
        .or_else(default_data_dir)
        .ok_or("cannot find a data directory; pass --data-dir")?;
    let mut config = ServerConfig::new(data_dir);
    config.port = port;
    config.extra_origins = origins;
    config.token = token.map(SecretString::from);
    config.passphrase = std::env::var("TRACE_SECRETS_PASSPHRASE")
        .ok()
        .filter(|p| !p.is_empty())
        .map(SecretString::from);
    config.ui = match ui_dir {
        Some(dir) => UiSource::Dir(dir),
        None => default_ui(),
    };
    Ok(Some(config))
}

fn default_ui() -> UiSource {
    #[cfg(feature = "embed-ui")]
    {
        UiSource::Embedded
    }
    #[cfg(not(feature = "embed-ui"))]
    {
        let dist = PathBuf::from("ui/dist");
        if dist.join("index.html").is_file() {
            UiSource::Dir(dist)
        } else {
            UiSource::None
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();

    let config = match parse_args() {
        Ok(Some(c)) => c,
        Ok(None) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let has_ui = !matches!(config.ui, UiSource::None);

    let mut handle = match serve(config).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("API:  {}/api", handle.base_url());
    if has_ui {
        println!(
            "Open: {}/#token={}",
            handle.base_url(),
            handle.token().expose_secret()
        );
    } else {
        println!(
            "No UI is being served (build it with `pnpm build` in ui/, or use the Vite dev server)."
        );
        println!("Token: {}", handle.token().expose_secret());
    }
    println!("Press Ctrl+C to stop.");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        () = handle.wait() => {}
    }
    handle.shutdown().await;
    ExitCode::SUCCESS
}
