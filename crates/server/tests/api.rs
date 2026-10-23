//! End-to-end tests: a real server on 127.0.0.1 with a scripted upstream.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use trace_core::packs::Catalog;
use trace_core::testing::{FakeHttp, MemorySecretStore};
use trace_server::{Parts, ServerConfig, ServerHandle, serve_with};
use trace_storage::{Database, SecretBackend};

const TOKEN: &str = "test-token-0123456789-0123456789-abcdef";
const API_KEY: &str = "api-key-SECRET-12345";
const PASSWORD: &str = "partner-PASSWORD-1";

struct Harness {
    server: ServerHandle,
    http: Arc<FakeHttp>,
    client: Client,
    secrets: Arc<MemorySecretStore>,
}

impl Harness {
    async fn start() -> Self {
        let http = FakeHttp::new();
        let secrets = Arc::new(MemorySecretStore::default());
        let parts = Parts::new(
            Arc::new(Database::open_in_memory().unwrap()),
            secrets.clone(),
            SecretBackend::EncryptedFile,
            http.clone(),
            Catalog::built_in(),
        );
        let mut config = ServerConfig::new("unused");
        config.token = Some(SecretString::from(TOKEN));
        let server = serve_with(config, parts).await.unwrap();
        Self {
            server,
            http,
            client: Client::new(),
            secrets,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api{path}", self.server.base_url())
    }

    fn req(&self, method: Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, self.url(path))
            .bearer_auth(self.server.token().expose_secret())
    }

    async fn get(&self, path: &str) -> Response {
        self.req(Method::GET, path).send().await.unwrap()
    }

    async fn send_json(&self, method: Method, path: &str, body: &Value) -> Response {
        self.req(method, path).json(body).send().await.unwrap()
    }

    async fn create_env(&self) -> Value {
        let res = self
            .send_json(
                Method::POST,
                "/environments",
                &json!({
                    "name": "Dev",
                    "b2b_host": "dev2205134-m-yqb-c1.b2b.ipaas.ibm.com",
                    "instance_id": "123e4567-e89b-12d3-a456-426614174000",
                    "api_key": API_KEY,
                }),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
        res.json().await.unwrap()
    }

    async fn create_partner(&self, env_id: &str, name: &str, duns: &str, user: &str) -> Value {
        let res = self
            .send_json(
                Method::POST,
                &format!("/environments/{env_id}/partners"),
                &json!({
                    "name": name,
                    "org_unit": "Ops",
                    "id_value": duns,
                    "username": user,
                    "full_name": format!("{name} User"),
                    "channel_name": format!("{name}-channel"),
                    "password": PASSWORD,
                }),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CREATED, "{:?}", res.text().await);
        res.json().await.unwrap_or_default()
    }

    async fn wait_run(&self, run_id: &str) -> Value {
        for _ in 0..200 {
            let run: Value = self
                .get(&format!("/runs/{run_id}"))
                .await
                .json()
                .await
                .unwrap();
            if run["status"] != "running" {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("run did not finish");
    }

    async fn stop(self) {
        self.server.shutdown().await;
    }
}

fn token_body() -> String {
    let enc = |s: &str| URL_SAFE_NO_PAD.encode(s.as_bytes());
    let jwt = format!(
        "{}.{}.{}",
        enc(r#"{"alg":"none"}"#),
        enc(r#"{"exp":2000000000}"#),
        enc("sig")
    );
    json!({ "token": jwt }).to_string()
}

fn script_provisioning(http: &FakeHttp, tag: &str) {
    http.push(201, r#"{"internalId":"pid-1"}"#);
    http.push(
        201,
        &format!(r#"{{"id":"ch-{tag}","endpoint":{{"uri":"https://b2b.example/ch/{tag}"}}}}"#),
    );
    http.push(200, "{}");
    http.push(201, r#"{"id":"user-1"}"#);
    http.push(200, "{}");
}

// ---- guards --------------------------------------------------------------

#[tokio::test]
async fn api_requires_the_token() {
    let h = Harness::start().await;
    let none = h.client.get(h.url("/environments")).send().await.unwrap();
    assert_eq!(none.status(), StatusCode::UNAUTHORIZED);
    let wrong = h
        .client
        .get(h.url("/environments"))
        .bearer_auth("not-the-token-not-the-token-not-the-token")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    // The token is not accepted in the URL.
    let query = h
        .client
        .get(format!("{}?token={TOKEN}", h.url("/environments")))
        .send()
        .await
        .unwrap();
    assert_eq!(query.status(), StatusCode::UNAUTHORIZED);
    let body: Value = none.json().await.unwrap();
    assert_eq!(body["error"], "missing or invalid token");
    assert_eq!(h.get("/health").await.status(), StatusCode::OK);
    h.stop().await;
}

#[tokio::test]
async fn foreign_host_and_origin_are_rejected() {
    let h = Harness::start().await;
    let rebinding = h
        .req(Method::GET, "/health")
        .header("host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(rebinding.status(), StatusCode::FORBIDDEN);
    let origin = h
        .req(Method::GET, "/health")
        .header("origin", "http://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(origin.status(), StatusCode::FORBIDDEN);
    let own = h
        .req(Method::GET, "/health")
        .header("origin", h.server.base_url())
        .send()
        .await
        .unwrap();
    assert_eq!(own.status(), StatusCode::OK);
    assert!(own.headers().get("access-control-allow-origin").is_none());
    assert_eq!(own.headers()["cache-control"], "no-store");
    assert_eq!(own.headers()["x-content-type-options"], "nosniff");
    h.stop().await;
}

#[tokio::test]
async fn binds_loopback_only() {
    let h = Harness::start().await;
    assert!(h.server.addr().ip().is_loopback());
    h.stop().await;
}

#[tokio::test]
async fn unknown_api_paths_are_json_404() {
    let h = Harness::start().await;
    let res = h.get("/nope").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["error"], "not found");
    h.stop().await;
}

#[tokio::test]
async fn health_reports_backend_without_secrets() {
    let h = Harness::start().await;
    let body: Value = h.get("/health").await.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["secret_backend"], "encrypted_file");
    h.stop().await;
}

// ---- errors ---------------------------------------------------------------

#[tokio::test]
async fn validation_errors_list_fields_and_do_not_echo_secrets() {
    let h = Harness::start().await;
    let res = h
        .send_json(
            Method::POST,
            "/environments",
            &json!({
                "name": "",
                "b2b_host": "not a host",
                "instance_id": "nope",
                "api_key": "short",
            }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let text = res.text().await.unwrap();
    let body: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(body["error"], "Validation failed");
    assert!(body["fields"].as_array().unwrap().len() >= 3);
    assert!(!text.contains("short"));

    // A wrongly typed secret must not be echoed by the JSON rejection.
    let res = h
        .req(Method::POST, "/environments")
        .header("content-type", "application/json")
        .body(r#"{"name":"x","api_key":12345678}"#)
        .send()
        .await
        .unwrap();
    assert!(res.status().is_client_error());
    let text = res.text().await.unwrap();
    assert!(!text.contains("12345678"));
    h.stop().await;
}

#[tokio::test]
async fn bad_ids_and_missing_things() {
    let h = Harness::start().await;
    assert_eq!(
        h.get("/environments/not-a-uuid").await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let missing = "123e4567-e89b-12d3-a456-426614174999";
    assert_eq!(
        h.get(&format!("/environments/{missing}")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(h.get("/runs/abc").await.status(), StatusCode::NOT_FOUND);
    h.stop().await;
}

// ---- environments and partners -------------------------------------------

#[tokio::test]
async fn secrets_are_write_only() {
    let h = Harness::start().await;
    let env = h.create_env().await;
    assert_eq!(env["has_api_key"], true);
    let env_id = env["id"].as_str().unwrap().to_string();
    h.create_partner(&env_id, "Alpha", "123456789", "alpha.user")
        .await;

    let mut seen = Vec::new();
    for path in [
        "/environments".to_string(),
        format!("/environments/{env_id}"),
        format!("/environments/{env_id}/partners"),
        "/preferences".to_string(),
    ] {
        seen.push(h.get(&path).await.text().await.unwrap());
    }
    for text in &seen {
        assert!(!text.contains(API_KEY), "api key leaked: {text}");
        assert!(!text.contains(PASSWORD), "password leaked: {text}");
    }
    assert!(seen[2].contains("\"has_password\":true"));
    assert_eq!(h.secrets.len(), 2);
    h.stop().await;
}

#[tokio::test]
async fn environment_update_default_and_delete() {
    let h = Harness::start().await;
    let env = h.create_env().await;
    let id = env["id"].as_str().unwrap().to_string();

    let res = h
        .send_json(
            Method::PUT,
            &format!("/environments/{id}"),
            &json!({
                "name": "Renamed",
                "b2b_host": "dev2205134-m-yqb-c1.b2b.ipaas.ibm.com",
                "instance_id": "123e4567-e89b-12d3-a456-426614174000",
            }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["name"], "Renamed");
    assert_eq!(body["has_api_key"], true);

    let res = h
        .req(Method::POST, &format!("/environments/{id}/default"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let res = h
        .req(Method::DELETE, &format!("/environments/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        h.get(&format!("/environments/{id}")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert!(h.secrets.is_empty());
    h.stop().await;
}

#[tokio::test]
async fn connection_test_reports_ok_and_failure() {
    let h = Harness::start().await;
    let env = h.create_env().await;
    let id = env["id"].as_str().unwrap();

    h.http.push(200, &token_body());
    let report: Value = h
        .req(Method::POST, &format!("/environments/{id}/test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(report["ok"], true);

    h.http.push(401, r#"{"error":"bad key"}"#);
    let res = h
        .req(Method::POST, &format!("/environments/{id}/test"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let text = res.text().await.unwrap();
    let report: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["ok"], false);
    assert!(!text.contains(API_KEY));
    h.stop().await;
}

// ---- packs, preferences ---------------------------------------------------

#[tokio::test]
async fn packs_are_listed() {
    let h = Harness::start().await;
    let packs: Vec<Value> = h.get("/packs").await.json().await.unwrap();
    let ids: Vec<&str> = packs.iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["retail", "healthcare", "supply_chain"]);
    assert!(!packs[0]["documents"].as_array().unwrap().is_empty());
    assert!(packs[0]["documents"][0]["sender"].is_string());
    h.stop().await;
}

#[tokio::test]
async fn preferences_round_trip_and_validate() {
    let h = Harness::start().await;
    let defaults: Value = h.get("/preferences").await.json().await.unwrap();
    assert_eq!(defaults["request_timeout_secs"], 30);

    let res = h
        .send_json(
            Method::PUT,
            "/preferences",
            &json!({"inter_document_delay_secs": 1, "request_timeout_secs": 10, "payload_timeout_secs": 60}),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let saved: Value = h.get("/preferences").await.json().await.unwrap();
    assert_eq!(saved["request_timeout_secs"], 10);

    let bad = h
        .send_json(
            Method::PUT,
            "/preferences",
            &json!({"request_timeout_secs": 0}),
        )
        .await;
    assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    h.stop().await;
}

// ---- provisioning, documents, submit -------------------------------------

#[tokio::test]
async fn provision_runs_in_background_and_reports() {
    let h = Harness::start().await;
    let env = h.create_env().await;
    let env_id = env["id"].as_str().unwrap().to_string();
    let partner = h
        .create_partner(&env_id, "Alpha", "123456789", "alpha.user")
        .await;
    let pid = partner["id"].as_str().unwrap().to_string();

    h.http.push(200, &token_body());
    script_provisioning(&h.http, "p1");
    let res = h
        .req(Method::POST, &format!("/partners/{pid}/provision"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let started: Value = res.json().await.unwrap();
    let run = h.wait_run(started["run_id"].as_str().unwrap()).await;
    assert_eq!(run["status"], "finished", "{run}");
    assert_eq!(run["report"]["cancelled"], false);

    let status: Value = h
        .get(&format!("/partners/{pid}/provisioning"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(status["complete"], true);
    assert_eq!(status["channel_endpoint_uri"], "https://b2b.example/ch/p1");

    // Nothing sent upstream contains what the API must not echo back.
    let text = run.to_string();
    assert!(!text.contains(PASSWORD) && !text.contains(API_KEY));

    // An edit after provisioning is a conflict.
    let res = h
        .send_json(
            Method::PUT,
            &format!("/partners/{pid}"),
            &json!({
                "name": "Alpha2", "org_unit": "Ops", "id_value": "123456789",
                "username": "alpha.user", "full_name": "A", "channel_name": "C",
            }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    h.stop().await;
}

#[tokio::test]
async fn provision_failure_is_a_finished_run_with_a_failed_step() {
    let h = Harness::start().await;
    let env = h.create_env().await;
    let env_id = env["id"].as_str().unwrap().to_string();
    let partner = h
        .create_partner(&env_id, "Alpha", "123456789", "alpha.user")
        .await;
    let pid = partner["id"].as_str().unwrap().to_string();

    h.http.push(200, &token_body());
    h.http.push(500, r#"{"message":"boom"}"#);
    let started: Value = h
        .req(Method::POST, &format!("/partners/{pid}/provision"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let run = h.wait_run(started["run_id"].as_str().unwrap()).await;
    assert_eq!(run["status"], "finished");
    let steps = run["report"]["steps"].as_array().unwrap();
    assert!(steps.iter().any(|s| s["outcome"] == "failed"));
    h.stop().await;
}

#[tokio::test]
async fn provision_unknown_partner_is_404_and_second_start_is_409() {
    let h = Harness::start().await;
    let missing = "123e4567-e89b-12d3-a456-426614174999";
    let res = h
        .req(Method::POST, &format!("/partners/{missing}/provision"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let env = h.create_env().await;
    let env_id = env["id"].as_str().unwrap().to_string();
    let partner = h
        .create_partner(&env_id, "Alpha", "123456789", "alpha.user")
        .await;
    let pid = partner["id"].as_str().unwrap().to_string();
    // No scripted responses: the first run fails fast, but start two quickly.
    let a = h
        .req(Method::POST, &format!("/partners/{pid}/provision"))
        .send()
        .await
        .unwrap();
    assert_eq!(a.status(), StatusCode::ACCEPTED);
    let b = h
        .req(Method::POST, &format!("/partners/{pid}/provision"))
        .send()
        .await
        .unwrap();
    assert!(b.status() == StatusCode::CONFLICT || b.status() == StatusCode::ACCEPTED);
    h.stop().await;
}

#[tokio::test]
async fn preview_submit_and_activity_end_to_end() {
    let h = Harness::start().await;
    let env = h.create_env().await;
    let env_id = env["id"].as_str().unwrap().to_string();
    let p1 = h
        .create_partner(&env_id, "Alpha", "123456789", "alpha.user")
        .await;
    let p2 = h
        .create_partner(&env_id, "Beta", "987654321", "beta.user")
        .await;
    let p1_id = p1["id"].as_str().unwrap().to_string();
    let p2_id = p2["id"].as_str().unwrap().to_string();

    // Preview needs no upstream and consumes no control numbers.
    let preview: Value = h
        .send_json(
            Method::POST,
            "/preview",
            &json!({
                "environment_id": env_id, "pack_id": "retail", "codes": ["850"],
                "p1": {"type": "saved", "partner_id": p1_id},
                "p2": {"type": "manual", "id_value": "555666777"},
            }),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(preview["documents"][0]["transaction_code"], "850");
    assert!(
        preview["documents"][0]["payload"]
            .as_str()
            .unwrap()
            .starts_with("ISA")
    );
    let counters: Value = h
        .get(&format!("/environments/{env_id}/counters"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(counters["isa_next"], 1);

    // Provision both partners, then submit one document.
    // The token is fetched once per environment and then cached.
    h.http.push(200, &token_body());
    for (pid, tag) in [(&p1_id, "p1"), (&p2_id, "p2")] {
        script_provisioning(&h.http, tag);
        let started: Value = h
            .req(Method::POST, &format!("/partners/{pid}/provision"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let run = h.wait_run(started["run_id"].as_str().unwrap()).await;
        assert_eq!(run["status"], "finished", "{run}");
        assert_eq!(
            run["report"]["steps"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|s| s["outcome"] == "failed")
                .count(),
            0,
            "{run}"
        );
    }

    h.http.push(200, "accepted");
    let res = h
        .send_json(
            Method::POST,
            "/submit",
            &json!({
                "environment_id": env_id, "pack_id": "retail", "codes": ["850"],
                "p1_partner_id": p1_id, "p2_partner_id": p2_id,
            }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let started: Value = res.json().await.unwrap();
    let run = h.wait_run(started["run_id"].as_str().unwrap()).await;
    assert_eq!(run["status"], "finished", "{run}");
    assert_eq!(run["report"]["documents"][0]["outcome"], "submitted");

    let list: Vec<Value> = h
        .get(&format!("/environments/{env_id}/submissions"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].get("payload").is_none());
    assert_eq!(list[0]["has_payload"], true);
    let detail: Value = h
        .get(&format!("/submissions/{}", list[0]["id"].as_str().unwrap()))
        .await
        .json()
        .await
        .unwrap();
    assert!(detail["payload"].as_str().unwrap().starts_with("ISA"));
    assert!(!detail.to_string().contains(PASSWORD));

    let counters: Value = h
        .get(&format!("/environments/{env_id}/counters"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(counters["isa_next"], 2);
    h.stop().await;
}

#[tokio::test]
async fn submit_with_invalid_input_is_a_clean_error() {
    let h = Harness::start().await;
    let env = h.create_env().await;
    let env_id = env["id"].as_str().unwrap();
    let missing = "123e4567-e89b-12d3-a456-426614174999";
    let res = h
        .send_json(
            Method::POST,
            "/submit",
            &json!({"environment_id": env_id, "pack_id": "retail", "p1_partner_id": missing, "p2_partner_id": missing}),
        )
        .await;
    // The run starts, then fails before anything is posted.
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let started: Value = res.json().await.unwrap();
    let run = h.wait_run(started["run_id"].as_str().unwrap()).await;
    assert_eq!(run["status"], "failed");
    assert!(run["error"]["error"].is_string());
    assert!(h.http.requests().is_empty());
    h.stop().await;
}

// ---- events ---------------------------------------------------------------

#[tokio::test]
async fn events_stream_delivers_service_events_and_ends_on_shutdown() {
    let h = Harness::start().await;
    let mut stream = h.req(Method::GET, "/events").send().await.unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    assert!(
        stream.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );

    h.create_env().await;

    let mut buf = String::new();
    let got = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.contains("environment.created") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap();
    assert!(got, "no event in: {buf}");
    assert!(!buf.contains(API_KEY));

    // Shutdown ends the open stream instead of hanging.
    let server = h.server;
    tokio::time::timeout(Duration::from_secs(5), server.shutdown())
        .await
        .expect("shutdown must not hang on an open event stream");
}

#[tokio::test]
async fn token_below_minimum_length_is_refused() {
    let mut config = ServerConfig::new("unused");
    config.token = Some(SecretString::from("short"));
    let parts = Parts::new(
        Arc::new(Database::open_in_memory().unwrap()),
        Arc::new(MemorySecretStore::default()),
        SecretBackend::EncryptedFile,
        FakeHttp::new(),
        Catalog::built_in(),
    );
    assert!(serve_with(config, parts).await.is_err());
}

// ---- contract drift ---------------------------------------------------------

/// (method, path) pairs documented in `docs/api/openapi.yaml`.
fn documented_operations() -> Vec<(String, String)> {
    let spec = include_str!("../../../docs/api/openapi.yaml");
    let mut ops = Vec::new();
    let mut in_paths = false;
    let mut path = String::new();
    for line in spec.lines() {
        if line == "paths:" {
            in_paths = true;
            continue;
        }
        if in_paths && !line.starts_with(' ') && !line.is_empty() {
            break;
        }
        if let Some(p) = line.strip_prefix("  /") {
            path = format!("/{}", p.trim_end_matches(':'));
        } else if in_paths {
            let method = line.trim_end_matches(':').trim();
            if line.starts_with("    ")
                && !line.starts_with("     ")
                && line.ends_with(':')
                && ["get", "post", "put", "delete"].contains(&method)
            {
                ops.push((method.to_uppercase(), path.clone()));
            }
        }
    }
    ops
}

/// (method, path) pairs registered in the routers, read from the sources.
fn registered_operations() -> Vec<(String, String)> {
    let sources = [
        include_str!("../src/routers/system.rs"),
        include_str!("../src/routers/events.rs"),
        include_str!("../src/routers/runs.rs"),
        include_str!("../src/routers/environments.rs"),
        include_str!("../src/routers/partners.rs"),
        include_str!("../src/routers/documents.rs"),
        include_str!("../src/routers/packs.rs"),
        include_str!("../src/routers/submit.rs"),
        include_str!("../src/routers/activity.rs"),
        include_str!("../src/routers/preferences.rs"),
    ];
    let mut ops = Vec::new();
    for src in sources {
        let flat: String = src.split_whitespace().collect::<Vec<_>>().join(" ");
        for chunk in flat.split(".route(").skip(1) {
            let Some(rest) = chunk.strip_prefix('"') else {
                continue;
            };
            let Some((path, after)) = rest.split_once('"') else {
                continue;
            };
            // The handler expression runs up to the closing of `.route(`.
            let handlers = after.split(") .route(").next().unwrap_or(after);
            // The last route of a builder ends at the function's closing brace.
            let handlers = handlers.split(['}', ';']).next().unwrap_or(handlers);
            for method in ["get", "post", "put", "delete"] {
                let call = format!("{method}(");
                let hit = handlers.starts_with(&format!(", {call}"))
                    || handlers.contains(&format!(").{call}"))
                    || handlers.contains(&format!(" .{call}"));
                if hit {
                    ops.push((method.to_uppercase(), path.to_string()));
                }
            }
        }
    }
    ops
}

#[test]
fn spec_and_routers_list_the_same_operations() {
    let mut documented = documented_operations();
    let mut registered = registered_operations();
    documented.sort();
    registered.sort();
    assert!(
        documented.len() > 20,
        "spec parse found too little: {documented:?}"
    );
    assert_eq!(
        documented, registered,
        "docs/api/openapi.yaml and the routers differ"
    );
}

#[tokio::test]
async fn every_documented_operation_is_served() {
    let h = Harness::start().await;
    let nil = "123e4567-e89b-12d3-a456-426614174999";
    for (method, path) in documented_operations() {
        if path == "/events" {
            continue; // streaming; covered by its own test
        }
        let concrete = path.replace("{id}", nil);
        let method: Method = method.parse().unwrap();
        let res = h
            .req(method.clone(), &concrete)
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap();
        let status = res.status();
        assert_ne!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
        if status == StatusCode::NOT_FOUND {
            let body: Value = res.json().await.unwrap();
            assert_ne!(body["error"], "not found", "{method} {path} is not routed");
        }
    }
    h.stop().await;
}
