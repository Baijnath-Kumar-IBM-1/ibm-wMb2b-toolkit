# Adding a feature (Toolkit v2)

Every feature follows the same layout so that it can be tested without a
database, a network or a UI. Use `environments` and `partners` in
`crates/core/src` as the reference.

```
trace-core      model + validation + repository trait + service   (no I/O)
trace-storage   SQLite implementation of the trait + migration
trace-server    HTTP routes that call the service                 (sub-task 6)
ui/             a screen registered in the feature registry       (sub-task 7)
```

Dependencies point one way: `server -> core <- storage`. Core never imports
storage, and a service never talks to SQLite, the keychain or the network
directly.

## 1. Core module (`crates/core/src/<feature>.rs`)

1. **Model.** Plain structs. A secret is never a field; keep only a
   `SecretId` (for example `api_key_secret: Option<SecretId>`).
2. **View.** A `Serialize` struct for API responses. Secrets show up only as
   `has_<name>: bool`. The model itself does not derive `Serialize`, so it
   cannot be returned by accident.
3. **Input.** A `Deserialize` struct with a `validated()` method that trims,
   normalises and returns every problem at once (`validation::Collector`).
   Validate on the server side; the UI check is only a convenience.
4. **Repository trait.** `pub trait XRepository: Send + Sync`, synchronous,
   returning `trace_core::Result`. Document the contract: uniqueness rules,
   what `save` does on a duplicate (`Error::Conflict`), what `delete`
   cascades to.
5. **Service.** `XService::new(repo, secrets, clock, events, ...)` taking
   `Arc<dyn ...>` ports. Services do the work: validate, store secrets,
   save, publish an event. Order matters when a secret is involved: write the
   secret first, save the row, and delete the secret again if the save
   fails; on delete, remove the row first, then the secrets.
6. **Events.** Publish `"<feature>.<verb>"` events (`partner.created`).
   Event `data` holds IDs and names only, never secrets or payloads.
7. Register the module in `lib.rs`.

Long-running flows (anything that calls the remote API in several steps)
implement `steps::Step` and run through `steps::StepRunner` with a scope such
as `partner:<uuid>`. Each success is saved, so a retry resumes at the first
step that has not succeeded. Step outputs are IDs only.

Time comes from `Clock`, never `OffsetDateTime::now_utc()`. HTTP goes through
the `HttpClient` port.

## 2. Fakes (`crates/core/src/testing.rs`)

Add an in-memory implementation of the new trait to `MemoryDb` (it is behind
the `testing` feature so other crates can use it). Mirror the contract of the
SQLite version, including `Error::Conflict` on duplicates.

## 3. Storage (`crates/storage`)

1. Add `migrations/000N_<name>.sql` and append it to `MIGRATIONS` in `src/migrations.rs`.
   Never edit a migration that has shipped; add a new one. Use `STRICT`
   tables, store timestamps as integer microseconds, and add `CHECK` and
   `UNIQUE` constraints for every rule the trait documents.
2. Implement the trait for `Database` in `src/sqlite.rs`:
   bound parameters only (never format values into SQL), a transaction for
   anything that touches more than one row, and convert rows back through
   validating constructors so a corrupted row becomes `Error::Storage`.
3. UNIQUE violations map to `Error::Conflict` through `SqlExt::db_unique`;
   other failures map to `Error::Storage` through `SqlExt::db`. Driver text
   never reaches the user.

## 4. Tests

- **Core:** service tests in `crates/core/src/tests/<feature>.rs` using
  `MemoryDb`, `MemorySecretStore`, `FixedClock` and `RecordingPublisher`.
  Cover validation failures, the happy path, conflicts, that secrets are
  removed with their owners, and that events carry no secret values.
- **Storage:** extend `crates/storage/src/tests/mod.rs`. It runs the real
  services against a temp-file database. Cover persistence across reopen,
  unique constraints, cascades and anything concurrent.
- Network code is tested with `FakeHttp`; no test may need a real tenant.

## 5. Checklist before merging

- [ ] No secret in a struct that derives `Debug` or `Serialize`, in a log
      line, an event, an error message, or an API response.
- [ ] Errors for users are short and generic; detail goes to the log only.
- [ ] New columns/indexes enforce the same rules as the service.
- [ ] `cargo fmt --check`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo test --workspace --locked`

## Secrets in short

- `SecretStore` keys are opaque IDs (`sec-<uuid>`). The database stores the ID
  only.
- `trace_storage::open_secret_store` uses the OS keychain and falls back to an
  encrypted file (Argon2id + AES-256-GCM) when no keychain is available; the
  fallback needs a passphrase in `TRACE_SECRETS_PASSPHRASE`.
- Read a secret just before use and let it drop afterwards (`SecretString`
  zeroizes on drop).
