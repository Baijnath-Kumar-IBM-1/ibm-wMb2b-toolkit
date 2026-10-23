//! Real [`HttpClient`]: `reqwest` with rustls.
//!
//! Security properties (all enforced here, none configurable off):
//!
//! - Only `https://` URLs are accepted.
//! - TLS certificates and host names are always validated, TLS 1.2 or newer.
//!   There is no "insecure" switch. A private CA is trusted by pointing
//!   [`HttpRequest::ca_bundle`] at a PEM file; the system roots stay trusted.
//! - Redirects are not followed, so credentials are never re-sent elsewhere.
//! - Every request has an explicit total timeout plus a connect timeout.
//! - Response bodies are capped at [`MAX_RESPONSE_BYTES`].
//! - Error text never contains request headers or bodies.

use std::collections::HashMap;
use std::error::Error as StdError;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use reqwest::{Certificate, Client, Method as ReqMethod, redirect, tls};
use trace_core::Error;
use trace_core::Result;
use trace_core::http::{BoxFuture, HttpClient, HttpRequest, HttpResponse, Method};

/// Largest response body that is read; anything bigger is an error.
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// Largest CA bundle file that is read.
const MAX_CA_BUNDLE_BYTES: u64 = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Cache key: no bundle, or a bundle path with the file's modification time
/// so an edited bundle is picked up without restarting.
type ClientKey = Option<(String, SystemTime)>;

/// reqwest-backed implementation of the [`HttpClient`] port.
#[derive(Default)]
pub struct ReqwestClient {
    clients: Mutex<HashMap<ClientKey, Client>>,
}

impl ReqwestClient {
    pub fn new() -> Self {
        Self::default()
    }

    fn client_for(&self, ca_bundle: Option<&str>) -> Result<Client> {
        let (key, pem) = match ca_bundle {
            None => (None, None),
            Some(path) => {
                let meta = std::fs::metadata(path)
                    .map_err(|e| Error::Http(format!("cannot read CA bundle: {}", e.kind())))?;
                if !meta.is_file() || meta.len() > MAX_CA_BUNDLE_BYTES {
                    return Err(Error::Http(
                        "CA bundle must be a regular file of at most 1 MiB".into(),
                    ));
                }
                let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                (Some((path.to_string(), modified)), Some(path))
            }
        };

        let mut cache = self
            .clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(client) = cache.get(&key) {
            return Ok(client.clone());
        }

        let mut builder = Client::builder()
            .https_only(true)
            .redirect(redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .tls_version_min(tls::Version::TLS_1_2)
            .user_agent(concat!("ibm-b2b-toolkit-trace/", env!("CARGO_PKG_VERSION")));
        if let Some(path) = pem {
            let bytes = std::fs::read(path)
                .map_err(|e| Error::Http(format!("cannot read CA bundle: {}", e.kind())))?;
            let certs = Certificate::from_pem_bundle(&bytes)
                .map_err(|_| Error::Http("CA bundle is not a valid PEM certificate file".into()))?;
            if certs.is_empty() {
                return Err(Error::Http("CA bundle contains no certificates".into()));
            }
            builder = builder.tls_certs_merge(certs);
        }
        let client = builder.build().map_err(|e| Error::Http(describe(&e)))?;
        // Entries for older versions of an edited bundle are not reused.
        if cache.len() >= 16 {
            cache.clear();
        }
        cache.insert(key, client.clone());
        Ok(client)
    }
}

impl HttpClient for ReqwestClient {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse>> {
        Box::pin(async move {
            if !request.url.starts_with("https://") {
                return Err(Error::Http("only https:// URLs are allowed".into()));
            }
            let client = self.client_for(request.ca_bundle.as_deref())?;
            let method = match request.method {
                Method::Get => ReqMethod::GET,
                Method::Post => ReqMethod::POST,
                Method::Put => ReqMethod::PUT,
                Method::Delete => ReqMethod::DELETE,
            };
            let mut builder = client
                .request(method, &request.url)
                .timeout(request.timeout);
            for (name, value) in &request.headers {
                let mut v = reqwest::header::HeaderValue::from_str(value.expose())
                    .map_err(|_| Error::Http(format!("invalid value for header '{name}'")))?;
                v.set_sensitive(true);
                builder = builder.header(name.as_str(), v);
            }
            if let Some(body) = request.body {
                builder = builder.body(body);
            }

            let mut response = builder
                .send()
                .await
                .map_err(|e| Error::Http(describe(&e)))?;
            let status = response.status().as_u16();
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| Error::Http(describe(&e)))?
            {
                if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                    return Err(Error::Http("response body is too large".into()));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(HttpResponse { status, body })
        })
    }
}

/// Short, secret-free description of a transport failure. The URL is left
/// out; the cause chain is kept because it names TLS problems such as an
/// unknown issuer or a host name mismatch.
fn describe(e: &reqwest::Error) -> String {
    let kind = if e.is_timeout() {
        "request timed out"
    } else if e.is_connect() {
        "could not connect"
    } else if e.is_builder() {
        "invalid request"
    } else if e.is_body() || e.is_decode() {
        "could not read response"
    } else {
        "request failed"
    };
    let mut text = kind.to_string();
    let mut source = e.source();
    let mut depth = 0;
    while let Some(cause) = source {
        if depth >= 4 {
            break;
        }
        let part = cause.to_string();
        if !part.is_empty() && !text.contains(&part) {
            text.push_str(": ");
            text.push_str(&part);
        }
        source = cause.source();
        depth += 1;
    }
    text
}

#[cfg(test)]
mod tests;
