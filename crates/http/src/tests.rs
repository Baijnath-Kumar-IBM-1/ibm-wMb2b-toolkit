//! Transport tests against a local TLS server with a throw-away CA.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use trace_core::Error;
use trace_core::http::{HttpClient, HttpRequest, Method};

use super::{MAX_RESPONSE_BYTES, ReqwestClient};

struct TestPki {
    ca_pem: String,
    server_chain: Vec<CertificateDer<'static>>,
    server_key_der: Vec<u8>,
}

/// CA plus a server certificate valid for `sans`.
fn pki(sans: &[&str]) -> TestPki {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Trace test CA");
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);

    let server_key = KeyPair::generate().unwrap();
    let server_params =
        CertificateParams::new(sans.iter().map(|s| (*s).to_string()).collect::<Vec<_>>()).unwrap();
    let server_cert = server_params.signed_by(&server_key, &issuer).unwrap();

    TestPki {
        ca_pem: ca_cert.pem(),
        server_chain: vec![server_cert.der().clone()],
        server_key_der: server_key.serialize_der(),
    }
}

/// Serves `response` (raw HTTP bytes) to every connection. After the TLS
/// handshake it reads the request head and replies. Returns the bound address.
async fn serve(pki: &TestPki, response: Vec<u8>) -> SocketAddr {
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pki.server_key_der.clone()));
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(pki.server_chain.clone(), key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            let response = response.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = tls.read(&mut buf).await;
                let _ = tls.write_all(&response).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    addr
}

fn ok_response(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn get(url: String) -> HttpRequest {
    HttpRequest::new(Method::Get, url, Duration::from_secs(10))
}

fn write_bundle(pem: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ca.pem");
    std::fs::write(&path, pem).unwrap();
    let path = path.to_str().unwrap().to_string();
    (dir, path)
}

#[tokio::test]
async fn trusts_a_private_ca_only_through_the_bundle() {
    let pki = pki(&["127.0.0.1"]);
    let addr = serve(&pki, ok_response("hello")).await;
    let url = format!("https://{addr}/x");
    let client = ReqwestClient::new();

    // No bundle: the certificate is not trusted, so the request must fail.
    let err = client.send(get(url.clone())).await.unwrap_err();
    assert!(matches!(err, Error::Http(_)), "{err:?}");

    let (_dir, path) = write_bundle(&pki.ca_pem);
    let res = client.send(get(url).ca_bundle(Some(path))).await.unwrap();
    assert_eq!(res.status, 200);
    assert_eq!(res.body, b"hello");
}

#[tokio::test]
async fn rejects_a_certificate_for_another_host_even_with_the_ca() {
    let pki = pki(&["not-this-host.example"]);
    let addr = serve(&pki, ok_response("hello")).await;
    let (_dir, path) = write_bundle(&pki.ca_pem);
    let err = ReqwestClient::new()
        .send(get(format!("https://{addr}/")).ca_bundle(Some(path)))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Http(_)), "{err:?}");
}

#[tokio::test]
async fn rejects_a_certificate_from_a_different_ca() {
    let served = pki(&["127.0.0.1"]);
    let other = pki(&["127.0.0.1"]);
    let addr = serve(&served, ok_response("hello")).await;
    let (_dir, path) = write_bundle(&other.ca_pem);
    let err = ReqwestClient::new()
        .send(get(format!("https://{addr}/")).ca_bundle(Some(path)))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Http(_)), "{err:?}");
}

#[tokio::test]
async fn refuses_plain_http() {
    let err = ReqwestClient::new()
        .send(get("http://127.0.0.1:1/".into()))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("https"), "{err}");
}

#[tokio::test]
async fn invalid_ca_bundles_are_rejected_with_safe_messages() {
    let client = ReqwestClient::new();
    for (content, label) in [("not a certificate", "garbage"), ("", "empty")] {
        let (_dir, path) = write_bundle(content);
        let err = client
            .send(get("https://127.0.0.1:1/".into()).ca_bundle(Some(path.clone())))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Http(_)), "{label}: {err:?}");
        assert!(!err.to_string().contains(&path), "{label}: path leaked");
    }
    let err = client
        .send(get("https://127.0.0.1:1/".into()).ca_bundle(Some("/nonexistent/ca.pem".into())))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Http(_)), "{err:?}");
}

#[tokio::test]
async fn does_not_follow_redirects() {
    let pki = pki(&["127.0.0.1"]);
    let redirect = b"HTTP/1.1 302 Found\r\nLocation: https://127.0.0.1:1/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
    let addr = serve(&pki, redirect).await;
    let (_dir, path) = write_bundle(&pki.ca_pem);
    let res = ReqwestClient::new()
        .send(get(format!("https://{addr}/")).ca_bundle(Some(path)))
        .await
        .unwrap();
    assert_eq!(res.status, 302);
}

#[tokio::test]
async fn non_2xx_statuses_are_ok_responses_with_the_body() {
    let pki = pki(&["127.0.0.1"]);
    let body = r#"{"error":"already exists"}"#;
    let raw = format!(
        "HTTP/1.1 409 Conflict\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes();
    let addr = serve(&pki, raw).await;
    let (_dir, path) = write_bundle(&pki.ca_pem);
    let res = ReqwestClient::new()
        .send(get(format!("https://{addr}/")).ca_bundle(Some(path)))
        .await
        .unwrap();
    assert_eq!(res.status, 409);
    assert_eq!(res.body, body.as_bytes());
}

#[tokio::test]
async fn sends_method_headers_and_body() {
    let pki = pki(&["127.0.0.1"]);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pki.server_key_der.clone()));
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(pki.server_chain.clone(), key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tls = acceptor.accept(tcp).await.unwrap();
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        while !String::from_utf8_lossy(&seen).contains("payload") {
            let n = tls.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            seen.extend_from_slice(&buf[..n]);
        }
        let _ = tx.send(String::from_utf8_lossy(&seen).to_string());
        tls.write_all(&ok_response("{}")).await.unwrap();
        let _ = tls.shutdown().await;
    });
    let (_dir, path) = write_bundle(&pki.ca_pem);
    let request = HttpRequest::new(
        Method::Put,
        format!("https://{addr}/b2b/api/v1/thing"),
        Duration::from_secs(10),
    )
    .header("Authorization", "Bearer abc")
    .header("Content-Type", "application/json")
    .body(b"payload".to_vec())
    .ca_bundle(Some(path));
    let res = ReqwestClient::new().send(request).await.unwrap();
    assert!(res.is_success());
    let head = rx.await.unwrap().to_lowercase();
    assert!(head.starts_with("put /b2b/api/v1/thing http/1.1"), "{head}");
    assert!(head.contains("authorization: bearer abc"));
    assert!(head.contains("content-type: application/json"));
}

#[tokio::test]
async fn enforces_the_request_timeout() {
    // Accepts the TCP connection and never answers.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((tcp, _)) = listener.accept().await {
            held.push(tcp);
        }
    });
    let started = std::time::Instant::now();
    let err = ReqwestClient::new()
        .send(HttpRequest::new(
            Method::Get,
            format!("https://{addr}/"),
            Duration::from_millis(300),
        ))
        .await
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(8));
    assert!(err.to_string().contains("timed out"), "{err}");
}

#[tokio::test]
async fn oversized_responses_are_rejected() {
    let pki = pki(&["127.0.0.1"]);
    let big = "x".repeat(MAX_RESPONSE_BYTES + 1);
    let addr = serve(&pki, ok_response(&big)).await;
    let (_dir, path) = write_bundle(&pki.ca_pem);
    let err = ReqwestClient::new()
        .send(get(format!("https://{addr}/")).ca_bundle(Some(path)))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Http(_)), "{err:?}");
}

#[tokio::test]
async fn error_text_does_not_contain_header_values() {
    let err = ReqwestClient::new()
        .send(
            HttpRequest::new(
                Method::Get,
                "https://127.0.0.1:1/".to_string(),
                Duration::from_secs(2),
            )
            .header("Mcsp-ApiKey", "super-secret-key"),
        )
        .await
        .unwrap_err();
    assert!(!err.to_string().contains("super-secret-key"));
}
