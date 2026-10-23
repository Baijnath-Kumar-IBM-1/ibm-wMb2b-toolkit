//! Static UI hosting with single-page-app fallback.

use axum::Router;
use axum::http::StatusCode;
use tower_http::services::{ServeDir, ServeFile};

use crate::config::UiSource;
use crate::routers::Shared;

/// Adds the UI as the router's fallback. Unknown `/api/...` paths never
/// reach it: they match the `/api` router first and answer 404 as JSON.
pub fn attach(router: Router<Shared>, source: &UiSource) -> Router<Shared> {
    match source {
        UiSource::None => router,
        UiSource::Dir(dir) => {
            let index = dir.join("index.html");
            router.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        #[cfg(feature = "embed-ui")]
        UiSource::Embedded => router.fallback(embedded::serve),
    }
}

pub async fn api_not_found() -> crate::error::AppError {
    crate::error::AppError::new(StatusCode::NOT_FOUND, "not found")
}

#[cfg(feature = "embed-ui")]
mod embedded {
    use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
    use axum::http::{StatusCode, Uri};
    use axum::response::{IntoResponse, Response};
    use rust_embed::RustEmbed;

    #[derive(RustEmbed)]
    #[folder = "../../ui/dist"]
    struct Assets;

    pub async fn serve(uri: Uri) -> Response {
        let path = uri.path().trim_start_matches('/');
        let (file, name) = match Assets::get(path) {
            Some(f) if !path.is_empty() => (f, path),
            // SPA fallback: unknown paths get the app shell.
            _ => match Assets::get("index.html") {
                Some(f) => (f, "index.html"),
                None => return StatusCode::NOT_FOUND.into_response(),
            },
        };
        let mime = mime_for(name);
        // Vite names assets by content hash, so they can be cached for good.
        let cache = if name.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        (
            [(CONTENT_TYPE, mime), (CACHE_CONTROL, cache)],
            file.data.into_owned(),
        )
            .into_response()
    }

    fn mime_for(name: &str) -> &'static str {
        match name.rsplit('.').next().unwrap_or("") {
            "html" => "text/html; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "json" => "application/json",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "ico" => "image/x-icon",
            "woff2" => "font/woff2",
            "woff" => "font/woff",
            _ => "application/octet-stream",
        }
    }
}
