//! Tests for the submission engine over the in-memory fakes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::*;
use crate::b2b::api::test_jwt;
use crate::environments::Environment;
use crate::partners::Partner;
use crate::partners::partner_scope;
use crate::steps::{StepRecord, StepStatus, StepStore};
use crate::testing::FakeHttp;
use crate::tests::{Fixture, env_input, partner_input, secret};

const API_KEY: &str = "api-key-value-12345";
const P1_PASSWORD: &str = "P1-S3cret-Pass!";
const P2_PASSWORD: &str = "P2-Other-S3cret!";
const P1_DUNS: &str = "123456789";
const P2_DUNS: &str = "987654321";
const P1_URL: &str = "https://b2b.example/ch/p1";
const P2_URL: &str = "https://b2b.example/ch/p2";

struct Harness {
    f: Fixture,
    http: Arc<FakeHttp>,
    env: Environment,
    p1: Partner,
    p2: Partner,
    prov: Arc<ProvisioningService>,
}

impl Harness {
    fn service(&self) -> SubmissionService {
        self.service_without_delay_override()
            .with_delay(Duration::ZERO)
    }

    fn service_without_delay_override(&self) -> SubmissionService {
        SubmissionService::new(SubmissionDeps {
            http: self.http.clone(),
            catalog: Arc::new(Catalog::built_in()),
            envs: self.f.db.clone(),
            partners: self.f.db.clone(),
            counters: self.f.db.clone(),
            submissions: self.f.db.clone(),
            prefs: self.f.db.clone(),
            secrets: self.f.secrets.clone(),
            provisioning: self.prov.clone(),
            clock: self.f.clock.clone(),
            events: self.f.events.clone(),
        })
    }

    fn request(&self, codes: &[&str]) -> SubmitRequest {
        SubmitRequest {
            environment_id: self.env.id.clone(),
            pack_id: "retail".into(),
            codes: Some(codes.iter().map(ToString::to_string).collect()),
            p1_partner_id: self.p1.id.clone(),
            p2_partner_id: self.p2.id.clone(),
            trace_overrides: BTreeMap::new(),
        }
    }

    fn preview_request(&self, codes: &[&str]) -> PreviewRequest {
        PreviewRequest {
            environment_id: self.env.id.clone(),
            pack_id: "retail".into(),
            codes: Some(codes.iter().map(ToString::to_string).collect()),
            p1: Party::Saved {
                partner_id: self.p1.id.clone(),
            },
            p2: Party::Saved {
                partner_id: self.p2.id.clone(),
            },
            trace_overrides: BTreeMap::new(),
        }
    }

    /// Provisions a partner against the fake tenant (the MCSP token is
    /// cached after the first call).
    async fn provision(&self, partner: &Partner, endpoint: &str, with_token: bool) {
        if with_token {
            self.http.push(
                200,
                &format!(r#"{{"token":"{}"}}"#, test_jwt(2_000_000_000)),
            );
        }
        self.http.push(201, r#"{"internalId":"pid"}"#);
        self.http.push(
            201,
            &format!(r#"{{"id":"ch","endpoint":{{"uri":"{endpoint}"}}}}"#),
        );
        self.http.push(200, "{}");
        self.http.push(201, r#"{"id":"user"}"#);
        self.http.push(200, "{}");
        let report = self
            .prov
            .provision(&partner.id, &CancelToken::new())
            .await
            .unwrap();
        assert!(report.is_complete());
    }

    /// Saved submissions, oldest control number first.
    fn submissions(&self) -> Vec<Submission> {
        let mut v = self.f.db.list_recent(&self.env.id, 100).unwrap();
        v.sort_by_key(|s| s.isa_control);
        v
    }

    fn counters(&self) -> crate::counters::Counters {
        self.f.db.peek(&self.env.id).unwrap()
    }
}

async fn harness() -> Harness {
    let f = Fixture::new();
    let env = f.envs.create(&env_input("Dev"), &secret(API_KEY)).unwrap();
    let p1 = f
        .partners
        .create(
            &env.id,
            &partner_input("Buyer", P1_DUNS, "p1user", "p1-channel"),
            &secret(P1_PASSWORD),
        )
        .unwrap();
    let p2 = f
        .partners
        .create(
            &env.id,
            &partner_input("Supplier", P2_DUNS, "p2user", "p2-channel"),
            &secret(P2_PASSWORD),
        )
        .unwrap();
    let http = FakeHttp::new();
    let api = crate::b2b::B2bApi::new(
        http.clone(),
        f.secrets.clone(),
        f.db.clone(),
        f.clock.clone(),
    );
    let prov = Arc::new(ProvisioningService::new(
        api,
        f.db.clone(),
        f.db.clone(),
        f.db.clone(),
        f.secrets.clone(),
        f.clock.clone(),
        f.events.clone(),
    ));
    let h = Harness {
        f,
        http,
        env,
        p1,
        p2,
        prov,
    };
    h.provision(&h.p1, P1_URL, true).await;
    h.provision(&h.p2, P2_URL, false).await;
    // Forget the provisioning traffic so tests only see submissions.
    h.http.clear_requests();
    h
}

fn header(r: &HttpRequest, name: &str) -> Option<String> {
    r.headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.expose().to_string())
}

fn basic(user: &str, password: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!("{user}:{password}")))
}

fn isa_field(payload: &str, index: usize) -> String {
    payload
        .lines()
        .next()
        .unwrap()
        .split('*')
        .nth(index)
        .unwrap()
        .trim()
        .to_string()
}

/// ISA13.
fn isa_control_of(payload: &str) -> String {
    isa_field(payload, 13)
}

/// ISA06.
fn isa_sender_of(payload: &str) -> String {
    isa_field(payload, 6)
}

fn sent_body(r: &HttpRequest) -> String {
    String::from_utf8(r.body.clone().unwrap()).unwrap()
}

#[tokio::test]
async fn single_document_is_posted_to_the_sender_channel_with_basic_auth() {
    let h = harness().await;
    let mut env = h.env.clone();
    env.ca_bundle_path = Some("/etc/ssl/private-ca.pem".into());
    EnvironmentRepository::save(&*h.f.db, &env).unwrap();
    h.http.push(200, "accepted");

    let report = h
        .service()
        .submit_document(&h.request(&[]), "850", &CancelToken::new())
        .await
        .unwrap();

    assert!(report.is_complete());
    assert_eq!(report.documents.len(), 1);
    assert_eq!(report.documents[0].http_status, Some(200));
    let requests = h.http.requests();
    assert_eq!(requests.len(), 1, "no token request for channel posts");
    let r = &requests[0];
    assert_eq!(r.method, Method::Post);
    assert_eq!(r.url, P1_URL);
    assert_eq!(
        header(r, "Authorization").unwrap(),
        basic("p1user", P1_PASSWORD)
    );
    assert_eq!(header(r, "Content-Type").unwrap(), "application/EDI");
    assert_eq!(r.timeout, Duration::from_secs(120));
    assert_eq!(r.ca_bundle.as_deref(), Some("/etc/ssl/private-ca.pem"));
    let sent = sent_body(r);
    assert_eq!(isa_control_of(&sent), "000000001");
    assert_eq!(isa_sender_of(&sent), P1_DUNS);
    assert!(sent.contains("PO-20250101-0001"), "{sent}");

    let saved = h.submissions();
    assert_eq!(saved.len(), 1);
    let s = &saved[0];
    assert_eq!(s.payload.as_deref(), Some(sent.as_str()));
    assert_eq!(s.http_status, Some(200));
    assert_eq!(s.result, "success");
    assert_eq!(s.response_excerpt.as_deref(), Some("accepted"));
    assert!(s.duration_ms.is_some());
    assert_eq!((s.isa_control, s.gs_control), (1, 1));
    assert_eq!(s.domain, "retail");
    assert_eq!(s.transaction_code, "850");
    assert_eq!(s.sender_partner_id.as_ref(), Some(&h.p1.id));
    assert_eq!(s.receiver_partner_id.as_ref(), Some(&h.p2.id));
    assert_eq!(s.trace_values["PO_NUMBER"], "PO-20250101-0001");
    assert!(s.trace_values.get("INVOICE_NUMBER").is_none());
}

#[tokio::test]
async fn sequence_routes_by_sender_with_consecutive_control_numbers() {
    let h = harness().await;
    h.http.push(200, "ok");
    h.http.push(202, "ok");
    h.http.push(200, "ok");

    let report = h
        .service()
        .submit(&h.request(&["850", "855", "860"]), &CancelToken::new())
        .await
        .unwrap();

    assert!(report.is_complete());
    let nums: Vec<_> = report
        .documents
        .iter()
        .map(|d| (d.isa_control.unwrap(), d.gs_control.unwrap()))
        .collect();
    assert_eq!(nums, vec![(1, 1), (2, 2), (3, 3)]);

    let requests = h.http.requests();
    let urls: Vec<&str> = requests.iter().map(|r| r.url.as_str()).collect();
    assert_eq!(urls, vec![P1_URL, P2_URL, P1_URL]);
    assert_eq!(
        header(&requests[1], "Authorization").unwrap(),
        basic("p2user", P2_PASSWORD)
    );
    for (r, expected_sender) in requests.iter().zip([P1_DUNS, P2_DUNS, P1_DUNS]) {
        assert_eq!(isa_sender_of(&sent_body(r)), expected_sender);
    }
    let c = h.counters();
    assert_eq!((c.isa_next, c.gs_next, c.run_count), (4, 4, 1));
    assert_eq!(h.submissions().len(), 3);
}

#[tokio::test]
async fn whole_pack_is_sent_in_sequence_when_no_codes_are_given() {
    let h = harness().await;
    for _ in 0..9 {
        h.http.push(200, "ok");
    }
    let mut request = h.request(&[]);
    request.codes = None;
    let report = h
        .service()
        .submit(&request, &CancelToken::new())
        .await
        .unwrap();
    let codes: Vec<&str> = report
        .documents
        .iter()
        .map(|d| d.transaction_code.as_str())
        .collect();
    assert_eq!(
        codes,
        vec![
            "840", "843", "850", "855", "860", "865", "856", "810", "820"
        ]
    );
    assert!(report.is_complete());
}

#[tokio::test]
async fn control_numbers_stay_unique_across_runs() {
    let h = harness().await;
    for _ in 0..4 {
        h.http.push(200, "ok");
    }
    let service = h.service();
    service
        .submit(&h.request(&["850", "855"]), &CancelToken::new())
        .await
        .unwrap();
    service
        .submit(&h.request(&["850", "855"]), &CancelToken::new())
        .await
        .unwrap();

    let saved = h.submissions();
    let isa: Vec<u32> = saved.iter().map(|s| s.isa_control).collect();
    let gs: Vec<u32> = saved.iter().map(|s| s.gs_control).collect();
    assert_eq!(isa, vec![1, 2, 3, 4]);
    assert_eq!(gs, vec![1, 2, 3, 4]);
    // The second run uses the next run number in its trace values.
    assert_eq!(saved[2].trace_values["PO_NUMBER"], "PO-20250101-0002");
    assert_eq!(h.counters().run_count, 2);
}

#[tokio::test]
async fn concurrent_runs_never_share_a_control_number() {
    let h = harness().await;
    for _ in 0..6 {
        h.http.push(200, "ok");
    }
    let service = h.service();
    let a = h.request(&["850", "855", "860"]);
    let b = h.request(&["850", "855", "860"]);
    let cancel = CancelToken::new();
    let (ra, rb) = tokio::join!(service.submit(&a, &cancel), service.submit(&b, &cancel));

    let mut all: Vec<u32> = ra
        .unwrap()
        .documents
        .iter()
        .chain(rb.unwrap().documents.iter())
        .map(|d| d.isa_control.unwrap())
        .collect();
    all.sort_unstable();
    assert_eq!(all, vec![1, 2, 3, 4, 5, 6]);
}

#[tokio::test]
async fn sequence_stops_at_the_first_failure() {
    let h = harness().await;
    h.http.push(200, "ok");
    h.http.push(500, "boom\nline two");
    h.http.push(200, "must not be requested");

    let report = h
        .service()
        .submit(&h.request(&["850", "855", "860"]), &CancelToken::new())
        .await
        .unwrap();

    let outcomes: Vec<_> = report.documents.iter().map(|d| d.outcome).collect();
    assert_eq!(
        outcomes,
        vec![
            SubmissionOutcome::Submitted,
            SubmissionOutcome::Failed,
            SubmissionOutcome::NotRun
        ]
    );
    assert!(!report.is_complete());
    assert_eq!(report.documents[1].http_status, Some(500));
    assert!(
        report.documents[1]
            .error
            .as_deref()
            .unwrap()
            .contains("boom line two")
    );
    assert_eq!(report.documents[2].isa_control, None);
    assert_eq!(h.http.requests().len(), 2);

    let saved = h.submissions();
    assert_eq!(saved.len(), 2, "the not-run document leaves no record");
    assert_eq!(saved[0].result, "success");
    assert_eq!(saved[1].result, "failed");
    assert_eq!(saved[1].http_status, Some(500));
    assert_eq!(saved[1].response_excerpt.as_deref(), Some("boom line two"));

    // The next run continues after the numbers that were reserved (gaps are
    // allowed in X12, reuse is not).
    h.http.push(200, "ok");
    let next = h
        .service()
        .submit(&h.request(&["850"]), &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(next.documents[0].isa_control, Some(4));
}

#[tokio::test]
async fn transport_failure_is_recorded_and_stops_the_run() {
    let h = harness().await;
    h.http.push_error("connection reset");
    h.http.push(200, "unused");

    let report = h
        .service()
        .submit(&h.request(&["850", "855"]), &CancelToken::new())
        .await
        .unwrap();

    assert_eq!(report.documents[0].outcome, SubmissionOutcome::Failed);
    assert_eq!(report.documents[0].http_status, None);
    assert_eq!(report.documents[1].outcome, SubmissionOutcome::NotRun);
    let saved = h.submissions();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].result, "failed");
    assert_eq!(saved[0].http_status, None);
    assert_eq!(
        saved[0].response_excerpt.as_deref(),
        Some("connection reset")
    );
}

#[tokio::test]
async fn preview_renders_without_touching_counters_http_or_history() {
    let h = harness().await;
    let before = h.counters();

    let preview = h
        .service()
        .preview(&h.preview_request(&["850", "855"]))
        .unwrap();

    assert_eq!(h.counters(), before);
    assert!(h.http.requests().is_empty());
    assert!(h.submissions().is_empty());
    assert_eq!(preview.documents.len(), 2);
    assert_eq!(preview.documents[0].sender, "p1");
    assert_eq!(preview.documents[1].sender, "p2");
    assert_eq!(
        (
            preview.documents[0].isa_control,
            preview.documents[1].isa_control
        ),
        (1, 2)
    );
    assert_eq!(isa_control_of(&preview.documents[1].payload), "000000002");
    assert_eq!(isa_sender_of(&preview.documents[1].payload), P2_DUNS);
    assert_eq!(preview.run_count, 1);

    // What was previewed is what the next submission sends.
    h.http.push(200, "ok");
    h.http.push(200, "ok");
    h.service()
        .submit(&h.request(&["850", "855"]), &CancelToken::new())
        .await
        .unwrap();
    let sent: Vec<String> = h.http.requests().iter().map(sent_body).collect();
    assert_eq!(sent[0], preview.documents[0].payload);
    assert_eq!(sent[1], preview.documents[1].payload);
}

#[tokio::test]
async fn preview_accepts_manually_entered_ids_and_rejects_bad_ones() {
    let h = harness().await;
    let mut request = h.preview_request(&["850"]);
    request.p1 = Party::Manual {
        id_value: "555000111".into(),
    };
    request.p2 = Party::Manual {
        id_value: "555000222".into(),
    };
    let preview = h.service().preview(&request).unwrap();
    assert_eq!(isa_sender_of(&preview.documents[0].payload), "555000111");

    request.p1 = Party::Manual {
        id_value: "12ab".into(),
    };
    assert!(matches!(
        h.service().preview(&request),
        Err(Error::Validation(_))
    ));
}

#[tokio::test]
async fn unprovisioned_partner_can_be_previewed_but_not_submitted() {
    let h = harness().await;
    let p3 =
        h.f.partners
            .create(
                &h.env.id,
                &partner_input("Third", "111222333", "p3user", "p3-channel"),
                &secret("P3-S3cret-Pass!"),
            )
            .unwrap();
    let mut request = h.preview_request(&["850"]);
    request.p1 = Party::Saved {
        partner_id: p3.id.clone(),
    };
    assert!(h.service().preview(&request).is_ok());

    let mut submit = h.request(&["850"]);
    submit.p1_partner_id = p3.id;
    let err = h
        .service()
        .submit(&submit, &CancelToken::new())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err}");
    assert!(h.http.requests().is_empty());
    assert_eq!(h.counters().run_count, 0);
}

#[tokio::test]
async fn trace_overrides_replace_generated_values_and_are_validated() {
    let h = harness().await;
    h.http.push(200, "ok");
    let mut request = h.request(&["850"]);
    request
        .trace_overrides
        .insert("PO_NUMBER".into(), "PO-MINE-7".into());
    h.service()
        .submit(&request, &CancelToken::new())
        .await
        .unwrap();
    let sent = sent_body(&h.http.requests()[0]);
    assert!(sent.contains("BEG*00*SA*PO-MINE-7**"), "{sent}");
    assert_eq!(h.submissions()[0].trace_values["PO_NUMBER"], "PO-MINE-7");

    for (key, value) in [("NOPE", "x"), ("PO_NUMBER", "a*b~c"), ("PO_NUMBER", "")] {
        let mut bad = h.request(&["850"]);
        bad.trace_overrides.insert(key.into(), value.into());
        assert!(matches!(
            h.service().submit(&bad, &CancelToken::new()).await,
            Err(Error::Validation(_))
        ));
    }
    assert_eq!(h.http.requests().len(), 1);
}

#[tokio::test]
async fn invalid_requests_fail_before_anything_is_reserved_or_posted() {
    let h = harness().await;
    let service = h.service();
    let cancel = CancelToken::new();

    let mut unknown_pack = h.request(&["850"]);
    unknown_pack.pack_id = "nope".into();
    assert!(matches!(
        service.submit(&unknown_pack, &cancel).await,
        Err(Error::NotFound("domain pack"))
    ));
    for codes in [vec!["999"], vec!["85"], vec![]] {
        let request = h.request(&codes);
        assert!(matches!(
            service.submit(&request, &cancel).await,
            Err(Error::Validation(_))
        ));
    }
    let mut unknown_env = h.request(&["850"]);
    unknown_env.environment_id = EnvironmentId::generate();
    assert!(matches!(
        service.submit(&unknown_env, &cancel).await,
        Err(Error::NotFound("environment"))
    ));
    let mut unknown_partner = h.request(&["850"]);
    unknown_partner.p2_partner_id = PartnerId::generate();
    assert!(matches!(
        service.submit(&unknown_partner, &cancel).await,
        Err(Error::NotFound("partner"))
    ));

    assert!(h.http.requests().is_empty());
    let c = h.counters();
    assert_eq!((c.isa_next, c.gs_next, c.run_count), (1, 1, 0));
}

#[tokio::test]
async fn partner_of_another_environment_is_rejected() {
    let h = harness().await;
    let other =
        h.f.envs
            .create(&env_input("Other"), &secret(API_KEY))
            .unwrap();
    let foreign =
        h.f.partners
            .create(
                &other.id,
                &partner_input("Foreign", "444555666", "fuser", "f-channel"),
                &secret("F-S3cret-Pass!"),
            )
            .unwrap();
    let mut request = h.request(&["850"]);
    request.p1_partner_id = foreign.id;
    assert!(matches!(
        h.service().submit(&request, &CancelToken::new()).await,
        Err(Error::Validation(_))
    ));
}

#[test]
fn channel_url_adds_https_and_refuses_plain_http() {
    assert_eq!(
        channel_url("b2b.example/x").unwrap(),
        "https://b2b.example/x"
    );
    assert_eq!(
        channel_url(" HTTPS://b2b.example/x ").unwrap(),
        "HTTPS://b2b.example/x"
    );
    assert!(channel_url("http://b2b.example/x").is_err());
    assert!(channel_url("").is_err());
    assert!(channel_url("b2b.example/a b").is_err());
}

#[tokio::test]
async fn endpoint_without_scheme_is_posted_over_https() {
    let h = harness().await;
    StepStore::save(
        &*h.f.db,
        &partner_scope(&h.p1.id),
        &StepRecord {
            name: "create_channel".into(),
            status: StepStatus::Succeeded,
            output: json!({ "channel_id": "ch", "endpoint_uri": "b2b.example/bare" }),
            error: None,
            updated_at: h.f.clock.now(),
        },
    )
    .unwrap();
    h.http.push(200, "ok");
    h.service()
        .submit(&h.request(&["850"]), &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(h.http.requests()[0].url, "https://b2b.example/bare");
}

#[tokio::test]
async fn inter_document_delay_applies_between_documents_only() {
    let h = harness().await;
    h.http.push(200, "ok");
    h.http.push(200, "ok");
    let slow = h.service().with_delay(Duration::from_millis(250));
    let started = Instant::now();
    slow.submit(&h.request(&["850", "855"]), &CancelToken::new())
        .await
        .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(250));

    // A single document is the last one, so no delay follows it.
    h.http.push(200, "ok");
    let slower = h.service().with_delay(Duration::from_secs(30));
    let started = Instant::now();
    slower
        .submit(&h.request(&["850"]), &CancelToken::new())
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn delay_and_timeout_come_from_preferences_by_default() {
    let h = harness().await;
    h.f.db
        .set(&crate::preferences::Preferences {
            inter_document_delay_secs: 0,
            request_timeout_secs: 30,
            payload_timeout_secs: 45,
        })
        .unwrap();
    h.http.push(200, "ok");
    h.http.push(200, "ok");
    h.service_without_delay_override()
        .submit(&h.request(&["850", "855"]), &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(h.http.requests()[0].timeout, Duration::from_secs(45));
}

#[tokio::test]
async fn cancelled_before_start_changes_nothing() {
    let h = harness().await;
    let cancel = CancelToken::new();
    cancel.cancel();
    let report = h
        .service()
        .submit(&h.request(&["850", "855"]), &cancel)
        .await
        .unwrap();
    assert!(report.cancelled);
    assert!(
        report
            .documents
            .iter()
            .all(|d| d.outcome == SubmissionOutcome::NotRun)
    );
    assert!(h.http.requests().is_empty());
    let c = h.counters();
    assert_eq!((c.isa_next, c.run_count), (1, 0));
    assert!(
        h.f.events
            .kinds()
            .contains(&"submission.cancelled".to_string())
    );
}

#[tokio::test]
async fn cancel_during_the_delay_stops_before_the_next_document() {
    let h = harness().await;
    h.http.push(200, "ok");
    h.http.push(200, "unused");
    let service = h.service().with_delay(Duration::from_secs(30));
    let request = h.request(&["850", "855"]);
    let cancel = CancelToken::new();
    let canceller = cancel.clone();
    let started = Instant::now();
    let (report, ()) = tokio::join!(service.submit(&request, &cancel), async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        canceller.cancel();
    });
    let report = report.unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(report.cancelled);
    assert_eq!(report.documents[0].outcome, SubmissionOutcome::Submitted);
    assert_eq!(report.documents[1].outcome, SubmissionOutcome::NotRun);
    assert_eq!(h.http.requests().len(), 1);
}

#[tokio::test]
async fn events_report_rendered_posting_and_result_without_secrets() {
    let h = harness().await;
    // The channel echoes the credentials back in its error.
    let echo = format!(
        "bad login {P1_PASSWORD} header {}",
        basic("p1user", P1_PASSWORD)
    );
    h.http.push(401, &echo);
    h.http
        .push_error(&format!("tls failure near {P2_PASSWORD}"));

    let service = h.service();
    let failed = service
        .submit(&h.request(&["850"]), &CancelToken::new())
        .await
        .unwrap();
    let failed_transport = service
        .submit(&h.request(&["855"]), &CancelToken::new())
        .await
        .unwrap();

    let kinds = h.f.events.kinds();
    let submission_kinds: Vec<&str> = kinds
        .iter()
        .map(String::as_str)
        .filter(|k| k.starts_with("submission."))
        .collect();
    assert_eq!(
        submission_kinds,
        vec![
            "submission.rendered",
            "submission.posting",
            "submission.result",
            "submission.rendered",
            "submission.posting",
            "submission.result"
        ]
    );
    let result =
        h.f.events
            .events()
            .into_iter()
            .find(|e| e.kind == "submission.result")
            .unwrap();
    assert_eq!(result.level, EventLevel::Error);
    assert_eq!(result.data["http_status"], 401);
    assert_eq!(result.data["transaction_code"], "850");

    // Everything observable: events, reports, stored rows and Debug output.
    let mut observed = String::new();
    observed.push_str(&serde_json::to_string(&h.f.events.events()).unwrap());
    observed.push_str(&serde_json::to_string(&failed).unwrap());
    observed.push_str(&serde_json::to_string(&failed_transport).unwrap());
    observed.push_str(&serde_json::to_string(&h.submissions()).unwrap());
    observed.push_str(&format!("{:?}", h.submissions()));
    observed.push_str(&format!("{:?}", h.http.requests()));
    observed.push_str(&format!("{failed:?}{failed_transport:?}"));
    for secret_text in [
        P1_PASSWORD,
        P2_PASSWORD,
        API_KEY,
        &STANDARD.encode(format!("p1user:{P1_PASSWORD}")),
        &STANDARD.encode(format!("p2user:{P2_PASSWORD}")),
    ] {
        assert!(!observed.contains(secret_text), "leaked {secret_text}");
    }
    assert!(
        failed.documents[0]
            .error
            .as_deref()
            .unwrap()
            .contains("***")
    );
}

#[tokio::test]
async fn missing_password_secret_is_a_conflict_before_anything_happens() {
    let h = harness().await;
    let partner = PartnerRepository::get(&*h.f.db, &h.p1.id).unwrap().unwrap();
    h.f.secrets
        .delete(partner.password_secret.as_ref().unwrap())
        .unwrap();
    let err = h
        .service()
        .submit(&h.request(&["850"]), &CancelToken::new())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err}");
    assert!(h.http.requests().is_empty());
    assert_eq!(h.counters().run_count, 0);
}
