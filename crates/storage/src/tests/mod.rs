//! Tests of the SQLite repositories, driven through the real `trace-core`
//! services against a temp-file database.

mod submit;

use std::collections::BTreeSet;
use std::sync::Arc;

use secrecy::SecretString;
use serde_json::json;
use tempfile::TempDir;
use time::OffsetDateTime;
use trace_core::Error;
use trace_core::counters::{CounterRepository, Counters, MAX_CONTROL_NUMBER};
use trace_core::environments::{
    Environment, EnvironmentInput, EnvironmentRepository, EnvironmentService, IAM_HOST_TEST,
};
use trace_core::ids::{EnvironmentId, PartnerId, SubmissionId};
use trace_core::installed::{InstalledDocument, InstalledDocumentRepository};
use trace_core::partners::{PartnerInput, PartnerRepository, PartnerService, partner_scope};
use trace_core::preferences::{PreferenceRepository, Preferences};
use trace_core::steps::{StepRecord, StepStatus, StepStore};
use trace_core::submissions::{Submission, SubmissionRepository};
use trace_core::testing::{FixedClock, MemorySecretStore, RecordingPublisher};

use crate::Database;

const API_KEY: &str = "super-secret-api-key-0123456789";
const PASSWORD: &str = "very-secret-password-ABC";

struct Fixture {
    _dir: TempDir,
    path: std::path::PathBuf,
    db: Database,
    secrets: Arc<MemorySecretStore>,
    envs: EnvironmentService,
    partners: PartnerService,
}

fn services(
    db: &Database,
    secrets: &Arc<MemorySecretStore>,
) -> (EnvironmentService, PartnerService) {
    let clock = Arc::new(FixedClock::default());
    let events = Arc::new(RecordingPublisher::default());
    let envs = EnvironmentService::new(
        Arc::new(db.clone()),
        Arc::new(db.clone()),
        secrets.clone(),
        clock.clone(),
        events.clone(),
    );
    let partners = PartnerService::new(
        Arc::new(db.clone()),
        Arc::new(db.clone()),
        Arc::new(db.clone()),
        secrets.clone(),
        clock,
        events,
    );
    (envs, partners)
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("data").join("trace.db");
        let db = Database::open(&path).expect("open");
        let secrets = Arc::new(MemorySecretStore::default());
        let (envs, partners) = services(&db, &secrets);
        Self {
            _dir: dir,
            path,
            db,
            secrets,
            envs,
            partners,
        }
    }

    fn env(&self, name: &str) -> Environment {
        self.envs
            .create(&env_input(name), &secret(API_KEY))
            .expect("create env")
    }
}

fn secret(v: &str) -> SecretString {
    SecretString::from(v.to_string())
}

fn env_input(name: &str) -> EnvironmentInput {
    EnvironmentInput {
        name: name.into(),
        b2b_host: "dev2205134-m-yqb-c1.b2b.ipaas.ibm.com".into(),
        instance_id: "123e4567-e89b-12d3-a456-426614174000".into(),
        iam_host: IAM_HOST_TEST.into(),
        ca_bundle_path: None,
    }
}

fn partner_input(name: &str, duns: &str, user: &str, channel: &str) -> PartnerInput {
    PartnerInput {
        name: name.into(),
        org_unit: "Retail Procurement".into(),
        id_type: Default::default(),
        id_value: duns.into(),
        username: user.into(),
        full_name: "Demo User".into(),
        channel_name: channel.into(),
    }
}

fn step(name: &str, status: StepStatus) -> StepRecord {
    StepRecord {
        name: name.into(),
        status,
        output: json!({ "id": format!("{name}-1") }),
        error: (status == StepStatus::Failed).then(|| "boom".to_string()),
        updated_at: OffsetDateTime::UNIX_EPOCH,
    }
}

// ---- environments ---------------------------------------------------------

#[test]
fn environment_round_trips_and_survives_reopen() {
    let f = Fixture::new();
    let created = f.env("Dev");
    assert_eq!(
        f.db.schema_version().unwrap(),
        crate::migrations::latest_version()
    );

    let reopened = Database::open(&f.path).unwrap();
    let got = EnvironmentRepository::get(&reopened, &created.id)
        .unwrap()
        .expect("exists");
    assert_eq!(got, created);
    assert_eq!(
        EnvironmentRepository::list(&reopened).unwrap(),
        vec![created]
    );
}

#[test]
fn environment_names_are_unique_case_insensitively() {
    let f = Fixture::new();
    f.env("Dev");
    let err = f
        .envs
        .create(&env_input("dEV"), &secret(API_KEY))
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    // The rejected create must not leave its secret behind.
    assert_eq!(f.secrets.len(), 1);
}

#[test]
fn only_one_environment_is_default() {
    let f = Fixture::new();
    let a = f.env("A");
    let b = f.env("B");
    f.envs.set_default(&a.id).unwrap();
    f.envs.set_default(&b.id).unwrap();
    let defaults: Vec<_> = EnvironmentRepository::list(&f.db)
        .unwrap()
        .into_iter()
        .filter(|e| e.is_default)
        .map(|e| e.id)
        .collect();
    assert_eq!(defaults, vec![b.id]);
}

#[test]
fn set_default_on_unknown_environment_is_not_found_and_keeps_current_default() {
    let f = Fixture::new();
    let a = f.env("A");
    f.envs.set_default(&a.id).unwrap();
    let err = EnvironmentRepository::set_default(&f.db, &EnvironmentId::generate()).unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
    let still = EnvironmentRepository::get(&f.db, &a.id).unwrap().unwrap();
    assert!(still.is_default, "failed set_default must roll back");
}

#[test]
fn delete_environment_cascades_to_everything_it_owns() {
    let f = Fixture::new();
    let env = f.env("Dev");
    let partner = f
        .partners
        .create(
            &env.id,
            &partner_input("Acme", "123456789", "acme.user", "ACME_CH"),
            &secret(PASSWORD),
        )
        .unwrap();
    f.db.allocate(&env.id, 3).unwrap();
    InstalledDocumentRepository::save(
        &f.db,
        &env.id,
        &InstalledDocument {
            transaction_code: "810".into(),
            document_type_id: "dt-1".into(),
            installed_at: OffsetDateTime::UNIX_EPOCH,
        },
    )
    .unwrap();
    StepStore::save(
        &f.db,
        &partner_scope(&partner.id),
        &step("create", StepStatus::Succeeded),
    )
    .unwrap();
    assert_eq!(f.secrets.len(), 2);

    f.envs.delete(&env.id).unwrap();

    assert!(EnvironmentRepository::list(&f.db).unwrap().is_empty());
    assert!(
        PartnerRepository::get(&f.db, &partner.id)
            .unwrap()
            .is_none()
    );
    assert!(
        StepStore::list(&f.db, &partner_scope(&partner.id))
            .unwrap()
            .is_empty()
    );
    assert!(
        InstalledDocumentRepository::list(&f.db, &env.id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(f.db.peek(&env.id).unwrap(), Counters::default());
    assert!(
        f.secrets.is_empty(),
        "secrets are removed with their owners"
    );
}

// ---- partners -------------------------------------------------------------

#[test]
fn partner_round_trips_and_survives_reopen() {
    let f = Fixture::new();
    let env = f.env("Dev");
    let p = f
        .partners
        .create(
            &env.id,
            &partner_input("Acme", "123456789", "acme.user", "ACME_CH"),
            &secret(PASSWORD),
        )
        .unwrap();
    let reopened = Database::open(&f.path).unwrap();
    assert_eq!(
        PartnerRepository::get(&reopened, &p.id).unwrap(),
        Some(p.clone())
    );
    assert_eq!(
        PartnerRepository::list(&reopened, &env.id).unwrap(),
        vec![p]
    );
}

#[test]
fn partner_uniqueness_is_enforced_per_environment() {
    let f = Fixture::new();
    let env = f.env("Dev");
    let other = f.env("Other");
    let base = partner_input("Acme", "123456789", "acme.user", "ACME_CH");
    f.partners
        .create(&env.id, &base, &secret(PASSWORD))
        .unwrap();

    let cases = [
        partner_input("acme", "222222222", "u2.user", "CH2"),
        partner_input("Two", "123456789", "u3.user", "CH3"),
        partner_input("Three", "333333333", "ACME.USER", "CH4"),
        partner_input("Four", "444444444", "u5.user", "acme_ch"),
    ];
    for dup in &cases {
        let err = f
            .partners
            .create(&env.id, dup, &secret(PASSWORD))
            .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "{dup:?}: {err:?}");
    }
    // The same values are fine in another environment.
    f.partners
        .create(&other.id, &base, &secret(PASSWORD))
        .unwrap();
    // Only the two successful creates left secrets behind (plus 2 API keys).
    assert_eq!(f.secrets.len(), 4);
}

#[test]
fn partner_for_unknown_environment_is_rejected() {
    let f = Fixture::new();
    let err = f
        .partners
        .create(
            &EnvironmentId::generate(),
            &partner_input("Acme", "123456789", "acme.user", "ACME_CH"),
            &secret(PASSWORD),
        )
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}

#[test]
fn deleting_a_partner_removes_its_steps_and_secret() {
    let f = Fixture::new();
    let env = f.env("Dev");
    let p = f
        .partners
        .create(
            &env.id,
            &partner_input("Acme", "123456789", "acme.user", "ACME_CH"),
            &secret(PASSWORD),
        )
        .unwrap();
    let scope = partner_scope(&p.id);
    StepStore::save(&f.db, &scope, &step("create", StepStatus::Failed)).unwrap();
    f.partners.delete(&p.id).unwrap();
    assert!(PartnerRepository::get(&f.db, &p.id).unwrap().is_none());
    assert!(StepStore::list(&f.db, &scope).unwrap().is_empty());
    assert_eq!(f.secrets.len(), 1, "only the API key remains");
}

// ---- counters -------------------------------------------------------------

#[test]
fn allocation_is_sequential_and_persistent() {
    let f = Fixture::new();
    let env = f.env("Dev");
    assert_eq!(f.db.peek(&env.id).unwrap(), Counters::default());

    let first = f.db.allocate(&env.id, 3).unwrap();
    assert_eq!(
        first.iter().map(|c| c.isa).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        first.iter().map(|c| c.gs).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );

    let reopened = Database::open(&f.path).unwrap();
    let next = reopened.allocate(&env.id, 2).unwrap();
    assert_eq!(next.iter().map(|c| c.isa).collect::<Vec<_>>(), vec![4, 5]);
    assert_eq!(reopened.peek(&env.id).unwrap().isa_next, 6);
}

#[test]
fn allocation_counters_are_per_environment() {
    let f = Fixture::new();
    let a = f.env("A");
    let b = f.env("B");
    f.db.allocate(&a.id, 5).unwrap();
    assert_eq!(f.db.allocate(&b.id, 1).unwrap()[0].isa, 1);
}

#[test]
fn allocation_wraps_after_the_nine_digit_maximum() {
    let f = Fixture::new();
    let env = f.env("Dev");
    f.db.run(|c| {
        c.execute(
            "INSERT INTO counters (environment_id, isa_next, gs_next, run_count) \
             VALUES (?1, ?2, ?3, 0)",
            rusqlite::params![env.id.as_str(), MAX_CONTROL_NUMBER - 1, MAX_CONTROL_NUMBER],
        )
        .map(drop)
        .map_err(|e| Error::Storage(e.to_string()))
    })
    .unwrap();
    let got = f.db.allocate(&env.id, 3).unwrap();
    assert_eq!(
        got.iter().map(|c| c.isa).collect::<Vec<_>>(),
        vec![MAX_CONTROL_NUMBER - 1, MAX_CONTROL_NUMBER, 1]
    );
    assert_eq!(
        got.iter().map(|c| c.gs).collect::<Vec<_>>(),
        vec![MAX_CONTROL_NUMBER, 1, 2]
    );
    assert_eq!(f.db.peek(&env.id).unwrap().isa_next, 2);
}

#[test]
fn allocation_validates_count_and_environment() {
    let f = Fixture::new();
    let env = f.env("Dev");
    for bad in [0, 1001] {
        assert!(matches!(
            f.db.allocate(&env.id, bad),
            Err(Error::Validation { .. })
        ));
    }
    assert!(f.db.allocate(&env.id, 1000).is_ok());
    assert!(matches!(
        f.db.allocate(&EnvironmentId::generate(), 1),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn concurrent_allocation_never_hands_out_a_number_twice() {
    let f = Fixture::new();
    let env = f.env("Dev");
    let mut handles = Vec::new();
    for _ in 0..8 {
        // A separate connection per thread, as two processes would have.
        let db = Database::open(&f.path).unwrap();
        let id = env.id.clone();
        handles.push(std::thread::spawn(move || {
            (0..10)
                .flat_map(|_| db.allocate(&id, 5).unwrap())
                .map(|c| c.isa)
                .collect::<Vec<_>>()
        }));
    }
    let mut all = Vec::new();
    for h in handles {
        all.extend(h.join().unwrap());
    }
    let unique: BTreeSet<u32> = all.iter().copied().collect();
    assert_eq!(all.len(), 400);
    assert_eq!(unique.len(), 400, "duplicate control numbers were issued");
    assert_eq!(unique.first(), Some(&1));
    assert_eq!(unique.last(), Some(&400), "numbers must be consecutive");
}

#[test]
fn run_count_increments() {
    let f = Fixture::new();
    let env = f.env("Dev");
    assert_eq!(f.db.increment_run_count(&env.id).unwrap(), 1);
    assert_eq!(f.db.increment_run_count(&env.id).unwrap(), 2);
    assert_eq!(f.db.peek(&env.id).unwrap().run_count, 2);
    // Allocating does not reset it.
    f.db.allocate(&env.id, 1).unwrap();
    assert_eq!(f.db.peek(&env.id).unwrap().run_count, 2);
    assert!(matches!(
        f.db.increment_run_count(&EnvironmentId::generate()),
        Err(Error::NotFound(_))
    ));
}

// ---- installed documents, preferences, submissions, steps ----------------------

#[test]
fn installed_documents_upsert_and_remove() {
    let f = Fixture::new();
    let env = f.env("Dev");
    let doc = |id: &str| InstalledDocument {
        transaction_code: "810".into(),
        document_type_id: id.into(),
        installed_at: OffsetDateTime::UNIX_EPOCH,
    };
    InstalledDocumentRepository::save(&f.db, &env.id, &doc("dt-1")).unwrap();
    InstalledDocumentRepository::save(&f.db, &env.id, &doc("dt-2")).unwrap();
    let mut other = doc("dt-3");
    other.transaction_code = "820".into();
    InstalledDocumentRepository::save(&f.db, &env.id, &other).unwrap();

    let listed = InstalledDocumentRepository::list(&f.db, &env.id).unwrap();
    assert_eq!(listed, vec![doc("dt-2"), other]);

    InstalledDocumentRepository::remove(&f.db, &env.id, "810").unwrap();
    assert_eq!(
        InstalledDocumentRepository::list(&f.db, &env.id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn preferences_default_then_persist() {
    let f = Fixture::new();
    assert_eq!(
        PreferenceRepository::get(&f.db).unwrap(),
        Preferences::default()
    );
    let prefs = Preferences {
        inter_document_delay_secs: 5,
        request_timeout_secs: 45,
        payload_timeout_secs: 200,
    };
    PreferenceRepository::set(&f.db, &prefs).unwrap();
    let reopened = Database::open(&f.path).unwrap();
    assert_eq!(PreferenceRepository::get(&reopened).unwrap(), prefs);
}

#[test]
fn submissions_list_newest_first_and_respect_limit() {
    let f = Fixture::new();
    let env = f.env("Dev");
    let make = |secs: i64, isa: u32| Submission {
        id: SubmissionId::generate(),
        environment_id: env.id.clone(),
        submitted_at: OffsetDateTime::from_unix_timestamp(secs).unwrap(),
        domain: "retail".into(),
        transaction_code: "810".into(),
        sender_partner_id: None,
        receiver_partner_id: None,
        isa_control: isa,
        gs_control: isa,
        http_status: Some(200),
        result: "success".into(),
        trace_values: json!({ "invoice": format!("INV-{isa}") }),
        payload: Some(format!("ISA*{isa}~")),
        response_excerpt: Some("ok".into()),
        duration_ms: Some(12),
    };
    let (old, mid, new) = (make(1_000, 1), make(2_000, 2), make(3_000, 3));
    for s in [&mid, &old, &new] {
        SubmissionRepository::save(&f.db, s).unwrap();
    }
    let got = f.db.list_recent(&env.id, 10).unwrap();
    assert_eq!(got, vec![new.clone(), mid, old]);
    assert_eq!(f.db.list_recent(&env.id, 1).unwrap(), vec![new]);
}

#[test]
fn submission_keeps_history_when_a_partner_is_deleted() {
    let f = Fixture::new();
    let env = f.env("Dev");
    let p = f
        .partners
        .create(
            &env.id,
            &partner_input("Acme", "123456789", "acme.user", "ACME_CH"),
            &secret(PASSWORD),
        )
        .unwrap();
    let s = Submission {
        id: SubmissionId::generate(),
        environment_id: env.id.clone(),
        submitted_at: OffsetDateTime::UNIX_EPOCH,
        domain: "retail".into(),
        transaction_code: "810".into(),
        sender_partner_id: Some(p.id.clone()),
        receiver_partner_id: None,
        isa_control: 1,
        gs_control: 1,
        http_status: None,
        result: "generated".into(),
        trace_values: json!({}),
        payload: None,
        response_excerpt: None,
        duration_ms: None,
    };
    SubmissionRepository::save(&f.db, &s).unwrap();
    f.partners.delete(&p.id).unwrap();
    let got = f.db.list_recent(&env.id, 10).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].sender_partner_id, None);
}

#[test]
fn step_records_keep_first_run_order_and_upsert() {
    let f = Fixture::new();
    let scope = "partner:test";
    StepStore::save(&f.db, scope, &step("one", StepStatus::Succeeded)).unwrap();
    StepStore::save(&f.db, scope, &step("two", StepStatus::Failed)).unwrap();
    StepStore::save(&f.db, scope, &step("one", StepStatus::Failed)).unwrap();
    StepStore::save(&f.db, "partner:other", &step("one", StepStatus::Succeeded)).unwrap();

    let got = StepStore::list(&f.db, scope).unwrap();
    assert_eq!(
        got.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    assert_eq!(got[0].status, StepStatus::Failed);
    assert_eq!(got[1].error.as_deref(), Some("boom"));

    f.db.clear(scope).unwrap();
    assert!(StepStore::list(&f.db, scope).unwrap().is_empty());
    assert_eq!(StepStore::list(&f.db, "partner:other").unwrap().len(), 1);
}

// ---- secrets never reach the database ---------------------------------------

#[test]
fn database_files_contain_only_opaque_secret_ids() {
    let f = Fixture::new();
    let env = f.env("Dev");
    let p = f
        .partners
        .create(
            &env.id,
            &partner_input("Acme", "123456789", "acme.user", "ACME_CH"),
            &secret(PASSWORD),
        )
        .unwrap();
    StepStore::save(
        &f.db,
        &partner_scope(&p.id),
        &step("create", StepStatus::Succeeded),
    )
    .unwrap();

    // Make sure WAL content is included in what we scan.
    let mut bytes = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let mut name = f.path.as_os_str().to_owned();
        name.push(suffix);
        if let Ok(b) = std::fs::read(std::path::PathBuf::from(name)) {
            bytes.extend(b);
        }
    }
    let haystack = String::from_utf8_lossy(&bytes);
    assert!(
        !haystack.contains(API_KEY),
        "API key found in database files"
    );
    assert!(
        !haystack.contains(PASSWORD),
        "password found in database files"
    );
    assert!(
        haystack.contains("sec-"),
        "secret ids are expected to be stored"
    );
}

#[cfg(unix)]
#[test]
fn database_file_and_directory_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let file = std::fs::metadata(&f.path).unwrap().permissions().mode() & 0o777;
    let dir = std::fs::metadata(f.path.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file, 0o600);
    assert_eq!(dir, 0o700);
}

#[test]
fn partner_ids_that_are_not_uuids_are_rejected_before_sql() {
    // Defence in depth: ids are validated types, so a hostile string cannot
    // reach a query through the typed API.
    assert!("x'; DROP TABLE partners; --".parse::<PartnerId>().is_err());
}
