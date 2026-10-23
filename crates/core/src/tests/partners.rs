use super::*;
use crate::Error;
use crate::ids::EnvironmentId;
use crate::partners::{IdType, partner_scope};
use crate::secrets::SecretStore;
use crate::steps::{StepRecord, StepStatus, StepStore};

fn setup() -> (Fixture, crate::environments::Environment) {
    let f = Fixture::new();
    let env = f
        .envs
        .create(&env_input("Dev"), &secret("key-aaaaaaaa"))
        .unwrap();
    (f, env)
}

fn acme() -> crate::partners::PartnerInput {
    partner_input(
        "Acme Industries",
        "777888900",
        "acme_demo_user",
        "retail-demo-channel",
    )
}

#[test]
fn partner_validation_rules() {
    let mut i = acme();
    i.id_value = "12345".into();
    i.username = "a b".into();
    i.channel_name = "bad/channel".into();
    i.name = "".into();
    match i.validated() {
        Err(Error::Validation(errs)) => {
            let fields: Vec<_> = errs.iter().map(|e| e.field.as_str()).collect();
            for f in ["name", "id_value", "username", "channel_name"] {
                assert!(fields.contains(&f), "{f} missing in {fields:?}");
            }
        }
        other => panic!("expected validation error, got {other:?}"),
    }
    // DUNS with 4-digit suffix and a 15-digit maximum are accepted.
    let mut ok = acme();
    ok.id_value = "9012345330000".into();
    assert!(ok.validated().is_ok());
    ok.id_value = "1234567890123456".into();
    assert!(ok.validated().is_err());
}

#[test]
fn only_duns_is_supported() {
    assert_eq!(IdType::parse("DUNS").unwrap(), IdType::Duns);
    assert!(IdType::parse("GLN").is_err());
    let bad: Result<crate::partners::PartnerInput, _> = serde_json::from_str(
        r#"{"name":"a","org_unit":"b","id_type":"GLN","id_value":"777888900",
            "username":"abc","full_name":"x","channel_name":"c"}"#,
    );
    assert!(bad.is_err());
}

#[test]
fn id_type_uses_the_lowercase_api_spelling() {
    let ok: crate::partners::PartnerInput = serde_json::from_str(
        r#"{"name":"a","org_unit":"b","id_type":"duns","id_value":"777888900",
            "username":"abc","full_name":"x","channel_name":"c"}"#,
    )
    .unwrap();
    assert_eq!(ok.id_type, IdType::Duns);
    assert_eq!(serde_json::to_string(&IdType::Duns).unwrap(), "\"duns\"");
}

#[test]
fn password_rules_and_storage() {
    let (f, env) = setup();
    assert!(
        f.partners
            .create(&env.id, &acme(), &secret("short"))
            .is_err()
    );
    assert!(
        f.partners
            .create(&env.id, &acme(), &secret("tab\tinside1"))
            .is_err()
    );
    assert_eq!(f.secrets.len(), 1);

    let p = f
        .partners
        .create(&env.id, &acme(), &secret("Passw0rd!x"))
        .unwrap();
    use secrecy::ExposeSecret;
    let stored = f
        .secrets
        .get(p.password_secret.as_ref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(stored.expose_secret(), "Passw0rd!x");
    let view = serde_json::to_string(&p.view()).unwrap();
    assert!(view.contains("\"has_password\":true"));
    assert!(!view.contains("Passw0rd!x"));
}

#[test]
fn partner_requires_existing_environment() {
    let f = Fixture::new();
    let err = f
        .partners
        .create(&EnvironmentId::generate(), &acme(), &secret("Passw0rd!x"))
        .unwrap_err();
    assert!(matches!(err, Error::NotFound("environment")));
}

#[test]
fn uniqueness_is_per_environment() {
    let (f, env) = setup();
    let other = f
        .envs
        .create(&env_input("Prod"), &secret("key-bbbbbbbb"))
        .unwrap();
    f.partners
        .create(&env.id, &acme(), &secret("Passw0rd!x"))
        .unwrap();

    // Same DUNS in the same environment: conflict, and no orphan secret.
    let before = f.secrets.len();
    let mut dup = partner_input("Other", "777888900", "other_user", "other-channel");
    dup.org_unit = "X".into();
    let err = f
        .partners
        .create(&env.id, &dup, &secret("Passw0rd!x"))
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)));
    assert_eq!(f.secrets.len(), before);

    // Same values in another environment are fine.
    f.partners
        .create(&other.id, &acme(), &secret("Passw0rd!x"))
        .unwrap();
}

#[test]
fn update_rotates_password_and_edits_fields_before_provisioning() {
    let (f, env) = setup();
    let p = f
        .partners
        .create(&env.id, &acme(), &secret("Passw0rd!x"))
        .unwrap();
    let old = p.password_secret.clone().unwrap();
    let mut input = acme();
    input.full_name = "Renamed".into();
    let upd = f
        .partners
        .update(&p.id, &input, Some(&secret("N3wPassw0rd")))
        .unwrap();
    assert_eq!(upd.full_name, "Renamed");
    assert!(f.secrets.get(&old).unwrap().is_none());
    assert!(
        f.secrets
            .get(upd.password_secret.as_ref().unwrap())
            .unwrap()
            .is_some()
    );
}

#[test]
fn provisioned_partner_cannot_be_edited() {
    let (f, env) = setup();
    let p = f
        .partners
        .create(&env.id, &acme(), &secret("Passw0rd!x"))
        .unwrap();
    f.db.save(
        &partner_scope(&p.id),
        &StepRecord {
            name: "create_partner".into(),
            status: StepStatus::Succeeded,
            output: serde_json::json!({"internal_id": "p-1"}),
            error: None,
            updated_at: f.clock_now(),
        },
    )
    .unwrap();
    let err = f.partners.update(&p.id, &acme(), None).unwrap_err();
    assert!(matches!(err, Error::Conflict(_)));
}

#[test]
fn failed_steps_do_not_block_editing() {
    let (f, env) = setup();
    let p = f
        .partners
        .create(&env.id, &acme(), &secret("Passw0rd!x"))
        .unwrap();
    f.db.save(
        &partner_scope(&p.id),
        &StepRecord {
            name: "create_partner".into(),
            status: StepStatus::Failed,
            output: serde_json::Value::Null,
            error: Some("boom".into()),
            updated_at: f.clock_now(),
        },
    )
    .unwrap();
    assert!(f.partners.update(&p.id, &acme(), None).is_ok());
}

#[test]
fn delete_removes_password_and_step_records() {
    let (f, env) = setup();
    let p = f
        .partners
        .create(&env.id, &acme(), &secret("Passw0rd!x"))
        .unwrap();
    f.db.save(
        &partner_scope(&p.id),
        &StepRecord {
            name: "create_partner".into(),
            status: StepStatus::Succeeded,
            output: serde_json::Value::Null,
            error: None,
            updated_at: f.clock_now(),
        },
    )
    .unwrap();
    f.partners.delete(&p.id).unwrap();
    assert_eq!(f.secrets.len(), 1, "only the environment key remains");
    assert!(f.db.list(&partner_scope(&p.id)).unwrap().is_empty());
    assert!(matches!(
        f.partners.get(&p.id),
        Err(Error::NotFound("partner"))
    ));
}

impl Fixture {
    fn clock_now(&self) -> time::OffsetDateTime {
        use crate::clock::Clock;
        self.clock.now()
    }
}
