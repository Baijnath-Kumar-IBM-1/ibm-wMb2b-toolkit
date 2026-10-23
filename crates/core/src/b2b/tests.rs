//! Tests for token handling, connection test, provisioning and documents,
//! using `FakeHttp` (scripted responses, recorded requests).

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use time::Duration as TimeDuration;

use crate::b2b::api::test_jwt;
use crate::b2b::documents::DocumentOutcome;
use crate::b2b::{
    B2bApi, ConnectionService, DocumentService, PROVISION_STEPS, ProvisioningService, doctype_key,
};
use crate::environments::Environment;
use crate::http::{HttpRequest, Method};
use crate::packs::Catalog;
use crate::partners::Partner;
use crate::steps::{CancelToken, StepOutcome};
use crate::testing::FakeHttp;
use crate::tests::{Fixture, env_input, partner_input, secret};

const API_KEY: &str = "api-key-value-12345";
const PASSWORD: &str = "S3cretPass!word";
/// Far after the fixed test clock (2025-01-01).
const LONG_LIVED: i64 = 2_000_000_000;

struct Harness {
    f: Fixture,
    http: Arc<FakeHttp>,
    env: Environment,
    partner: Partner,
    api: Arc<B2bApi>,
    prov: ProvisioningService,
    docs: DocumentService,
    conn: ConnectionService,
}

fn harness() -> Harness {
    let f = Fixture::new();
    let env = f.envs.create(&env_input("Dev"), &secret(API_KEY)).unwrap();
    let partner = f
        .partners
        .create(
            &env.id,
            &partner_input("P1 Retail", "123456789", "p1user", "p1-channel"),
            &secret(PASSWORD),
        )
        .unwrap();
    let http = FakeHttp::new();
    let api = B2bApi::new(
        http.clone(),
        f.secrets.clone(),
        f.db.clone(),
        f.clock.clone(),
    );
    let prov = ProvisioningService::new(
        api.clone(),
        f.db.clone(),
        f.db.clone(),
        f.db.clone(),
        f.secrets.clone(),
        f.clock.clone(),
        f.events.clone(),
    );
    let docs = DocumentService::new(
        api.clone(),
        Arc::new(Catalog::built_in()),
        f.db.clone(),
        f.db.clone(),
        f.clock.clone(),
        f.events.clone(),
    )
    .with_pause(Duration::ZERO);
    let conn = ConnectionService::new(api.clone(), f.db.clone(), f.clock.clone(), f.events.clone());
    Harness {
        f,
        http,
        env,
        partner,
        api,
        prov,
        docs,
        conn,
    }
}

fn token_body(exp: i64) -> String {
    format!(r#"{{"token":"{}"}}"#, test_jwt(exp))
}

fn push_token(h: &Harness) {
    h.http.push(200, &token_body(LONG_LIVED));
}

/// Responses for a complete, successful provisioning run (after the token).
fn push_provisioning_ok(h: &Harness) {
    h.http.push(201, r#"{"internalId":"pid-1"}"#);
    h.http.push(
        201,
        r#"{"id":"ch-1","endpoint":{"uri":"https://b2b.example/ch/p1"}}"#,
    );
    h.http.push(200, "{}");
    h.http.push(201, r#"{"id":"user-1"}"#);
    h.http.push(200, "{}");
}

fn header(r: &HttpRequest, name: &str) -> Option<String> {
    r.headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.expose().to_string())
}

fn body(r: &HttpRequest) -> Value {
    serde_json::from_slice(r.body.as_deref().unwrap()).unwrap()
}

fn summary(r: &HttpRequest) -> String {
    let m = match r.method {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Delete => "DELETE",
    };
    let path = r.url.split_once("://").unwrap().1;
    let path = path.split_once('/').map_or("", |(_, p)| p);
    format!("{m} /{}", path.split('?').next().unwrap())
}

fn outcomes(report: &crate::steps::RunReport) -> Vec<StepOutcome> {
    report.steps.iter().map(|s| s.outcome).collect()
}

// ---------------------------------------------------------------- doc type key

#[test]
fn doctype_key_has_the_documented_shape_for_4010() {
    // Nine trailing dashes. This format was taken from the v1 script
    // (branch `v1-bash`, `txn_to_doctype_key`), which no longer ships here.
    for txn in ["850", "820", "837", "204"] {
        assert_eq!(
            doctype_key("X12", "004010", txn).unwrap(),
            format!("WMX12-4010--{txn}---------"),
            "{txn}"
        );
    }
}

#[test]
fn doctype_key_derives_the_version_from_the_pack() {
    assert_eq!(
        doctype_key("X12", "005010", "837").unwrap(),
        "WMX12-5010--837---------"
    );
    assert!(doctype_key("X12", "0000", "850").is_err());
    assert!(doctype_key("X12", "40x0", "850").is_err());
}

#[test]
fn built_in_packs_all_use_004010() {
    for pack in Catalog::built_in().all() {
        assert_eq!(pack.manifest.pack.standard, "X12");
        assert_eq!(pack.manifest.pack.version, "004010");
    }
}

// ------------------------------------------------------------ token + testing

#[tokio::test]
async fn connection_test_requests_a_token_with_the_api_key_header() {
    let h = harness();
    push_token(&h);
    let report = h.conn.test(&h.env.id).await.unwrap();
    assert!(report.ok, "{}", report.message);

    let reqs = h.http.requests();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.method, Method::Post);
    assert_eq!(
        r.url,
        format!(
            "https://{}/api/2.0/services/{}/apikeys/token?includeBuiltinActions=false&includeCustomActions=false&includeRoles=true&prefixRolesWithDefinitionScope=false",
            h.env.iam_host, h.env.instance_id
        )
    );
    assert_eq!(header(r, "Mcsp-ApiKey").as_deref(), Some(API_KEY));
    assert_eq!(
        header(r, "Content-Type").as_deref(),
        Some("application/json")
    );
    assert_eq!(r.timeout, Duration::from_secs(30));
    assert!(r.url.starts_with("https://"));
}

#[tokio::test]
async fn connection_test_failure_is_reported_without_secrets() {
    let h = harness();
    h.http
        .push(401, &format!(r#"{{"error":"bad key {API_KEY}"}}"#));
    let report = h.conn.test(&h.env.id).await.unwrap();
    assert!(!report.ok);
    assert!(report.message.contains("401"));
    assert!(!report.message.contains(API_KEY));

    h.http
        .push_error("could not connect: certificate unknown issuer");
    let report = h.conn.test(&h.env.id).await.unwrap();
    assert!(!report.ok);
    assert!(report.message.contains("certificate"));

    h.http.push(200, r#"{"no_token":true}"#);
    let report = h.conn.test(&h.env.id).await.unwrap();
    assert!(!report.ok);
    assert!(report.message.contains("did not contain a token"));
}

#[tokio::test]
async fn connection_test_never_reuses_a_cached_token() {
    let h = harness();
    push_token(&h);
    push_token(&h);
    assert!(h.conn.test(&h.env.id).await.unwrap().ok);
    assert!(h.conn.test(&h.env.id).await.unwrap().ok);
    assert_eq!(h.http.requests().len(), 2);
}

#[tokio::test]
async fn unknown_environment_is_not_found() {
    let h = harness();
    let other = crate::ids::EnvironmentId::generate();
    assert!(matches!(
        h.conn.test(&other).await,
        Err(crate::Error::NotFound(_))
    ));
}

#[tokio::test]
async fn missing_stored_api_key_is_a_clear_failure() {
    let h = harness();
    crate::secrets::SecretStore::delete(&*h.f.secrets, h.env.api_key_secret.as_ref().unwrap())
        .unwrap();
    let report = h.conn.test(&h.env.id).await.unwrap();
    assert!(!report.ok);
    assert!(report.message.contains("API key"));
    assert!(h.http.requests().is_empty());
}

#[tokio::test]
async fn token_is_cached_across_calls() {
    let h = harness();
    push_token(&h);
    push_provisioning_ok(&h);
    h.prov
        .provision(&h.partner.id, &CancelToken::new())
        .await
        .unwrap();
    let reqs = h.http.requests();
    let token_requests = reqs
        .iter()
        .filter(|r| r.url.contains("/apikeys/token"))
        .count();
    assert_eq!(token_requests, 1);
    assert_eq!(reqs.len(), 6);
}

#[tokio::test]
async fn expired_token_is_replaced_before_the_next_call() {
    let h = harness();
    let env = h.env.clone();
    let post = || async {
        h.api
            .call(
                &env,
                Method::Post,
                "/b2b/api/v1/partners",
                Some(&serde_json::json!({})),
            )
            .await
            .unwrap()
    };
    let token_requests = || {
        h.http
            .requests()
            .iter()
            .filter(|r| r.url.contains("/apikeys/token"))
            .count()
    };

    // Expires 2025-01-01T00:00:30Z, less than the 60 s safety margin from the
    // fixed clock's start, so the next call must fetch a new token.
    h.http.push(200, &token_body(1_735_689_600 + 30));
    h.http.push(201, "{}");
    post().await;
    assert_eq!(token_requests(), 1);

    h.http.push(200, &token_body(LONG_LIVED));
    h.http.push(201, "{}");
    post().await;
    assert_eq!(token_requests(), 2, "short-lived token must be replaced");

    // A token that is valid for long is kept.
    h.http.push(201, "{}");
    post().await;
    assert_eq!(token_requests(), 2);
    assert_eq!(h.http.requests().len(), 5);

    // Time passing past expiry replaces it.
    h.f.clock.advance(TimeDuration::seconds(2_000_000_000));
    h.http.push(200, &token_body(4_000_000_000));
    h.http.push(201, "{}");
    post().await;
    assert_eq!(token_requests(), 3);
    assert_eq!(h.http.requests().len(), 7);
}

#[tokio::test]
async fn a_401_refreshes_the_token_once_and_repeats_the_call() {
    let h = harness();
    push_token(&h);
    h.http.push(401, r#"{"error":"expired"}"#);
    push_token(&h);
    h.http.push(201, r#"{"internalId":"pid-9"}"#);

    let response = h
        .api
        .call(
            &h.env,
            Method::Post,
            "/b2b/api/v1/partners",
            Some(&serde_json::json!({"a": 1})),
        )
        .await
        .unwrap();
    assert_eq!(response.status, 201);
    let kinds: Vec<String> = h.http.requests().iter().map(summary).collect();
    assert_eq!(
        kinds,
        [
            "POST /api/2.0/services/123e4567-e89b-12d3-a456-426614174000/apikeys/token",
            "POST /b2b/api/v1/partners",
            "POST /api/2.0/services/123e4567-e89b-12d3-a456-426614174000/apikeys/token",
            "POST /b2b/api/v1/partners",
        ]
    );
    // Same body is sent again.
    let reqs = h.http.requests();
    assert_eq!(body(&reqs[1]), body(&reqs[3]));
}

#[tokio::test]
async fn a_second_401_is_returned_not_retried_forever() {
    let h = harness();
    push_token(&h);
    h.http.push(401, "{}");
    push_token(&h);
    h.http.push(401, "{}");
    let response = h
        .api
        .call(&h.env, Method::Get, "/b2b/api/v1/documents/x", None)
        .await
        .unwrap();
    assert_eq!(response.status, 401);
    assert_eq!(h.http.requests().len(), 4);
}

// -------------------------------------------------------------- provisioning

#[tokio::test]
async fn provisioning_runs_all_five_steps_with_the_scripts_paths_and_payloads() {
    let h = harness();
    push_token(&h);
    push_provisioning_ok(&h);
    let report = h
        .prov
        .provision(&h.partner.id, &CancelToken::new())
        .await
        .unwrap();
    assert!(report.is_complete());
    assert_eq!(
        report
            .steps
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        PROVISION_STEPS
    );

    let reqs = h.http.requests();
    let calls: Vec<String> = reqs.iter().skip(1).map(summary).collect();
    assert_eq!(
        calls,
        [
            "POST /b2b/api/v1/partners",
            "POST /b2b/api/v1/channels",
            "PUT /b2b/api/v1/partners/pid-1/b2b-gateways",
            "POST /b2b/api/v1/partner-users",
            "PUT /b2b/api/v1/partners/pid-1/users",
        ]
    );
    for r in reqs.iter().skip(1) {
        assert!(r.url.starts_with(&format!("https://{}/", h.env.b2b_host)));
        assert!(header(r, "Authorization").unwrap().starts_with("Bearer "));
        assert_eq!(
            header(r, "Content-Type").as_deref(),
            Some("application/json")
        );
    }

    assert_eq!(
        body(&reqs[1]),
        serde_json::json!({
            "corporation": {
                "corporationName": "P1 Retail",
                "orgUnitName": "Retail Procurement",
                "routingOff": false,
                "status": "Active"
            },
            "identities": [{ "externalIdType": "DUNS", "externalIdValue": "123456789" }]
        })
    );
    assert_eq!(
        body(&reqs[2]),
        serde_json::json!({
            "channeltype": "HTTP-IN",
            "name": "p1-channel",
            "description": "",
            "partnerSpecific": "false",
            "state": "Active",
            "groups": [{
                "name": "flatFile",
                "fields": [
                    { "name": "keyValuePairs", "value": [] },
                    { "name": "overwrite", "value": "false" }
                ],
                "passwordChange": false
            }]
        })
    );
    assert_eq!(
        body(&reqs[3]),
        serde_json::json!({ "b2bGateways": [{ "id": "ch-1" }] })
    );
    assert_eq!(
        body(&reqs[4]),
        serde_json::json!({ "user_name": "p1user", "password": PASSWORD, "full_name": "Demo User" })
    );
    assert_eq!(
        body(&reqs[5]),
        serde_json::json!({ "users": [{ "id": "user-1", "username": "p1user" }] })
    );

    let status = h.prov.status(&h.partner.id).unwrap();
    assert!(status.complete);
    assert_eq!(status.partner_internal_id.as_deref(), Some("pid-1"));
    assert_eq!(status.channel_id.as_deref(), Some("ch-1"));
    assert_eq!(
        status.channel_endpoint_uri.as_deref(),
        Some("https://b2b.example/ch/p1")
    );
    assert_eq!(status.user_id.as_deref(), Some("user-1"));
    assert!(status.steps.iter().all(|s| s.status == "succeeded"));
}

#[tokio::test]
async fn status_before_any_run_is_all_pending() {
    let h = harness();
    let status = h.prov.status(&h.partner.id).unwrap();
    assert!(!status.complete);
    assert_eq!(status.steps.len(), 5);
    assert!(status.steps.iter().all(|s| s.status == "pending"));
    assert!(status.partner_internal_id.is_none());
    assert!(h.http.requests().is_empty());
}

#[tokio::test]
async fn rerunning_a_complete_partner_makes_no_tenant_calls() {
    let h = harness();
    push_token(&h);
    push_provisioning_ok(&h);
    h.prov
        .provision(&h.partner.id, &CancelToken::new())
        .await
        .unwrap();
    let before = h.http.requests().len();

    let report = h
        .prov
        .provision(&h.partner.id, &CancelToken::new())
        .await
        .unwrap();
    assert!(report.is_complete());
    assert!(outcomes(&report).iter().all(|o| *o == StepOutcome::Skipped));
    assert_eq!(h.http.requests().len(), before);
}

#[tokio::test]
async fn failure_stops_the_run_and_resume_continues_at_the_failed_step() {
    let h = harness();
    push_token(&h);
    h.http.push(201, r#"{"internalId":"pid-1"}"#);
    h.http.push(
        201,
        r#"{"id":"ch-1","endpoint":{"uri":"https://b2b.example/ch/p1"}}"#,
    );
    h.http.push(200, "{}");
    // Creating the user fails; the server echoes the request, password included.
    h.http
        .push(500, &format!(r#"{{"message":"cannot create {PASSWORD}"}}"#));

    let report = h
        .prov
        .provision(&h.partner.id, &CancelToken::new())
        .await
        .unwrap();
    assert!(!report.is_complete());
    assert_eq!(
        outcomes(&report),
        [
            StepOutcome::Succeeded,
            StepOutcome::Succeeded,
            StepOutcome::Succeeded,
            StepOutcome::Failed,
            StepOutcome::NotRun
        ]
    );
    let error = report.steps[3].error.clone().unwrap();
    assert!(error.contains("500"));
    assert!(!error.contains(PASSWORD), "password leaked: {error}");

    let status = h.prov.status(&h.partner.id).unwrap();
    assert!(!status.complete);
    assert_eq!(status.steps[3].status, "failed");
    assert_eq!(status.steps[4].status, "pending");
    assert_eq!(status.partner_internal_id.as_deref(), Some("pid-1"));
    assert!(status.user_id.is_none());

    // Retry: only the last two steps hit the tenant; the token is still cached.
    let seen_before = h.http.requests().len();
    h.http.push(201, r#"{"id":"user-1"}"#);
    h.http.push(200, "{}");
    let report = h
        .prov
        .provision(&h.partner.id, &CancelToken::new())
        .await
        .unwrap();
    assert!(report.is_complete());
    assert_eq!(
        outcomes(&report),
        [
            StepOutcome::Skipped,
            StepOutcome::Skipped,
            StepOutcome::Skipped,
            StepOutcome::Succeeded,
            StepOutcome::Succeeded
        ]
    );
    let new_calls: Vec<String> = h.http.requests()[seen_before..]
        .iter()
        .map(summary)
        .collect();
    assert_eq!(
        new_calls,
        [
            "POST /b2b/api/v1/partner-users",
            "PUT /b2b/api/v1/partners/pid-1/users"
        ]
    );
    // The association reused ids saved in the first run.
    let last = h.http.requests().last().cloned().unwrap();
    assert_eq!(
        body(&last),
        serde_json::json!({ "users": [{ "id": "user-1", "username": "p1user" }] })
    );
}

#[tokio::test]
async fn transport_failure_on_the_first_step_can_be_retried() {
    let h = harness();
    h.http.push_error("could not connect");
    let report = h
        .prov
        .provision(&h.partner.id, &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(report.steps[0].outcome, StepOutcome::Failed);
    assert!(
        report.steps[0]
            .error
            .as_deref()
            .unwrap()
            .contains("could not connect")
    );
    assert_eq!(
        h.prov.status(&h.partner.id).unwrap().steps[0].status,
        "failed"
    );

    push_token(&h);
    push_provisioning_ok(&h);
    let report = h
        .prov
        .provision(&h.partner.id, &CancelToken::new())
        .await
        .unwrap();
    assert!(report.is_complete());
}

#[tokio::test]
async fn missing_ids_in_a_success_response_fail_the_step() {
    let h = harness();
    push_token(&h);
    h.http.push(201, r#"{"somethingElse":1}"#);
    let report = h
        .prov
        .provision(&h.partner.id, &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(report.steps[0].outcome, StepOutcome::Failed);
    assert!(
        report.steps[0]
            .error
            .as_deref()
            .unwrap()
            .contains("internalId")
    );
}

#[tokio::test]
async fn ids_with_path_characters_are_rejected_before_use() {
    let h = harness();
    push_token(&h);
    h.http.push(201, r#"{"internalId":"../admin"}"#);
    let report = h
        .prov
        .provision(&h.partner.id, &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(report.steps[0].outcome, StepOutcome::Failed);
    assert_eq!(h.http.requests().len(), 2);
}

#[tokio::test]
async fn cancel_before_start_runs_nothing() {
    let h = harness();
    let cancel = CancelToken::new();
    cancel.cancel();
    let report = h.prov.provision(&h.partner.id, &cancel).await.unwrap();
    assert!(report.cancelled);
    assert!(h.http.requests().is_empty());
}

#[tokio::test]
async fn provisioning_requires_known_partner_and_secrets() {
    let h = harness();
    let unknown = crate::ids::PartnerId::generate();
    assert!(matches!(
        h.prov.provision(&unknown, &CancelToken::new()).await,
        Err(crate::Error::NotFound(_))
    ));

    // Missing stored password: the user-creation step fails with a clear text.
    crate::secrets::SecretStore::delete(&*h.f.secrets, h.partner.password_secret.as_ref().unwrap())
        .unwrap();
    push_token(&h);
    h.http.push(201, r#"{"internalId":"pid-1"}"#);
    h.http.push(
        201,
        r#"{"id":"ch-1","endpoint":{"uri":"https://b2b.example/ch"}}"#,
    );
    h.http.push(200, "{}");
    let report = h
        .prov
        .provision(&h.partner.id, &CancelToken::new())
        .await
        .unwrap();
    assert!(!report.is_complete());
    assert_eq!(report.steps[3].outcome, StepOutcome::Failed);
    let text = format!("{report:?}");
    assert!(text.contains("password"));
    assert!(!text.contains(PASSWORD));
}

#[tokio::test]
async fn two_partners_provision_independently() {
    let h = harness();
    let second =
        h.f.partners
            .create(
                &h.env.id,
                &partner_input("P2 Supplier", "987654321", "p2user", "p2-channel"),
                &secret("An0therPassw0rd"),
            )
            .unwrap();
    push_token(&h);
    push_provisioning_ok(&h);
    h.http.push(201, r#"{"internalId":"pid-2"}"#);
    h.http.push(
        201,
        r#"{"id":"ch-2","endpoint":{"uri":"https://b2b.example/ch/p2"}}"#,
    );
    h.http.push(200, "{}");
    h.http.push(201, r#"{"id":"user-2"}"#);
    h.http.push(200, "{}");

    assert!(
        h.prov
            .provision(&h.partner.id, &CancelToken::new())
            .await
            .unwrap()
            .is_complete()
    );
    assert!(
        h.prov
            .provision(&second.id, &CancelToken::new())
            .await
            .unwrap()
            .is_complete()
    );

    let a = h.prov.status(&h.partner.id).unwrap();
    let b = h.prov.status(&second.id).unwrap();
    assert_eq!(a.partner_internal_id.as_deref(), Some("pid-1"));
    assert_eq!(b.partner_internal_id.as_deref(), Some("pid-2"));
    assert_eq!(
        b.channel_endpoint_uri.as_deref(),
        Some("https://b2b.example/ch/p2")
    );
    // One token for both partners.
    let tokens = h
        .http
        .requests()
        .iter()
        .filter(|r| r.url.contains("/apikeys/token"))
        .count();
    assert_eq!(tokens, 1);
}

#[tokio::test]
async fn no_secret_appears_in_events_records_or_debug_output() {
    let h = harness();
    push_token(&h);
    h.http.push(201, r#"{"internalId":"pid-1"}"#);
    h.http.push(
        201,
        r#"{"id":"ch-1","endpoint":{"uri":"https://b2b.example/ch/p1"}}"#,
    );
    h.http.push(200, "{}");
    h.http.push(400, &format!(r#"{{"password":"{PASSWORD}"}}"#));
    h.prov
        .provision(&h.partner.id, &CancelToken::new())
        .await
        .unwrap();

    let mut text = format!("{:?}", h.f.events.events());
    text.push_str(&format!("{:?}", h.http.requests()));
    text.push_str(&serde_json::to_string(&h.prov.status(&h.partner.id).unwrap()).unwrap());
    text.push_str(&format!(
        "{:?}",
        crate::steps::StepStore::list(&*h.f.db, &crate::partners::partner_scope(&h.partner.id))
            .unwrap()
    ));
    for secret in [PASSWORD, API_KEY, &test_jwt(LONG_LIVED)] {
        assert!(!text.contains(secret), "leaked {secret}");
    }
    assert!(
        h.f.events
            .kinds()
            .contains(&"partner.provision.step.failed".to_string())
    );
    assert!(
        h.f.events
            .kinds()
            .contains(&"partner.provision_failed".to_string())
    );
}

// ----------------------------------------------------------------- documents

fn doc_ids(report: &crate::b2b::InstallReport) -> Vec<(String, DocumentOutcome)> {
    report
        .documents
        .iter()
        .map(|d| (d.transaction_code.clone(), d.outcome))
        .collect()
}

#[tokio::test]
async fn installing_a_pack_posts_each_document_in_sequence() {
    let h = harness();
    push_token(&h);
    for id in [
        "d840", "d843", "d850", "d855", "d860", "d865", "d856", "d810", "d820",
    ] {
        h.http.push(201, &format!(r#"{{"id":"{id}"}}"#));
    }
    let report = h
        .docs
        .install(&h.env.id, "retail", None, &CancelToken::new())
        .await
        .unwrap();
    assert!(report.is_complete());
    let codes: Vec<&str> = report
        .documents
        .iter()
        .map(|d| d.transaction_code.as_str())
        .collect();
    assert_eq!(
        codes,
        [
            "840", "843", "850", "855", "860", "865", "856", "810", "820"
        ]
    );

    let reqs = h.http.requests();
    assert_eq!(summary(&reqs[1]), "POST /b2b/api/v1/documents");
    assert_eq!(
        body(&reqs[1]),
        serde_json::json!({
            "customTransaction": "",
            "referenceDocument": null,
            "standard": "X12",
            "status": "Active",
            "transaction": "840",
            "type": { "id": "edi", "displayValue": "EDI" },
            "version": "4010",
            "referenceType": "WMINT"
        })
    );
    let saved = h.docs.list_installed(&h.env.id).unwrap();
    assert_eq!(saved.len(), 9);
    assert_eq!(
        saved
            .iter()
            .find(|d| d.transaction_code == "850")
            .unwrap()
            .document_type_id,
        "d850"
    );
}

#[tokio::test]
async fn existing_document_is_detected_by_409_and_confirmed_with_get() {
    let h = harness();
    push_token(&h);
    h.http.push(409, r#"{"error":"conflict"}"#);
    h.http.push(200, "{}");
    let report = h
        .docs
        .install(
            &h.env.id,
            "retail",
            Some(&["850".to_string()]),
            &CancelToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        doc_ids(&report),
        [("850".into(), DocumentOutcome::AlreadyExisted)]
    );
    let reqs = h.http.requests();
    assert_eq!(
        summary(&reqs[2]),
        "GET /b2b/api/v1/documents/WMX12-4010--850---------"
    );
    assert!(reqs[2].body.is_none());
    assert_eq!(
        h.docs.list_installed(&h.env.id).unwrap()[0].document_type_id,
        "WMX12-4010--850---------"
    );
}

#[tokio::test]
async fn existing_document_is_detected_from_the_message_text() {
    for text in [
        "Document ALREADY EXISTS",
        "already installed",
        "Duplicate entry",
    ] {
        let h = harness();
        push_token(&h);
        h.http.push(400, &format!(r#"{{"message":"{text}"}}"#));
        h.http.push(200, "{}");
        let report = h
            .docs
            .install(
                &h.env.id,
                "retail",
                Some(&["810".to_string()]),
                &CancelToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            doc_ids(&report),
            [("810".into(), DocumentOutcome::AlreadyExisted)],
            "{text}"
        );
    }
}

#[tokio::test]
async fn other_errors_are_not_treated_as_existing() {
    let h = harness();
    push_token(&h);
    h.http.push(400, r#"{"message":"invalid standard"}"#);
    let report = h
        .docs
        .install(
            &h.env.id,
            "retail",
            Some(&["850".to_string()]),
            &CancelToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(report.documents[0].outcome, DocumentOutcome::Failed);
    assert!(
        report.documents[0]
            .error
            .as_deref()
            .unwrap()
            .contains("400")
    );
    assert!(h.docs.list_installed(&h.env.id).unwrap().is_empty());
    assert_eq!(h.http.requests().len(), 2, "no GET fallback");
}

#[tokio::test]
async fn failed_lookup_after_a_conflict_fails_the_document() {
    let h = harness();
    push_token(&h);
    h.http.push(409, "{}");
    h.http.push(404, r#"{"message":"missing"}"#);
    let report = h
        .docs
        .install(
            &h.env.id,
            "retail",
            Some(&["850".to_string()]),
            &CancelToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(report.documents[0].outcome, DocumentOutcome::Failed);
    assert!(h.docs.list_installed(&h.env.id).unwrap().is_empty());
}

#[tokio::test]
async fn failure_stops_the_run_and_a_rerun_resumes_and_is_idempotent() {
    let h = harness();
    push_token(&h);
    h.http.push(201, r#"{"id":"d840"}"#);
    h.http.push(500, "boom");
    let sel: Vec<String> = ["840", "843", "850"].map(String::from).to_vec();
    let report = h
        .docs
        .install(&h.env.id, "retail", Some(&sel), &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(
        doc_ids(&report),
        [
            ("840".into(), DocumentOutcome::Installed),
            ("843".into(), DocumentOutcome::Failed),
            ("850".into(), DocumentOutcome::NotRun)
        ]
    );
    assert!(!report.is_complete());
    assert_eq!(h.docs.list_installed(&h.env.id).unwrap().len(), 1);

    h.http.push(201, r#"{"id":"d843"}"#);
    h.http.push(201, r#"{"id":"d850"}"#);
    let report = h
        .docs
        .install(&h.env.id, "retail", Some(&sel), &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(
        doc_ids(&report),
        [
            ("840".into(), DocumentOutcome::AlreadyInstalled),
            ("843".into(), DocumentOutcome::Installed),
            ("850".into(), DocumentOutcome::Installed)
        ]
    );
    let calls_after_resume = h.http.requests().len();

    // Third run: nothing to do, no tenant calls at all.
    let report = h
        .docs
        .install(&h.env.id, "retail", Some(&sel), &CancelToken::new())
        .await
        .unwrap();
    assert!(report.is_complete());
    assert!(
        report
            .documents
            .iter()
            .all(|d| d.outcome == DocumentOutcome::AlreadyInstalled)
    );
    assert_eq!(h.http.requests().len(), calls_after_resume);
}

#[tokio::test]
async fn a_code_shared_by_packs_is_installed_once_per_environment() {
    let h = harness();
    push_token(&h);
    h.http.push(201, r#"{"id":"d820"}"#);
    h.docs
        .install(
            &h.env.id,
            "retail",
            Some(&["820".to_string()]),
            &CancelToken::new(),
        )
        .await
        .unwrap();
    let calls = h.http.requests().len();
    let report = h
        .docs
        .install(
            &h.env.id,
            "healthcare",
            Some(&["820".to_string()]),
            &CancelToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        doc_ids(&report),
        [("820".into(), DocumentOutcome::AlreadyInstalled)]
    );
    assert_eq!(h.http.requests().len(), calls);
}

#[tokio::test]
async fn selection_is_validated() {
    let h = harness();
    let cancel = CancelToken::new();
    let bad = |codes: &[&str]| codes.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    for codes in [bad(&["837"]), bad(&["85"]), bad(&["8a0"]), bad(&["../x"])] {
        assert!(
            matches!(
                h.docs
                    .install(&h.env.id, "retail", Some(&codes), &cancel)
                    .await,
                Err(crate::Error::Validation(_))
            ),
            "{codes:?}"
        );
    }
    assert!(matches!(
        h.docs.install(&h.env.id, "nope", None, &cancel).await,
        Err(crate::Error::NotFound(_))
    ));
    // Duplicates in a selection are collapsed.
    push_token(&h);
    h.http.push(201, r#"{"id":"d850"}"#);
    let report = h
        .docs
        .install(&h.env.id, "retail", Some(&bad(&["850", "850"])), &cancel)
        .await
        .unwrap();
    assert_eq!(report.documents.len(), 1);
    assert!(
        h.http
            .requests()
            .iter()
            .filter(|r| r.url.ends_with("/documents"))
            .count()
            == 1
    );
}

#[tokio::test]
async fn cancelled_install_stops_before_the_next_document() {
    let h = harness();
    let cancel = CancelToken::new();
    cancel.cancel();
    let report = h
        .docs
        .install(&h.env.id, "retail", None, &cancel)
        .await
        .unwrap();
    assert!(report.cancelled);
    assert!(
        report
            .documents
            .iter()
            .all(|d| d.outcome == DocumentOutcome::NotRun)
    );
    assert!(h.http.requests().is_empty());
}
