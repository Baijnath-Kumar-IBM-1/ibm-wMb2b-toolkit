//! SQLite implementations of the `trace-core` repository ports.
//!
//! All statements use bound parameters. Multi-statement operations run in a
//! transaction. Rows are converted back into core types through their
//! validating constructors, so a corrupted row surfaces as `Error::Storage`
//! instead of an invalid value.

use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde_json::Value;
use trace_core::counters::{self, ControlNumbers, CounterRepository, Counters, MAX_ALLOCATION};
use trace_core::environments::{Environment, EnvironmentRepository};
use trace_core::ids::{EnvironmentId, PartnerId, SubmissionId};
use trace_core::installed::{InstalledDocument, InstalledDocumentRepository};
use trace_core::partners::{IdType, Partner, PartnerRepository, partner_scope};
use trace_core::preferences::{PreferenceRepository, Preferences};
use trace_core::secrets::SecretId;
use trace_core::steps::{StepRecord, StepStatus, StepStore};
use trace_core::submissions::{Submission, SubmissionRepository};
use trace_core::{Error, Result};

use crate::db::{Database, SqlExt, from_micros, storage, to_micros};

const PREFS_KEY: &str = "app";

fn parse_secret(value: Option<String>) -> Result<Option<SecretId>> {
    value.as_deref().map(SecretId::parse).transpose()
}

fn parse_env_id(s: &str) -> Result<EnvironmentId> {
    s.parse()
        .map_err(|_| storage("invalid environment id in database"))
}

fn parse_partner_id(s: &str) -> Result<PartnerId> {
    s.parse()
        .map_err(|_| storage("invalid partner id in database"))
}

// ---- environments -------------------------------------------------------

const ENV_COLS: &str = "id, name, b2b_host, instance_id, iam_host, ca_bundle_path, is_default, \
                        api_key_secret, created_at, updated_at";

struct EnvRow {
    id: String,
    name: String,
    b2b_host: String,
    instance_id: String,
    iam_host: String,
    ca_bundle_path: Option<String>,
    is_default: bool,
    api_key_secret: Option<String>,
    created_at: i64,
    updated_at: i64,
}

fn env_row(r: &Row<'_>) -> rusqlite::Result<EnvRow> {
    Ok(EnvRow {
        id: r.get(0)?,
        name: r.get(1)?,
        b2b_host: r.get(2)?,
        instance_id: r.get(3)?,
        iam_host: r.get(4)?,
        ca_bundle_path: r.get(5)?,
        is_default: r.get(6)?,
        api_key_secret: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

impl EnvRow {
    fn into_env(self) -> Result<Environment> {
        Ok(Environment {
            id: parse_env_id(&self.id)?,
            name: self.name,
            b2b_host: self.b2b_host,
            instance_id: self.instance_id,
            iam_host: self.iam_host,
            ca_bundle_path: self.ca_bundle_path,
            is_default: self.is_default,
            api_key_secret: parse_secret(self.api_key_secret)?,
            created_at: from_micros(self.created_at)?,
            updated_at: from_micros(self.updated_at)?,
        })
    }
}

impl EnvironmentRepository for Database {
    fn list(&self) -> Result<Vec<Environment>> {
        self.run(|c| {
            let mut stmt = c
                .prepare(&format!(
                    "SELECT {ENV_COLS} FROM environments ORDER BY name COLLATE NOCASE"
                ))
                .db()?;
            let rows: Vec<EnvRow> = stmt
                .query_map([], env_row)
                .db()?
                .collect::<rusqlite::Result<_>>()
                .db()?;
            rows.into_iter().map(EnvRow::into_env).collect()
        })
    }

    fn get(&self, id: &EnvironmentId) -> Result<Option<Environment>> {
        self.run(|c| {
            c.query_row(
                &format!("SELECT {ENV_COLS} FROM environments WHERE id = ?1"),
                [id.as_str()],
                env_row,
            )
            .optional()
            .db()?
            .map(EnvRow::into_env)
            .transpose()
        })
    }

    fn save(&self, env: &Environment) -> Result<()> {
        self.run(|c| {
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .db()?;
            if env.is_default {
                tx.execute(
                    "UPDATE environments SET is_default = 0 WHERE is_default = 1 AND id <> ?1",
                    [env.id.as_str()],
                )
                .db()?;
            }
            // `is_default` is only written on insert; later changes go through
            // `set_default` so an edit cannot accidentally demote the default.
            tx.execute(
                "INSERT INTO environments (id, name, b2b_host, instance_id, iam_host, \
                     ca_bundle_path, is_default, api_key_secret, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
                 ON CONFLICT (id) DO UPDATE SET \
                     name = excluded.name, b2b_host = excluded.b2b_host, \
                     instance_id = excluded.instance_id, iam_host = excluded.iam_host, \
                     ca_bundle_path = excluded.ca_bundle_path, \
                     api_key_secret = excluded.api_key_secret, \
                     updated_at = excluded.updated_at",
                params![
                    env.id.as_str(),
                    env.name,
                    env.b2b_host,
                    env.instance_id,
                    env.iam_host,
                    env.ca_bundle_path,
                    env.is_default,
                    env.api_key_secret.as_ref().map(SecretId::as_str),
                    to_micros(env.created_at)?,
                    to_micros(env.updated_at)?,
                ],
            )
            .db_unique("an environment with this name already exists")?;
            tx.commit().db()
        })
    }

    fn delete(&self, id: &EnvironmentId) -> Result<()> {
        self.run(|c| {
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .db()?;
            // Step records are keyed by scope text, not by foreign key.
            let ids: Vec<String> = tx
                .prepare("SELECT id FROM partners WHERE environment_id = ?1")
                .db()?
                .query_map([id.as_str()], |r| r.get(0))
                .db()?
                .collect::<rusqlite::Result<_>>()
                .db()?;
            for p in ids {
                let scope = partner_scope(&parse_partner_id(&p)?);
                tx.execute("DELETE FROM step_records WHERE scope = ?1", [scope])
                    .db()?;
            }
            tx.execute("DELETE FROM environments WHERE id = ?1", [id.as_str()])
                .db()?;
            tx.commit().db()
        })
    }

    fn set_default(&self, id: &EnvironmentId) -> Result<()> {
        self.run(|c| {
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .db()?;
            tx.execute(
                "UPDATE environments SET is_default = 0 WHERE is_default = 1",
                [],
            )
            .db()?;
            let n = tx
                .execute(
                    "UPDATE environments SET is_default = 1 WHERE id = ?1",
                    [id.as_str()],
                )
                .db()?;
            if n == 0 {
                return Err(Error::NotFound("environment"));
            }
            tx.commit().db()
        })
    }
}

// ---- partners -----------------------------------------------------------

const PARTNER_COLS: &str = "id, environment_id, name, org_unit, id_type, id_value, username, \
                            full_name, channel_name, password_secret, created_at, updated_at";

struct PartnerRow {
    id: String,
    environment_id: String,
    name: String,
    org_unit: String,
    id_type: String,
    id_value: String,
    username: String,
    full_name: String,
    channel_name: String,
    password_secret: Option<String>,
    created_at: i64,
    updated_at: i64,
}

fn partner_row(r: &Row<'_>) -> rusqlite::Result<PartnerRow> {
    Ok(PartnerRow {
        id: r.get(0)?,
        environment_id: r.get(1)?,
        name: r.get(2)?,
        org_unit: r.get(3)?,
        id_type: r.get(4)?,
        id_value: r.get(5)?,
        username: r.get(6)?,
        full_name: r.get(7)?,
        channel_name: r.get(8)?,
        password_secret: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

impl PartnerRow {
    fn into_partner(self) -> Result<Partner> {
        Ok(Partner {
            id: parse_partner_id(&self.id)?,
            environment_id: parse_env_id(&self.environment_id)?,
            name: self.name,
            org_unit: self.org_unit,
            id_type: IdType::parse(&self.id_type)
                .map_err(|_| storage("invalid id type in database"))?,
            id_value: self.id_value,
            username: self.username,
            full_name: self.full_name,
            channel_name: self.channel_name,
            password_secret: parse_secret(self.password_secret)?,
            created_at: from_micros(self.created_at)?,
            updated_at: from_micros(self.updated_at)?,
        })
    }
}

impl PartnerRepository for Database {
    fn list(&self, environment_id: &EnvironmentId) -> Result<Vec<Partner>> {
        self.run(|c| {
            let mut stmt = c
                .prepare(&format!(
                    "SELECT {PARTNER_COLS} FROM partners WHERE environment_id = ?1 \
                     ORDER BY name COLLATE NOCASE"
                ))
                .db()?;
            let rows: Vec<PartnerRow> = stmt
                .query_map([environment_id.as_str()], partner_row)
                .db()?
                .collect::<rusqlite::Result<_>>()
                .db()?;
            rows.into_iter().map(PartnerRow::into_partner).collect()
        })
    }

    fn get(&self, id: &PartnerId) -> Result<Option<Partner>> {
        self.run(|c| {
            c.query_row(
                &format!("SELECT {PARTNER_COLS} FROM partners WHERE id = ?1"),
                [id.as_str()],
                partner_row,
            )
            .optional()
            .db()?
            .map(PartnerRow::into_partner)
            .transpose()
        })
    }

    fn save(&self, p: &Partner) -> Result<()> {
        self.run(|c| {
            c.execute(
                "INSERT INTO partners (id, environment_id, name, org_unit, id_type, id_value, \
                     username, full_name, channel_name, password_secret, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
                 ON CONFLICT (id) DO UPDATE SET \
                     name = excluded.name, org_unit = excluded.org_unit, \
                     id_type = excluded.id_type, id_value = excluded.id_value, \
                     username = excluded.username, full_name = excluded.full_name, \
                     channel_name = excluded.channel_name, \
                     password_secret = excluded.password_secret, \
                     updated_at = excluded.updated_at",
                params![
                    p.id.as_str(),
                    p.environment_id.as_str(),
                    p.name,
                    p.org_unit,
                    p.id_type.as_str(),
                    p.id_value,
                    p.username,
                    p.full_name,
                    p.channel_name,
                    p.password_secret.as_ref().map(SecretId::as_str),
                    to_micros(p.created_at)?,
                    to_micros(p.updated_at)?,
                ],
            )
            .map(drop)
            .map_err(|e| map_partner_save_error(&e))
        })
    }

    fn delete(&self, id: &PartnerId) -> Result<()> {
        self.run(|c| {
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .db()?;
            tx.execute(
                "DELETE FROM step_records WHERE scope = ?1",
                [partner_scope(id)],
            )
            .db()?;
            tx.execute("DELETE FROM partners WHERE id = ?1", [id.as_str()])
                .db()?;
            tx.commit().db()
        })
    }
}

fn map_partner_save_error(e: &rusqlite::Error) -> Error {
    use rusqlite::ffi::{SQLITE_CONSTRAINT_FOREIGNKEY, SQLITE_CONSTRAINT_UNIQUE};
    if let rusqlite::Error::SqliteFailure(f, _) = e {
        if f.extended_code == SQLITE_CONSTRAINT_UNIQUE {
            return Error::Conflict(
                "name, ID value, username and channel name must be unique within the environment"
                    .into(),
            );
        }
        if f.extended_code == SQLITE_CONSTRAINT_FOREIGNKEY {
            return Error::NotFound("environment");
        }
    }
    storage(e)
}

// ---- step records -------------------------------------------------------

impl StepStore for Database {
    fn list(&self, scope: &str) -> Result<Vec<StepRecord>> {
        self.run(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT name, status, output, error, updated_at FROM step_records \
                     WHERE scope = ?1 ORDER BY rowid",
                )
                .db()?;
            let rows: Vec<(String, String, String, Option<String>, i64)> = stmt
                .query_map([scope], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })
                .db()?
                .collect::<rusqlite::Result<_>>()
                .db()?;
            rows.into_iter()
                .map(|(name, status, output, error, at)| {
                    Ok(StepRecord {
                        name,
                        status: StepStatus::parse(&status)
                            .ok_or_else(|| storage("invalid step status in database"))?,
                        output: serde_json::from_str(&output).map_err(storage)?,
                        error,
                        updated_at: from_micros(at)?,
                    })
                })
                .collect()
        })
    }

    fn save(&self, scope: &str, record: &StepRecord) -> Result<()> {
        self.run(|c| {
            c.execute(
                "INSERT INTO step_records (scope, name, status, output, error, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT (scope, name) DO UPDATE SET \
                     status = excluded.status, output = excluded.output, \
                     error = excluded.error, updated_at = excluded.updated_at",
                params![
                    scope,
                    record.name,
                    record.status.as_str(),
                    serde_json::to_string(&record.output).map_err(storage)?,
                    record.error,
                    to_micros(record.updated_at)?,
                ],
            )
            .db()
            .map(drop)
        })
    }

    fn clear(&self, scope: &str) -> Result<()> {
        self.run(|c| {
            c.execute("DELETE FROM step_records WHERE scope = ?1", [scope])
                .db()
                .map(drop)
        })
    }
}

// ---- counters -----------------------------------------------------------

fn counters_in(tx: &Transaction<'_>, env: &EnvironmentId) -> Result<Counters> {
    let row = tx
        .query_row(
            "SELECT isa_next, gs_next, run_count FROM counters WHERE environment_id = ?1",
            [env.as_str()],
            |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .db()?;
    match row {
        None => Ok(Counters::default()),
        Some((isa_next, gs_next, run_count)) => Ok(Counters {
            isa_next,
            gs_next,
            run_count: u64::try_from(run_count)
                .map_err(|_| Error::Storage("negative run_count".to_owned()))?,
        }),
    }
}

fn store_counters(tx: &Transaction<'_>, env: &EnvironmentId, c: &Counters) -> Result<()> {
    let run_count =
        i64::try_from(c.run_count).map_err(|_| Error::Storage("run_count overflow".to_owned()))?;
    tx.execute(
        "INSERT INTO counters (environment_id, isa_next, gs_next, run_count) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT (environment_id) DO UPDATE SET \
             isa_next = excluded.isa_next, gs_next = excluded.gs_next, \
             run_count = excluded.run_count",
        params![env.as_str(), c.isa_next, c.gs_next, run_count],
    )
    .db()
    .map(drop)
}

fn require_environment(tx: &Transaction<'_>, env: &EnvironmentId) -> Result<()> {
    let found = tx
        .query_row(
            "SELECT 1 FROM environments WHERE id = ?1",
            [env.as_str()],
            |_| Ok(()),
        )
        .optional()
        .db()?;
    found.ok_or(Error::NotFound("environment"))
}

impl CounterRepository for Database {
    fn peek(&self, environment_id: &EnvironmentId) -> Result<Counters> {
        self.run(|c| {
            let tx = c.transaction().db()?;
            counters_in(&tx, environment_id)
        })
    }

    fn allocate(&self, environment_id: &EnvironmentId, count: u32) -> Result<Vec<ControlNumbers>> {
        if !(1..=MAX_ALLOCATION).contains(&count) {
            return Err(Error::validation("count", "must be between 1 and 1000"));
        }
        self.run(|c| {
            // IMMEDIATE takes the write lock up front, so two processes
            // sharing the file cannot read the same value.
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .db()?;
            require_environment(&tx, environment_id)?;
            let mut state = counters_in(&tx, environment_id)?;
            let out = (0..count)
                .map(|i| ControlNumbers {
                    isa: counters::advance(state.isa_next, i),
                    gs: counters::advance(state.gs_next, i),
                })
                .collect();
            state.isa_next = counters::advance(state.isa_next, count);
            state.gs_next = counters::advance(state.gs_next, count);
            store_counters(&tx, environment_id, &state)?;
            tx.commit().db()?;
            Ok(out)
        })
    }

    fn increment_run_count(&self, environment_id: &EnvironmentId) -> Result<u64> {
        self.run(|c| {
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .db()?;
            require_environment(&tx, environment_id)?;
            let mut state = counters_in(&tx, environment_id)?;
            state.run_count += 1;
            store_counters(&tx, environment_id, &state)?;
            tx.commit().db()?;
            Ok(state.run_count)
        })
    }
}

// ---- installed documents ------------------------------------------------

impl InstalledDocumentRepository for Database {
    fn list(&self, environment_id: &EnvironmentId) -> Result<Vec<InstalledDocument>> {
        self.run(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT transaction_code, document_type_id, installed_at \
                     FROM installed_documents WHERE environment_id = ?1 \
                     ORDER BY transaction_code",
                )
                .db()?;
            let rows: Vec<(String, String, i64)> = stmt
                .query_map([environment_id.as_str()], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .db()?
                .collect::<rusqlite::Result<_>>()
                .db()?;
            rows.into_iter()
                .map(|(transaction_code, document_type_id, at)| {
                    Ok(InstalledDocument {
                        transaction_code,
                        document_type_id,
                        installed_at: from_micros(at)?,
                    })
                })
                .collect()
        })
    }

    fn save(&self, environment_id: &EnvironmentId, doc: &InstalledDocument) -> Result<()> {
        self.run(|c| {
            c.execute(
                "INSERT INTO installed_documents \
                     (environment_id, transaction_code, document_type_id, installed_at) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (environment_id, transaction_code) DO UPDATE SET \
                     document_type_id = excluded.document_type_id, \
                     installed_at = excluded.installed_at",
                params![
                    environment_id.as_str(),
                    doc.transaction_code,
                    doc.document_type_id,
                    to_micros(doc.installed_at)?,
                ],
            )
            .map(drop)
            .map_err(|e| match &e {
                rusqlite::Error::SqliteFailure(f, _)
                    if f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY =>
                {
                    Error::NotFound("environment")
                }
                _ => storage(e),
            })
        })
    }

    fn remove(&self, environment_id: &EnvironmentId, transaction_code: &str) -> Result<()> {
        self.run(|c| {
            c.execute(
                "DELETE FROM installed_documents WHERE environment_id = ?1 AND transaction_code = ?2",
                params![environment_id.as_str(), transaction_code],
            )
            .db()
            .map(drop)
        })
    }
}

// ---- preferences --------------------------------------------------------

impl PreferenceRepository for Database {
    fn get(&self) -> Result<Preferences> {
        self.run(|c| {
            let json: Option<String> = c
                .query_row(
                    "SELECT value FROM preferences WHERE key = ?1",
                    [PREFS_KEY],
                    |r| r.get(0),
                )
                .optional()
                .db()?;
            json.map_or_else(
                || Ok(Preferences::default()),
                |j| serde_json::from_str(&j).map_err(storage),
            )
        })
    }

    fn set(&self, preferences: &Preferences) -> Result<()> {
        self.run(|c| {
            c.execute(
                "INSERT INTO preferences (key, value) VALUES (?1, ?2) \
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![
                    PREFS_KEY,
                    serde_json::to_string(preferences).map_err(storage)?
                ],
            )
            .db()
            .map(drop)
        })
    }
}

// ---- submissions --------------------------------------------------------

impl SubmissionRepository for Database {
    fn save(&self, s: &Submission) -> Result<()> {
        self.run(|c| {
            c.execute(
                "INSERT INTO submissions (id, environment_id, submitted_at, domain, \
                     transaction_code, sender_partner_id, receiver_partner_id, isa_control, \
                     gs_control, http_status, result, trace_values, payload, \
                     response_excerpt, duration_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    s.id.as_str(),
                    s.environment_id.as_str(),
                    to_micros(s.submitted_at)?,
                    s.domain,
                    s.transaction_code,
                    s.sender_partner_id.as_ref().map(PartnerId::as_str),
                    s.receiver_partner_id.as_ref().map(PartnerId::as_str),
                    s.isa_control,
                    s.gs_control,
                    s.http_status,
                    s.result,
                    serde_json::to_string(&s.trace_values).map_err(storage)?,
                    s.payload,
                    s.response_excerpt,
                    s.duration_ms
                        .map(|ms| i64::try_from(ms).unwrap_or(i64::MAX)),
                ],
            )
            .db_unique("submission already recorded")
            .map(drop)
        })
    }

    fn list_recent(&self, environment_id: &EnvironmentId, limit: u32) -> Result<Vec<Submission>> {
        self.run(|c| {
            fetch_submissions(
                c,
                "WHERE environment_id = ?1 ORDER BY submitted_at DESC, rowid DESC LIMIT ?2",
                params![environment_id.as_str(), limit],
            )
        })
    }

    fn get(&self, id: &SubmissionId) -> Result<Option<Submission>> {
        self.run(|c| {
            Ok(
                fetch_submissions(c, "WHERE id = ?1 LIMIT 1", params![id.as_str()])?
                    .into_iter()
                    .next(),
            )
        })
    }
}

type SubmissionRow = (
    String,
    String,
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    u32,
    u32,
    Option<u16>,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
);

/// Runs the submissions query with the given `WHERE ... ` tail.
fn fetch_submissions(
    c: &Connection,
    tail: &str,
    query_params: impl rusqlite::Params,
) -> Result<Vec<Submission>> {
    let sql = format!(
        "SELECT id, environment_id, submitted_at, domain, transaction_code, \
             sender_partner_id, receiver_partner_id, isa_control, gs_control, http_status, \
             result, trace_values, payload, response_excerpt, duration_ms \
         FROM submissions {tail}"
    );
    let mut stmt = c.prepare(&sql).db()?;
    let rows: Vec<SubmissionRow> = stmt
        .query_map(query_params, |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
                r.get(12)?,
                r.get(13)?,
                r.get(14)?,
            ))
        })
        .db()?
        .collect::<rusqlite::Result<_>>()
        .db()?;
    rows.into_iter()
        .map(
            |(
                id,
                environment_id,
                at,
                domain,
                txn,
                sender,
                receiver,
                isa,
                gs,
                status,
                result,
                trace,
                payload,
                excerpt,
                duration,
            )| {
                Ok(Submission {
                    id: id
                        .parse::<SubmissionId>()
                        .map_err(|_| storage("invalid submission id in database"))?,
                    environment_id: environment_id
                        .parse::<EnvironmentId>()
                        .map_err(|_| storage("invalid environment id in database"))?,
                    submitted_at: from_micros(at)?,
                    domain,
                    transaction_code: txn,
                    sender_partner_id: sender.as_deref().map(parse_partner_id).transpose()?,
                    receiver_partner_id: receiver.as_deref().map(parse_partner_id).transpose()?,
                    isa_control: isa,
                    gs_control: gs,
                    http_status: status,
                    result,
                    trace_values: serde_json::from_str::<Value>(&trace).map_err(storage)?,
                    payload,
                    response_excerpt: excerpt,
                    duration_ms: duration.and_then(|ms| u64::try_from(ms).ok()),
                })
            },
        )
        .collect()
}
