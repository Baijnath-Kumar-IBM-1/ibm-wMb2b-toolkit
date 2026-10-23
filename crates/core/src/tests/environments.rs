use super::*;
use crate::Error;
use crate::environments::validate_api_key;
use crate::secrets::SecretStore;

#[test]
fn input_validation_reports_every_bad_field() {
    let bad = EnvironmentInput {
        name: " ".into(),
        b2b_host: "https://x.example.com".into(),
        instance_id: "nope".into(),
        iam_host: "bad host".into(),
        ca_bundle_path: Some("relative.pem".into()),
    };
    match bad.validated() {
        Err(Error::Validation(errs)) => {
            let fields: Vec<_> = errs.iter().map(|e| e.field.as_str()).collect();
            for f in [
                "name",
                "b2b_host",
                "instance_id",
                "iam_host",
                "ca_bundle_path",
            ] {
                assert!(fields.contains(&f), "{f} missing in {fields:?}");
            }
        }
        other => panic!("expected validation error, got {other:?}"),
    }
}

#[test]
fn input_is_normalised() {
    let mut i = env_input("  Dev  ");
    i.b2b_host = "DEV.B2B.Example.COM".into();
    i.instance_id = "123E4567-E89B-12D3-A456-426614174000".into();
    i.ca_bundle_path = Some("  ".into());
    let v = i.validated().unwrap();
    assert_eq!(v.name, "Dev");
    assert_eq!(v.b2b_host, "dev.b2b.example.com");
    assert_eq!(v.instance_id, "123e4567-e89b-12d3-a456-426614174000");
    assert_eq!(v.ca_bundle_path, None);
}

#[test]
fn api_key_rules() {
    assert!(validate_api_key(&secret("abcdefgh")).is_ok());
    assert!(validate_api_key(&secret("short")).is_err());
    assert!(validate_api_key(&secret("has space in it")).is_err());
    assert!(validate_api_key(&secret("new\nline12345")).is_err());
}

#[test]
fn first_environment_is_default_second_is_not() {
    let f = Fixture::new();
    let a = f
        .envs
        .create(&env_input("A"), &secret("key-aaaaaaaa"))
        .unwrap();
    let b = f
        .envs
        .create(&env_input("B"), &secret("key-bbbbbbbb"))
        .unwrap();
    assert!(a.is_default);
    assert!(!b.is_default);
    f.envs.set_default(&b.id).unwrap();
    assert!(!f.envs.get(&a.id).unwrap().is_default);
    assert!(f.envs.get(&b.id).unwrap().is_default);
}

#[test]
fn api_key_goes_to_secret_store_and_never_into_view_or_events() {
    let f = Fixture::new();
    let env = f
        .envs
        .create(&env_input("A"), &secret("super-secret-api-key"))
        .unwrap();
    let sid = env.api_key_secret.clone().unwrap();
    use secrecy::ExposeSecret;
    assert_eq!(
        f.secrets.get(&sid).unwrap().unwrap().expose_secret(),
        "super-secret-api-key"
    );
    let view = serde_json::to_string(&env.view()).unwrap();
    assert!(view.contains("\"has_api_key\":true"));
    assert!(!view.contains("super-secret-api-key"));
    assert!(
        !view.contains(sid.as_str()),
        "secret id should not be exposed"
    );
    for e in f.events.events() {
        assert!(
            !serde_json::to_string(&e)
                .unwrap()
                .contains("super-secret-api-key")
        );
    }
}

#[test]
fn duplicate_name_is_conflict_and_leaves_no_orphan_secret() {
    let f = Fixture::new();
    f.envs
        .create(&env_input("Dev"), &secret("key-aaaaaaaa"))
        .unwrap();
    let err = f
        .envs
        .create(&env_input("dev"), &secret("key-bbbbbbbb"))
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)));
    assert_eq!(f.secrets.len(), 1);
}

#[test]
fn invalid_input_stores_nothing() {
    let f = Fixture::new();
    let mut bad = env_input("A");
    bad.b2b_host = "https://nope".into();
    assert!(f.envs.create(&bad, &secret("key-aaaaaaaa")).is_err());
    assert!(f.envs.create(&env_input("A"), &secret("x")).is_err());
    assert!(f.secrets.is_empty());
    assert!(f.envs.list().unwrap().is_empty());
}

#[test]
fn update_keeps_key_unless_replaced_and_rotates_secret_id() {
    let f = Fixture::new();
    let env = f
        .envs
        .create(&env_input("A"), &secret("key-aaaaaaaa"))
        .unwrap();
    let first = env.api_key_secret.clone().unwrap();

    let mut input = env_input("A2");
    input.b2b_host = "other.example.com".into();
    let same_key = f.envs.update(&env.id, &input, None).unwrap();
    assert_eq!(same_key.api_key_secret.as_ref(), Some(&first));
    assert_eq!(same_key.b2b_host, "other.example.com");

    let rotated = f
        .envs
        .update(&env.id, &input, Some(&secret("key-cccccccc")))
        .unwrap();
    let second = rotated.api_key_secret.clone().unwrap();
    assert_ne!(first, second);
    assert!(f.secrets.get(&first).unwrap().is_none(), "old key removed");
    assert!(f.secrets.get(&second).unwrap().is_some());
    assert_eq!(f.secrets.len(), 1);
}

#[test]
fn update_unknown_is_not_found() {
    let f = Fixture::new();
    let err = f
        .envs
        .update(
            &crate::ids::EnvironmentId::generate(),
            &env_input("A"),
            None,
        )
        .unwrap_err();
    assert!(matches!(err, Error::NotFound("environment")));
}

#[test]
fn delete_cascades_partners_and_secrets_and_moves_default() {
    let f = Fixture::new();
    let a = f
        .envs
        .create(&env_input("A"), &secret("key-aaaaaaaa"))
        .unwrap();
    let b = f
        .envs
        .create(&env_input("B"), &secret("key-bbbbbbbb"))
        .unwrap();
    f.partners
        .create(
            &a.id,
            &partner_input("Acme", "777888900", "acme_user", "acme-channel"),
            &secret("Password1!"),
        )
        .unwrap();
    assert_eq!(f.secrets.len(), 3);

    f.envs.delete(&a.id).unwrap();
    assert_eq!(f.secrets.len(), 1, "only B's key remains");
    assert!(f.envs.get(&b.id).unwrap().is_default);
    assert!(matches!(f.envs.get(&a.id), Err(Error::NotFound(_))));
}
