//! The state database: one SQLite file under the ratchet home, with SQLite compiled into the
//! binary (`rusqlite` `bundled`). Opening is the only thing this module does for group 0's hot
//! path: `pre-tool` never calls in here.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior};

pub const BUSY_TIMEOUT_MS: u64 = 5000;

#[derive(Debug)]
pub enum DbError {
    Sql(rusqlite::Error),
    Io(std::io::Error),
    /// The file is older than the binary expects and nothing here migrates it.
    Stale {
        found: i64,
        expected: i64,
    },
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Sql(e) => write!(f, "database: {e}"),
            DbError::Io(e) => write!(f, "database file: {e}"),
            DbError::Stale { found, expected } => write!(
                f,
                "database schema is at version {found}, this build expects {expected}: run `ratchet db migrate`"
            ),
        }
    }
}

impl std::error::Error for DbError {}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        DbError::Sql(e)
    }
}

impl From<std::io::Error> for DbError {
    fn from(e: std::io::Error) -> Self {
        DbError::Io(e)
    }
}

pub fn db_path(home: &Path) -> PathBuf {
    home.join("ratchet.db")
}

/// Opens (creating the file and its directory if needed) with the pragmas every face shares.
/// Does not migrate: that is `connect`, reached only from `session-start` and
/// `ratchet db migrate`.
pub fn open(path: &Path) -> Result<Connection, DbError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))?;
    // `PRAGMA journal_mode` answers with a row, so it cannot go through `execute`/`pragma_update`.
    conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}

/// In-memory database for unit tests: same pragmas minus WAL, which is meaningless without a file.
pub fn open_memory() -> Result<Connection, DbError> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}

/// Round-trip used by `ratchet db selftest`: proves the linked SQLite really executes SQL.
pub fn selftest() -> Result<String, DbError> {
    let conn = open_memory()?;
    conn.execute_batch("CREATE TABLE probe(v TEXT NOT NULL); INSERT INTO probe(v) VALUES ('ok');")?;
    let version: String = conn.query_row("SELECT sqlite_version()", [], |r| r.get(0))?;
    let value: String = conn.query_row("SELECT v FROM probe", [], |r| r.get(0))?;
    Ok(format!("sqlite {version} {value}"))
}

/// Ordered, embedded migrations: (version, name, SQL). Never reorder or rewrite an applied one;
/// add a new pair instead.
pub const MIGRATIONS: &[(i64, &str, &str)] =
    &[(1, "0001_init", include_str!("migrations/0001_init.sql"))];

pub const LATEST_VERSION: i64 = 1;

/// Version of the schema in `conn`; 0 when nothing has ever been applied.
pub fn current_version(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

/// Applies every pending migration, each inside its own IMMEDIATE transaction, and returns the
/// ones applied in this call. Idempotent: two processes opening the same new database race on the
/// transaction, not on the script.
pub fn migrate(conn: &mut Connection) -> Result<Vec<(i64, &'static str)>, DbError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);",
    )?;
    let mut applied = Vec::new();
    for (version, name, sql) in MIGRATIONS {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let already: i64 = tx.query_row(
            "SELECT COUNT(*) FROM schema_version WHERE version = ?1",
            [version],
            |r| r.get(0),
        )?;
        if already > 0 {
            drop(tx); // rollback: nothing was done
            continue;
        }
        tx.execute_batch(sql)?;
        let stamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        tx.execute(
            "INSERT INTO schema_version(version, applied_at) VALUES (?1, ?2)",
            rusqlite::params![version, stamp],
        )?;
        tx.commit()?;
        applied.push((*version, *name));
    }
    Ok(applied)
}

/// Open the database of `home` and bring it up to date. Only the session-start hook and
/// `ratchet db migrate` may call this (spec §6).
#[allow(dead_code)] // Wired by the session-start hook and `ratchet db migrate` (later task).
pub fn connect(home: &Path) -> Result<Connection, DbError> {
    let mut conn = open(&db_path(home))?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// Open the database of `home` for use, without migrating. Refuses an older schema, and refuses
/// a database that is not there: a read-only face must not leave an empty file behind on a
/// machine where nothing has ever run (that file would then be reported stale forever).
#[allow(dead_code)] // Wired by every read-only face (later task); nothing calls it yet.
pub fn open_ready(home: &Path) -> Result<Connection, DbError> {
    let path = db_path(home);
    if !path.is_file() {
        return Err(DbError::Stale {
            found: 0,
            expected: LATEST_VERSION,
        });
    }
    let conn = open(&path)?;
    let found = current_version(&conn);
    if found < LATEST_VERSION {
        return Err(DbError::Stale {
            found,
            expected: LATEST_VERSION,
        });
    }
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selftest_reports_a_version_and_ok() {
        let text = selftest().unwrap();
        assert!(text.starts_with("sqlite 3."), "{text}");
        assert!(text.ends_with(" ok"), "{text}");
    }

    #[test]
    fn open_creates_the_file_and_its_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = db_path(&dir.path().join("nested"));
        let conn = open(&path).unwrap();
        conn.execute_batch("CREATE TABLE t(x INTEGER);").unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn db_path_is_ratchet_db_under_the_home() {
        assert!(db_path(Path::new("C:/tmp/home")).ends_with("ratchet.db"));
    }

    #[test]
    fn migrate_applies_once_and_is_idempotent() {
        let mut conn = open_memory().unwrap();
        let first = migrate(&mut conn).unwrap();
        assert_eq!(first.len(), MIGRATIONS.len());
        assert_eq!(first[0], (1, "0001_init"));
        assert_eq!(current_version(&conn), LATEST_VERSION);
        let second = migrate(&mut conn).unwrap();
        assert!(second.is_empty());
        assert_eq!(current_version(&conn), LATEST_VERSION);
    }

    #[test]
    fn migrate_creates_every_table_the_services_need() {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        for table in ["tasks", "checklist_items", "sessions", "events", "task_seq"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "missing table {table}");
        }
    }

    #[test]
    fn version_of_a_fresh_database_is_zero() {
        let conn = open_memory().unwrap();
        assert_eq!(current_version(&conn), 0);
    }

    #[test]
    fn open_ready_refuses_a_stale_database() {
        let dir = tempfile::TempDir::new().unwrap();
        drop(open(&db_path(dir.path())).unwrap());
        let err = open_ready(dir.path()).unwrap_err();
        assert!(
            matches!(
                err,
                DbError::Stale {
                    found: 0,
                    expected: 1
                }
            ),
            "{err:?}"
        );
        assert!(err.to_string().contains("ratchet db migrate"), "{err}");
    }

    #[test]
    fn open_ready_creates_nothing_when_there_is_no_database() {
        let dir = tempfile::TempDir::new().unwrap();
        let err = open_ready(dir.path()).unwrap_err();
        assert!(
            matches!(
                err,
                DbError::Stale {
                    found: 0,
                    expected: 1
                }
            ),
            "{err:?}"
        );
        assert!(
            !db_path(dir.path()).exists(),
            "a read-only face created the database"
        );
    }

    #[test]
    fn connect_migrates_and_open_ready_then_accepts() {
        let dir = tempfile::TempDir::new().unwrap();
        drop(connect(dir.path()).unwrap());
        let conn = open_ready(dir.path()).unwrap();
        assert_eq!(current_version(&conn), LATEST_VERSION);
    }

    #[test]
    fn migrations_declare_no_transaction_of_their_own() {
        for (_, name, sql) in MIGRATIONS {
            let lowered = sql.to_lowercase();
            assert!(!lowered.contains("begin"), "{name} opens a transaction");
            assert!(!lowered.contains("commit"), "{name} commits");
        }
    }
}
