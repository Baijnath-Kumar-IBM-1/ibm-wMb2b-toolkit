//! Authenticated access to the B2B API: MCSP token handling and a small
//! request helper shared by provisioning, documents and the connection test.
//!
//! The API key is read from the [`SecretStore`] only when a token is needed.
//! Tokens are cached in memory (never stored, logged or returned) and are
//! re-acquired when they are about to expire or when the API answers 401.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::environments::Environment;
use crate::error::{Error, Result};
use crate::http::{HttpClient, HttpRequest, HttpResponse, Method};
use crate::ids::EnvironmentId;
use crate::preferences::PreferenceRepository;
use crate::secrets::SecretStore;

/// Tokens are replaced this long before they expire.
const EXPIRY_MARGIN_SECS: i64 = 60;
/// Used when the token carries no readable `exp` claim. A 401 triggers an
/// earlier refresh anyway.
const FALLBACK_TTL_SECS: i64 = 300;
const EXCERPT_CHARS: usize = 300;

struct CachedToken {
    token: SecretString,
    /// Which key/host/instance the token was issued for; a changed
    /// environment must not reuse it.
    fingerprint: String,
    expires_at: OffsetDateTime,
}

/// Shared B2B access. Cheap to share behind an `Arc`.
pub struct B2bApi {
    http: Arc<dyn HttpClient>,
    secrets: Arc<dyn SecretStore>,
    prefs: Arc<dyn PreferenceRepository>,
    clock: Arc<dyn Clock>,
    tokens: Mutex<HashMap<EnvironmentId, CachedToken>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl B2bApi {
    pub fn new(
        http: Arc<dyn HttpClient>,
        secrets: Arc<dyn SecretStore>,
        prefs: Arc<dyn PreferenceRepository>,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            http,
            secrets,
            prefs,
            clock,
            tokens: Mutex::new(HashMap::new()),
        })
    }

    /// Drops the cached token of an environment (for example after its
    /// settings changed).
    pub fn forget_token(&self, environment_id: &EnvironmentId) {
        lock(&self.tokens).remove(environment_id);
    }

    fn timeout(&self) -> Result<Duration> {
        Ok(Duration::from_secs(u64::from(
            self.prefs.get()?.request_timeout_secs,
        )))
    }

    fn cached(
        &self,
        environment_id: &EnvironmentId,
        fingerprint: &str,
        now: OffsetDateTime,
    ) -> Option<SecretString> {
        let tokens = lock(&self.tokens);
        let c = tokens.get(environment_id)?;
        (c.fingerprint == fingerprint
            && c.expires_at > now + time::Duration::seconds(EXPIRY_MARGIN_SECS))
        .then(|| c.token.clone())
    }

    /// Returns a valid token, acquiring a new one when `force` is set, when
    /// none is cached, or when the cached one is about to expire.
    pub(crate) async fn token(&self, env: &Environment, force: bool) -> Result<SecretString> {
        let key_id = env.api_key_secret.as_ref().ok_or_else(|| {
            Error::Conflict("this environment has no API key; add one in its settings".into())
        })?;
        let fingerprint = format!("{key_id}|{}|{}", env.iam_host, env.instance_id);
        let now = self.clock.now();
        if !force && let Some(token) = self.cached(&env.id, &fingerprint, now) {
            return Ok(token);
        }

        let api_key = self.secrets.get(key_id)?.ok_or_else(|| {
            Error::Conflict("the stored API key is missing; enter it again in the settings".into())
        })?;
        let url = format!(
            "https://{}/api/2.0/services/{}/apikeys/token?includeBuiltinActions=false&includeCustomActions=false&includeRoles=true&prefixRolesWithDefinitionScope=false",
            env.iam_host, env.instance_id
        );
        let request = HttpRequest::new(Method::Post, url, self.timeout()?)
            .header("Mcsp-ApiKey", api_key.expose_secret())
            .header("Content-Type", "application/json")
            .ca_bundle(env.ca_bundle_path.clone());
        let response = self.http.send(request).await?;
        if !response.is_success() {
            return Err(Error::Http(format!(
                "token request failed (HTTP {}): {}",
                response.status,
                excerpt(&response.body, &[api_key.expose_secret()])
            )));
        }
        // The body of a success response holds the token, so it is never
        // included in error text.
        let token = serde_json::from_slice::<Value>(&response.body)
            .ok()
            .and_then(|v| v.get("token")?.as_str().map(str::to_string))
            .filter(|t| !t.is_empty())
            .ok_or_else(|| Error::Http("token response did not contain a token".into()))?;

        let expires_at =
            jwt_expiry(&token).unwrap_or_else(|| now + time::Duration::seconds(FALLBACK_TTL_SECS));
        let token = SecretString::from(token);
        lock(&self.tokens).insert(
            env.id.clone(),
            CachedToken {
                token: token.clone(),
                fingerprint,
                expires_at,
            },
        );
        Ok(token)
    }

    /// Sends one authenticated request to the B2B host. On a 401 the token is
    /// replaced once and the request is repeated. Any other status is
    /// returned to the caller.
    pub(crate) async fn call(
        &self,
        env: &Environment,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<HttpResponse> {
        let payload = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|_| Error::Http("could not encode the request body".into()))?;
        let first = self
            .call_once(env, method, path, payload.as_deref(), false)
            .await?;
        if first.status != 401 {
            return Ok(first);
        }
        self.call_once(env, method, path, payload.as_deref(), true)
            .await
    }

    async fn call_once(
        &self,
        env: &Environment,
        method: Method,
        path: &str,
        payload: Option<&[u8]>,
        fresh_token: bool,
    ) -> Result<HttpResponse> {
        let token = self.token(env, fresh_token).await?;
        let mut request = HttpRequest::new(
            method,
            format!("https://{}{path}", env.b2b_host),
            self.timeout()?,
        )
        .header("Authorization", format!("Bearer {}", token.expose_secret()))
        .ca_bundle(env.ca_bundle_path.clone());
        if let Some(p) = payload {
            request = request
                .header("Content-Type", "application/json")
                .body(p.to_vec());
        }
        self.http.send(request).await
    }
}

/// Parses a success response into JSON (`Null` when the body is empty or not
/// JSON) or turns any other status into an error with a redacted excerpt.
pub(crate) fn expect_json(response: &HttpResponse, what: &str, redact: &[&str]) -> Result<Value> {
    if response.is_success() {
        Ok(serde_json::from_slice(&response.body).unwrap_or(Value::Null))
    } else {
        Err(status_error(response, what, redact))
    }
}

pub(crate) fn status_error(response: &HttpResponse, what: &str, redact: &[&str]) -> Error {
    Error::Http(format!(
        "{what} failed (HTTP {}): {}",
        response.status,
        excerpt(&response.body, redact)
    ))
}

/// A string or number at a JSON pointer, as text (like `jq -r`). Empty
/// strings count as missing.
pub(crate) fn json_text(value: &Value, pointer: &str) -> Option<String> {
    match value.pointer(pointer)? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Identifiers that are placed into URL paths must be plain tokens.
pub(crate) fn path_segment(value: &str) -> Result<&str> {
    let ok = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if ok {
        Ok(value)
    } else {
        Err(Error::Http(
            "an identifier from the B2B API has an unexpected format".into(),
        ))
    }
}

/// Short single-line excerpt of a response body for error messages. Known
/// secret values are replaced first, so a server that echoes a request does
/// not leak them.
pub(crate) fn excerpt(body: &[u8], redact: &[&str]) -> String {
    let mut text = String::from_utf8_lossy(body).into_owned();
    for secret in redact.iter().filter(|s| !s.is_empty()) {
        text = text.replace(secret, "***");
    }
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.trim();
    if flat.chars().count() > EXCERPT_CHARS {
        let cut: String = flat.chars().take(EXCERPT_CHARS).collect();
        format!("{cut}...")
    } else {
        flat.to_string()
    }
}

/// Expiry from the `exp` claim of a JWT, if the token is one. The signature
/// is not checked; the value only schedules a refresh.
fn jwt_expiry(token: &str) -> Option<OffsetDateTime> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let exp = serde_json::from_slice::<Value>(&bytes)
        .ok()?
        .get("exp")?
        .as_i64()?;
    OffsetDateTime::from_unix_timestamp(exp).ok()
}

#[cfg(test)]
pub(crate) fn test_jwt(exp: i64) -> String {
    let enc = |s: &str| URL_SAFE_NO_PAD.encode(s.as_bytes());
    format!(
        "{}.{}.{}",
        enc(r#"{"alg":"none"}"#),
        enc(&format!(r#"{{"exp":{exp}}}"#)),
        enc("sig")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_redacts_flattens_and_truncates() {
        let body = b"bad\npassword hunter2 here";
        assert_eq!(excerpt(body, &["hunter2"]), "bad password *** here");
        let long = "x".repeat(1000);
        let e = excerpt(long.as_bytes(), &[]);
        assert!(e.len() < 320 && e.ends_with("..."));
    }

    #[test]
    fn json_text_handles_strings_numbers_and_missing() {
        let v: Value =
            serde_json::json!({"id": 7, "s": "a", "e": "", "n": null, "o": {"uri": "u"}});
        assert_eq!(json_text(&v, "/id").as_deref(), Some("7"));
        assert_eq!(json_text(&v, "/s").as_deref(), Some("a"));
        assert_eq!(json_text(&v, "/o/uri").as_deref(), Some("u"));
        for p in ["/e", "/n", "/nope"] {
            assert!(json_text(&v, p).is_none(), "{p}");
        }
    }

    #[test]
    fn path_segments_reject_separators() {
        assert!(path_segment("abc-1.2_x").is_ok());
        for bad in ["", "a/b", "a b", "..%2f", "a?b", "a#b"] {
            assert!(path_segment(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn jwt_expiry_reads_exp_and_ignores_other_tokens() {
        let t = test_jwt(2_000_000_000);
        assert_eq!(jwt_expiry(&t).unwrap().unix_timestamp(), 2_000_000_000);
        assert!(jwt_expiry("opaque-token").is_none());
        assert!(jwt_expiry("a.b.c").is_none());
    }
}
