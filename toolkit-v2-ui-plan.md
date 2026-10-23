# Toolkit v2 — Rust core + local UI (web + desktop) Plan

## Top-Level Overview

**Goal:** Release Toolkit v2: a general-purpose, single-user, local-first application to **initialize an IBM webMethods B2B environment** (partners, channels, users, document types), **generate sample EDI documents**, and **submit them easily for demos**. It runs as a web app (local server opened in a browser) and as a desktop app (Tauri), with built-in activity reporting. It is designed so new features can be added without reworking the foundation.

**v1 is frozen.** `b2b-toolkit.sh`, `toolkit.env.example`, `templates/` and the `.b2b-toolkit/` workspace stay exactly as they are. v2 does not read `toolkit.env`, does not read or write v1 workspace files, and has no v1 importer. The script remains only as a behavioural reference (tests compare rendered EDI with its `--dry-run` output) and is marked deprecated in the README.

**Approach**
- **Rust core crate:** feature modules (services + storage ports), no HTTP, no UI.
- **axum server crate:** composes one router per feature on `127.0.0.1` only.
- **React + Carbon UI:** ported from `wMB2BToolKit_Design/`; talks only to the HTTP API; screens come from a feature registry.
- **Tauri shell:** starts the same axum server in-process on a random port with a per-launch token and loads the UI. One transport, one UI client, no sidecar process.
- **`trace-server` binary:** same crates with embedded UI for web mode.

**Decisions made**
- Single user, local machine. No OIDC, sessions or multi-user audit.
- UI library is **IBM Carbon** (what the design uses), not shadcn.
- Environment uses **B2B host** instead of the design's "Tenant" field.
- Partners: any number per environment, shared across domains; each owns a channel and a partner user.
- Documents install per domain selection; document types are keyed by transaction code (`820` is installed once).
- New configuration and storage model (below). Secrets in the OS keychain.

**Document catalogs**
- Retail: 840, 843, 850, 855, 860, 865, 856, 810, 820
- Healthcare: 837, 275, 277, 835, 820
- Supply chain: 204, 990, 211, 214, 210, 820

**Non-goals**
- Multi-user hosting, SSO, remote deployment.
- Importing or migrating v1 data or `toolkit.env`.
- Modifying or removing the legacy shell script.

**Security constraints (project rules)**
- Server binds to `127.0.0.1` only. The design's `vite.config.ts` binds `0.0.0.0`; do not copy it.
- Per-launch bearer token on API calls, `Host` header validation (DNS rebinding), no wildcard CORS.
- Secrets (API key, partner passwords) only in the OS keychain; never in the database, files, logs or API responses.
- TLS certificate validation always on. Eight of the script's nine curl calls use `--insecure`; none of that is ported. Optional per-environment CA bundle for private CAs.
- Latest stable, actively maintained dependencies; lock files committed (`Cargo.lock`, `pnpm-lock.yaml`); dependency scanning in CI.

## New Configuration and Storage Model

Three layers replace `toolkit.env` and the JSON workspace:

1. **Application database (SQLite, one file in the app data dir)** — all non-secret data: environments, partners, provisioning state, installed document types, control-number counters, submissions, app preferences. Versioned migrations; control numbers allocated in a transaction. Chosen over JSON files for atomic counters, queryable activity and safe schema evolution. (Confirmation requested at the end.)
2. **Secret store (OS keychain)** — environment API keys and partner passwords, referenced from the database by opaque secret IDs only. Write-only through the API.
3. **Packs (data, not code)** — built-in packs for retail, healthcare and supply chain are embedded in the binary; a user packs directory can add or override packs. A pack is a manifest plus templates that defines: domain id and label, documents (code, name, note, standard, version, template file), EDI delimiters and ISA/GS version, and trace-value rules (names and patterns, for example `RFQ-{date}-{seq}`). Pack contents are ported from `load_domain_config`, `generate_trace_values` and `templates/` (copied, not moved, so v1 is untouched).

Everything is configured in the UI. Optional small addition (confirm): export/import an **environment profile** file for backup and sharing, secrets excluded.

## Extensible Architecture

- **Feature modules.** Each feature (environments, partners, documents, studio, activity) is a module in each layer: core service + storage port, server router, UI route. Adding a feature means adding a module, not editing shared code beyond registration.
- **Core:** services depend on small storage and HTTP-client traits, so they are testable with fakes and the SQLite/keychain implementations are swappable.
- **Resumable step runner.** Provisioning is a list of named steps whose outputs (IDs) are saved as each step succeeds; reused by any future multi-step flow.
- **Event bus.** Services publish progress and result events; the server streams them (SSE) and the activity feature records them.
- **Data-driven packs.** New domains, document types or trace rules are data. New standards (for example EDIFACT) would add a pack format handler, not change existing code.
- **Typed API contract.** A single schema (OpenAPI or equivalent) generates the TypeScript client, so server and UI cannot drift.
- **UI feature registry.** Each screen registers nav item, route and page; the shell renders navigation from the registry.

## Target Layout (indicative)

- `Cargo.toml` — workspace
- `crates/core` — features, storage traits, pack loader, rendering, B2B client
- `crates/storage` — SQLite repositories, migrations, keychain secret store
- `crates/server` — axum routers per feature, embedded UI, `trace-server` binary
- `apps/desktop/src-tauri` — Tauri shell (depends on `crates/server`)
- `ui/` — Vite + React 19 + TypeScript + Carbon
- `packs/` — built-in packs (manifests + templates), embedded at build time
- `wMB2BToolKit_Design/` — design reference, untouched
- `b2b-toolkit.sh`, `toolkit.env.example`, `templates/`, `.b2b-toolkit/` — legacy v1, untouched

## Data Model (conceptual)

- **Environment:** id, name, B2B host, MCSP instance ID, IAM host, CA bundle path (optional), isDefault. Secret ref: API key.
- **Partner** (belongs to an environment): id, name, org unit, ID type, ID value, username, full name, channel name. Secret ref: password. Provisioning state: partner internal ID, channel ID and endpoint URI, partner user ID, per-step status.
- **Installed document type** (per environment): transaction code → document type ID.
- **Counters** (per environment): ISA and GS control numbers, run count.
- **Submission:** time, environment, domain, transaction, sender and receiver partner, ISA/GS used, HTTP status, result, trace values.
- **Preferences:** default environment, inter-document delay, timeouts.

## Changes Required to the Design (to confirm)

| # | Design today | v2 |
|---|---|---|
| a | Environment modal asks for **Tenant** with help "Administration → Tenant settings → API access" (`App.tsx` L448-541) | **B2B host** (no scheme). Help text from `README.md` L65-126 (SaaS console → instance → Instance ID, API key, B2B host) with a link to `https://aws.console.saas.ibm.com`. Advanced: MCSP IAM host (Test/Prod) and CA bundle. Test connection button |
| b | `domainDocuments` lists other codes (L70-94) | Catalogs above, loaded from packs |
| c | Healthcare labelled "X12 005010" | Packs carry standard and version; built-ins use X12 004010 as the script installs. Confirm |
| d | Setup creates a P1/P2 pair in `localStorage`; nothing created on the tenant | Partner directory: add or edit one partner; "sample pair" shortcut; Create provisions on the tenant with per-step status and retry |
| e | Documents "Install selected" stores IDs locally | Calls the B2B API; installed state per environment from the database |
| f | Message studio builds one hard-coded 850 (`edi850`, `generateEdi`) | Renders real pack templates with selected partners and trace values; Submit posts to the sender's channel |
| g | Overview metrics static | Real values from the database |
| h | State and API key in `localStorage` (L193-233, 259-308) | API calls; key goes to the keychain and never returns to the browser |
| i | No history screen | New **Activity** screen with export (needs sign-off) |
| j | Vite config has Figma Make plugins, Tailwind, `.figma/`, host `0.0.0.0` | Clean Vite config, SCSS + Carbon only, dev host `127.0.0.1` |

## Sub-Tasks

### 1. Workspace scaffolding and v1 deprecation notice
- **Intent:** Establish repo structure and CI without touching v1.
- **Expected Outcomes:** Cargo workspace builds empty crates; `ui/` created from the design builds with a clean Vite config; README marks v1 deprecated and points to v2; `.gitignore` covers `target/`, `node_modules/`, UI build output.
- **Todo List:**
  1. Create the Cargo workspace (`core`, `storage`, `server`).
  2. Create `ui/` from the design: copy `package.json`, `index.html`, `src/`, `public/`, `tsconfig.json`; drop `.figma/`, Figma Make plugins, Tailwind and the `0.0.0.0` host. Check dependency versions are current and maintained; regenerate the lock file.
  3. Add the deprecation notice to `README.md` only (script unchanged).
  4. Extend `.gitignore`.
  5. CI: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, UI typecheck and build, dependency audit.
- **Relevant Context:** `wMB2BToolKit_Design/package.json`, `vite.config.ts`, `src/index.scss`; `README.md`; `.gitignore`.
- **Status:** `[x] done`
- **Completion notes (for later sub-tasks):**
  - **Workspace:** root `Cargo.toml` (resolver 3, edition 2024, `unsafe_code = "forbid"`, shared `[workspace.lints]`). Crates: `trace-core` (`crates/core`), `trace-storage` (`crates/storage`), `trace-server` (`crates/server`). No dependencies yet; sub-task 2 adds them (check latest versions then). `trace-server` is currently a lib with `BIND_HOST = "127.0.0.1"` and one test; the `trace-server` binary is added in sub-task 6. Crates inherit lints with `[lints] workspace = true`. `Cargo.lock` exists but is untracked until the user commits.
  - **UI:** `ui/` is a copy of the design's `src/`, `public/`, `index.html`, `tsconfig.json` (App.tsx still one file; split is sub-task 7). New `package.json` (`trace-ui`, pnpm 10.33, Node `^20.19 || >=22.12`), scripts `dev`, `build`, `preview`, `typecheck`. Deps: React 19.3, Carbon (`@carbon/react` 1.119, icons 11.91), Vite 8.3, TypeScript 7.0, sass, `@vitejs/plugin-react` 6.1. No Tailwind, Figma plugins, `@types/node`, `@/` alias or oxfmt. `pnpm-lock.yaml` regenerated; `pnpm audit` reports no known vulnerabilities.
  - **Vite:** `ui/vite.config.ts` is minimal; dev server `127.0.0.1:5173`, preview `127.0.0.1:4173`, `strictPort`. A dev proxy to the API server is added in sub-task 6.
  - **Fixes needed for TypeScript 7 / current Carbon** (the design did not typecheck): `tsconfig` lib/target raised to ES2021 (`replaceAll`); `InlineNotification` has no `actions` prop, so the Overview "Configure" button is now a child of the notification.
  - **Logo:** the user supplied a logo (cloud with three cubes). Source PNG was `~/Downloads/ffb9b8eb-5eed-4fc4-a6c8-4a1382852dcc.png` (1536x1024); resized copies are `ui/public/logo.png` (512 px, used in the header via `.app-logo`) and `ui/public/favicon.png` (64 px). The design's Figma `favicon.svg` was removed. A square or SVG logo would be better for the desktop app icon (sub-task 9); the full-size original is not stored in the repo.
  - **Known build noise:** `pnpm build` prints about 90 "didn't resolve at build time" warnings for `~@ibm/plex/...woff2` font URLs from Carbon's SCSS (same in the design). The build succeeds; the CSS bundle is about 1.2 MB because all Plex font faces are included. Trimming fonts is a candidate for sub-task 7 or 10. pnpm also reports "Ignored build scripts" for Carbon packages; they are not needed.
  - **README:** only an 8-line deprecation block was added at the top. v2 usage docs come in sub-task 10.
  - **.gitignore:** added `target/`, `node_modules/`, `ui/dist/`, `*.sqlite*`, `*.db`, `.env`, `.env.*` (except `.env.example`).
  - **CI:** `.github/workflows/ci.yml` (push to main, PRs, weekly): Rust fmt, clippy `-D warnings`, test with `--locked`; UI `--frozen-lockfile` install, typecheck, build; audit job with `rustsec/audit-check` and `pnpm audit --audit-level=moderate`. Action majors: checkout v7, setup-node v7, pnpm/action-setup v6, rust-cache v2, audit-check v2. The workflow has not run on GitHub yet, and `cargo audit` is not installed locally, so the Rust audit is unverified until CI runs. Consider `cargo deny` for licences once dependencies exist.
  - **Validated locally:** `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked` (1 test), and in `ui/`: clean `pnpm install --frozen-lockfile`, `pnpm typecheck`, `pnpm build`, `pnpm audit`.
  - **Untouched as required:** `b2b-toolkit.sh`, `toolkit.env.example`, `templates/`, `wMB2BToolKit_Design/`. `templates/retail/810.edi` and `820.edi` were already modified in the working tree before this work started (not by this sub-task); sub-task 3 should confirm which version is intended before copying them into packs.

### 2. Core foundations: storage, secrets, feature-module pattern
- **Intent:** Build the extensible base every feature sits on.
- **Expected Outcomes:** SQLite database with migrations; keychain-backed secret store; storage traits and fakes; event bus; step-runner; a documented pattern for adding a feature module; environments and partners can be created, validated and persisted.
- **Todo List:**
  1. Define core traits: repositories per feature, secret store, clock, HTTP client, event publisher.
  2. Implement SQLite repositories and versioned migrations (initial schema from the data model); atomic control-number allocation.
  3. Implement keychain secret store keyed by opaque IDs; define the fallback when no keychain exists (see risks).
  4. Implement the resumable step runner and the event bus.
  5. Environment and Partner types with validation: host without scheme, Instance ID format, IAM host Test/Prod or custom, partner ID format.
  6. Write a short "adding a feature" guide in the repo docs.
  7. Unit tests with in-memory fakes and a temp-file SQLite database.
- **Relevant Context:** data model above; design types `Environment` (L48) and `Partner` (L56).
- **Status:** `[x] done`
- **Completion notes (for later sub-tasks):**
  - **Crates and layout.** `trace-core` (no I/O: models, validation, ports, services, step runner, event bus) and `trace-storage` (SQLite `Database` + secret stores). `trace-server` is unchanged. Guide: `docs/adding-a-feature.md`. `rust-version` is now 1.88 (required by `keyring` 4); CI uses `dtolnay/rust-toolchain@stable`, which is fine.
  - **Dependencies** (all latest at the time, pinned by the committed `Cargo.lock`, declared once in `[workspace.dependencies]`): rusqlite 0.40 (bundled SQLite), keyring 4.2 / keyring-core 1.0, aes-gcm 0.11, argon2 0.6, secrecy 0.10, zeroize, getrandom 0.4, base64 0.23, serde, serde_json, thiserror 2, time 0.3, tokio 1.54 (core uses only `sync`; tests add `rt`, `macros`), uuid 1.28, tempfile (dev). `tokio` is `default-features = false`; the server crate must enable `rt-multi-thread`, `net`, etc. itself. No `reqwest`/HTTP server crate yet.
  - **Ports in `trace-core`** (all `Send + Sync`, synchronous except HTTP/steps): `EnvironmentRepository`, `PartnerRepository`, `CounterRepository` (`peek`, `allocate(env, n)`, `increment_run_count`), `InstalledDocumentRepository`, `PreferenceRepository`, `SubmissionRepository`, `StepStore`, `SecretStore`, `Clock`, `EventPublisher`, `HttpClient` (async, returns `BoxFuture`; request/response `Debug` redacts headers and bodies). `Database` implements every repository and `StepStore`; one `Arc<Database>` can be passed for each port.
  - **Real HTTP client is NOT written.** Only the `HttpClient` port and `FakeHttp` exist. Sub-task 4 implements it (explicit timeouts, TLS validation on, optional per-environment CA bundle from `Environment::ca_bundle_path`, no `--insecure`). `Preferences` carries the timeouts (defaults 2 / 30 / 120 s as in the script).
  - **Fakes** are in `trace_core::testing` behind the `testing` feature (`MemoryDb` implements all repositories, `MemorySecretStore`, `FixedClock`, `RecordingPublisher`, `FakeHttp`). Add `trace-core = { path = "../core", features = ["testing"] }` under `[dev-dependencies]` in other crates.
  - **Services.** `EnvironmentService::new(env_repo, partner_repo, secrets, clock, events)` and `PartnerService::new(partner_repo, env_repo, step_store, secrets, clock, events)`. Both return model types; use `.view()` to get `EnvironmentView` / `PartnerView` for API responses (secrets appear only as `has_api_key` / `has_password`). Models deliberately do not derive `Serialize`. Environment name unique (case-insensitive); exactly one optional default (`set_default`); delete cascades to partners, counters, installed docs, step records and removes secrets after the rows. API key rotation writes a new secret id first, then deletes the old one. Partner uniqueness per environment: name, DUNS value, username, channel name. Only `IdType::Duns` exists (DUNS 9 to 15 digits). A partner cannot be edited once any step has succeeded (`Error::Conflict`); failed steps do not block editing. Passwords 8 to 128 chars, API keys printable ASCII.
  - **Validation rules** (in `validation.rs`): hosts lower-cased with no scheme, path or credentials (the B2B host may carry an optional `:port`, the IAM host may not); instance id is a UUID (canonicalised); IAM host presets `IAM_HOST_TEST` (`account-iam.platform.test.saas.ibm.com`) and `IAM_HOST_PROD` (`account-iam.platform.saas.ibm.com`), custom host allowed; CA bundle path must be absolute. `Collector` returns every field error at once as `Error::Validation`.
  - **Errors.** `trace_core::Error` variants `Validation`, `NotFound`, `Conflict`, `Cancelled` carry user-safe text; `Storage`, `Secret`, `Http` carry internal detail and `public_message()` hides it. The server (sub-task 6) must map `Validation` -> 400/422, `NotFound` -> 404, `Conflict` -> 409, others -> 500 with a generic body, and log the detail.
  - **Events.** `Event { at, level, kind, message, data }`; `EventBus` wraps a tokio broadcast channel (capacity 256, slow subscribers get `Lagged` rather than blocking). `bus.subscribe()` for SSE/WebSocket; `bus.into_publisher()` gives the `Arc<dyn EventPublisher>`. Kinds used: `environment.created|updated|deleted`, `partner.created|updated|deleted`, and `{prefix}.step.started|succeeded|skipped|failed` from the step runner (a cancelled run is reported through `RunReport::cancelled`, no event). Event data holds ids and names only.
  - **Step runner.** `StepRunner::new(store, events, clock).run(scope, kind_prefix, &[Box<dyn Step>], &CancelToken) -> RunReport`. Succeeded steps are skipped on re-run (their saved output is put back into `StepContext`), failed/missing steps run again, a failing step stops the run and is reported in the `RunReport` (not as `Err`). Step outputs must be ids only. `FnStep::new(name, closure)` builds steps from closures. Partner scope is `partner_scope(&id)` = `partner:<uuid>`; `PartnerService::update/delete` look at / clear that scope. Sub-task 4 should use the step names consistently, since they are the persisted keys.
  - **SQLite.** `Database::open(path)` creates parent dirs 0700 and the file 0600 (Unix), turns on WAL and foreign keys, and runs migrations (`PRAGMA user_version`; refuses a newer database; each migration is transactional). Schema is `crates/storage/migrations/0001_initial.sql`: `environments`, `partners`, `step_records`, `counters`, `installed_documents`, `submissions`, `preferences`, STRICT tables, timestamps as INTEGER microseconds (UTC), unique indexes enforce the rules above. `Database` is synchronous behind one `Mutex<Connection>` and cheap to `clone()`; in async server code call it through `tokio::task::spawn_blocking`. `Database::open_in_memory()` exists for tools.
  - **Control numbers.** `allocate(env, n)` (1 to 1000) runs in a `BEGIN IMMEDIATE` transaction and returns consecutive `ControlNumbers { isa, gs }`; both counters start at 1 and wrap 999,999,999 -> 1 (`counters::advance`). The two counters are independent columns (`isa_next`, `gs_next`), so both advance together today. An unknown environment gives `NotFound`. A concurrency test with 8 separate connections confirms there are no duplicates and no gaps. Format as `{:09}` when rendering. `run_count` feeds `{seq}` in trace values (sub-task 3). The script's counters are not migrated.
  - **Submissions and installed docs.** `Submission` keeps sender/receiver partner ids as `Option` (set null when a partner is deleted, so history stays). `trace_values` is a JSON object, never secrets. `InstalledDocument { transaction_code (3 digits), document_type_id, installed_at }` is upserted per environment and transaction code. Pack/domain are not part of installed docs yet; if sub-task 3/4 needs per-domain tracking, add a migration `0002` (never edit `0001`).
  - **Secrets.** `SecretId` is `sec-<uuid>`; only this id is stored in SQLite (tested by scanning the DB + WAL bytes for the real values). `trace_storage::open_secret_store(data_dir, SecretBackendPolicy::{Auto, Keychain, EncryptedFile}, passphrase)` returns `(Arc<dyn SecretStore>, SecretBackend)`: `Auto` probes the OS keychain (keychain service name `ibm-b2b-toolkit-trace`) and falls back to `data_dir/secrets.enc` (Argon2id with the crate defaults 19 MiB / t=2 / p=1, AES-256-GCM, header as AAD, fresh nonce per write, atomic 0600 write). The fallback needs a passphrase; the CLI/server should read it from `TRACE_SECRETS_PASSPHRASE` (the error text names it) and never log it. `trace-server` (sub-task 6) must call this once at start-up and report which backend is active (without secrets) so the UI can show it. Tauri (sub-task 9) can use the same function.
  - **Not exercised for real:** the macOS Keychain / Windows / Linux secret service were only tested through the keyring mock store plus the probe logic; no real keychain write happened in tests (avoids prompts in CI). Verify manually once on each OS before release (sub-task 10).
  - **Tests.** 45 in `trace-core` (validation, services, steps, counters, fakes), 49 in `trace-storage` (migrations incl. rollback, encrypted-file store incl. tamper/wrong passphrase, repositories against a temp-file DB, cascades, concurrency, file permissions, no secrets in DB), 1 in `trace-server`. Run with `cargo test --workspace --locked`.
  - **Validated locally:** `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked` all pass. `cargo audit` / `cargo deny` are still not installed locally; the audit job in CI is the first real check. keyring's macOS backend pulls in `security-framework`; Linux and Windows backends were not compiled here, so CI on those OSes (if added) may need extra `keyring` features or system packages (`libdbus` is NOT needed for the current backends, but check when adding a Linux job).
  - **Untouched as required:** `b2b-toolkit.sh`, `toolkit.env.example`, `templates/`, `wMB2BToolKit_Design/`.

### 3. Packs and EDI rendering
- **Intent:** Make domains, documents and trace rules data, and reproduce the script's EDI output exactly.
- **Expected Outcomes:** Built-in retail, healthcare and supply chain packs load from embedded files; a user packs directory can add or override packs; rendering with the same inputs matches the script's `--dry-run` output byte for byte.
- **Todo List:**
  1. Define the pack manifest schema and loader with validation and clear errors.
  2. Create the three built-in packs from `load_domain_config`, `generate_trace_values` and `templates/` (copies); catalog names and notes checked against template contents.
  3. Port template rendering: ISA/GS control (ISA 9 digits), dates and times, `P1_ISA`/`P2_ISA` padded and truncated to 15, `P1_GS`/`P2_GS`, trace values, delivery date +14 days. Injectable clock.
  4. Linked trace values with user overrides.
  5. "Peek" next control numbers for preview without consuming them.
  6. Golden tests against captured `--dry-run` output with a fixed clock.
- **Relevant Context:** `b2b-toolkit.sh` L349-421, L880-983; `templates/{retail,healthcare,supply_chain}/*.edi` (each template fixes its own direction: 850 is P1 to P2, 840 is P2 to P1); design `generateEdi` (L146).
- **Status:** `[x] done`
- **Completion notes (for later sub-tasks):**
  - **New files.** `crates/core/src/packs/` (mod.rs, manifest.rs, loader.rs, catalog.rs, trace_values.rs, render.rs, tests.rs). Registered as `pub mod packs` in `crates/core/src/lib.rs`. New workspace dep: `toml = "1.1.8"` (also added to `trace-core` `[dependencies]`). `tempfile` added to `trace-core` `[dev-dependencies]`. `Cargo.lock` updated (3 new packages: `serde_spanned`, `toml`, `toml_writer`).
  - **Pack manifests.** `packs/retail/pack.toml`, `packs/healthcare/pack.toml`, `packs/supply_chain/pack.toml`. Templates copied (not moved) from `templates/` into `packs/{domain}/templates/`. The working-tree versions of `templates/retail/810.edi` and `templates/retail/820.edi` (already modified before this task, not by this sub-task) were used as-is; the copies in `packs/retail/templates/` reflect those local modifications.
  - **Manifest schema** (`manifest.rs`). `PackManifest { pack: PackMeta, trace_rules: BTreeMap<String,String>, documents: Vec<DocumentDef> }`. `PackMeta` carries id, label, standard, version, terminator, isa_version. `DocumentDef` carries code, name, note, template (relative path), sender. `Sender` enum `{ P1, P2 }` is TOML-deserialised as `"p1"` / `"p2"`.
  - **Loader** (`loader.rs`). `parse_pack(id_hint, toml_str, lookup_fn)` validates and resolves templates. `load_built_in()` uses `embed_pack!` macro with `include_str!` from `CARGO_MANIFEST_DIR/../../packs/...` (built-in packs embedded in binary at compile time). `load_from_dir(path)` reads a directory for user packs; returns `(Vec<LoadedPack>, Vec<LoadError>)` so a single bad pack doesn't abort loading. `LoadError` variants: `ManifestParse`, `MissingTemplate`, `Io`, `EmptyId`.
  - **Catalog** (`catalog.rs`). `Catalog::built_in()` → three packs in domain-sequence order. `Catalog::with_user_dir(path)` → built-ins + user packs; user pack with same `id` replaces the built-in. `catalog.get(id)` / `catalog.all()`.
  - **Trace values** (`trace_values.rs`). `generate_trace_values(trace_rules, clock, run_count) -> TraceValues`. Pattern language: `{date}` = YYYYMMDD, `{seq}` = run_count mod 10,000 zero-padded to 4 digits. Patterns match `b2b-toolkit.sh` `generate_trace_values` exactly (same prefix-date-seq format). `delivery_date(clock)` = +14 days. `TraceValues` wraps `BTreeMap<String,String>`; callers call `.set(key, value)` to apply user overrides before passing to the renderer.
  - **Renderer** (`render.rs`). `render_template(template, ctx) -> Result<String, RenderError>`. `RenderContext<'a>` holds p1_duns, p2_duns, isa_ctrl (u32), gs_ctrl (u32), trace (&TraceValues), clock (&dyn Clock). Fixed substitutions applied first (ISA_CTRL, GS_CTRL, ISA_DATE, DATE, TIME, P1_ISA, P2_ISA, P1_GS, P2_GS, DELIVERY_DATE), then all TraceValues keys. Unresolved `{{KEY}}` placeholders after all substitutions return `RenderError::UnknownPlaceholder`. ISA_CTRL formatted as `{:09}` (9 digits, zero-padded). P1_ISA/P2_ISA left-padded to exactly 15 chars, truncated if longer (matching `printf '%-15s' "${DUNS:0:15}"` in the script). P1_GS/P2_GS = raw DUNS. ISA_DATE = last 6 chars of YYYYMMDD (i.e. YYMMDD). GS_CTRL unpadded. The renderer does NOT allocate control numbers; that is the caller's responsibility (sub-task 5).
  - **"Peek" for preview.** The `CounterRepository::peek` port from sub-task 2 is already present and is the correct way to get next control numbers for preview without consuming them. The renderer is stateless and can be called with peeked numbers.
  - **Sender selection.** Each `DocumentDef` carries `sender: Sender`. The submission engine (sub-task 5) reads this to determine which partner's credentials and channel URL to use: `Sender::P1` → post to P1's channel with P1's Basic Auth; `Sender::P2` → post to P2's channel with P2's Basic Auth. ISA06 in the template matches this: P1-sender templates have `{{P1_ISA}}` in ISA field 6, P2-sender templates have `{{P2_ISA}}` there. Tests confirm alignment across all 20 templates.
  - **doctype_key for sub-task 4.** The script's `txn_to_doctype_key` (L418-421) is `printf "WMX12-4010--%s---------"` (nine trailing dashes). *Correction made in sub-task 4:* the format string suggested here earlier had the wrong dash count; the implemented `doctype_key(standard, version, txn)` matches the script byte for byte (see sub-task 4 notes).
  - **Tests.** 41 new pack-system tests: 19 in `render.rs`, 4 in `trace_values.rs`, 1 doctest, 17 in `tests.rs` (catalog, sequences, sender direction, trace value generation, user overrides, user dir override, error paths, full golden renders for all 20 templates). All 137 workspace tests pass.
  - **Validated locally:** `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked` all pass. The `Cargo.lock` was updated (3 new packages) and should be committed.
  - **Template files note:** `templates/retail/810.edi` and `templates/retail/820.edi` had uncommitted local modifications before this sub-task started (noted in sub-task 1 completion notes and confirmed). The copies at `packs/retail/templates/810.edi` and `packs/retail/templates/820.edi` reflect the current working-tree versions. If the modifications were intentional they should be committed; if not, both `templates/retail/` and `packs/retail/templates/` files should be reverted together.
  - **Untouched as required:** `b2b-toolkit.sh`, `toolkit.env.example`, `templates/`, `wMB2BToolKit_Design/`.

### 4. MCSP token and tenant provisioning
- **Intent:** Port authentication and idempotent provisioning for any number of partners and per-domain document types.
- **Expected Outcomes:** Test connection works. Creating a partner runs create partner, create channel, associate channel, create user, associate user as saved steps with retry. Installing documents handles "already exists". Token is cached in memory and re-acquired from the keychain API key on expiry or 401.
- **Todo List:**
  1. Token acquisition (`Mcsp-ApiKey` header to the MCSP IAM apikeys/token URL); cache and refresh on 401.
  2. Port `create_partner`, `create_channel`, `associate_channel_to_partner`, `create_partner_user`, `associate_user_to_partner` as step-runner steps.
  3. Port `install_document_type` including the conflict path (409 or "already exists", then GET by key `WMX12-4010--<txn>---------`); standard and version come from the pack.
  4. HTTP client with explicit timeouts, certificate validation on, optional CA bundle.
  5. Mock-server tests including partial failure and resume.
- **Relevant Context:** `b2b-toolkit.sh` L429-456, L458-693, L695-780, L782-865; `README.md` troubleshooting.
- **Status:** `[x] done`
- **Completion notes (for later sub-tasks):**
  - **New crate `crates/http` (`trace-http`).** `ReqwestClient::new()` implements the core `HttpClient` port with reqwest 0.13.5 (`default-features=false`, features `rustls`, `http2`, `system-proxy`; rustls + aws-lc-rs + platform verifier). Build it once and share as `Arc<dyn HttpClient>`; sub-task 6 wires it in (`trace-server`/`trace-storage` do not depend on it yet). Rules enforced: https only, no redirects, TLS >= 1.2, certificate validation always on (there is no insecure switch and `--insecure` was deliberately not ported), connect timeout 10 s, per-request total timeout from `HttpRequest.timeout`, 8 MiB response cap, optional CA bundle (<= 1 MiB, PEM, merged with system roots; clients are cached per bundle path + mtime). Error text never contains URL, headers, body or the CA path. Dev deps for tests: `rcgen` 0.14.10, `tokio-rustls` 0.26.6 (11 tests against a local TLS server with a throw-away CA).
  - **Core change.** `HttpRequest` gained `ca_bundle: Option<String>` plus a `.ca_bundle()` builder (shown in Debug; header values stay redacted). `trace-core` now also depends on `base64` (JWT `exp` parsing) and tokio `time` (`DocumentService` pause).
  - **New module `crates/core/src/b2b/`** (`api.rs`, `connection.rs`, `documents.rs`, `provisioning.rs`, `tests.rs`), exported from `trace_core::b2b`: `B2bApi`, `ConnectionService`/`ConnectionReport`, `ProvisioningService`/`PartnerProvisioning`/`StepState`/`PROVISION_STEPS`, `DocumentService`/`DocumentReport`/`InstallReport`/`DocumentOutcome`, `doctype_key`.
  - **`B2bApi::new(http, secrets, prefs_repo, clock)`.** `call(&env, method, path, body)` adds the bearer token, uses `Preferences.request_timeout_secs` and the environment's `ca_bundle_path`. Token is cached per environment (fingerprint of API-key secret id + IAM host + instance id), refreshed 60 s before JWT `exp` (fallback TTL 300 s) and force-refreshed once on a 401. Call `forget_token(&env_id)` after an environment is updated or deleted (sub-task 6). Ids placed in URL paths are validated (`path_segment`). Error excerpts redact known secrets and are cut to 300 chars; this matters because `steps::public_message` keeps `Error::Http`/`Conflict` text in step records.
  - **Connection test.** `ConnectionService::test(&env_id) -> ConnectionReport{ok,message}` always fetches a fresh token (never uses the cache) and returns failures as `ok:false` with a secret-free message (not as `Err`, except `NotFound`). It validates IAM host, instance id and API key only; it does not touch the B2B host. Emits `environment.connection_tested`.
  - **Provisioning.** `ProvisioningService::new(api, envs, partners, runs/steps repo, secrets, clock, events)`; `provision(&partner_id, &CancelToken) -> Result<RunReport>` and `status(&partner_id) -> PartnerProvisioning` (no tenant calls). Runs are serialised per partner; a second concurrent run returns `Error::Conflict`. A step failure still returns `Ok(RunReport)` (with the failed step), so the server must map by the report, not by `Err`. Persisted step names and outputs: `create_partner` -> `{partner_id}`, `create_channel` -> `{channel_id, endpoint_uri}`, `associate_channel` -> `{}`, `create_user` -> `{user_id}`, `associate_user` -> `{}`. Succeeded steps are skipped on re-run, so a complete partner makes zero tenant calls and a failed run resumes at the failed step. Paths, payloads and "already exists" handling follow the script. The partner password is read from the SecretStore only when the `create_user` step runs; a missing secret fails that step with a "set it again" message.
  - **For sub-task 5 (submission).** Post to `PartnerProvisioning.channel_endpoint_uri` (from the `create_channel` output). Basic Auth uses `partner.username` and the password from the SecretStore (`partner.password_secret`); never log or return it. Reuse `HttpClient` with `timeout = Preferences.payload_timeout_secs` and `.ca_bundle(env.ca_bundle_path)`. Do not use the MCSP bearer token for channel posts (as in the script).
  - **Documents.** `DocumentService::install(env, pack_id, Option<&[codes]>, &CancelToken)` posts document types in order and stops at the first failure like the script. Documents already in `installed_documents` are skipped. A 409 (or "already exists" text) is confirmed with GET by key; the key is stored as the id (as the script does) -> `AlreadyExisted`. Other outcomes: `Installed`, `AlreadyInstalled`, `Failed`, `NotRun`. 500 ms pause after created documents (`.with_pause`). Emits `document.installed` / `document.install_failed`.
  - **Document type key (verified).** The script is `printf "WMX12-4010--%s---------"` (nine trailing dashes, checked with `cat -e`). `doctype_key(standard, version, txn)` builds `WM{standard}-{version minus leading zeros}--{txn}---------`; for `X12`/`004010` this equals the script output byte for byte. A test extracts the printf format from `b2b-toolkit.sh` (`include_str!`) and asserts equality for several transactions; another asserts that all built-in packs use `X12`/`004010`. Invalid versions (non-digits, all zeros) are rejected.
  - **Events.** The step runner emits `partner.provision.step.*`; the service adds `partner.provisioned|provision_failed|provision_cancelled`, so Activity (sub-task 8) will see both kinds and may want to show only one.
  - **Tests.** 36 b2b tests with `FakeHttp` (token headers/URL/caching/expiry/401 retry, connection failures, exact paths and payloads for all five steps, resume after failure, idempotent re-runs, cancel, two partners, documents incl. 409 and message-text conflict, secrets absent from events, records and Debug output) plus 11 in `trace-http`. Workspace totals: core 122, http 11, storage 49, others 2, all passing. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace --locked` pass; `Cargo.lock` changed (reqwest/rustls/rcgen tree) and must be committed.
  - **Caveats.** Not run against a real tenant; endpoint shapes come from the script and `FakeHttp`. A partner that already exists on the tenant (but not in local state) is still not handled: `create_partner` surfaces the tenant error (existing open item). The `4010` part of the key is derived from the pack version and only verified for `004010`. reqwest pulls in aws-lc-rs (C/cmake build), so Linux/Windows CI and packaging (sub-task 10) must be checked. `cargo audit` has not been run locally. Untouched as required: `b2b-toolkit.sh`, `toolkit.env.example`, `templates/`, `wMB2BToolKit_Design/`.

### 5. Submission engine
- **Intent:** Port document submission with sender routing and observable progress.
- **Expected Outcomes:** Submitting N documents renders each with consecutive ISA/GS numbers, posts to the sender partner's channel with that partner's credentials, applies the delay, records every result, and publishes events. Generate-only mode returns EDI without submitting. A manually entered partner ID works for generation only.
- **Todo List:**
  1. Port `post_edi_document`: sender from ISA06, Basic Auth with the sender's credentials, `Content-Type: application/EDI`, URL normalisation for bare hosts.
  2. Allocate ISA/GS numbers in a transaction at submit time; update run count.
  3. Delay between submissions; cancel support.
  4. Events (rendered, posting, result); persist submissions.
  5. Tests with mock channel endpoints.
- **Relevant Context:** `b2b-toolkit.sh` L985-1130; design `Generator` (L844).
- **Status:** `[x] done`
- **Completion notes (for later sub-tasks):**
  - **New module `crates/core/src/submit/`** (`mod.rs`, `tests.rs`), exported as `trace_core::submit`. Construct with `SubmissionService::new(SubmissionDeps { http, catalog, envs, partners, counters, submissions, prefs, secrets, provisioning: Arc<ProvisioningService>, clock, events })`. All fields are `Arc<dyn Port>` except `provisioning` and `catalog`; `MemoryDb`/`Database` can fill every repository field. `.with_delay(Duration)` overrides the inter-document delay (tests use `Duration::ZERO`); otherwise it comes from `Preferences.inter_document_delay_secs`. Sub-task 6 must hand the same `Arc<ProvisioningService>` to this service, because the channel URL is read from `ProvisioningService::status`.
  - **API.**
    - `submit(&SubmitRequest, &CancelToken) -> Result<SubmitReport>`.
    - `submit_document(&SubmitRequest, code, &CancelToken)`, which is `submit` with `codes = [code]`.
    - `preview(&PreviewRequest) -> Result<PreviewReport>` (synchronous, no HTTP).
    - `SubmitRequest { environment_id, pack_id, codes: Option<Vec<String>>, p1_partner_id, p2_partner_id, trace_overrides }` (`codes: None` sends the whole pack sequence). It derives `Deserialize`, so the server can take it as a JSON body.
    - `PreviewRequest` is the same but `p1`/`p2` are `Party` values (`{"type":"saved","partner_id":..}` or `{"type":"manual","id_value":"555000111"}`). A manual DUNS (9-15 digits) works for generation only, as the plan requires; the studio's "manual ID" mode maps to `preview` only.
    - Reports derive `Serialize`.
  - **Reports.** `SubmitReport { run_count, trace_values, documents: Vec<DocumentSubmission>, cancelled }`. Each `DocumentSubmission` has `transaction_code`, `outcome` (`submitted | failed | not_run`), `isa_control`, `gs_control`, `http_status`, `error`, `submission_id` and `duration_ms`. Like provisioning, a failed post is `Ok(report)` with `outcome: failed`; map the HTTP response from the report, not from `Err`. `Err` is only returned before anything is reserved or posted: `Validation` (unknown or empty codes, bad overrides, partner of another environment), `NotFound` (environment, pack, partner), `Conflict` (partner not fully provisioned, no endpoint, plain `http://` endpoint, no password or secret missing). `PreviewReport { run_count, trace_values, documents: Vec<PreviewDocument{transaction_code, name, sender, isa_control, gs_control, payload}> }`.
  - **Sender routing.** The sender is the pack's `DocumentDef.sender` (`p1`/`p2`), which is what the templates encode in ISA06; the script derived the same thing by matching ISA06 to the DUNS values. The caller maps the pack's P1/P2 to saved partners. POST goes to that partner's `channel_endpoint_uri` with `Authorization: Basic base64(username:password)` and `Content-Type: application/EDI`. A bare host gets `https://` prefixed (as in the script); `http://` endpoints are refused (the `trace-http` client is https-only). Timeout is `Preferences.payload_timeout_secs`; the CA bundle is the environment's `ca_bundle_path`. No MCSP token is requested for channel posts (a test asserts there is exactly one request per document).
  - **Run flow (same order as the script).**
    1. Validate and check credentials for every sender used in the run.
    2. Dry-render all documents with placeholder numbers, so a broken template fails before any counter is touched.
    3. Cancel check.
    4. `increment_run_count`, then trace values derived from the new count (`{seq}` = run count; `{date}` from the clock) plus `trace_overrides`.
    5. One `allocate(env, n)` call for the whole run, so ISA and GS numbers are consecutive and atomic.
    6. For each document: render, post, persist, emit events, then sleep (not after the last document). The sleep wakes every 100 ms to notice cancellation.
  - **Stop on first failure** (non-2xx or transport error). Remaining documents are `not_run`, have no numbers and no submission rows. Their reserved numbers are skipped for good (gaps are legal in X12; re-use is not), so the next run continues after them. This differs from the script, which carried on after failures, and is a deliberate plan decision. Cancel (before start or during the delay) sets `cancelled: true`; cancelling before start changes nothing (no run count, no numbers).
  - **Trace overrides.** Keys must exist in the pack's `trace_rules`; values are 1-50 chars of letters, digits and `- _ . / :`, so user text cannot inject EDI delimiters. `SubmitReport.trace_values` and the stored `trace_values` JSON are, respectively, the complete set and only the values used by that document's template.
  - **Preview contract.** `preview` uses `CounterRepository::peek` and `counters::advance` for consecutive ISA/GS numbers and trace values from `run_count + 1`, so a preview is byte-identical to the next submission as long as the counters and the clock minute do not change (a test sends right after a preview and compares payloads). It allocates nothing, increments nothing, posts nothing, saves nothing, emits no events and needs no provisioning or password. The environment is only needed for the counters.
  - **Persistence (migration `0002_submission_details.sql`, schema version 2).** `submissions` gained nullable `payload`, `response_excerpt`, `duration_ms`; `Submission` has matching `Option` fields. Old rows keep NULLs (a migration test checks this). `result` is `success` or `failed`; `generated` stays reserved for stored preview records (the engine does not write them). Payload contains no secrets. Excerpts use `b2b::api::excerpt` (now `pub(crate)`): known secrets are replaced by `***`, control characters are flattened, 300 chars max. The redaction list is the password and the Base64 `user:password` value, which covers servers that echo the request.
  - **Events** (ids, status and counts only): `submission.rendered`, `submission.posting`, `submission.result` (level `error` on failure; data has `submission_id`, `http_status`, `duration_ms`, `result`, `error`) and `submission.cancelled`. Every event carries `environment_id`, `transaction_code`, `sender` and the control numbers. Activity (sub-task 8) can read the same data from `submissions`; it can show `submission.result` only.
  - **Test helper.** `FakeHttp::clear_requests()` was added to the `testing` module.
  - **Tests.** 21 in `crates/core/src/submit/tests.rs` (`MemoryDb` + `FakeHttp`): URL, headers, timeout, CA bundle, sender routing, scheme normalisation, consecutive and cross-run unique control numbers, concurrent runs, stop on failure and transport failure, preview purity and equality with the later send, manual IDs, unprovisioned partner, overrides, validation without side effects, delay (between documents only; preferences default), cancel (before start and during the delay), event sequence, and secrets absent from events, reports, rows, `Debug` and the recorded requests, including a server that echoes the credentials. 4 in `crates/storage/src/tests/submit.rs` (SQLite file): every field survives a reopen, unique numbers across concurrent runs, preview leaves the database untouched, and the database file, events and reports contain no password or Base64 credentials. Plus one migration test (v1 database with a row upgraded to v2). Workspace totals: core 143, http 11, storage 54, others 2, all passing. `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace --locked` pass; `Cargo.lock` is unchanged by this sub-task.
  - **For sub-task 6.** Wire one `SubmissionService` next to `ProvisioningService`. Studio submit should run `submit` in a spawned task with a `CancelToken` (keyed by a run id) and stream `submission.*` events over SSE; return the `SubmitReport` when the task ends. Map `Error::Validation`/`NotFound`/`Conflict` through the existing mapping, and map `outcome: failed` to a normal 200 with the report. For the studio's "list of past submissions" use `SubmissionRepository::list_recent` (now includes `payload`, `response_excerpt`, `duration_ms`; consider omitting `payload` from list responses and exposing it in a detail call because it can be large). Nothing prevents two concurrent submit runs for the same environment; control numbers stay unique, but the server may want to serialise per environment for a readable UI.
  - **Caveats.**
    - Not run against a real tenant: the channel behaviour is simulated with `FakeHttp`, and no real TLS channel POST has been tested (only the generic `trace-http` TLS tests exist).
    - The Basic Auth credentials are the partner user created during provisioning. If the tenant needs a different realm or header, only `Route::authorization` changes.
    - A partner whose provisioning has succeeded locally but whose `create_channel` output has no `endpoint_uri` is reported as a `Conflict`.
    - The run count is incremented before posting, so a run that fails at the first post still consumes a run number (as the script's counter would). Sequence stops are the only behavioural difference from the script, and the script's `--insecure` was not ported.
    - There are no persisted "generated" records for previews, and no per-environment run lock.
    - Untouched as required: `b2b-toolkit.sh`, `toolkit.env.example`, `templates/`, `wMB2BToolKit_Design/`.

### 6. Server: local HTTP API
- **Intent:** One API surface for web and desktop, composed from feature routers.
- **Expected Outcomes:** axum server on `127.0.0.1` with token auth; routers for environments (with test connection), partners, documents, studio (preview and submit with streamed progress), activity and preferences; generated typed client for the UI; embedded UI with SPA fallback; `trace-server` prints the URL including the token.
- **Todo List:**
  1. Define the API schema and generate the TypeScript client.
  2. Bearer-token middleware, `Host` validation, strict CORS.
  3. One router per feature, composed in a single registration point.
  4. SSE for submission and provisioning events.
  5. Secrets write-only; responses carry "is set" flags only.
  6. Serve embedded UI; Vite dev proxy to the server.
  7. API tests.
- **Relevant Context:** sub-tasks 2 to 5.
- **Status:** `[x] done`
- **Completion notes (sub-task 6):**
  - **Run it.** `cargo run -p trace-server -- --ui-dir ui/dist` prints `API:` and `Open: http://127.0.0.1:<port>/#token=<token>`. `--help` lists `--data-dir`, `--port`, `--ui-dir`, `--allow-origin` (repeatable, dev servers) and `--token` (>= 32 chars, dev only; env `TRACE_API_TOKEN`). `TRACE_SECRETS_PASSPHRASE` enables the encrypted-file secret store when there is no keychain. The `embed-ui` cargo feature compiles `ui/dist` into the binary (build the UI first); it passes clippy but was not run end to end. Full guide: `docs/server.md`.
  - **Crate layout.** `trace_server` lib: `serve(config)`/`serve_with(config, Parts)` -> `ServerHandle {addr, base_url, token, shutdown, wait}`; `state.rs` wires every service once from one `Arc<Database>`; `routers/<feature>.rs` plus a single `api()` in `routers/mod.rs`; `extract.rs` (`ApiJson`, `ApiQuery`, `PathId`) gives rejections that never echo input; `error.rs` maps `CoreError`; `auth.rs` holds the guards. The Tauri shell (sub-task 9) should call `serve`/`serve_with` and hand `handle.token()` to the webview.
  - **Security.** Binds 127.0.0.1 only. `Host` must be `127.0.0.1:<port>` or `localhost:<port>` (all routes). `/api`: `Origin`, when present, must be the server's own or an `--allow-origin` value; then `Authorization: Bearer` is compared in constant time; the token is never accepted in a URL. CORS only exists for the extra origins. CSP, nosniff, X-Frame-Options DENY on everything, `no-store` on the API. Blocking core calls run through `spawn_blocking` (`error::blocking`).
  - **Token handoff.** Web: token in the URL fragment, moved to `sessionStorage` and removed from the address bar (`ui/src/api/token.ts`). Desktop: the shell sets `window.__TRACE_TOKEN__` before load. SSE is read with `fetch` (`ui/src/api/events.ts`, reconnects) because `EventSource` cannot send the header.
  - **API.** Contract: `docs/api/openapi.yaml`. Environments (CRUD, default, test connection), partners (CRUD under an environment), `GET /partners/{id}/provisioning`, `POST /partners/{id}/provision`, installed documents and `POST .../documents/install`, `/packs`, `/preview`, `/submit`, `/environments/{id}/submissions` (no payload, `has_payload`), `/submissions/{id}` (with payload), `/environments/{id}/counters`, `/preferences`, `/runs/{id}` (+ `/cancel`), `/events`, `/health` (reports `secret_backend`). Secrets are write-only (`api_key`, `password` in requests; `has_api_key`, `has_password` in responses). Environment update and delete call `B2bApi::forget_token`. Errors: 422 `{"error","fields"}`, 404, 409, generic 500/502.
  - **Runs.** Provision, install and submit answer `202 {run_id}`; the report is read from `GET /api/runs/{id}` (`running`/`finished`+`report`/`failed`+`error`). A failed step or post is a *finished* run whose report says so. Scopes: one active run per partner (provision), per environment (install), per environment (submit); a second start is 409. The registry keeps the last 50 finished runs in memory (lost on restart). Environment deletion while a run is active is not blocked.
  - **Core changes.** `SubmissionRepository::get(&SubmissionId)` added (SQLite and `MemoryDb`); the list query was refactored into `fetch_submissions`.
  - **Contract checks.** `cargo test -p trace-server`: 21 integration tests plus 3 unit tests, including secrets-absent-from-responses, guard behaviour, SSE delivery and shutdown, end-to-end provision/preview/submit with `FakeHttp`, and two spec tests (routes vs `openapi.yaml`, every documented operation routed). The route scan reads the router sources, so a new router file must be added to `registered_operations()`. The TypeScript client types are generated by `pnpm gen:api` (`ui/src/api/schema.d.ts`, committed); CI runs `pnpm check:api`. `ui/tools/gen` is a second pnpm workspace package with TypeScript 6.0.3 because `openapi-typescript` needs the JS compiler API that TypeScript 7 lacks. The runtime client is `openapi-fetch` in `ui/src/api/client.ts` (`api`, `unwrap`, `ApiFailure`).
  - **Dev proxy.** `ui/vite.config.ts` proxies `/api` to `127.0.0.1:${TRACE_API_PORT:-8787}` with `changeOrigin: true`; start the server with `--allow-origin http://127.0.0.1:5173 --token ...`. Smoke-tested by hand against the built binary: 401/403/404 paths, SPA fallback, CORS preflight, proxied request, macOS keychain backend, data dir mode 0700 / files 0600. The SSE stream through the proxy was not observed by hand (the curl timeout helper was missing), only in the Rust test.
  - **For sub-task 7.** No UI code uses the client yet. `App.tsx` is unchanged and still uses mock data. Needed in the UI: call `initToken()` (done in `main.tsx`), handle 401 (token missing: show a "open the link printed by the server" screen), subscribe to events once and refresh lists on `partner.provision.*`, `documents.*`, `submission.*` events, and poll `GET /runs/{id}` after a `202`. The B2B host field belongs to `EnvironmentInput.b2b_host`.
  - **Caveats.** Run reports are secret-free by construction but a failed step's `error` text comes from core (already redacted there). `Cargo.lock`, `ui/pnpm-lock.yaml` and all new files are uncommitted; CI has never run on GitHub; `cargo audit` is not installed locally (the CI job runs it); the 14 pnpm "ignored build scripts" warnings are for Carbon font packages and were left unapproved on purpose.

### 7. UI: port the design on a feature registry
- **Intent:** Turn the Carbon prototype into the real interface with a structure new screens can plug into.
- **Expected Outcomes:** All design screens work against the API with the changes in the table; navigation is generated from a feature registry.
- **Todo List:**
  1. Split `App.tsx` into the shell (Header, SideNav, registry) and feature folders: environments, setup (partners), documents, studio, overview.
  2. API client wrapper with token handling (URL fragment in web mode, injected by Tauri on desktop).
  3. Environment modal: B2B host, README-based help, Test connection, write-only API key, advanced IAM host and CA bundle.
  4. Setup: partner directory, add or edit, sample pair, provisioning status per step with retry.
  5. Documents: pack-driven catalog, install per domain, installed badges.
  6. Studio: partner pickers (saved or manual ID), document multi-select, editable trace values, tabbed real EDI preview with next control numbers, submit with live per-document results, copy and download.
  7. Overview with real metrics.
  8. Empty and error states, accessibility, theme as in the design.
- **Relevant Context:** `wMB2BToolKit_Design/src/App.tsx` (settings modal L448-541, `InstanceSetup` L558, `Documents` L741, `Generator` L844, `Overview` L1079), `src/index.scss`.
- **Status:** `[x] done`
- **Completion notes (sub-task 7):**
  - **Structure.** `ui/src/shell/` (AppContext, Shell, registry, Unauthorized), `components/` (shared states, StatusTag, RunProgress, EnvironmentSelect, ActionBar), `lib/` (`useLoad`, `useRun`, `format`), `features/{environments,setup,documents,studio,overview}/`. Navigation is generated from `shell/registry.ts`; a feature is a `Feature` object (`shell/feature.ts`).
  - **Bug fixed in Rust.** `IdType` serialised as `DUNS` while the contract and UI use `duns`; every partner create returned 422 "invalid request body". `crates/core/src/partners.rs` now uses `rename_all = "lowercase"`, with a regression test. The DB column still stores `DUNS` (`as_str`/`parse` unchanged).
  - **Carbon gotchas.** `InlineNotification` throws if it holds a `<Button>`; use `ActionableNotification inline` with `actionButtonLabel`/`onActionButtonClick`, and pass `hasFocus={false}` or it steals focus from open modal inputs.
  - **Verified in a real browser against the real server** (Chromium via Playwright): token gate (missing and wrong token), environment create and Test connection (readable DNS error), partner pair create, provision failure with per-step status and Retry, Documents catalog, Studio preview with saved partners (correct ISA sender/receiver, SE/GE/IEA), submit blocked with a clear message for unprovisioned partners, Overview metrics, 390px viewport with no page overflow, no console errors.
  - **Not verified.** A successful provision, install and submit (needs a real tenant); cancel of a running run; keyboard-only and screen-reader passes; dark theme visual check beyond the design's toggle; Firefox and Safari.
  - **For sub-task 8.** Add `features/activity/` and register it in `shell/registry.ts` only. Data: `GET /environments/{id}/submissions?limit=` (no payload) and `GET /submissions/{id}` (payload). Reuse `useLoad`, `EnvironmentSelect`, `States`, `format.downloadText`.

### 8. Activity and reports
- **Intent:** Deliver the "better report" goal from stored submissions, as the first feature added through the extension pattern.
- **Expected Outcomes:** Activity screen lists submissions with filters (environment, domain, status, date), shows ISA/GS numbers and linked trace values, and exports CSV and JSON.
- **Todo List:**
  1. Activity module (core, router, UI) following the feature-module pattern.
  2. Filters, detail view, export.
  3. Activity screen in Carbon style (needs design sign-off).
- **Relevant Context:** `README.md` linking-attributes section for trace value meaning.
- **Status:** `[x] done`
- **Completion notes (sub-task 8):**
  - **Scope change.** No Rust or API change was needed: `GET /environments/{id}/submissions` and `GET /submissions/{id}` already carried everything. It is a UI-only feature: `ui/src/features/activity/` (`index.tsx`, `SubmissionDetail.tsx`, `exportRows.ts`, `resultTag.ts`) plus one line in `shell/registry.ts`. That is the extension pattern working as intended.
  - **Behaviour.** Filters (domain, result, from/to date) run client-side over the newest 1000 rows; the screen says so when the cap is reached. Table shows 50 rows with "Show more". Detail modal fetches the payload on demand and shows trace values, response, copy and download. Export CSV and JSON export the filtered rows (no payloads). CSV cells starting with `= + - @` are prefixed with `'` to stop spreadsheet formula injection from remote response text.
  - **Filter by environment** is the existing environment picker. Date filters use the browser's local day boundaries.
  - **Carbon gotcha.** Mount `Modal` only while it is needed (`{id && <Modal open />}`) rather than toggling `open` from an effect; an effect-driven `open` left a ghost modal that blocked clicks. Carbon's `Table` needs `position: relative` on `.cds--data-table-content` or its absolutely placed cell content widens the page on narrow screens.
  - **Verified in a real browser** with 64 seeded rows (inserted directly into SQLite; no tenant): filters, "Show more", detail modal with open/close by Escape and button, CSV (header, quoting, formula guard) and JSON content, 390px and 820px with no page overflow, no console errors. Timestamps are stored in microseconds.
  - **Not verified.** Live updates from `submission.*` events during a real run; screen-reader pass; the plan's "needs design sign-off" item still applies, since this screen has no counterpart in `wMB2BToolKit_Design/`.
  - **Known limit.** Older rows than the newest 1000 are not reachable from the UI; add server-side filtering and paging if that matters.
  - **For sub-task 9.** Nothing in the UI changes. The desktop shell must set `window.__TRACE_TOKEN__` before the page loads (see `ui/src/api/token.ts`).

### 9. Tauri desktop shell
- **Intent:** Ship the same app as a desktop application.
- **Expected Outcomes:** Tauri app starts the axum server in-process on `127.0.0.1:<random>` with a per-launch token, loads the UI, stops the server on exit. Keychain access works.
- **Todo List:**
  1. Create `apps/desktop/src-tauri` depending on `crates/server`.
  2. Random port and token passed to the webview.
  3. Restrict the webview to the local origin; minimal Tauri capabilities.
  4. App data directory for the database.
  5. Smoke test on macOS, then other platforms.
- **Relevant Context:** Tauri v2 docs; sub-task 6.
- **Status:** `[x] done`
- **Completion notes (sub-task 9):**
  - **Location and workspace.** `apps/desktop/src-tauri/` (`trace-desktop`, lib `trace_desktop_lib`). It is its **own Cargo workspace** with its own `Cargo.lock`, so the main workspace, its CI jobs and `cargo test --workspace` do not need WebKitGTK on Linux. Consequence: root validation commands do not cover it; the desktop crate has its own commands (see below) and its own CI job and audit step.
  - **Tauri version.** `tauri` 2.11.6 and `tauri-build` 2.6.3 (latest stable). The crates.io "latest" `tauri` is the 3.0 alpha; it is excluded on purpose (stable, maintained dependencies only). `default-features = false` with `wry, compression, common-controls-v6, x11`.
  - **Behaviour.** On launch the app starts the same axum server in-process (`serve` with `UiSource::Embedded`, `ui/dist` embedded through the `embed-ui` feature) on `127.0.0.1` with a random port and a per-launch token, then opens the `main` window. The token reaches the page through an `initialization_script` setting `window.__TRACE_TOKEN__` (JSON-encoded). The database lives in the OS app-data directory (`com.ibm.trace.toolkit`). The server is shut down on `RunEvent::Exit`. Errors are logged; the message on stderr is generic.
  - **Hardening.** No `capabilities/` directory, so the page has **no Tauri IPC**. `on_navigation` allows only the local origin, using a parsed-URL check on scheme, host, port and no userinfo (not a string prefix). CSP set in `tauri.conf.json`. Secrets still go only to the OS keychain; nothing secret is written to files or logs.
  - **CI.** New `desktop` job on `macos-latest` (build UI, fmt, clippy `-D warnings`, test, all `--locked`). The `audit` job also runs `cargo audit --file apps/desktop/src-tauri/Cargo.lock`, because `rustsec/audit-check` does not document a `working-directory` input.
  - **Validation commands for the desktop crate** (run in `apps/desktop/src-tauri`, after `pnpm build` in `ui/`): `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` (2 tests). All pass.
  - **Verified on macOS (real launch).** Server on a random port; DB created under `~/Library/Application Support/com.ibm.trace.toolkit/`; API without token returned 401, wrong `Host` returned 403, `/` returned 200; a WKWebView window titled "Trace" opened and, via the accessibility tree, rendered the real UI (nav, Overview, "Connect your B2B environment"), so token injection works; SIGTERM stopped the process and closed the port. The test data directory was removed afterwards.
  - **Not verified.** Graceful shutdown by window close or Cmd+Q (SIGTERM may bypass the `RunEvent::Exit` path); keychain access from the desktop build against a real tenant; Linux (needs WebKitGTK 4.1 and related packages) and Windows builds; the new CI job and audit step have never run on GitHub; bundling with `tauri build`.
  - **For sub-task 10.** `tauri.conf.json` already has `beforeBuildCommand` (`pnpm --dir ../../../ui build`) and `bundle.targets: all` with icons. Decide how to install the Tauri CLI: pnpm dev-dependency `@tauri-apps/cli` 2.12.4 or `cargo install tauri-cli --locked`. Standalone `trace-server` needs `--features embed-ui` and a built `ui/dist`. Linux runners need the WebKitGTK packages installed before building.

### 10. Packaging, release, docs
- **Intent:** Produce installable artifacts and document v2.
- **Expected Outcomes:** Desktop bundles per OS and a standalone `trace-server`; README documents v2 usage, environment setup and adding packs or features.
- **Todo List:**
  1. Release pipeline for desktop bundles and `trace-server`.
  2. macOS signing and notarization (credentials outside the repo).
  3. Document the Linux WebKitGTK prerequisite.
  4. Write the v2 README and the extension guide; mark v1 deprecated.
  5. Final validation: fmt, clippy, tests, UI build, audit, golden tests against script dry-run.
- **Relevant Context:** `README.md`.
- **Status:** `[x] done`
- **Completion notes (sub-task 10):**
  - **Tauri CLI.** Installed as a pinned pnpm dev-dependency of `ui/` (`@tauri-apps/cli` 2.12.4, `pnpm add -Dw`), so it is covered by `pnpm-lock.yaml` and `pnpm audit`. Run it as `ui/node_modules/.bin/tauri` from `apps/desktop`.
  - **Bug fixed.** `beforeBuildCommand` in `tauri.conf.json` ran from `apps/desktop`, not `src-tauri`; the path is now `pnpm --dir ../../ui build`.
  - **Desktop bundle (macOS).** `tauri build --bundles app` produced an unsigned `Trace.app` (22 MiB). Launched from the bundle, then quit with the app's normal Quit: the process exited and the port closed, so the graceful-shutdown caveat from sub-task 9 is resolved for that path.
  - **Standalone server.** `cargo build --release --locked -p trace-server --features embed-ui` built; run with a temporary data dir: `/` returned 200, the API without a token returned 401, and it stopped on signal.
  - **Release workflow.** `.github/workflows/release.yml`: on a `v*` tag (or manual run) builds desktop bundles (macOS app+dmg, Linux deb+AppImage on ubuntu-22.04, Windows NSIS) and `trace-server` per OS, uploads artifacts, and on tags creates a **draft** release with `SHA256SUMS.txt`. Workflow permissions are `contents: read`; only the publish job has `contents: write`. YAML parses; it has never run on GitHub.
  - **Signing.** Not done. Builds are unsigned and unnotarised, and the README says so. Signing needs an Apple Developer ID certificate, notarisation credentials and a Windows code-signing certificate, supplied as repository secrets; none exist, so nothing was added that cannot be tested.
  - **README.** Rewritten with a v2 section first (install, use, run server and desktop, packs, adding a feature, checks) and the v1 text kept under "v1 — bash script (deprecated)". Claims were checked against the UI and the pack loader.
  - **Linux prerequisite** (WebKitGTK 4.1 and build packages) is documented in the README and installed in the release workflow. `libdbus-1-dev` is not needed: the Linux keychain store is the pure-Rust `zbus`.
  - **Golden tests against the script dry-run.** Not repeated here; the golden/render tests from sub-task 3 run in `cargo test --workspace`.
  - **Not verified.** Any GitHub Actions run; Linux and Windows builds; dmg/deb/AppImage/NSIS bundling (only the macOS `.app` was built); keychain use in a bundled app against a real tenant.


## Cleanup of v1 and the design reference
- **Decision (after sub-task 10):** v1 and the design reference are not needed in `main`.
- **v1 preserved** on local branch `v1-bash` (the script, `toolkit.env.example` and `templates/`, plus a commit holding the earlier local edits to `templates/retail/810.edi` and `820.edi`). The branch has **not been pushed**; push it before relying on it (`git push origin v1-bash`).
- **Removed from the working tree:** `b2b-toolkit.sh`, `toolkit.env.example`, `templates/` (staged deletions, uncommitted) and `wMB2BToolKit_Design/` (it was untracked, so it is not in any branch). A tarball of the design folder is at `~/wMB2BToolKit_Design.tgz`.
- **Kept:** the user's own `toolkit.env` and `.b2b-toolkit/` (credentials and v1 state, git-ignored) and their `.gitignore` entries.
- **Code changes:** the one test that read the script with `include_str!` now asserts the document-type key shape directly (`WMX12-4010--<txn>---------`). Comments citing script line numbers now point at the `v1-bash` branch. The README no longer documents v1.
- The earlier statements in this plan that v1 "stays untouched" describe the state while v2 was being built.

## Open Items / Risks
- **Storage engine:** SQLite is recommended; JSON files are simpler but weaker for counters, activity queries and migrations. Needs confirmation.
- **Partner ID types:** the design offers DUNS, GLN, Tax ID, Mutually defined; the script and templates support DUNS only (`externalIdType: DUNS`, ISA qualifier `01`). Other types need the B2B API's accepted values, which are not in this repo. Default: DUNS only until verified.
- **TLS:** if your environments rely on the script's `--insecure`, use the optional CA bundle or fix the certificates.
- **Keychain availability:** `trace-server` on machines without a keychain service needs a fallback, for example an encrypted file with a passphrase.
- **Partner already exists on the tenant:** the script handles "already exists" only for document types; re-creating a partner after local state is lost will fail. A "link existing partner" action is out of scope unless wanted.
- **Catalog names:** names for codes such as 860, 865, 275, 277, 990 must be checked against each template.

## Plan Confirmed
The user confirmed this plan and chose to revisit changes later. Open items below proceed with their stated defaults: SQLite storage, DUNS-only partner ID type, Activity screen included, healthcare labelled X12 004010, keychain with encrypted-file fallback for `trace-server`, optional per-environment CA bundle (no `--insecure`), README-only v1 deprecation notice.
