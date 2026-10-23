-- Initial schema (version 1).
--
-- Holds non-secret data only. Secrets (API keys, partner passwords) live in
-- the secret store; columns named *_secret hold opaque secret ids, never
-- values. Timestamps are INTEGER microseconds since the Unix epoch (UTC) so
-- that ordering is numeric.

CREATE TABLE environments (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    b2b_host       TEXT NOT NULL,
    instance_id    TEXT NOT NULL,
    iam_host       TEXT NOT NULL,
    ca_bundle_path TEXT,
    is_default     INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    api_key_secret TEXT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX environments_name_uq ON environments (name COLLATE NOCASE);
-- At most one default environment.
CREATE UNIQUE INDEX environments_default_uq ON environments (is_default) WHERE is_default = 1;

CREATE TABLE partners (
    id              TEXT PRIMARY KEY,
    environment_id  TEXT NOT NULL REFERENCES environments (id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    org_unit        TEXT NOT NULL,
    id_type         TEXT NOT NULL CHECK (id_type IN ('DUNS')),
    id_value        TEXT NOT NULL,
    username        TEXT NOT NULL,
    full_name       TEXT NOT NULL,
    channel_name    TEXT NOT NULL,
    password_secret TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX partners_name_uq     ON partners (environment_id, name COLLATE NOCASE);
CREATE UNIQUE INDEX partners_id_value_uq ON partners (environment_id, id_value);
CREATE UNIQUE INDEX partners_username_uq ON partners (environment_id, username COLLATE NOCASE);
CREATE UNIQUE INDEX partners_channel_uq  ON partners (environment_id, channel_name COLLATE NOCASE);

-- Resumable step runner records. `scope` is chosen by the feature (for
-- example 'partner:<uuid>'). Rows are listed in insertion (rowid) order.
CREATE TABLE step_records (
    scope      TEXT NOT NULL,
    name       TEXT NOT NULL,
    status     TEXT NOT NULL CHECK (status IN ('succeeded', 'failed')),
    output     TEXT NOT NULL,
    error      TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (scope, name)
) STRICT;

CREATE TABLE counters (
    environment_id TEXT PRIMARY KEY REFERENCES environments (id) ON DELETE CASCADE,
    isa_next       INTEGER NOT NULL DEFAULT 1 CHECK (isa_next BETWEEN 1 AND 999999999),
    gs_next        INTEGER NOT NULL DEFAULT 1 CHECK (gs_next BETWEEN 1 AND 999999999),
    run_count      INTEGER NOT NULL DEFAULT 0 CHECK (run_count >= 0)
) STRICT;

CREATE TABLE installed_documents (
    environment_id   TEXT NOT NULL REFERENCES environments (id) ON DELETE CASCADE,
    transaction_code TEXT NOT NULL,
    document_type_id TEXT NOT NULL,
    installed_at     INTEGER NOT NULL,
    PRIMARY KEY (environment_id, transaction_code)
) STRICT;

CREATE TABLE submissions (
    id                  TEXT PRIMARY KEY,
    environment_id      TEXT NOT NULL REFERENCES environments (id) ON DELETE CASCADE,
    submitted_at        INTEGER NOT NULL,
    domain              TEXT NOT NULL,
    transaction_code    TEXT NOT NULL,
    sender_partner_id   TEXT REFERENCES partners (id) ON DELETE SET NULL,
    receiver_partner_id TEXT REFERENCES partners (id) ON DELETE SET NULL,
    isa_control         INTEGER NOT NULL,
    gs_control          INTEGER NOT NULL,
    http_status         INTEGER,
    result              TEXT NOT NULL,
    trace_values        TEXT NOT NULL
) STRICT;

CREATE INDEX submissions_env_time_ix ON submissions (environment_id, submitted_at DESC);

-- App preferences as one JSON document per key (currently only 'app').
CREATE TABLE preferences (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
