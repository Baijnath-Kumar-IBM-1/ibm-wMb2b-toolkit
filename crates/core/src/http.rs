//! HTTP client port.
//!
//! Services talk to the B2B API and MCSP IAM only through [`HttpClient`], so
//! they can be tested with [`crate::testing::FakeHttp`]. The real client is
//! the `trace-http` crate; implementations must validate TLS certificates
//! (never offer an "insecure" switch), only speak HTTPS, and apply the
//! request's timeout.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

use crate::error::Result;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

/// Header value that is redacted in `Debug` output.
#[derive(Clone)]
pub struct HeaderValue {
    value: SecretString,
}

impl HeaderValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: SecretString::from(value.into()),
        }
    }

    pub fn expose(&self) -> &str {
        self.value.expose_secret()
    }
}

impl fmt::Debug for HeaderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HeaderValue(<redacted>)")
    }
}

#[derive(Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, HeaderValue)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    /// Absolute path of a PEM bundle with extra trusted CAs (private
    /// certificates). The system roots stay trusted; validation is never off.
    pub ca_bundle: Option<String>,
}

impl HttpRequest {
    pub fn new(method: Method, url: impl Into<String>, timeout: Duration) -> Self {
        Self {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout,
            ca_bundle: None,
        }
    }

    #[must_use]
    pub fn ca_bundle(mut self, path: Option<String>) -> Self {
        self.ca_bundle = path;
        self
    }

    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers
            .push((name.to_string(), HeaderValue::new(value)));
        self
    }

    #[must_use]
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }
}

impl fmt::Debug for HttpRequest {
    /// Header values and body are never printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("body_len", &self.body.as_ref().map(Vec::len))
            .field("timeout", &self.timeout)
            .field("ca_bundle", &self.ca_bundle)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// Transport failures (DNS, TLS, timeout) are `Err(Error::Http)`; any HTTP
/// status, including 4xx and 5xx, is `Ok`.
pub trait HttpClient: Send + Sync {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_header_values_or_body() {
        let req = HttpRequest::new(Method::Post, "https://h/x", Duration::from_secs(1))
            .header("Authorization", "Bearer super-secret-token")
            .body(b"password=hunter2".to_vec());
        let shown = format!("{req:?}");
        assert!(!shown.contains("super-secret-token"));
        assert!(!shown.contains("hunter2"));
        assert!(shown.contains("Authorization"));
    }
}
