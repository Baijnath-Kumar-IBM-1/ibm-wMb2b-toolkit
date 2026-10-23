//! SQLite connection management.

use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::Connection;
use time::OffsetDateTime;
use trace_core::{Error, Result};

use crate::{fsutil, migrations};

pub(crate) fn storage(e: impl fmt::Display) -> Error {
    Error::Storage(e.to_string())
}

/// Maps `rusqlite` results into core results.
pub(crate) trait SqlExt<T> {
    /// Any failure becomes `Error::Storage`.
    fn db(self) -> Result<T>;
    /// A UNIQUE or PRIMARY KEY violation becomes `Error::Conflict(message)`;
    /// anything else becomes `Error::Storage`.
    fn db_unique(self, message: &str) -> Result<T>;
}

impl<T> SqlExt<T> for rusqlite::Result<T> {
    fn db(self) -> Result<T> {
        self.map_err(storage)
    }

    fn db_unique(self, message: &str) -> Result<T> {
        self.map_err(|e| {
            if is_unique_violation(&e) {
                Error::Conflict(message.to_string())
            } else {
                storage(e)
            }
        })
    }
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    use rusqlite::ffi::{SQLITE_CONSTRAINT_PRIMARYKEY, SQLITE_CONSTRAINT_UNIQUE};
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _)
            if f.extended_code == SQLITE_CONSTRAINT_UNIQUE
                || f.extended_code == SQLITE_CONSTRAINT_PRIMARYKEY
    )
}

/// Timestamps are stored as microseconds since the Unix epoch (UTC).
pub(crate) fn to_micros(t: OffsetDateTime) -> Result<i64> {
    i64::try_from(t.unix_timestamp_nanos() / 1_000).map_err(storage)
}

pub(crate) fn from_micros(us: i64) -> Result<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(us) * 1_000).map_err(storage)
}

/// The application database. Cheap to clone; clones share one connection.
///
/// Calls are synchronous and short. Async callers should use
/// `tokio::task::spawn_blocking` when holding the runtime matters.
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Database")
    }
}

impl Database {
    /// Opens (creating if needed) the database file and applies migrations.
    /// On Unix the file is created with mode 0600 and new parent directories
    /// with mode 0700.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fsutil::create_private_dir_all(dir).map_err(storage)?;
        }
        fsutil::ensure_private_file(path).map_err(storage)?;
        let conn = Connection::open(path).db()?;
        Self::init(conn, true)
    }

    /// Private in-memory database (tests and tools).
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().db()?, false)
    }

    fn init(mut conn: Connection, file_backed: bool) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5)).db()?;
        conn.pragma_update(None, "foreign_keys", true).db()?;
        conn.pragma_update(None, "trusted_schema", false).db()?;
        if file_backed {
            let _mode: String = conn
                .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
                .db()?;
        }
        migrations::migrate(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock does not corrupt SQLite state
        // (open transactions roll back on drop), so recover from poisoning.
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn run<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        f(&mut self.lock())
    }

    /// Schema version of the open database.
    pub fn schema_version(&self) -> Result<u32> {
        self.run(|c| migrations::current_version(c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn micros_round_trip() {
        let t = OffsetDateTime::from_unix_timestamp_nanos(1_735_689_600_123_456_000).unwrap();
        assert_eq!(from_micros(to_micros(t).unwrap()).unwrap(), t);
    }

    #[test]
    fn file_database_has_wal_foreign_keys_and_private_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("app.sqlite");
        let db = Database::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), migrations::latest_version());
        db.run(|c| {
            let fk: i64 = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).db()?;
            assert_eq!(fk, 1);
            let mode: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0)).db()?;
            assert_eq!(mode, "wal");
            Ok(())
        })
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(file, 0o600);
            let parent = std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(parent, 0o700);
        }
    }

    #[test]
    fn reopening_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.sqlite");
        let db = Database::open(&path).unwrap();
        db.run(|c| {
            c.execute("INSERT INTO preferences VALUES ('k','v')", [])
                .db()
        })
        .unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        let v: String = db
            .run(|c| {
                c.query_row("SELECT value FROM preferences WHERE key='k'", [], |r| {
                    r.get(0)
                })
                .db()
            })
            .unwrap();
        assert_eq!(v, "v");
    }
}
