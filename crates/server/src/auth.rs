//! Request guards for the local API.
//!
//! - **Host**: only the loopback names with the server's own port are
//!   accepted, which stops DNS-rebinding attacks.
//! - **Origin**: a browser request must come from the server's own origin or
//!   an explicitly configured one. Requests without `Origin` (non-browser
//!   clients) still need the token.
//! - **Token**: `Authorization: Bearer <token>`, compared in constant time.
//!   The token is never accepted in a URL, so it cannot end up in logs.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, HOST, ORIGIN};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use secrecy::{ExposeSecret, SecretString};

use crate::error::AppError;

pub struct Guard {
    token: SecretString,
    hosts: Vec<String>,
    origins: Vec<String>,
}

impl Guard {
    pub fn new(token: SecretString, port: u16, extra_origins: &[String]) -> Self {
        let hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        let mut origins: Vec<String> = hosts.iter().map(|h| format!("http://{h}")).collect();
        origins.extend(extra_origins.iter().cloned());
        Self {
            token,
            hosts,
            origins,
        }
    }

    fn host_ok(&self, value: Option<&HeaderValue>) -> bool {
        value
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| self.hosts.iter().any(|h| h.eq_ignore_ascii_case(v)))
    }

    fn origin_ok(&self, value: Option<&HeaderValue>) -> bool {
        match value {
            None => true,
            Some(v) => v
                .to_str()
                .is_ok_and(|v| self.origins.iter().any(|o| o.eq_ignore_ascii_case(v))),
        }
    }

    fn token_ok(&self, value: Option<&HeaderValue>) -> bool {
        let Some(presented) = value
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            return false;
        };
        constant_time_eq(presented.as_bytes(), self.token.expose_secret().as_bytes())
    }

    pub fn origins(&self) -> &[String] {
        &self.origins
    }
}

/// Compares without stopping at the first difference. Lengths are not
/// secret (the token has a fixed size).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0_u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Applies to every route (API and UI).
pub async fn host_guard(State(guard): State<Arc<Guard>>, req: Request, next: Next) -> Response {
    if !guard.host_ok(req.headers().get(HOST)) {
        return AppError::new(StatusCode::FORBIDDEN, "host not allowed").into_response();
    }
    next.run(req).await
}

/// Applies to `/api` routes.
pub async fn api_guard(State(guard): State<Arc<Guard>>, req: Request, next: Next) -> Response {
    if !guard.origin_ok(req.headers().get(ORIGIN)) {
        return AppError::new(StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    if !guard.token_ok(req.headers().get(AUTHORIZATION)) {
        return AppError::new(StatusCode::UNAUTHORIZED, "missing or invalid token").into_response();
    }
    next.run(req).await
}

/// Response headers for everything the server sends.
pub async fn security_headers(req: Request, next: Next) -> Response {
    let is_api = req.uri().path().starts_with("/api");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
             connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        ),
    );
    if is_api {
        h.insert("cache-control", HeaderValue::from_static("no-store"));
    }
    res
}

/// Checks an extra origin: `http(s)://host[:port]` with nothing else.
pub fn valid_origin(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    !rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
}

/// 256-bit random token, URL-safe base64.
pub fn generate_token() -> Result<SecretString, getrandom::Error> {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let mut raw = [0_u8; 32];
    getrandom::fill(&mut raw)?;
    Ok(SecretString::from(URL_SAFE_NO_PAD.encode(raw)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn origins_must_be_bare() {
        assert!(valid_origin("http://localhost:5173"));
        assert!(valid_origin("https://example.test"));
        assert!(!valid_origin("localhost:5173"));
        assert!(!valid_origin("http://x/path"));
        assert!(!valid_origin("http://"));
        assert!(!valid_origin("javascript:alert(1)"));
    }

    #[test]
    fn generated_tokens_are_long_and_distinct() {
        let a = generate_token().unwrap();
        let b = generate_token().unwrap();
        assert!(a.expose_secret().len() >= 43);
        assert_ne!(a.expose_secret(), b.expose_secret());
    }
}
