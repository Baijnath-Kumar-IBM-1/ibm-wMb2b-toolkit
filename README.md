> **Looking for the bash script?** The original `b2b-toolkit.sh` (v1) is no
> longer in `main`. It is kept on the `v1-bash` branch
> (`git checkout v1-bash`). v2 does not read `toolkit.env` or the
> `.b2b-toolkit/` workspace; re-enter environments in the UI.

# IBM webMethods B2B Toolkit v2

Initialise an IBM webMethods B2B SaaS environment, generate sample EDI
documents and submit linked multi-document traces for demos. It runs as a
**web app** (a local server you open in a browser) or as a **desktop app**
(Tauri). Both use the same Rust core and the same UI (IBM Carbon).

> Everything runs on your machine. The server listens on `127.0.0.1` only and
> every request needs a per-launch token. API keys and partner passwords are
> kept in the OS keychain, never in files or logs.

| | |
|---|---|
| `crates/core` | Domain logic: environments, partners, packs, setup and submit runs (no I/O) |
| `crates/storage` | SQLite storage and the secret store (keychain, or encrypted file) |
| `crates/http` | HTTP client for the B2B and MCSP APIs |
| `crates/server` | `trace-server`: local HTTP API, events and UI hosting |
| `ui/` | React + Carbon UI |
| `apps/desktop/src-tauri` | Tauri desktop shell (its own Cargo workspace) |
| `packs/` | Built-in domain packs: retail, healthcare (X12 004010), supply chain |

## Install

Download a build from the GitHub releases page, or build it yourself below.
Release builds are **not code-signed or notarised**:

- **macOS:** open the `.dmg`, drag Trace to Applications. On first launch,
  right-click the app and choose Open, or allow it in System Settings →
  Privacy & Security.
- **Windows:** run the installer; SmartScreen may warn about an unknown publisher.
- **Linux:** needs WebKitGTK 4.1 (`libwebkit2gtk-4.1-0` on Debian and Ubuntu).
  Install the `.deb`, or make the `.AppImage` executable and run it.
- **Standalone server:** `trace-server-<os>` from the same release; see below.

Check each download against `SHA256SUMS.txt` on the release.

## Use it

1. **Environments** — add the B2B host, MCSP instance ID and API key, then
   *Test connection*. For a tenant with a private CA, attach a CA bundle to the
   environment; TLS validation is never turned off.
2. **Setup** — add a pair of trading partners (DUNS IDs) for a domain, then
   *Provision* each partner. Provisioning creates the partner, channel and user
   on the tenant step by step, and can be retried or resumed.
3. **Documents** — pick a domain and install its document types on the tenant.
4. **Message studio** — choose the saved partner pair, preview the generated
   EDI (nothing is sent), then submit one document or the full trace sequence.
5. **Activity** — review stored submissions, filter, open the payload and
   response, and export CSV or JSON (newest 1000 rows).
6. **Overview** — setup progress and counts for the selected environment.

### Run the server in a browser

```sh
cd ui && pnpm install --frozen-lockfile && pnpm build && cd ..
cargo run --release -p trace-server --features embed-ui
```

It prints `http://127.0.0.1:<port>/#token=...`; open that URL. Without
`--features embed-ui`, pass `--ui-dir ui/dist`. If the machine has no OS
keychain, set `TRACE_SECRETS_PASSPHRASE` to use the encrypted-file store.
Options and the security model are in [`docs/server.md`](docs/server.md).

### Run the desktop app

```sh
cd ui && pnpm install --frozen-lockfile && cd ..
cd apps/desktop && ../../ui/node_modules/.bin/tauri dev      # development
cd apps/desktop && ../../ui/node_modules/.bin/tauri build    # installers
```

`tauri build` runs the UI build itself. Bundles are written to
`apps/desktop/src-tauri/target/release/bundle/`. On Linux, install first:
`build-essential libwebkit2gtk-4.1-dev libxdo-dev libssl-dev librsvg2-dev pkg-config`.

Data (database, user packs) lives in the per-user app-data directory
(`~/Library/Application Support/com.ibm.trace.toolkit` on macOS).

## Add a domain or document set (packs)

A pack is a folder with a `pack.toml` and EDI templates, like those in
[`packs/`](packs). Copy one to `<data dir>/packs/<name>/`, edit it and restart.
A pack with the same `id` as a built-in one replaces it; a broken pack is
skipped and a warning is logged (for example, a document whose template file
is missing). An unknown `{{PLACEHOLDER}}` is reported when the document is
rendered.

## Add a feature

Features follow one layout: a core module, a storage implementation, an HTTP
router and a screen registered in `ui/src/shell/registry.ts`. See
[`docs/adding-a-feature.md`](docs/adding-a-feature.md) and
[`docs/server.md`](docs/server.md). The API contract is
`docs/api/openapi.yaml`; the UI client is generated from it
(`pnpm check:api` fails if they drift).

## Develop and check

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cd ui && pnpm install --frozen-lockfile && pnpm check:api && pnpm typecheck && pnpm build && pnpm audit --audit-level=moderate
# desktop crate (needs ui/dist built first)
cd apps/desktop/src-tauri && cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked
```
