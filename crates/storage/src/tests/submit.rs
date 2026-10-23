//! The submission engine over the real SQLite repositories.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use trace_core::b2b::{B2bApi, ProvisioningService};
use trace_core::counters::CounterRepository;
use trace_core::environments::Environment;
use trace_core::packs::Catalog;
use trace_core::partners::Partner;
use trace_core::steps::CancelToken;
use trace_core::submissions::SubmissionRepository;
use trace_core::submit::{
    Party, PreviewRequest, SubmissionDeps, SubmissionOutcome, SubmissionService, SubmitRequest,
};
use trace_core::testing::{FakeHttp, FixedClock, RecordingPublisher};

use super::{API_KEY, Fixture, PASSWORD, partner_input, secret};

const P2_PASSWORD: &str = "another-secret-password-XYZ";

struct Setup {
    f: Fixture,
    http: Arc<FakeHttp>,
    env: Environment,
    p1: Partner,
    p2: Partner,
    events: Arc<RecordingPublisher>,
    service: SubmissionService,
}

fn token_body() -> String {
    let enc = |s: &str| URL_SAFE_NO_PAD.encode(s.as_bytes());
    format!(
        r#"{{"token":"{}.{}.{}"}}"#,
        enc(r#"{"alg":"none"}"#),
        enc(r#"{"exp":2000000000}"#),
        enc("sig")
    )
}

async fn provision(prov: &ProvisioningService, http: &FakeHttp, partner: &Partner, uri: &str) {
    http.push(201, r#"{"internalId":"pid"}"#);
    http.push(
        201,
        &format!(r#"{{"id":"ch","endpoint":{{"uri":"{uri}"}}}}"#),
    );
    http.push(200, "{}");
    http.push(201, r#"{"id":"user"}"#);
    http.push(200, "{}");
    let report = prov
        .provision(&partner.id, &CancelToken::new())
        .await
        .unwrap();
    assert!(report.is_complete());
}

async fn setup() -> Setup {
    let f = Fixture::new();
    let env = f.env("Dev");
    let p1 = f
        .partners
        .create(
            &env.id,
            &partner_input("Buyer", "123456789", "p1user", "p1-channel"),
            &secret(PASSWORD),
        )
        .unwrap();
    let p2 = f
        .partners
        .create(
            &env.id,
            &partner_input("Supplier", "987654321", "p2user", "p2-channel"),
            &secret(P2_PASSWORD),
        )
        .unwrap();

    let http = FakeHttp::new();
    let clock = Arc::new(FixedClock::default());
    let events = Arc::new(RecordingPublisher::default());
    let api = B2bApi::new(
        http.clone(),
        f.secrets.clone(),
        Arc::new(f.db.clone()),
        clock.clone(),
    );
    let prov = Arc::new(ProvisioningService::new(
        api,
        Arc::new(f.db.clone()),
        Arc::new(f.db.clone()),
        Arc::new(f.db.clone()),
        f.secrets.clone(),
        clock.clone(),
        events.clone(),
    ));
    http.push(200, &token_body());
    provision(&prov, &http, &p1, "https://b2b.example/ch/p1").await;
    provision(&prov, &http, &p2, "https://b2b.example/ch/p2").await;
    http.clear_requests();

    let service = SubmissionService::new(SubmissionDeps {
        http: http.clone(),
        catalog: Arc::new(Catalog::built_in()),
        envs: Arc::new(f.db.clone()),
        partners: Arc::new(f.db.clone()),
        counters: Arc::new(f.db.clone()),
        submissions: Arc::new(f.db.clone()),
        prefs: Arc::new(f.db.clone()),
        secrets: f.secrets.clone(),
        provisioning: prov,
        clock,
        events: events.clone(),
    })
    .with_delay(Duration::ZERO);
    Setup {
        f,
        http,
        env,
        p1,
        p2,
        events,
        service,
    }
}

impl Setup {
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
}

fn database_bytes(f: &Fixture) -> String {
    let mut bytes = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let mut name = f.path.as_os_str().to_owned();
        name.push(suffix);
        if let Ok(b) = std::fs::read(std::path::PathBuf::from(name)) {
            bytes.extend(b);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn submissions_persist_every_field_and_survive_reopen() {
    let s = setup().await;
    s.http.push(200, "accepted by channel");
    s.http.push(500, "denied");
    let report = s
        .service
        .submit(&s.request(&["850", "855", "860"]), &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(report.documents[2].outcome, SubmissionOutcome::NotRun);

    let reopened = crate::Database::open(&s.f.path).unwrap();
    let mut saved = reopened.list_recent(&s.env.id, 10).unwrap();
    saved.sort_by_key(|x| x.isa_control);
    assert_eq!(saved.len(), 2);

    let ok = &saved[0];
    assert_eq!(ok.transaction_code, "850");
    assert_eq!(ok.result, "success");
    assert_eq!(ok.http_status, Some(200));
    assert_eq!(ok.response_excerpt.as_deref(), Some("accepted by channel"));
    assert!(ok.payload.as_deref().unwrap().starts_with("ISA*00*"));
    assert!(ok.duration_ms.is_some());
    assert_eq!(ok.sender_partner_id.as_ref(), Some(&s.p1.id));
    assert_eq!(ok.receiver_partner_id.as_ref(), Some(&s.p2.id));
    assert_eq!(ok.trace_values["PO_NUMBER"], "PO-20250101-0001");

    let bad = &saved[1];
    assert_eq!(bad.transaction_code, "855");
    assert_eq!(bad.result, "failed");
    assert_eq!(bad.http_status, Some(500));
    assert_eq!(bad.sender_partner_id.as_ref(), Some(&s.p2.id));
    assert_eq!((bad.isa_control, bad.gs_control), (2, 2));
}

#[tokio::test]
async fn control_numbers_are_unique_across_concurrent_runs_and_reopen() {
    let s = setup().await;
    for _ in 0..6 {
        s.http.push(200, "ok");
    }
    let a = s.request(&["850", "855", "860"]);
    let b = s.request(&["850", "855", "860"]);
    let cancel = CancelToken::new();
    let (ra, rb) = tokio::join!(s.service.submit(&a, &cancel), s.service.submit(&b, &cancel));
    ra.unwrap();
    rb.unwrap();

    let saved = s.f.db.list_recent(&s.env.id, 100).unwrap();
    let isa: BTreeSet<u32> = saved.iter().map(|x| x.isa_control).collect();
    let gs: BTreeSet<u32> = saved.iter().map(|x| x.gs_control).collect();
    assert_eq!(saved.len(), 6);
    assert_eq!(isa, (1..=6).collect());
    assert_eq!(gs, (1..=6).collect());

    // A fresh connection to the same file continues where the run stopped.
    let reopened = crate::Database::open(&s.f.path).unwrap();
    let counters = reopened.peek(&s.env.id).unwrap();
    assert_eq!(
        (counters.isa_next, counters.gs_next, counters.run_count),
        (7, 7, 2)
    );
}

#[tokio::test]
async fn preview_leaves_the_database_untouched() {
    let s = setup().await;
    let before = s.f.db.peek(&s.env.id).unwrap();
    let preview = s
        .service
        .preview(&PreviewRequest {
            environment_id: s.env.id.clone(),
            pack_id: "retail".into(),
            codes: None,
            p1: Party::Manual {
                id_value: "555000111".into(),
            },
            p2: Party::Saved {
                partner_id: s.p2.id.clone(),
            },
            trace_overrides: BTreeMap::new(),
        })
        .unwrap();
    assert_eq!(preview.documents.len(), 9);
    assert_eq!(s.f.db.peek(&s.env.id).unwrap(), before);
    assert!(s.f.db.list_recent(&s.env.id, 10).unwrap().is_empty());
    assert!(s.http.requests().is_empty());
}

#[tokio::test]
async fn passwords_and_credentials_never_reach_the_database_file_or_events() {
    let s = setup().await;
    // The channel echoes the credentials back.
    let basic = STANDARD.encode(format!("p1user:{PASSWORD}"));
    s.http
        .push(401, &format!("rejected {PASSWORD} / Basic {basic}"));
    let report = s
        .service
        .submit(&s.request(&["850"]), &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(report.documents[0].outcome, SubmissionOutcome::Failed);

    let bytes = database_bytes(&s.f);
    let events = serde_json::to_string(&s.events.events()).unwrap();
    let report_json = serde_json::to_string(&report).unwrap();
    for text in [PASSWORD, P2_PASSWORD, API_KEY, basic.as_str()] {
        assert!(!bytes.contains(text), "database file leaked {text}");
        assert!(!events.contains(text), "events leaked {text}");
        assert!(!report_json.contains(text), "report leaked {text}");
    }
    let saved = s.f.db.list_recent(&s.env.id, 10).unwrap();
    assert!(
        saved[0]
            .response_excerpt
            .as_deref()
            .unwrap()
            .contains("***")
    );
}
