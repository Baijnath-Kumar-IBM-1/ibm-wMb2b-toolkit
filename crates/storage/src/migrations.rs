//! Versioned schema migrations.
//!
//! The applied version is kept in SQLite's `user_version`. Each migration
//! runs in its own transaction together with the version bump, so a failed
//! migration leaves the database at the previous version. A database written
//! by a newer build is refused instead of being modified.
//!
//! To change the schema, add `migrations/000N_name.sql` and append it to
//! [`MIGRATIONS`]. Never edit a released migration.

use rusqlite::Connection;
use trace_core::{Error, Result};

use crate::db::SqlExt;

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial schema",
        sql: include_str!("../migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "submission details",
        sql: include_str!("../migrations/0002_submission_details.sql"),
    },
];

/// Schema version after all built-in migrations.
pub fn latest_version() -> u32 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}

/// Applies pending built-in migrations.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    migrate_with(conn, MIGRATIONS)
}

pub(crate) fn current_version(conn: &Connection) -> Result<u32> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0)).db()
}

pub(crate) fn migrate_with(conn: &mut Connection, migrations: &[Migration]) -> Result<()> {
    let latest = migrations.last().map_or(0, |m| m.version);
    let current = current_version(conn)?;
    if current > latest {
        return Err(Error::Storage(format!(
            "database schema version {current} is newer than this application supports ({latest}); \
             update the application"
        )));
    }
    for m in migrations.iter().filter(|m| m.version > current) {
        let tx = conn.transaction().db()?;
        tx.execute_batch(m.sql).map_err(|e| {
            Error::Storage(format!("migration {} ({}) failed: {e}", m.version, m.name))
        })?;
        // `user_version` takes no bound parameters; the value is a u32.
        tx.execute_batch(&format!("PRAGMA user_version = {}", m.version))
            .db()?;
        tx.commit().db()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.pragma_update(None, "foreign_keys", true).unwrap();
        c
    }

    #[test]
    fn built_in_versions_are_contiguous_from_one() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(m.version as usize, i + 1, "{}", m.name);
        }
    }

    #[test]
    fn fresh_database_reaches_latest_and_is_idempotent() {
        let mut c = mem();
        migrate(&mut c).unwrap();
        assert_eq!(current_version(&c).unwrap(), latest_version());
        migrate(&mut c).unwrap();
        assert_eq!(current_version(&c).unwrap(), latest_version());
        let tables: Vec<String> = c
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        for t in [
            "counters",
            "environments",
            "installed_documents",
            "partners",
            "preferences",
            "step_records",
            "submissions",
        ] {
            assert!(tables.iter().any(|x| x == t), "missing table {t}");
        }
    }

    #[test]
    fn upgrade_applies_only_pending_migrations_in_order() {
        let mut c = mem();
        let v1 = [Migration {
            version: 1,
            name: "a",
            sql: "CREATE TABLE a(x INTEGER);",
        }];
        migrate_with(&mut c, &v1).unwrap();
        c.execute("INSERT INTO a VALUES (7)", []).unwrap();
        let v2 = [
            Migration {
                version: 1,
                name: "a",
                sql: "CREATE TABLE a(x INTEGER);",
            },
            Migration {
                version: 2,
                name: "b",
                sql: "ALTER TABLE a ADD COLUMN y INTEGER DEFAULT 5;",
            },
        ];
        migrate_with(&mut c, &v2).unwrap();
        assert_eq!(current_version(&c).unwrap(), 2);
        let y: i64 = c
            .query_row("SELECT y FROM a WHERE x = 7", [], |r| r.get(0))
            .unwrap();
        assert_eq!(y, 5);
    }

    #[test]
    fn failed_migration_rolls_back_and_keeps_old_version() {
        let mut c = mem();
        let v1 = [Migration {
            version: 1,
            name: "a",
            sql: "CREATE TABLE a(x INTEGER);",
        }];
        migrate_with(&mut c, &v1).unwrap();
        let broken = [
            Migration {
                version: 1,
                name: "a",
                sql: "CREATE TABLE a(x INTEGER);",
            },
            Migration {
                version: 2,
                name: "broken",
                sql: "CREATE TABLE b(x INTEGER); THIS IS NOT SQL;",
            },
        ];
        let err = migrate_with(&mut c, &broken).unwrap_err();
        assert!(err.to_string().contains("migration 2"));
        assert_eq!(current_version(&c).unwrap(), 1);
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "partial migration must not remain");
    }

    #[test]
    fn submission_details_migration_keeps_existing_rows() {
        let mut c = mem();
        migrate_with(&mut c, &MIGRATIONS[..1]).unwrap();
        assert_eq!(current_version(&c).unwrap(), 1);
        c.execute_batch(
            "INSERT INTO environments (id, name, b2b_host, instance_id, iam_host, created_at, updated_at) \
             VALUES ('e1', 'Dev', 'h.example', 'i', 'iam.example', 0, 0); \
             INSERT INTO submissions (id, environment_id, submitted_at, domain, transaction_code, \
             isa_control, gs_control, http_status, result, trace_values) \
             VALUES ('s1', 'e1', 5, 'retail', '850', 1, 1, 200, 'success', '{}');",
        )
        .unwrap();

        migrate(&mut c).unwrap();
        assert_eq!(current_version(&c).unwrap(), latest_version());
        let (payload, excerpt, duration): (Option<String>, Option<String>, Option<i64>) = c
            .query_row(
                "SELECT payload, response_excerpt, duration_ms FROM submissions WHERE id = 's1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((payload, excerpt, duration), (None, None, None));
    }

    #[test]
    fn newer_database_is_refused_untouched() {
        let mut c = mem();
        c.execute_batch("PRAGMA user_version = 99").unwrap();
        let err = migrate(&mut c).unwrap_err();
        assert!(err.to_string().contains("newer"));
        assert_eq!(current_version(&c).unwrap(), 99);
    }
}
