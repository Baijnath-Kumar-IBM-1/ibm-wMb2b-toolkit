# Local server and API

`trace-server` (crate `crates/server`) is the one backend for the web app and
the desktop app. Features live in `trace-core`; the server only wires them to
HTTP.

## Run it

```sh
cd ui && pnpm install --frozen-lockfile && pnpm build && cd ..
cargo run -p trace-server -- --ui-dir ui/dist
```

It prints a URL such as `http://127.0.0.1:53211/#token=...`. Open it in a
browser. The token is in the URL *fragment*, which browsers never send to a
server; the UI moves it to `sessionStorage` and removes it from the address
bar. Options: `trace-server --help`.

Secrets are stored in the OS keychain. When none is available, set
`TRACE_SECRETS_PASSPHRASE` to use the encrypted file in the data directory
(the passphrase is never stored). `GET /api/health` reports the active
backend (`keychain` or `encrypted_file`).

### UI development

```sh
cargo run -p trace-server -- --port 8787 \
  --allow-origin http://127.0.0.1:5173 --token <32+ characters>
cd ui && pnpm dev          # proxies /api to 127.0.0.1:8787 (TRACE_API_PORT)
# open http://127.0.0.1:5173/#token=<the same token>
```

## Security model

- Binds `127.0.0.1` only. `Host` must be `127.0.0.1:<port>` or
  `localhost:<port>` (blocks DNS rebinding).
- Every `/api` route needs `Authorization: Bearer <token>` (constant-time
  comparison). The token is random per launch, never accepted in a URL and
  never logged.
- A request with an `Origin` header must come from the server's own origin or
  one given with `--allow-origin`. CORS is only enabled for those extra
  origins.
- Secrets are write-only: the API key and partner passwords go in, and only
  `has_api_key` / `has_password` come out. They are not in events, run
  reports or errors.
- Errors are `{"error": "...", "fields": [...]}`. Storage, keychain and
  upstream details are logged on the server and replaced by a generic text.
- Responses carry a strict CSP, `nosniff`, `X-Frame-Options: DENY` and
  `Cache-Control: no-store` for the API.

## API contract

`docs/api/openapi.yaml` is the contract.

- `cargo test -p trace-server` fails when a route is added or removed without
  updating the file (`spec_and_routers_list_the_same_operations`) and checks
  that every documented operation is routed.
- `pnpm gen:api` (in `ui/`) regenerates `ui/src/api/schema.d.ts`; CI runs
  `pnpm check:api` and fails if the committed file is stale.
- The generator lives in `ui/tools/gen` with its own TypeScript 6, because
  `openapi-typescript` needs the compiler's JS API, which TypeScript 7 (used
  by the app) no longer has.

## Long operations and events

Provisioning, document installation and submissions answer `202` with a
`run_id`. Read the outcome with `GET /api/runs/{id}` (`running`, `finished`
with a `report`, or `failed` with an `error`) and cancel with
`POST /api/runs/{id}/cancel`. A step or document that fails is a *finished*
run whose report says what failed; `failed` means it could not start work.
One run per partner (provisioning), per environment (install) and per
environment (submit) at a time; a second start answers `409`.

`GET /api/events` is a Server-Sent-Events stream (`event`, and `lagged` when a
slow reader missed events). Read it with `fetch` so the bearer header can be
sent; `ui/src/api/events.ts` does this and reconnects.

## Adding an endpoint

1. Add the service to `trace-core` (see `docs/adding-a-feature.md`).
2. Wire it in `crates/server/src/state.rs`.
3. Add `crates/server/src/routers/<feature>.rs` and one `.merge(...)` line in
   `routers/mod.rs`.
4. Describe it in `docs/api/openapi.yaml`, add the router file to
   `registered_operations()` in `crates/server/tests/api.rs`, then run
   `pnpm gen:api`.
