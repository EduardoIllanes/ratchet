# ratchet — Group 1: state, sessions and the six non-pre-tool hooks

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give ratchet a local state layer — one embedded-SQLite database under `~/.ratchet/`, versioned migrations, an append-only event log and a sessions service — and give bodies to the six hooks that group 0 left as no-ops: register the session at start, heartbeat on every prompt/subagent-stop/compaction/stop, end it at session end, and return to `ready` the tasks of sessions that died.

**Architecture:** `db/` owns the connection and the embedded migrations; `model.rs` owns the types and their string forms; `services/` (events, sessions, tasks) is the only module that writes SQL; `hooks/dispatch.rs` and `cli/` are thin faces over it. `pre-tool` is untouched and still opens nothing. Every mutation appends one row to `events`. Session state (`live`/`idle`/`orphaned`/`ended`) is derived from `last_seen` and the repo's thresholds, never stored.

**Tech Stack:** Rust 2021, `rusqlite` with the `bundled` feature (SQLite compiled into the binary, no system library), `chrono` with `serde`, `serde_json`, `clap` 4; the group-0 stack unchanged (`toml`, `fancy-regex`, `dirs`; dev: `assert_cmd`, `predicates`, `tempfile`).

**Spec:** `docs/superpowers/specs/2026-09-16-ratchet-plugin-design.md` (§2 D-runtime, D-marker, D-state, D-p3, D-specs-first, D-roles, D-english, D-runner-out; §3 layout; §4.1 CLI, §4.2 hook table, §4.3 marker thresholds, §4.5 sessions; §5 data flow; §6 error handling; §7 testing; §8 group 1).

**Predecessor:** `docs/superpowers/plans/2026-09-16-ratchet-group-0-guardrails.md` (complete). Its ledger is `.superpowers/sdd/2026-09-16-ratchet-group-0-guardrails/progress.md`; its rulings R1-R10 still hold.

## Global Constraints

These are group 0's constraints, unchanged, plus the ones this group adds. Every task's requirements implicitly include this section.

**Carried over from group 0 (do not weaken):**

- **No git commands in `C:\repos\ratchet` on the owner's machine** (spec D-roles). Every task ends with a hand-off listing the files created or changed; the owner commits from another account. Tests may run `git` inside temporary directories only. No `git status`, no `git diff`, not even read-only, inside this repo.
- Work directly in `C:\repos\ratchet` (there is no worktree because there is no git flow here; the `main-tree` guardrail of `ops` does not apply to this directory).
- All content, identifiers, messages and docs in English (spec D-english). The stderr prefix of a block is exactly `[ratchet guardrail:<id>]`.
- Hook exit codes: 0 allow / internal error, 2 block. Nothing else, ever (spec D-p3).
- `pre-tool` never opens a database and never spawns a subprocess except `git ls-files` when a write targets the main tree (spec 4.2).
- Latency target: `pre-tool` median under 30 ms in release on Windows; the test ceiling in debug builds is 200 ms (spec 4.2, 7).
- State root: `RATCHET_HOME` or `~/.ratchet` (spec D-state).
- Repo marker file name: `ratchet.toml` at the repo root (spec 4.3). No global list of repos.
- Rust toolchain: `stable-x86_64-pc-windows-gnu` with WinLibs gcc on PATH (group-0 ruling R3, amended). Every shell that runs cargo starts with:
  ```
  export PATH="$HOME/.cargo/bin:/c/Users/eillanes/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
  ```
- The crate is **bin-only** (group-0 ruling R8): unit tests run with `cargo test -p ratchet --bin ratchet`, lint with `cargo clippy -p ratchet --bin ratchet -- -D warnings`. Never `--lib`.
- Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p ratchet`, plus `cargo test -p ratchet --test scenarios`.
- Example patterns in tests and docs use neutral method names (`purge_all`, `write_rows`), never the write methods of a real database driver: the owner's own harness scans written content and blocks those names.
- No `rm -rf` anywhere (group-0 ruling R5): leave temporary files or use the scratchpad.

**Added by group 1:**

- **SQLite is embedded**: `rusqlite` with the `bundled` feature, pinned to an exact version. No system `libsqlite3`, no `sqlite3` CLI, no other database crate (spec D-runtime, D-state).
- **One state database**: `<home>/ratchet.db`, where `<home>` is `RATCHET_HOME` or `~/.ratchet`. Opened with WAL, `busy_timeout = 5000`, `foreign_keys = ON`. An in-memory database skips WAL.
- **Only `session-start` and `ratchet db migrate` migrate** (spec §6). Every other face opens the database *ready* and fails with `run ratchet db migrate` when the schema is older than the binary expects; in a hook that failure is exit 0 plus one log line.
- **Migration files contain no `BEGIN`/`COMMIT`**: the runner owns one `IMMEDIATE` transaction per migration, so every migration is all-or-nothing and two processes racing the first open cannot half-apply one.
- **Timestamps** are RFC 3339 UTC with seconds precision and a `Z` suffix (`2026-09-16T12:00:00Z`), stored as TEXT so they sort lexicographically. One formatter (`clock::iso`), one parser (`clock::parse`).
- **`events` is append-only**: `INSERT` only, never `UPDATE` or `DELETE`, in any module, ever. Every mutation of a session or a task writes exactly one row per fact.
- **`services/` is the only writer** (spec §3 layer rule): no `INSERT`/`UPDATE`/`DELETE` in `db/`, `cli/`, `hooks/` or tests of the binary. Scenario tests may seed rows directly because they stand outside the binary, and they say so in a comment.
- **Every group-1 hook exits 0, always.** The only exit-2 paths in the binary stay the guardrail match (group 0) and the handoff rule on `Stop` (group 2). A group-1 hook that can return 2 is a bug.
- CLI commands exit 1 (not 2) on a user-facing error, with `error: …` on stderr.
- `hooks/hooks.json` is **not** modified: group 0 already registers all seven events; this group only gives six of them bodies.
- `Cargo.toml` of the crate is edited **once**, in Task 1. No later task adds a dependency.
- `RATCHET_NOW` (RFC 3339) overrides "now" in every face. It is a documented test seam (Ruling G1-R3); services always take `now` as a parameter and never read the clock themselves.

---

## File structure

```
crates/ratchet/
├── Cargo.toml                        + rusqlite (bundled), chrono serde feature      [Task 1]
├── src/main.rs                       + mod db/model/clock/services/cli, db & session  [1,5,6,9,11]
├── src/clock.rs                      now (RATCHET_NOW seam), iso, parse               [Task 5]
├── src/model.rs                      Session, Event, enums, their string forms        [Task 5]
├── src/db/mod.rs                     open, connect, open_ready, migrate, db_path      [Tasks 1,4]
├── src/db/migrations/0001_init.sql   tasks, checklist_items, sessions, events         [Task 4]
├── src/services/mod.rs               ServiceError + pub mod events/sessions/tasks     [Tasks 6,7,8]
├── src/services/events.rs            emit, for_session — append-only                  [Task 6]
├── src/services/sessions.rs          get, list, upsert_start, touch, end, state, resolve [Task 7]
├── src/services/tasks.rs             orphaned, release, release_dead, claimed_ids     [Task 8]
├── src/cli/mod.rs                    pub mod db_cmd, session_cmd                      [Task 9]
├── src/cli/db_cmd.rs                 `ratchet db migrate|path`                        [Task 9]
├── src/cli/session_cmd.rs            `ratchet session list|show`                      [Task 11]
├── src/hooks/dispatch.rs             session-start, heartbeat, session-end            [Task 10]
├── src/repo.rs                       + git_branch                                     [Task 10]
├── tests/db_open.rs                  the bundled-SQLite smoke test                    [Task 1]
└── tests/spec/{main,support,sessions}.rs   24 scenario tests                          [Task 3]

openspec/specs/sessions/spec.md       the ported capability                            [Task 2]
README.md                             one appended section + one corrected line        [Task 11]
```

Scenario slug rule (unchanged, enforced by `crates/ratchet/tests/scenarios.rs`): lowercase the scenario title, replace every run of non-alphanumerics with `_`, trim `_`; the test function is `fn <spec>__<slug>()` where `<spec>` is the spec directory name with `-` → `_`. For this group `<spec>` is `sessions`. Example: `#### Scenario: Idempotent registration` → `fn sessions__idempotent_registration()`. The checker ignores `#### Scenario:` lines inside fenced code blocks, so the fenced spec text in this plan does not count — only what lands in `openspec/specs/sessions/spec.md` does.

## Parallelism

Tasks with disjoint files may run at the same time; the cap is 5 concurrent agents (group-0 ruling R9) and these waves never exceed 3.

| Wave | Tasks | Why they don't collide |
|---|---|---|
| A | **1** alone | It is the technical risk of the group (bundled SQLite under gcc) and it edits `Cargo.toml`, which everything else depends on. |
| B | **2**, **4**, **5** | 2 = `openspec/**` only; 4 = `src/db/**`; 5 = `src/clock.rs`, `src/model.rs` (+ one `mod` line in `main.rs`, which 4 does not touch). |
| C | **3**, **6** | 3 = `tests/spec/**` only; 6 = `src/services/**` (+ one `mod` line in `main.rs`). |
| D | **7**, **9** | 7 = `src/services/{mod.rs,sessions.rs}`; 9 = `src/cli/**` + `main.rs`. Disjoint. |
| E | **8** alone | It appends to `src/services/mod.rs`, which 7 just changed. |
| F | **10**, **11** | 10 = `src/hooks/dispatch.rs`, `src/repo.rs`; 11 = `src/cli/session_cmd.rs`, `main.rs`, `README.md`. Disjoint. |
| G | **12** alone | Read-only review of everything. |

Reviews follow group 0's pattern: a task's review may run in the wave after it, alongside the next implementation task, as long as a fix round would touch only that task's files.

---

### Task 1: Embedded SQLite compiles and opens (the group's technical risk)

The whole group rests on `rusqlite`'s `bundled` feature building SQLite's C sources with the WinLibs gcc that group 0 settled on. Nothing else is in this task: if this does not build, the group stops here and the owner decides (system SQLite, a different toolchain, or a different store).

**Files:**
- Modify: `crates/ratchet/Cargo.toml`, `crates/ratchet/src/main.rs` (one line)
- Create: `crates/ratchet/src/db/mod.rs`
- Test: `crates/ratchet/tests/db_open.rs`

**Interfaces:**
- Produces: `db::db_path(home: &Path) -> PathBuf` (`<home>/ratchet.db`), `db::open(path: &Path) -> Result<Connection, DbError>`, `db::open_memory() -> Result<Connection, DbError>`, `db::DbError`. Task 4 extends the same module with `migrate`, `connect`, `open_ready`, `current_version`, `LATEST_VERSION`.
- Produces: the full dependency list of the group. **No later task edits `Cargo.toml`.**

- [ ] **Step 1: Write the failing test**

`crates/ratchet/tests/db_open.rs`:
```rust
//! The one thing group 1 cannot work around: SQLite compiled into the binary by this toolchain.

use assert_cmd::Command;

#[test]
fn binary_still_runs() {
    Command::cargo_bin("ratchet")
        .unwrap()
        .arg("version")
        .assert()
        .success();
}

#[test]
fn bundled_sqlite_is_linked_and_usable() {
    // Exercised through the binary's own module, not a separate connection: this proves the
    // crate links libsqlite3-sys built from source, not a system library.
    let out = Command::cargo_bin("ratchet")
        .unwrap()
        .args(["db", "selftest"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("sqlite "), "got: {text}");
    assert!(text.contains(" ok"), "got: {text}");
}
```

- [ ] **Step 2: Run it and watch it fail**

```
export PATH="$HOME/.cargo/bin:/c/Users/eillanes/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cd /c/repos/ratchet && cargo test -p ratchet --test db_open
```
Expected: `binary_still_runs` PASS, `bundled_sqlite_is_linked_and_usable` FAIL (`db` is not a subcommand yet).

- [ ] **Step 3: Add the dependencies**

`crates/ratchet/Cargo.toml` — replace the `[dependencies]` block with this one (nothing else in the file changes):
```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
fancy-regex = "0.13"
chrono = { version = "0.4", features = ["serde"] }
dirs = "5"
rusqlite = { version = "0.32.1", features = ["bundled"] }
```

`rusqlite` 0.32.1 declares no MSRV of its own (checked with `cargo info rusqlite@0.32.1`, 2026-09-16), so the crate's `rust-version = "1.79"` floor stands. If `cargo check` refuses because a transitive dependency declares a higher floor, do **not** loosen the pin silently: raise `rust-version` in the same edit, write the new value and the crate that forced it in the hand-off, and say so in the ledger as a ruling.

- [ ] **Step 4: Write the module**

`crates/ratchet/src/db/mod.rs`:
```rust
//! The state database: one SQLite file under the ratchet home, with SQLite compiled into the
//! binary (`rusqlite` `bundled`). Opening is the only thing this module does for group 0's hot
//! path: `pre-tool` never calls in here.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;

pub const BUSY_TIMEOUT_MS: u64 = 5000;

#[derive(Debug)]
pub enum DbError {
    Sql(rusqlite::Error),
    Io(std::io::Error),
    /// The file is older than the binary expects and nothing here migrates it.
    Stale { found: i64, expected: i64 },
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
/// Does not migrate: that is `connect` (Task 4), reached only from `session-start` and
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
}
```

- [ ] **Step 5: Wire the subcommand**

In `crates/ratchet/src/main.rs`, add `mod db;` next to the other `mod` lines (alphabetical: after `mod config;`), add the variant to `enum Cmd`:
```rust
    /// State database: `migrate`, `path`. `selftest` proves the embedded SQLite works.
    Db {
        #[command(subcommand)]
        cmd: DbCmd,
    },
```
the subcommand enum next to `GuardrailsCmd`:
```rust
#[derive(Subcommand)]
enum DbCmd {
    /// Print one line proving the bundled SQLite is linked and executes SQL.
    Selftest,
}
```
and the arm in `main`:
```rust
        Cmd::Db { cmd } => match cmd {
            DbCmd::Selftest => match db::selftest() {
                Ok(line) => {
                    println!("{line}");
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            },
        },
```
Task 9 replaces this arm with `cli::db_cmd::run` and adds `migrate` and `path`; `selftest` stays.

- [ ] **Step 6: Run everything**

```
cd /c/repos/ratchet && cargo test -p ratchet --test db_open && cargo test -p ratchet --bin ratchet && cargo test -p ratchet && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: all green, including group 0's 66 tests.

- [ ] **Step 7: Measure the latency again**

```
cd /c/repos/ratchet && cargo test -p ratchet --release --test latency -- --nocapture
```
Expected: still under the ceiling (group 0 measured a release median of 46.3 ms, p90 51.7 ms). Static SQLite adds roughly 1.5 MB to the binary; report the new median, the new p90 and the size of `target/release/ratchet.exe`. A regression above the ceiling is a finding for the owner, not something to fix by dropping `bundled`.

- [ ] **Step 8: Hand off (no git)**

List the four files. Report: the rusqlite version that built, the gcc line, the selftest output, the latency numbers and the binary size. Nothing is committed.

---

### Task 2: Ported spec — `openspec/specs/sessions/spec.md`

The behaviour comes from `C:\repos\ops`: `openspec/specs/sessions/spec.md` (identity, heartbeat, derived state, orphaned tasks, releasing a task) and the hook-facing requirements of `openspec/specs/agent-protocol/spec.md` (registry, headless sessions, hooks never break a session). Requirements about the web UI, the task board itself and the agent runner are **not** ported. The language is English and every mention of `ops` becomes `ratchet` (D-english); `_unknown` disappears because ratchet has no central repo registry — no marker means no session at all (D-marker).

**Files:**
- Create: `openspec/specs/sessions/spec.md`

**Interfaces:**
- Produces: the 24 scenario titles below are the contract for Task 3's tests and for the checker in `crates/ratchet/tests/scenarios.rs`. Nothing else in the repo changes.

- [ ] **Step 1: Write the spec**

`openspec/specs/sessions/spec.md`:
```markdown
# sessions

Which agent sessions exist, which are alive, and what happens to the work of the ones that
died. Group 1 covers the state layer and the six hooks that are not `PreToolUse`; the task
board, the briefing and the handoff rule arrive with group 2.

## Purpose

An agent session is the unit of accountability: a task is claimed by a session, and a
session that disappears must not keep work hostage. The registry is written by the hooks
themselves, so it cannot be skipped from the prompt, and it never invents an identity of its
own.

## Requirements

### Requirement: One local state database
All state SHALL live in a single database file under the state directory, created on first
use. The schema SHALL be versioned and applied by ordered migrations embedded in the binary;
applying them SHALL be idempotent and all-or-nothing per migration. Only the session-start
hook and the explicit migrate command SHALL migrate; any other face SHALL refuse to work on
an older schema, naming the command that fixes it, and in a hook that refusal SHALL be
silent success plus one log line. The path of the database SHALL be printable.

#### Scenario: First hook creates the database
- **WHEN** the session-start hook runs in an opted-in repo and no database exists yet
- **THEN** the database file exists afterwards and the session is registered

#### Scenario: Migrations apply once
- **WHEN** the migrate command runs twice in a row
- **THEN** the first run reports the migrations it applied and the second reports that nothing was pending

#### Scenario: The database path is printable
- **WHEN** the owner asks for the database path
- **THEN** one line with the absolute path inside the state directory is printed

#### Scenario: A stale schema is reported, not migrated
- **WHEN** a command that only reads state runs against a database whose schema is older than the binary expects
- **THEN** it fails with a message naming the migrate command, and the schema is left untouched

#### Scenario: The guardrail hook opens no database
- **WHEN** the pre-tool hook evaluates a tool call in an opted-in repo with no database yet
- **THEN** no database file is created

### Requirement: Session registry
A session SHALL be registered from the hooks with the identifier the agent harness gives
them; ratchet SHALL NOT invent another. The registry SHALL record the repo it opted into,
the working directory, the worktree when the directory is one, the branch when it can be
read, the mode (interactive or headless), who launched it (the user or the platform), the
start, the last signal and the end. Registering the same identifier twice SHALL keep one
session and refresh it instead of creating a second. The identifier SHALL be published to
the session's shell environment when the harness offers a file for it. In a directory with
no marker above it, nothing SHALL be registered.

#### Scenario: Idempotent registration
- **WHEN** the session-start hook receives the same session identifier twice, the second time from another directory of the same repo
- **THEN** exactly one session exists with that identifier, its last signal is the later one, and two start events are recorded

#### Scenario: A session in a worktree records branch and worktree
- **WHEN** a session starts inside a linked worktree of an opted-in repo
- **THEN** the session records that directory as its worktree and the branch checked out there

#### Scenario: A headless session launched by the platform
- **WHEN** a session starts with the environment declaring headless mode and the platform as launcher
- **THEN** the session is registered with that mode and that launcher, and with the identifier the environment fixed

#### Scenario: The session id reaches the shell
- **WHEN** the session-start hook runs and the harness offers an environment file
- **THEN** that file gains one line exporting the session identifier

#### Scenario: No marker, no session
- **WHEN** the session-start hook runs in a directory with no marker above it
- **THEN** it exits successfully, prints nothing, and no database is created

### Requirement: Heartbeat
Every user prompt, every end of a response, every end of a subagent and every compaction
SHALL refresh the last signal of the session. A prompt and an end of response SHALL also
record an event; a subagent end and a compaction SHALL NOT. A hook that arrives for a
session that is not registered yet SHALL register it instead of failing.

#### Scenario: A prompt updates the last signal
- **WHEN** a prompt arrives for a registered session
- **THEN** its last signal moves forward and a prompt event is recorded

#### Scenario: Compaction and subagent end update the last signal
- **WHEN** a compaction and then a subagent end arrive for a registered session
- **THEN** its last signal moves forward and no extra event is recorded for either

#### Scenario: A hook of an unregistered session registers it
- **WHEN** the first hook to arrive for a session identifier is a prompt, not a session start
- **THEN** the session is registered and its last signal is set

### Requirement: Derived session state
The state of a session SHALL be derived, never stored: ended when it has an end; otherwise
live while the last signal is recent, idle after the live threshold, and orphaned after the
idle threshold. Both thresholds SHALL be configurable per repo in the marker.

#### Scenario: A session with no signal for ninety minutes is orphaned
- **WHEN** a session has no end and its last signal was ninety minutes ago
- **THEN** its state is orphaned

#### Scenario: An ended session stays ended
- **WHEN** a session recorded its end one minute ago
- **THEN** its state is ended even though its last signal is recent

#### Scenario: The repo sets its own thresholds
- **WHEN** the marker sets a live threshold of one minute and an idle threshold of two, and the last signal was ninety seconds ago
- **THEN** the state is idle rather than live

### Requirement: Session end
The session-end hook SHALL record the end of the session and SHALL NOT block. Ending a
session that was never registered SHALL be silent success.

#### Scenario: Session end marks the session ended
- **WHEN** the session-end hook arrives for a live session
- **THEN** the session has an end, its state is ended, and an end event is recorded

### Requirement: Work of a dead session goes back
A task in progress claimed by a session that is ended or orphaned SHALL be released: it goes
back to ready, without a session, keeping its history. The release SHALL happen at the end
of the session that held it and, for sessions that died without a hook, at the next session
start in that repo. A task claimed by a live session SHALL NOT be released. Every release
SHALL record the change of state and a note saying it was released and by whom.

#### Scenario: Session end returns its tasks
- **WHEN** the session-end hook arrives for a session holding a task in progress
- **THEN** the task is ready, holds no session, and its history shows the release

#### Scenario: A new session releases tasks of a dead one
- **WHEN** a session starts in a repo where a task in progress is held by a session whose last signal is ninety minutes old
- **THEN** that task is ready and holds no session

#### Scenario: A task held by a live session is not released
- **WHEN** a session starts in a repo where a task in progress is held by another session that signalled a minute ago
- **THEN** that task is still in progress and still held by that session

### Requirement: Sessions can be listed and shown
Sessions SHALL be listable with their derived state, filterable to the live ones and by
repo, and one session SHALL be showable in detail with the tasks it holds. Asking for a
session without naming one SHALL resolve it: an explicit identifier first, then the one in
the environment, then the most recent live session whose directory or worktree covers the
current directory, most specific first. An identifier that does not exist SHALL fail with a
message, not silently.

#### Scenario: Listing shows the derived state
- **WHEN** two sessions exist, one signalling now and one ninety minutes ago, and the list is asked for
- **THEN** one line per session shows its identifier and its state, and the live filter leaves only the recent one

#### Scenario: Showing an unknown session fails with a message
- **WHEN** a session identifier that was never registered is shown
- **THEN** the command fails with a message naming that identifier

#### Scenario: Showing without an id resolves the session of the directory
- **WHEN** a session is shown without naming one, from a directory covered by a live session
- **THEN** the detail of that session is printed

### Requirement: State errors never break a session
A hook that cannot use the state database SHALL exit successfully, print nothing on the
context, and append one line to the log. It SHALL never block a tool call and SHALL never
interrupt the agent.

#### Scenario: An unusable database does not break the session
- **WHEN** the state directory holds a database file whose contents are not a database, and a prompt hook arrives
- **THEN** the hook exits successfully, prints nothing, and the log gains one line for the event
```

- [ ] **Step 2: Confirm the checker turns red for the right reason**

```
cd /c/repos/ratchet && cargo test -p ratchet --test scenarios
```
Expected: `every_scenario_has_a_test` FAILS listing exactly 24 missing tests, all with the prefix `sessions__`, and no `agent_protocol__` entry (group 0's 21 stay green).

- [ ] **Step 3: Hand off (no git)**

List the one file and the 24 scenario titles. Note for the owner: the checker is red on purpose until Task 3.

---
### Task 3: Scenario tests, red-clean (spec-test-author)

**Files:**
- Create: `crates/ratchet/tests/spec/sessions.rs`
- Modify: `crates/ratchet/tests/spec/main.rs` (one line), `crates/ratchet/tests/spec/support.rs` (append helpers; do not change the existing ones)

**Interfaces:**
- Consumes — this is the whole contract the author gets, because the spec names no type, flag or variable:
  - `ratchet hook <event>` reads the harness JSON payload on stdin; events `session-start`, `prompt`, `pre-tool`, `stop`, `subagent-stop`, `pre-compact`, `session-end`. Payload fields used here: `session_id`, `cwd`, `tool_name`, `tool_input`, `stop_hook_active`.
  - `ratchet db migrate`, `ratchet db path`, `ratchet db selftest`.
  - `ratchet session list [--repo <name>] [--live] [--json]`, `ratchet session show [<id>] [--json]`.
  - Environment: `RATCHET_HOME` (state directory), `RATCHET_SESSION_ID` (session identity when the payload has none), `RATCHET_SESSION_MODE` (`interactive`|`headless`), `RATCHET_LAUNCHED_BY` (`user`|`platform`), `RATCHET_NOW` (RFC 3339 instant that replaces the clock, per process), `CLAUDE_ENV_FILE` (file the hook appends the exported id to), `CLAUDE_SCRATCHPAD` (group 0).
  - Exit codes: every hook 0, except a guardrail block which is 2. CLI: 0 on success, 1 on a user-facing error with `error: …` on stderr.
  - Database: `<RATCHET_HOME>/ratchet.db`, tables `sessions(id, repo, repo_root, cwd, worktree, branch, mode, launched_by, started_at, last_seen, ended_at)`, `tasks(id, title, body, repo, repo_root, status, priority, parent_id, tags, claimed_by, created_at, updated_at, archived_at)`, `events(id, ts, session_id, task_id, kind, payload, source)`. Event kinds used: `session.start`, `session.prompt`, `session.stop`, `session.end`, `task.status`, `note`. Timestamps are RFC 3339 UTC seconds (`2026-09-16T12:00:00Z`).
- Produces: nothing for later tasks. These tests must be red for the right reason now and are **not edited by the implementer** of Tasks 4-11.

The author of this task reads only `openspec/specs/sessions/spec.md`, this task and `README.md`; not the design document and not the other tasks.

- [ ] **Step 1: Extend the support module**

In `crates/ratchet/tests/spec/main.rs` add one line, keeping the others:
```rust
mod sessions;
```

Append to `crates/ratchet/tests/spec/support.rs` (the existing helpers stay exactly as they are):
```rust
// --- group 1: state, sessions -------------------------------------------------------------

use rusqlite::Connection;

/// Fixed instant every group-1 scenario starts from. `RATCHET_NOW` replaces the process clock,
/// which is what makes "ninety minutes later" a test instead of a sleep.
pub const T0: &str = "2026-09-16T12:00:00Z";

/// `T0` plus `minutes`, in the same format the binary writes.
pub fn at(minutes: i64) -> String {
    at_secs(minutes * 60)
}

pub fn at_secs(seconds: i64) -> String {
    let base = chrono::DateTime::parse_from_rfc3339(T0).unwrap();
    (base + chrono::Duration::seconds(seconds))
        .with_timezone(&chrono::Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Payload of a session-scoped hook (no tool call).
pub fn session_payload(session_id: &str, cwd: &Path) -> Value {
    json!({ "session_id": session_id, "cwd": cwd.to_string_lossy() })
}

/// The linked worktree the sandbox creates.
pub fn worktree(sb: &Sandbox) -> PathBuf {
    sb.root().join(".worktrees").join("wt")
}

/// Key the binary stores in `repo_root`: the main root, absolute and lower-cased.
pub fn repo_root_key(sb: &Sandbox) -> String {
    sb.root().to_string_lossy().to_lowercase()
}

/// Run a hook with extra environment on top of the sandbox defaults.
pub fn hook_env(
    sb: &Sandbox,
    event: &str,
    payload: &Value,
    cwd: &Path,
    extra: &[(&str, &str)],
) -> Output {
    let mut cmd = Command::new(ratchet_bin());
    cmd.args(["hook", event])
        .current_dir(cwd)
        .env("RATCHET_HOME", sb.home.path())
        .env("CLAUDE_SCRATCHPAD", sb.scratchpad.path())
        .env_remove("RATCHET_SESSION_ID")
        .env_remove("RATCHET_SESSION_MODE")
        .env_remove("RATCHET_LAUNCHED_BY")
        .env_remove("RATCHET_NOW")
        .env_remove("CLAUDE_ENV_FILE")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// Run any CLI subcommand with the same isolation.
pub fn cli(sb: &Sandbox, args: &[&str], cwd: &Path, extra: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(ratchet_bin());
    cmd.args(args)
        .current_dir(cwd)
        .env("RATCHET_HOME", sb.home.path())
        .env_remove("RATCHET_SESSION_ID")
        .env_remove("RATCHET_NOW")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

pub fn db_file(sb: &Sandbox) -> PathBuf {
    sb.home.path().join("ratchet.db")
}

/// Read-only handle on the state database. Tests stand OUTSIDE the binary, so they are allowed
/// to look at (and seed) rows directly; production code never does this outside `services/`.
pub fn db(sb: &Sandbox) -> Connection {
    Connection::open(db_file(sb)).unwrap()
}

/// Seed one task straight into the database: the board CLI arrives with group 2, and the
/// release-of-dead-work scenarios need a claimed task to exist today.
pub fn seed_task(sb: &Sandbox, id: &str, status: &str, claimed_by: Option<&str>) {
    let conn = db(sb);
    conn.execute(
        "INSERT INTO tasks(id,title,body,repo,repo_root,status,priority,tags,claimed_by,created_at,updated_at) \
         VALUES (?1,?2,'',?3,?4,?5,3,'[]',?6,?7,?7)",
        rusqlite::params![
            id,
            format!("seeded {id}"),
            sb.root().file_name().unwrap().to_string_lossy(),
            repo_root_key(sb),
            status,
            claimed_by,
            T0
        ],
    )
    .unwrap();
}

/// `(status, claimed_by)` of a task.
pub fn task_row(sb: &Sandbox, id: &str) -> (String, Option<String>) {
    db(sb)
        .query_row(
            "SELECT status, claimed_by FROM tasks WHERE id = ?1",
            rusqlite::params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

pub fn count(sb: &Sandbox, sql: &str, args: &[&str]) -> i64 {
    db(sb)
        .query_row(sql, rusqlite::params_from_iter(args.iter()), |r| r.get(0))
        .unwrap()
}

pub fn last_seen(sb: &Sandbox, session_id: &str) -> String {
    db(sb)
        .query_row(
            "SELECT last_seen FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |r| r.get(0),
        )
        .unwrap()
}

/// Register a session through the real hook, at a chosen instant.
pub fn start_session(sb: &Sandbox, id: &str, cwd: &Path, when: &str) -> Output {
    hook_env(
        sb,
        "session-start",
        &session_payload(id, cwd),
        cwd,
        &[("RATCHET_NOW", when)],
    )
}
```

- [ ] **Step 2: Write the scenario tests**

`crates/ratchet/tests/spec/sessions.rs`:
```rust
//! One test per `#### Scenario:` of `openspec/specs/sessions/spec.md`, named `sessions__<slug>`.
//! Every test drives the real binary as a subprocess, exactly as the harness would.

use crate::support::*;
use serde_json::json;

// --- One local state database --------------------------------------------------------------

#[test]
fn sessions__first_hook_creates_the_database() {
    let sb = sandbox();
    assert!(!db_file(&sb).exists());
    let out = start_session(&sb, "s-1", &sb.root(), T0);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(db_file(&sb).is_file(), "no database after session-start");
    assert_eq!(
        count(&sb, "SELECT COUNT(*) FROM sessions WHERE id = ?1", &["s-1"]),
        1
    );
}

#[test]
fn sessions__migrations_apply_once() {
    let sb = sandbox();
    let first = cli(&sb, &["db", "migrate"], &sb.root(), &[]);
    assert_eq!(code(&first), 0, "stderr: {}", stderr(&first));
    assert!(stdout(&first).contains("0001_init"), "got: {}", stdout(&first));
    let second = cli(&sb, &["db", "migrate"], &sb.root(), &[]);
    assert_eq!(code(&second), 0);
    assert!(
        stdout(&second).contains("already at version 1"),
        "got: {}",
        stdout(&second)
    );
}

#[test]
fn sessions__the_database_path_is_printable() {
    let sb = sandbox();
    let out = cli(&sb, &["db", "path"], &sb.root(), &[]);
    assert_eq!(code(&out), 0);
    let printed = stdout(&out).trim().to_lowercase();
    assert!(printed.ends_with("ratchet.db"), "got: {printed}");
    assert!(
        printed.contains(&sb.home.path().to_string_lossy().to_lowercase()),
        "got: {printed}"
    );
}

#[test]
fn sessions__a_stale_schema_is_reported_not_migrated() {
    let sb = sandbox();
    // An empty file that is a valid, empty database: schema version 0, older than this build.
    drop(rusqlite::Connection::open(db_file(&sb)).unwrap());
    let out = cli(&sb, &["session", "list"], &sb.root(), &[]);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(
        stderr(&out).contains("ratchet db migrate"),
        "got: {}",
        stderr(&out)
    );
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
            &["sessions"]
        ),
        0,
        "the stale database was migrated anyway"
    );
}

#[test]
fn sessions__the_guardrail_hook_opens_no_database() {
    let sb = sandbox();
    let out = hook_env(
        &sb,
        "pre-tool",
        &bash("python scripts/x.py", &sb.root()),
        &sb.root(),
        &[],
    );
    assert_eq!(code(&out), 2, "stderr: {}", stderr(&out));
    assert!(!db_file(&sb).exists(), "pre-tool created the database");
}

// --- Session registry ----------------------------------------------------------------------

#[test]
fn sessions__idempotent_registration() {
    let sb = sandbox();
    start_session(&sb, "s-6", &sb.root(), T0);
    let deep = sb.root().join("src").join("deep");
    let out = start_session(&sb, "s-6", &deep, &at(5));
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(
        count(&sb, "SELECT COUNT(*) FROM sessions WHERE id = ?1", &["s-6"]),
        1
    );
    assert_eq!(last_seen(&sb, "s-6"), at(5));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'session.start'",
            &["s-6"]
        ),
        2
    );
}

#[test]
fn sessions__a_session_in_a_worktree_records_branch_and_worktree() {
    let sb = sandbox();
    let wt = worktree(&sb);
    let out = start_session(&sb, "s-7", &wt, T0);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let shown = stdout(&cli(&sb, &["session", "show", "s-7"], &sb.root(), &[])).to_lowercase();
    assert!(shown.contains("branch wt"), "got: {shown}");
    assert!(
        shown.contains(&wt.to_string_lossy().to_lowercase()),
        "got: {shown}"
    );
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM sessions WHERE id = ?1 AND worktree IS NOT NULL",
            &["s-7"]
        ),
        1
    );
}

#[test]
fn sessions__a_headless_session_launched_by_the_platform() {
    let sb = sandbox();
    // No session_id in the payload: the launcher fixed the identity in the environment.
    let out = hook_env(
        &sb,
        "session-start",
        &json!({ "cwd": sb.root().to_string_lossy() }),
        &sb.root(),
        &[
            ("RATCHET_NOW", T0),
            ("RATCHET_SESSION_ID", "s-8"),
            ("RATCHET_SESSION_MODE", "headless"),
            ("RATCHET_LAUNCHED_BY", "platform"),
        ],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let shown = stdout(&cli(&sb, &["session", "show", "s-8"], &sb.root(), &[]));
    assert!(shown.contains("headless"), "got: {shown}");
    assert!(shown.contains("platform"), "got: {shown}");
}

#[test]
fn sessions__the_session_id_reaches_the_shell() {
    let sb = sandbox();
    let env_file = sb.scratchpad.path().join("env.sh");
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-9", &sb.root()),
        &sb.root(),
        &[
            ("RATCHET_NOW", T0),
            ("CLAUDE_ENV_FILE", &env_file.to_string_lossy()),
        ],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let text = std::fs::read_to_string(&env_file).unwrap();
    assert!(
        text.contains("export RATCHET_SESSION_ID=s-9"),
        "got: {text}"
    );
}

#[test]
fn sessions__no_marker_no_session() {
    let sb = sandbox();
    let outside = unmanaged_dir();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-10", outside.path()),
        outside.path(),
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(!db_file(&sb).exists());
}

// --- Heartbeat -----------------------------------------------------------------------------

#[test]
fn sessions__a_prompt_updates_the_last_signal() {
    let sb = sandbox();
    start_session(&sb, "s-11", &sb.root(), T0);
    let out = hook_env(
        &sb,
        "prompt",
        &session_payload("s-11", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", &at(5))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(last_seen(&sb, "s-11"), at(5));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'session.prompt'",
            &["s-11"]
        ),
        1
    );
}

#[test]
fn sessions__compaction_and_subagent_end_update_the_last_signal() {
    let sb = sandbox();
    start_session(&sb, "s-12", &sb.root(), T0);
    for (event, when) in [("pre-compact", at(10)), ("subagent-stop", at(15))] {
        let out = hook_env(
            &sb,
            event,
            &session_payload("s-12", &sb.root()),
            &sb.root(),
            &[("RATCHET_NOW", &when)],
        );
        assert_eq!(code(&out), 0, "{event}: {}", stderr(&out));
    }
    assert_eq!(last_seen(&sb, "s-12"), at(15));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind <> 'session.start'",
            &["s-12"]
        ),
        0
    );
}

#[test]
fn sessions__a_hook_of_an_unregistered_session_registers_it() {
    let sb = sandbox();
    // Another session created the database first; this one never saw a session-start.
    start_session(&sb, "s-13a", &sb.root(), T0);
    let out = hook_env(
        &sb,
        "prompt",
        &session_payload("s-13b", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", &at(2))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(
        count(&sb, "SELECT COUNT(*) FROM sessions WHERE id = ?1", &["s-13b"]),
        1
    );
    assert_eq!(last_seen(&sb, "s-13b"), at(2));
}

// --- Derived session state ------------------------------------------------------------------

#[test]
fn sessions__a_session_with_no_signal_for_ninety_minutes_is_orphaned() {
    let sb = sandbox();
    start_session(&sb, "s-14", &sb.root(), T0);
    let out = cli(
        &sb,
        &["session", "list"],
        &sb.root(),
        &[("RATCHET_NOW", &at(90))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("orphaned"), "got: {}", stdout(&out));
}

#[test]
fn sessions__an_ended_session_stays_ended() {
    let sb = sandbox();
    start_session(&sb, "s-15", &sb.root(), T0);
    hook_env(
        &sb,
        "session-end",
        &session_payload("s-15", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", &at(1))],
    );
    let out = cli(
        &sb,
        &["session", "list"],
        &sb.root(),
        &[("RATCHET_NOW", &at(2))],
    );
    assert!(stdout(&out).contains("ended"), "got: {}", stdout(&out));
    assert!(!stdout(&out).contains("live"), "got: {}", stdout(&out));
}

#[test]
fn sessions__the_repo_sets_its_own_thresholds() {
    let sb = sandbox();
    sb.write_marker(
        "[repo]\nworktrees_dir = \".worktrees\"\n[thresholds]\nlive_minutes = 1\nidle_minutes = 2\n",
    );
    start_session(&sb, "s-16", &sb.root(), T0);
    let out = cli(
        &sb,
        &["session", "list"],
        &sb.root(),
        &[("RATCHET_NOW", &at_secs(90))],
    );
    assert!(stdout(&out).contains("idle"), "got: {}", stdout(&out));
}

// --- Session end ----------------------------------------------------------------------------

#[test]
fn sessions__session_end_marks_the_session_ended() {
    let sb = sandbox();
    start_session(&sb, "s-17", &sb.root(), T0);
    let out = hook_env(
        &sb,
        "session-end",
        &session_payload("s-17", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", &at(3))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM sessions WHERE id = ?1 AND ended_at IS NOT NULL",
            &["s-17"]
        ),
        1
    );
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'session.end'",
            &["s-17"]
        ),
        1
    );
}

// --- Work of a dead session goes back ---------------------------------------------------------

#[test]
fn sessions__session_end_returns_its_tasks() {
    let sb = sandbox();
    start_session(&sb, "s-18", &sb.root(), T0);
    seed_task(&sb, "T-0001", "in_progress", Some("s-18"));
    let out = hook_env(
        &sb,
        "session-end",
        &session_payload("s-18", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", &at(1))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(task_row(&sb, "T-0001"), ("ready".to_string(), None));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE task_id = ?1 AND kind = 'task.status'",
            &["T-0001"]
        ),
        1
    );
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE task_id = ?1 AND kind = 'note'",
            &["T-0001"]
        ),
        1
    );
}

#[test]
fn sessions__a_new_session_releases_tasks_of_a_dead_one() {
    let sb = sandbox();
    start_session(&sb, "s-19a", &sb.root(), T0);
    seed_task(&sb, "T-0002", "in_progress", Some("s-19a"));
    let out = start_session(&sb, "s-19b", &sb.root(), &at(90));
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(task_row(&sb, "T-0002"), ("ready".to_string(), None));
}

#[test]
fn sessions__a_task_held_by_a_live_session_is_not_released() {
    let sb = sandbox();
    start_session(&sb, "s-20a", &sb.root(), T0);
    seed_task(&sb, "T-0003", "in_progress", Some("s-20a"));
    let out = start_session(&sb, "s-20b", &sb.root(), &at(1));
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(
        task_row(&sb, "T-0003"),
        ("in_progress".to_string(), Some("s-20a".to_string()))
    );
}

// --- Listing and showing ----------------------------------------------------------------------

#[test]
fn sessions__listing_shows_the_derived_state() {
    let sb = sandbox();
    start_session(&sb, "s-21a", &sb.root(), T0);
    start_session(&sb, "s-21b", &sb.root(), &at(90));
    let all = cli(
        &sb,
        &["session", "list"],
        &sb.root(),
        &[("RATCHET_NOW", &at(90))],
    );
    let text = stdout(&all);
    assert!(text.contains("s-21a"), "got: {text}");
    assert!(text.contains("s-21b"), "got: {text}");
    assert!(text.contains("orphaned"), "got: {text}");
    let live = cli(
        &sb,
        &["session", "list", "--live"],
        &sb.root(),
        &[("RATCHET_NOW", &at(90))],
    );
    let text = stdout(&live);
    assert!(text.contains("s-21b"), "got: {text}");
    assert!(!text.contains("s-21a"), "got: {text}");
}

#[test]
fn sessions__showing_an_unknown_session_fails_with_a_message() {
    let sb = sandbox();
    start_session(&sb, "s-22", &sb.root(), T0);
    let out = cli(&sb, &["session", "show", "nope"], &sb.root(), &[]);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("nope"), "got: {}", stderr(&out));
}

#[test]
fn sessions__showing_without_an_id_resolves_the_session_of_the_directory() {
    let sb = sandbox();
    start_session(&sb, "s-23", &sb.root(), T0);
    let deep = sb.root().join("src").join("deep");
    let out = cli(&sb, &["session", "show"], &deep, &[("RATCHET_NOW", &at(1))]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("s-23"), "got: {}", stdout(&out));
}

// --- Never break a session ----------------------------------------------------------------------

#[test]
fn sessions__an_unusable_database_does_not_break_the_session() {
    let sb = sandbox();
    std::fs::create_dir_all(sb.home.path()).unwrap();
    std::fs::write(db_file(&sb), b"this is not a database").unwrap();
    let out = hook_env(
        &sb,
        "prompt",
        &session_payload("s-24", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(
        sb.log_text().lines().count() >= 1,
        "log: {:?}",
        sb.log_text()
    );
}
```

- [ ] **Step 3: Verify red-clean**

```
cd /c/repos/ratchet && cargo test -p ratchet --test spec 2>&1 | tail -40
cargo test -p ratchet --test scenarios
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: the checker (`scenarios`) is **green** — all 45 scenarios (21 from group 0 plus these 24) now have a test. The `spec` suite compiles and the 24 new tests fail on assertions or on a missing subcommand, never on a compile error; group 0's 21 stay green. Three tests may pass by accident before any implementation (`no_marker_no_session`, `the_guardrail_hook_opens_no_database`, `an_unusable_database_does_not_break_the_session`): say so in the hand-off and confirm each one fails when its guarantee is removed (e.g. temporarily assert the opposite) so the test is known to bite.

- [ ] **Step 4: Hand off (no git)**

List the three files, the count of red tests and the reason each is red. Nothing is committed. The implementer of Tasks 4-11 does not edit these files.

---
### Task 4: Embedded migrations and the schema

Ported from `C:\repos\ops\ops\core\db.py` and `ops/core/migrations/001_init.sql` (plus `003_task_archive.sql`, folded in: ratchet starts from a clean schema, so `archived_at` is a column of `0001`, not a later `ALTER`). Two deliberate departures from the Python original: the migration runner wraps each migration in one `IMMEDIATE` transaction (the ops version documents an unresolved race where two first-time connections half-apply a script), and the ops tables that belong to the desk (`execution_log`, the data-layer and runner tables) are not ported at all.

**Files:**
- Modify: `crates/ratchet/src/db/mod.rs`
- Create: `crates/ratchet/src/db/migrations/0001_init.sql`

**Interfaces:**
- Consumes: `db::open`, `db::open_memory`, `db::db_path`, `db::DbError` (Task 1).
- Produces:
  - `db::LATEST_VERSION: i64` = 1
  - `db::MIGRATIONS: &[(i64, &str, &str)]` — (version, name, SQL), ordered.
  - `db::migrate(conn: &mut Connection) -> Result<Vec<(i64, &'static str)>, DbError>` — applies what is pending, returns what it applied.
  - `db::current_version(conn: &Connection) -> i64` — 0 when the database has no `schema_version` table.
  - `db::connect(home: &Path) -> Result<Connection, DbError>` — open + migrate. Only `session-start` and `ratchet db migrate` call it.
  - `db::open_ready(home: &Path) -> Result<Connection, DbError>` — open + check, creating nothing; `DbError::Stale` when the file is missing or its version is below `LATEST_VERSION`. Every other face calls this.
  - Schema: `tasks`, `checklist_items`, `sessions`, `events`, `task_seq`, `schema_version` with the columns listed in Task 3's Interfaces block.
- Does **not** consume `clock.rs` (Task 5): `schema_version.applied_at` is written with an inline `chrono` call because nothing ever reads it for logic. Keep it that way so this task and Task 5 stay parallel.

- [ ] **Step 1: Write the failing unit tests**

Append to `crates/ratchet/src/db/mod.rs`, inside the existing `#[cfg(test)] mod tests`:
```rust
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
        assert!(matches!(err, DbError::Stale { found: 0, expected: 1 }), "{err:?}");
        assert!(err.to_string().contains("ratchet db migrate"), "{err}");
    }

    #[test]
    fn open_ready_creates_nothing_when_there_is_no_database() {
        let dir = tempfile::TempDir::new().unwrap();
        let err = open_ready(dir.path()).unwrap_err();
        assert!(matches!(err, DbError::Stale { found: 0, expected: 1 }), "{err:?}");
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
```

- [ ] **Step 2: Run them and watch them fail**

```
export PATH="$HOME/.cargo/bin:/c/Users/eillanes/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet db::
```
Expected: compile errors naming `migrate`, `MIGRATIONS`, `LATEST_VERSION`, `current_version`, `connect`, `open_ready`.

- [ ] **Step 3: Write the migration**

`crates/ratchet/src/db/migrations/0001_init.sql`:
```sql
-- 0001: the whole v1 schema. Tasks are mutable; events are append-only.
-- The runner owns the transaction: no BEGIN/COMMIT here.
--
-- `repo` is the display name from the marker (or the directory name); `repo_root` is the
-- absolute, lower-cased main root of the checkout, and it is what every lookup scopes by --
-- ratchet has no central registry of repos, so two checkouts may share a name.

CREATE TABLE IF NOT EXISTS task_seq (
    id INTEGER PRIMARY KEY AUTOINCREMENT
);

CREATE TABLE IF NOT EXISTS tasks (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    body        TEXT NOT NULL DEFAULT '',
    repo        TEXT NOT NULL,
    repo_root   TEXT NOT NULL,
    status      TEXT NOT NULL,
    priority    INTEGER NOT NULL DEFAULT 3,
    parent_id   TEXT REFERENCES tasks(id),
    tags        TEXT NOT NULL DEFAULT '[]',
    -- session id, deliberately without a foreign key: a task can be claimed by a session the
    -- registry has not seen yet; `claim` validates it instead.
    claimed_by  TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    archived_at TEXT
);
CREATE INDEX IF NOT EXISTS ix_tasks_repo_status ON tasks(repo_root, status);
CREATE INDEX IF NOT EXISTS ix_tasks_claimed_by ON tasks(claimed_by);

CREATE TABLE IF NOT EXISTS checklist_items (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id         TEXT NOT NULL REFERENCES tasks(id),
    position        INTEGER NOT NULL,
    text            TEXT NOT NULL,
    done            INTEGER NOT NULL DEFAULT 0,
    done_by_session TEXT,
    done_at         TEXT,
    UNIQUE (task_id, position)
);

CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY,
    repo        TEXT NOT NULL,
    repo_root   TEXT NOT NULL,
    cwd         TEXT NOT NULL,
    worktree    TEXT,
    branch      TEXT,
    mode        TEXT NOT NULL,
    launched_by TEXT NOT NULL,
    started_at  TEXT NOT NULL,
    last_seen   TEXT NOT NULL,
    ended_at    TEXT
);
CREATE INDEX IF NOT EXISTS ix_sessions_repo ON sessions(repo_root);
CREATE INDEX IF NOT EXISTS ix_sessions_last_seen ON sessions(last_seen);

CREATE TABLE IF NOT EXISTS events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    ts         TEXT NOT NULL,
    session_id TEXT,
    task_id    TEXT,
    kind       TEXT NOT NULL,
    payload    TEXT NOT NULL DEFAULT '{}',
    source     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_events_task ON events(task_id, id);
CREATE INDEX IF NOT EXISTS ix_events_session ON events(session_id, id);
```

- [ ] **Step 4: Write the runner**

Add to `crates/ratchet/src/db/mod.rs` (above the test module; `use rusqlite::{Connection, TransactionBehavior};` replaces the existing `use rusqlite::Connection;`):
```rust
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
pub fn connect(home: &Path) -> Result<Connection, DbError> {
    let mut conn = open(&db_path(home))?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// Open the database of `home` for use, without migrating. Refuses an older schema, and refuses
/// a database that is not there: a read-only face must not leave an empty file behind on a
/// machine where nothing has ever run (that file would then be reported stale forever).
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
```

- [ ] **Step 5: Run the tests**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet db:: && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: 9 `db::` unit tests green (3 from Task 1, 6 new).

- [ ] **Step 6: Hand off (no git)**

List the two files. Report the table list and the version. Nothing is committed.

---

### Task 5: `clock.rs` and `model.rs`

**Files:**
- Create: `crates/ratchet/src/clock.rs`, `crates/ratchet/src/model.rs`
- Modify: `crates/ratchet/src/main.rs` (two `mod` lines)

**Interfaces:**
- Produces:
  - `clock::now(env: &HashMap<String, String>) -> DateTime<Utc>` — `RATCHET_NOW` when set and parseable, else the system clock.
  - `clock::iso(ts: DateTime<Utc>) -> String`, `clock::parse(s: &str) -> Option<DateTime<Utc>>`.
  - `model::{SessionMode, LaunchedBy, SessionState, Source, EventKind, TaskStatus}` — each with `as_str(&self) -> &'static str` and `from_db(s: &str) -> Self` (lenient: an unknown string never fails a hook).
  - `model::Session { id, repo, repo_root, cwd, worktree, branch, mode, launched_by, started_at, last_seen, ended_at }` with `Session::from_row(row: &rusqlite::Row) -> rusqlite::Result<Session>`.
  - `model::Event { id, ts, session_id, task_id, kind, payload, source }` with `Event::from_row`.
  - Both structs derive `Serialize` (for `--json` in Task 11).
- Every service takes `now` as a parameter; only the faces (`hooks`, `cli`) call `clock::now`.

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/ratchet/src/clock.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(value: &str) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert("RATCHET_NOW".to_string(), value.to_string());
        env
    }

    #[test]
    fn now_honours_the_seam() {
        let ts = now(&env_with("2026-09-16T12:00:00Z"));
        assert_eq!(iso(ts), "2026-09-16T12:00:00Z");
    }

    #[test]
    fn now_falls_back_to_the_clock_when_the_seam_is_absent_or_junk() {
        assert!(now(&HashMap::new()).timestamp() > 0);
        assert!(now(&env_with("not a date")).timestamp() > 0);
    }

    #[test]
    fn iso_is_utc_seconds_with_a_z() {
        let ts = parse("2026-09-16T09:00:00-03:00").unwrap();
        assert_eq!(iso(ts), "2026-09-16T12:00:00Z");
    }

    #[test]
    fn iso_strings_sort_like_instants() {
        let mut v = vec![iso(parse("2026-09-16T12:00:00Z").unwrap()), iso(parse("2026-01-02T03:04:05Z").unwrap())];
        v.sort();
        assert_eq!(v[0], "2026-01-02T03:04:05Z");
    }
}
```

At the bottom of `crates/ratchet/src/model.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_round_trip_through_their_database_form() {
        assert_eq!(SessionMode::from_db("headless"), SessionMode::Headless);
        assert_eq!(SessionMode::Headless.as_str(), "headless");
        assert_eq!(LaunchedBy::from_db("platform"), LaunchedBy::Platform);
        assert_eq!(TaskStatus::from_db("in_progress"), TaskStatus::InProgress);
        assert_eq!(TaskStatus::InProgress.as_str(), "in_progress");
        assert_eq!(EventKind::SessionStart.as_str(), "session.start");
        assert_eq!(Source::Hook.as_str(), "hook");
        assert_eq!(SessionState::Orphaned.as_str(), "orphaned");
    }

    #[test]
    fn unknown_strings_fall_back_instead_of_failing() {
        assert_eq!(SessionMode::from_db("nonsense"), SessionMode::Interactive);
        assert_eq!(LaunchedBy::from_db(""), LaunchedBy::User);
        assert_eq!(TaskStatus::from_db("weird"), TaskStatus::Backlog);
    }

    #[test]
    fn a_session_serializes_with_its_string_forms() {
        let s = Session {
            id: "s-1".into(),
            repo: "demo".into(),
            repo_root: "c:\\repos\\demo".into(),
            cwd: "c:\\repos\\demo".into(),
            worktree: None,
            branch: Some("main".into()),
            mode: SessionMode::Interactive,
            launched_by: LaunchedBy::User,
            started_at: crate::clock::parse("2026-09-16T12:00:00Z").unwrap(),
            last_seen: crate::clock::parse("2026-09-16T12:00:00Z").unwrap(),
            ended_at: None,
        };
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"mode\":\"interactive\""), "{text}");
        assert!(text.contains("\"launched_by\":\"user\""), "{text}");
        assert!(text.contains("2026-09-16T12:00:00Z"), "{text}");
    }
}
```

- [ ] **Step 2: Run to see them fail**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet clock:: model::
```
Expected: compile errors (no such modules).

- [ ] **Step 3: Implement `clock.rs`**

```rust
//! One clock for the whole binary. Services never read it: they take `now` as a parameter, so a
//! test can place an event ninety minutes in the past without sleeping. The faces read it here,
//! and `RATCHET_NOW` (RFC 3339) replaces it — a documented test seam, harmless in a local tool.

use std::collections::HashMap;

use chrono::{DateTime, SecondsFormat, Utc};

pub fn now(env: &HashMap<String, String>) -> DateTime<Utc> {
    env.get("RATCHET_NOW")
        .and_then(|s| parse(s))
        .unwrap_or_else(Utc::now)
}

/// The single stored/printed form: UTC, seconds precision, `Z`. Sorts lexicographically.
pub fn iso(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn parse(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|d| d.with_timezone(&Utc))
}
```

- [ ] **Step 4: Implement `model.rs`**

```rust
//! Types of the state layer and the exact strings they take in the database. Parsing is lenient
//! on purpose: a row written by a newer build must never make a hook fail (D-p3).

use chrono::{DateTime, Utc};
use rusqlite::Row;
use serde::Serialize;
use serde_json::Value;

use crate::clock;

macro_rules! db_enum {
    ($name:ident, $default:ident, { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        #[serde(into = "String")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self {
                    $($name::$variant => $text),+
                }
            }

            /// Lenient: anything unexpected becomes the default variant.
            pub fn from_db(s: &str) -> Self {
                match s {
                    $($text => $name::$variant,)+
                    _ => $name::$default,
                }
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> String {
                v.as_str().to_string()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

db_enum!(SessionMode, Interactive, { Interactive => "interactive", Headless => "headless" });
db_enum!(LaunchedBy, User, { User => "user", Platform => "platform" });
db_enum!(SessionState, Live, {
    Live => "live", Idle => "idle", Orphaned => "orphaned", Ended => "ended",
});
db_enum!(Source, Cli, { Hook => "hook", Cli => "cli" });
db_enum!(TaskStatus, Backlog, {
    Backlog => "backlog",
    Ready => "ready",
    InProgress => "in_progress",
    Blocked => "blocked",
    Review => "review",
    Done => "done",
});
db_enum!(EventKind, Note, {
    SessionStart => "session.start",
    SessionPrompt => "session.prompt",
    SessionStop => "session.stop",
    SessionEnd => "session.end",
    TaskStatus => "task.status",
    Note => "note",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Session {
    pub id: String,
    pub repo: String,
    pub repo_root: String,
    pub cwd: String,
    pub worktree: Option<String>,
    pub branch: Option<String>,
    pub mode: SessionMode,
    pub launched_by: LaunchedBy,
    pub started_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

impl Session {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Session> {
        let started: String = row.get("started_at")?;
        let seen: String = row.get("last_seen")?;
        let ended: Option<String> = row.get("ended_at")?;
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch");
        Ok(Session {
            id: row.get("id")?,
            repo: row.get("repo")?,
            repo_root: row.get("repo_root")?,
            cwd: row.get("cwd")?,
            worktree: row.get("worktree")?,
            branch: row.get("branch")?,
            mode: SessionMode::from_db(&row.get::<_, String>("mode")?),
            launched_by: LaunchedBy::from_db(&row.get::<_, String>("launched_by")?),
            started_at: clock::parse(&started).unwrap_or(epoch),
            last_seen: clock::parse(&seen).unwrap_or(epoch),
            ended_at: ended.as_deref().and_then(clock::parse),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    pub id: i64,
    pub ts: DateTime<Utc>,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub kind: String,
    pub payload: Value,
    pub source: Source,
}

impl Event {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Event> {
        let ts: String = row.get("ts")?;
        let payload: String = row.get("payload")?;
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch");
        Ok(Event {
            id: row.get("id")?,
            ts: clock::parse(&ts).unwrap_or(epoch),
            session_id: row.get("session_id")?,
            task_id: row.get("task_id")?,
            kind: row.get("kind")?,
            payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
            source: Source::from_db(&row.get::<_, String>("source")?),
        })
    }
}
```

Add `mod clock;` and `mod model;` to the `mod` block of `crates/ratchet/src/main.rs`, in alphabetical order. Items not yet consumed get `#[allow(dead_code)]` with a comment naming the task that consumes them (group 0's convention), or the macro's generated items get one `#[allow(dead_code)]` at the `macro_rules!` call site.

- [ ] **Step 5: Run the tests**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet clock:: model:: && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: 7 new unit tests green, clippy clean.

- [ ] **Step 6: Hand off (no git)**

List the three files (two new, one `mod` block edit).

---

### Task 6: `services/` and the append-only event log

Ported from `C:\repos\ops\ops\core\services\events.py`. The rule it enforces is the one from `CLAUDE.md` of ops and §3 of the spec: `services/` is the only writer, and `events` is insert-only.

**Files:**
- Create: `crates/ratchet/src/services/mod.rs`, `crates/ratchet/src/services/events.rs`
- Modify: `crates/ratchet/src/main.rs` (one `mod` line)

**Interfaces:**
- Consumes: `model::{Event, EventKind, Source}`, `clock::iso`, `db::*` (Tasks 4, 5).
- Produces:
  - `services::ServiceError` — `Db(rusqlite::Error)`, `NotFound(String)`, `Invalid(String)`; `Display`; `From<rusqlite::Error>`.
  - `services::events::emit(conn: &Connection, kind: EventKind, payload: &Value, source: Source, session_id: Option<&str>, task_id: Option<&str>, now: DateTime<Utc>) -> Result<Event, ServiceError>` — works on a `Transaction` too, which derefs to `Connection`.
  - `services::events::for_session(conn: &Connection, session_id: &str, limit: i64) -> Result<Vec<Event>, ServiceError>`
  - `services::events::for_task(conn: &Connection, task_id: &str, limit: i64) -> Result<Vec<Event>, ServiceError>`

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/ratchet/src/services/events.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use serde_json::json;

    fn conn() -> rusqlite::Connection {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        c
    }

    fn at(s: &str) -> DateTime<Utc> {
        crate::clock::parse(s).unwrap()
    }

    #[test]
    fn emit_writes_one_row_and_returns_it() {
        let c = conn();
        let ev = emit(
            &c,
            EventKind::SessionStart,
            &json!({"repo": "demo"}),
            Source::Hook,
            Some("s-1"),
            None,
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        assert_eq!(ev.kind, "session.start");
        assert_eq!(ev.session_id.as_deref(), Some("s-1"));
        assert_eq!(ev.payload["repo"], "demo");
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let stored: String = c
            .query_row("SELECT ts FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, "2026-09-16T12:00:00Z");
    }

    #[test]
    fn events_come_back_newest_last_for_a_session_and_a_task() {
        let c = conn();
        for (i, kind) in [EventKind::SessionStart, EventKind::SessionPrompt].iter().enumerate() {
            emit(
                &c,
                *kind,
                &json!({}),
                Source::Hook,
                Some("s-2"),
                None,
                at(&format!("2026-09-16T12:0{i}:00Z")),
            )
            .unwrap();
        }
        emit(
            &c,
            EventKind::Note,
            &json!({"text": "hi"}),
            Source::Cli,
            None,
            Some("T-0001"),
            at("2026-09-16T12:05:00Z"),
        )
        .unwrap();
        let mine = for_session(&c, "s-2", 10).unwrap();
        assert_eq!(mine.len(), 2);
        assert_eq!(mine[1].kind, "session.prompt");
        let task = for_task(&c, "T-0001", 10).unwrap();
        assert_eq!(task.len(), 1);
        assert_eq!(task[0].payload["text"], "hi");
        assert_eq!(task[0].source, Source::Cli);
    }

    #[test]
    fn emit_inside_a_transaction_rolls_back_with_it() {
        let mut c = conn();
        {
            let tx = c
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            emit(&tx, EventKind::Note, &json!({}), Source::Cli, None, None, at("2026-09-16T12:00:00Z")).unwrap();
            // dropped without commit
        }
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
}
```

- [ ] **Step 2: Run to see them fail**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet services::
```
Expected: compile errors (no such module).

- [ ] **Step 3: Implement `services/mod.rs`**

```rust
//! The only module that writes to the database. Every mutation appends one event; `events` is
//! never updated or deleted. Faces (`hooks`, `cli`) call in here and translate errors.

pub mod events;

use std::fmt;

#[derive(Debug)]
pub enum ServiceError {
    Db(rusqlite::Error),
    NotFound(String),
    Invalid(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceError::Db(e) => write!(f, "database: {e}"),
            ServiceError::NotFound(m) => write!(f, "{m}"),
            ServiceError::Invalid(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<rusqlite::Error> for ServiceError {
    fn from(e: rusqlite::Error) -> Self {
        ServiceError::Db(e)
    }
}
```

- [ ] **Step 4: Implement `services/events.rs`**

```rust
//! Append-only log. INSERT only: no UPDATE, no DELETE, in this module or any other.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde_json::Value;

use super::ServiceError;
use crate::clock;
use crate::model::{Event, EventKind, Source};

pub fn emit(
    conn: &Connection,
    kind: EventKind,
    payload: &Value,
    source: Source,
    session_id: Option<&str>,
    task_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Event, ServiceError> {
    conn.execute(
        "INSERT INTO events(ts, session_id, task_id, kind, payload, source) VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            clock::iso(now),
            session_id,
            task_id,
            kind.as_str(),
            payload.to_string(),
            source.as_str()
        ],
    )?;
    Ok(Event {
        id: conn.last_insert_rowid(),
        ts: now,
        session_id: session_id.map(str::to_string),
        task_id: task_id.map(str::to_string),
        kind: kind.as_str().to_string(),
        payload: payload.clone(),
        source,
    })
}

pub fn for_session(conn: &Connection, session_id: &str, limit: i64) -> Result<Vec<Event>, ServiceError> {
    collect(
        conn,
        "SELECT * FROM events WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2",
        session_id,
        limit,
    )
}

pub fn for_task(conn: &Connection, task_id: &str, limit: i64) -> Result<Vec<Event>, ServiceError> {
    collect(
        conn,
        "SELECT * FROM events WHERE task_id = ?1 ORDER BY id DESC LIMIT ?2",
        task_id,
        limit,
    )
}

/// Reads the newest `limit` rows and returns them oldest first, so callers read a timeline.
fn collect(conn: &Connection, sql: &str, key: &str, limit: i64) -> Result<Vec<Event>, ServiceError> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![key, limit], |r| Event::from_row(r))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    out.reverse();
    Ok(out)
}
```

Add `mod services;` to the `mod` block of `crates/ratchet/src/main.rs`.

- [ ] **Step 5: Run the tests**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet services:: && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: 3 new unit tests green.

- [ ] **Step 6: Hand off (no git)**

List the three files. State the layer rule in one line for the reviewer: no SQL write outside `services/`.

---
### Task 7: The sessions service

Ported from `C:\repos\ops\ops\core\services\sessions.py` (`upsert_start`, `touch`, `end`, `state`) and `ops\cli\session_resolver.py` (`resolve`). Behaviour, not idioms: `repo_for_path` against a central registry becomes `repo_root` from the marker, and `_unknown` disappears.

**Files:**
- Create: `crates/ratchet/src/services/sessions.rs`
- Modify: `crates/ratchet/src/services/mod.rs` (one `pub mod` line)

**Interfaces:**
- Consumes: `services::ServiceError`, `services::events::emit` (Task 6), `model::*`, `clock::*` (Task 5), `config::Thresholds` and `repo::normalize` (group 0).
- Produces:
  - `sessions::StartInput<'a> { session_id, repo, repo_root, cwd, worktree: Option<&'a str>, branch: Option<&'a str>, mode, launched_by }` — all `&'a str` except the two enums.
  - `sessions::get(conn: &Connection, id: &str) -> Result<Option<Session>, ServiceError>`
  - `sessions::list(conn: &Connection, repo: Option<&str>) -> Result<Vec<Session>, ServiceError>` — newest signal first; `repo` filters on the display name.
  - `sessions::upsert_start(conn: &mut Connection, input: StartInput<'_>, now) -> Result<Session, ServiceError>` — insert or refresh, one `session.start` event either way.
  - `sessions::touch(conn: &mut Connection, id: &str, kind: Option<EventKind>, now) -> Result<Session, ServiceError>` — `NotFound` when the session is unknown.
  - `sessions::end(conn: &mut Connection, id: &str, now) -> Result<Session, ServiceError>` — one `session.end` event.
  - `sessions::state(s: &Session, th: &Thresholds, now) -> SessionState`
  - `sessions::resolve(conn: &Connection, explicit: Option<&str>, env: &HashMap<String,String>, cwd: &Path, th: &Thresholds, now) -> Result<Option<String>, ServiceError>`

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/ratchet/src/services/sessions.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn conn() -> Connection {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        c
    }

    fn at(s: &str) -> DateTime<Utc> {
        clock::parse(s).unwrap()
    }

    fn input<'a>(id: &'a str, cwd: &'a str) -> StartInput<'a> {
        StartInput {
            session_id: id,
            repo: "demo",
            repo_root: "c:\\repos\\demo",
            cwd,
            worktree: None,
            branch: Some("main"),
            mode: SessionMode::Interactive,
            launched_by: LaunchedBy::User,
        }
    }

    #[test]
    fn start_twice_keeps_one_session_and_refreshes_it() {
        let mut c = conn();
        upsert_start(&mut c, input("s-1", "c:\\repos\\demo"), at("2026-09-16T12:00:00Z")).unwrap();
        let s = upsert_start(
            &mut c,
            input("s-1", "c:\\repos\\demo\\src"),
            at("2026-09-16T12:05:00Z"),
        )
        .unwrap();
        assert_eq!(s.cwd, "c:\\repos\\demo\\src");
        assert_eq!(clock::iso(s.last_seen), "2026-09-16T12:05:00Z");
        assert_eq!(clock::iso(s.started_at), "2026-09-16T12:00:00Z");
        assert_eq!(list(&c, None).unwrap().len(), 1);
        assert_eq!(crate::services::events::for_session(&c, "s-1", 10).unwrap().len(), 2);
    }

    #[test]
    fn touch_moves_the_signal_and_optionally_records_an_event() {
        let mut c = conn();
        upsert_start(&mut c, input("s-2", "x"), at("2026-09-16T12:00:00Z")).unwrap();
        touch(&mut c, "s-2", Some(EventKind::SessionPrompt), at("2026-09-16T12:03:00Z")).unwrap();
        touch(&mut c, "s-2", None, at("2026-09-16T12:04:00Z")).unwrap();
        let s = get(&c, "s-2").unwrap().unwrap();
        assert_eq!(clock::iso(s.last_seen), "2026-09-16T12:04:00Z");
        let kinds: Vec<String> = crate::services::events::for_session(&c, "s-2", 10)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(kinds, vec!["session.start", "session.prompt"]);
    }

    #[test]
    fn touching_an_unknown_session_is_not_found() {
        let mut c = conn();
        let err = touch(&mut c, "ghost", None, at("2026-09-16T12:00:00Z")).unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)), "{err:?}");
    }

    #[test]
    fn end_records_the_end_and_the_event() {
        let mut c = conn();
        upsert_start(&mut c, input("s-3", "x"), at("2026-09-16T12:00:00Z")).unwrap();
        let s = end(&mut c, "s-3", at("2026-09-16T12:10:00Z")).unwrap();
        assert_eq!(s.ended_at.map(clock::iso).as_deref(), Some("2026-09-16T12:10:00Z"));
        assert_eq!(
            state(&s, &Thresholds::default(), at("2026-09-16T12:11:00Z")),
            SessionState::Ended
        );
    }

    #[test]
    fn state_is_derived_from_the_thresholds() {
        let mut c = conn();
        let s = upsert_start(&mut c, input("s-4", "x"), at("2026-09-16T12:00:00Z")).unwrap();
        let th = Thresholds::default(); // 10 / 60
        assert_eq!(state(&s, &th, at("2026-09-16T12:05:00Z")), SessionState::Live);
        assert_eq!(state(&s, &th, at("2026-09-16T12:30:00Z")), SessionState::Idle);
        assert_eq!(state(&s, &th, at("2026-09-16T13:30:00Z")), SessionState::Orphaned);
        let tight = Thresholds { live_minutes: 1, idle_minutes: 2 };
        assert_eq!(state(&s, &tight, at("2026-09-16T12:01:30Z")), SessionState::Idle);
    }

    #[test]
    fn resolve_prefers_explicit_then_environment_then_the_directory() {
        let mut c = conn();
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_string_lossy().to_string();
        let deep = dir.path().join("src").join("deep");
        std::fs::create_dir_all(&deep).unwrap();
        let mut covering = input("s-5", &root);
        covering.repo_root = &root;
        upsert_start(&mut c, covering, at("2026-09-16T12:00:00Z")).unwrap();
        let th = Thresholds::default();
        let now = at("2026-09-16T12:01:00Z");
        let mut env = HashMap::new();

        assert_eq!(
            resolve(&c, Some("explicit"), &env, &deep, &th, now).unwrap().as_deref(),
            Some("explicit")
        );
        env.insert("RATCHET_SESSION_ID".to_string(), "from-env".to_string());
        assert_eq!(
            resolve(&c, None, &env, &deep, &th, now).unwrap().as_deref(),
            Some("from-env")
        );
        env.clear();
        assert_eq!(
            resolve(&c, None, &env, &deep, &th, now).unwrap().as_deref(),
            Some("s-5")
        );
        // Dead sessions do not answer for a directory.
        let late = at("2026-09-16T13:30:00Z");
        assert_eq!(resolve(&c, None, &env, &deep, &th, late).unwrap(), None);
    }

    #[test]
    fn resolve_picks_the_most_specific_cover() {
        let mut c = conn();
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_string_lossy().to_string();
        let wt = dir.path().join(".worktrees").join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let wt_str = wt.to_string_lossy().to_string();
        let mut outer = input("s-outer", &root);
        outer.repo_root = &root;
        upsert_start(&mut c, outer, at("2026-09-16T12:00:00Z")).unwrap();
        let mut inner = input("s-inner", &wt_str);
        inner.repo_root = &root;
        upsert_start(&mut c, inner, at("2026-09-16T11:59:00Z")).unwrap();
        let th = Thresholds::default();
        assert_eq!(
            resolve(&c, None, &HashMap::new(), &wt, &th, at("2026-09-16T12:01:00Z"))
                .unwrap()
                .as_deref(),
            Some("s-inner"),
            "the deeper cover wins even though it signalled earlier"
        );
    }
}
```

- [ ] **Step 2: Run to see them fail**

```
export PATH="$HOME/.cargo/bin:/c/Users/eillanes/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet services::sessions
```
Expected: compile errors naming the missing functions.

- [ ] **Step 3: Implement**

`crates/ratchet/src/services/sessions.rs`:
```rust
//! Registry of agent sessions: who is alive, since when, in which checkout. Derived state is
//! computed from `last_seen` and the repo thresholds, never stored.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::json;

use super::{events, ServiceError};
use crate::clock;
use crate::config::Thresholds;
use crate::model::{EventKind, LaunchedBy, Session, SessionMode, SessionState, Source};
use crate::repo;

pub struct StartInput<'a> {
    pub session_id: &'a str,
    /// Display name of the repo (marker `name`, else the directory name).
    pub repo: &'a str,
    /// Absolute, lower-cased main root: the key everything scopes by.
    pub repo_root: &'a str,
    pub cwd: &'a str,
    pub worktree: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub mode: SessionMode,
    pub launched_by: LaunchedBy,
}

pub fn get(conn: &Connection, session_id: &str) -> Result<Option<Session>, ServiceError> {
    let mut stmt = conn.prepare("SELECT * FROM sessions WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![session_id], |r| Session::from_row(r))?;
    match rows.next() {
        None => Ok(None),
        Some(row) => Ok(Some(row?)),
    }
}

pub fn list(conn: &Connection, repo_name: Option<&str>) -> Result<Vec<Session>, ServiceError> {
    let mut out = Vec::new();
    match repo_name {
        None => {
            let mut stmt = conn.prepare("SELECT * FROM sessions ORDER BY last_seen DESC")?;
            for row in stmt.query_map([], |r| Session::from_row(r))? {
                out.push(row?);
            }
        }
        Some(name) => {
            let mut stmt =
                conn.prepare("SELECT * FROM sessions WHERE repo = ?1 ORDER BY last_seen DESC")?;
            for row in stmt.query_map(params![name], |r| Session::from_row(r))? {
                out.push(row?);
            }
        }
    }
    Ok(out)
}

/// Registers the session, or refreshes the one already registered with that identifier. Both
/// paths append one `session.start` event, with `resumed` saying which happened.
pub fn upsert_start(
    conn: &mut Connection,
    input: StartInput<'_>,
    now: DateTime<Utc>,
) -> Result<Session, ServiceError> {
    let ts = clock::iso(now);
    let resumed = get(conn, input.session_id)?.is_some();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if resumed {
        tx.execute(
            "UPDATE sessions SET last_seen = ?1, cwd = ?2, worktree = COALESCE(?3, worktree), \
             branch = COALESCE(?4, branch), ended_at = NULL WHERE id = ?5",
            params![ts, input.cwd, input.worktree, input.branch, input.session_id],
        )?;
    } else {
        tx.execute(
            "INSERT INTO sessions(id,repo,repo_root,cwd,worktree,branch,mode,launched_by,\
             started_at,last_seen,ended_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9,NULL)",
            params![
                input.session_id,
                input.repo,
                input.repo_root,
                input.cwd,
                input.worktree,
                input.branch,
                input.mode.as_str(),
                input.launched_by.as_str(),
                ts
            ],
        )?;
    }
    events::emit(
        &tx,
        EventKind::SessionStart,
        &json!({
            "repo": input.repo,
            "cwd": input.cwd,
            "branch": input.branch,
            "resumed": resumed,
        }),
        Source::Hook,
        Some(input.session_id),
        None,
        now,
    )?;
    tx.commit()?;
    load(conn, input.session_id)
}

pub fn touch(
    conn: &mut Connection,
    session_id: &str,
    kind: Option<EventKind>,
    now: DateTime<Utc>,
) -> Result<Session, ServiceError> {
    let ts = clock::iso(now);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        "UPDATE sessions SET last_seen = ?1 WHERE id = ?2",
        params![ts, session_id],
    )?;
    if changed == 0 {
        return Err(ServiceError::NotFound(format!(
            "session {session_id} does not exist"
        )));
    }
    if let Some(k) = kind {
        events::emit(&tx, k, &json!({}), Source::Hook, Some(session_id), None, now)?;
    }
    tx.commit()?;
    load(conn, session_id)
}

pub fn end(
    conn: &mut Connection,
    session_id: &str,
    now: DateTime<Utc>,
) -> Result<Session, ServiceError> {
    let ts = clock::iso(now);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        "UPDATE sessions SET ended_at = ?1, last_seen = ?1 WHERE id = ?2",
        params![ts, session_id],
    )?;
    if changed == 0 {
        return Err(ServiceError::NotFound(format!(
            "session {session_id} does not exist"
        )));
    }
    events::emit(
        &tx,
        EventKind::SessionEnd,
        &json!({}),
        Source::Hook,
        Some(session_id),
        None,
        now,
    )?;
    tx.commit()?;
    load(conn, session_id)
}

/// Derived, never stored.
pub fn state(session: &Session, th: &Thresholds, now: DateTime<Utc>) -> SessionState {
    if session.ended_at.is_some() {
        return SessionState::Ended;
    }
    let age = now.signed_duration_since(session.last_seen);
    if age < Duration::minutes(th.live_minutes as i64) {
        SessionState::Live
    } else if age < Duration::minutes(th.idle_minutes as i64) {
        SessionState::Idle
    } else {
        SessionState::Orphaned
    }
}

/// `explicit` → `RATCHET_SESSION_ID` → the live session whose directory or worktree covers `cwd`,
/// most specific first (a worktree hangs off the main tree, so "most recent" would choose wrong).
pub fn resolve(
    conn: &Connection,
    explicit: Option<&str>,
    env: &HashMap<String, String>,
    cwd: &Path,
    th: &Thresholds,
    now: DateTime<Utc>,
) -> Result<Option<String>, ServiceError> {
    if let Some(id) = explicit.filter(|s| !s.is_empty()) {
        return Ok(Some(id.to_string()));
    }
    if let Some(id) = env.get("RATCHET_SESSION_ID").filter(|s| !s.is_empty()) {
        return Ok(Some(id.clone()));
    }
    let here = repo::normalize(cwd);
    let mut best: Option<(usize, String, String)> = None;
    for s in list(conn, None)? {
        if state(&s, th, now) != SessionState::Live {
            continue;
        }
        let depth = [Some(s.cwd.as_str()), s.worktree.as_deref()]
            .into_iter()
            .flatten()
            .filter_map(|p| cover_depth(Path::new(p), &here))
            .max();
        let Some(depth) = depth else { continue };
        let candidate = (depth, clock::iso(s.last_seen), s.id.clone());
        match &best {
            Some(current) if *current >= candidate => {}
            _ => best = Some(candidate),
        }
    }
    Ok(best.map(|b| b.2))
}

/// How many components of `root` cover `target`; `None` when it does not cover it.
fn cover_depth(root: &Path, target: &Path) -> Option<usize> {
    let r = repo::normalize(root);
    if *target == r || target.starts_with(&r) {
        Some(r.components().count())
    } else {
        None
    }
}

fn load(conn: &Connection, session_id: &str) -> Result<Session, ServiceError> {
    get(conn, session_id)?.ok_or_else(|| {
        ServiceError::NotFound(format!("session {session_id} disappeared while writing it"))
    })
}
```

Add `pub mod sessions;` to `crates/ratchet/src/services/mod.rs`, after `pub mod events;`.

- [ ] **Step 4: Run the tests**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet services:: && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: 7 new unit tests green (10 in `services::` in total).

- [ ] **Step 5: Hand off (no git)**

List the two files. Note for group 2: `resolve` is the function the future `--session` global option and every board write hang off.

---

### Task 8: Releasing the work of dead sessions

Ported from `orphan_tasks`, `release_task` and `release_dead_tasks` in `C:\repos\ops\ops\core\services\sessions.py`. This is the **only** slice of the task board group 1 implements: enough to give a dead session's work back, no more.

**Files:**
- Create: `crates/ratchet/src/services/tasks.rs`
- Modify: `crates/ratchet/src/services/mod.rs` (one `pub mod` line)

**Interfaces:**
- Consumes: `sessions::{get, state}`, `events::emit`, `model::{TaskStatus, EventKind, Source}`, `config::Thresholds`.
- Produces:
  - `tasks::Orphan { task_id: String, title: String, claimed_by: String }`
  - `tasks::orphaned(conn: &Connection, repo_root: &str, th: &Thresholds, now) -> Result<Vec<Orphan>, ServiceError>` — pure: it lists, it does not release.
  - `tasks::release(conn: &mut Connection, task_id: &str, by: &str, source: Source, now) -> Result<(), ServiceError>` — `Invalid` when the task holds no session.
  - `tasks::release_dead(conn: &mut Connection, repo_root: &str, th: &Thresholds, now) -> Result<Vec<String>, ServiceError>` — the two triggers (session end, next session start) share this one mechanism.
  - `tasks::claimed_ids(conn: &Connection, session_id: &str) -> Result<Vec<String>, ServiceError>` — for `session list|show`.
- **Contract for group 2:** the event kinds and payloads written here are the contract, not the SQL. Group 2's `tasks::transition` will subsume `release`'s internals; it must keep emitting one `task.status` with `{"to": "ready", "why": …}` and one `note` with `{"text": …}`, so the scenario tests of this group keep passing unchanged.

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/ratchet/src/services/tasks.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::model::{LaunchedBy, SessionMode};
    use crate::services::sessions::{self, StartInput};

    fn at(s: &str) -> DateTime<Utc> {
        clock::parse(s).unwrap()
    }

    fn setup(session_at: &str) -> Connection {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        sessions::upsert_start(
            &mut c,
            StartInput {
                session_id: "s-holder",
                repo: "demo",
                repo_root: "root",
                cwd: "root",
                worktree: None,
                branch: None,
                mode: SessionMode::Interactive,
                launched_by: LaunchedBy::User,
            },
            at(session_at),
        )
        .unwrap();
        c.execute(
            "INSERT INTO tasks(id,title,body,repo,repo_root,status,priority,tags,claimed_by,created_at,updated_at) \
             VALUES ('T-0001','build it','','demo','root','in_progress',3,'[]','s-holder',?1,?1)",
            rusqlite::params![session_at],
        )
        .unwrap();
        c
    }

    #[test]
    fn a_task_of_a_live_session_is_not_orphaned() {
        let c = setup("2026-09-16T12:00:00Z");
        let found = orphaned(&c, "root", &Thresholds::default(), at("2026-09-16T12:05:00Z")).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn a_task_of_an_orphaned_session_is_listed_but_not_released_by_listing() {
        let c = setup("2026-09-16T12:00:00Z");
        let found = orphaned(&c, "root", &Thresholds::default(), at("2026-09-16T13:30:00Z")).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].task_id, "T-0001");
        assert_eq!(found[0].claimed_by, "s-holder");
        let status: String = c
            .query_row("SELECT status FROM tasks WHERE id = 'T-0001'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "in_progress", "listing must be pure");
    }

    #[test]
    fn release_returns_the_task_and_writes_two_events() {
        let mut c = setup("2026-09-16T12:00:00Z");
        release(&mut c, "T-0001", "end of session", Source::Hook, at("2026-09-16T13:30:00Z")).unwrap();
        let (status, holder): (String, Option<String>) = c
            .query_row("SELECT status, claimed_by FROM tasks WHERE id = 'T-0001'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(status, "ready");
        assert_eq!(holder, None);
        let kinds: Vec<String> = crate::services::events::for_task(&c, "T-0001", 10)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(kinds, vec!["task.status", "note"]);
    }

    #[test]
    fn releasing_an_unclaimed_task_is_rejected() {
        let mut c = setup("2026-09-16T12:00:00Z");
        c.execute("UPDATE tasks SET claimed_by = NULL WHERE id = 'T-0001'", [])
            .unwrap();
        let err = release(&mut c, "T-0001", "x", Source::Cli, at("2026-09-16T12:01:00Z")).unwrap_err();
        assert!(matches!(err, ServiceError::Invalid(_)), "{err:?}");
    }

    #[test]
    fn releasing_a_task_that_does_not_exist_is_not_found() {
        let mut c = setup("2026-09-16T12:00:00Z");
        let err = release(&mut c, "T-9999", "x", Source::Cli, at("2026-09-16T12:01:00Z")).unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)), "{err:?}");
    }

    #[test]
    fn release_dead_sweeps_only_the_dead() {
        let mut c = setup("2026-09-16T12:00:00Z");
        let none = release_dead(&mut c, "root", &Thresholds::default(), at("2026-09-16T12:05:00Z")).unwrap();
        assert!(none.is_empty());
        let swept = release_dead(&mut c, "root", &Thresholds::default(), at("2026-09-16T13:30:00Z")).unwrap();
        assert_eq!(swept, vec!["T-0001".to_string()]);
    }

    #[test]
    fn claimed_ids_lists_the_work_in_progress_of_a_session() {
        let c = setup("2026-09-16T12:00:00Z");
        assert_eq!(claimed_ids(&c, "s-holder").unwrap(), vec!["T-0001".to_string()]);
        assert!(claimed_ids(&c, "nobody").unwrap().is_empty());
    }
}
```

- [ ] **Step 2: Run to see them fail**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet services::tasks
```
Expected: compile errors naming `orphaned`, `release`, `release_dead`, `claimed_ids`.

- [ ] **Step 3: Implement**

`crates/ratchet/src/services/tasks.rs`:
```rust
//! Group 1 owns exactly one thing about tasks: giving back the work of a session that died.
//! The board (create, claim, checklist, notes, handoffs) arrives with group 2 and extends this
//! module; the events written here are the contract that must not change.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::json;

use super::{events, sessions, ServiceError};
use crate::clock;
use crate::config::Thresholds;
use crate::model::{EventKind, SessionState, Source, TaskStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    pub task_id: String,
    pub title: String,
    pub claimed_by: String,
}

/// Tasks in progress whose holder is ended, orphaned, or gone from the registry. Pure: an orphan
/// stays listed until something releases it, which is what makes the briefing of group 2 able to
/// show it once with its last handoff before it goes back to `ready`.
pub fn orphaned(
    conn: &Connection,
    repo_root: &str,
    th: &Thresholds,
    now: DateTime<Utc>,
) -> Result<Vec<Orphan>, ServiceError> {
    let mut stmt = conn.prepare(
        "SELECT id, title, claimed_by FROM tasks \
         WHERE repo_root = ?1 AND status = ?2 AND claimed_by IS NOT NULL AND archived_at IS NULL \
         ORDER BY id",
    )?;
    let rows = stmt.query_map(params![repo_root, TaskStatus::InProgress.as_str()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (task_id, title, claimed_by) = row?;
        let dead = match sessions::get(conn, &claimed_by)? {
            None => true,
            Some(holder) => matches!(
                sessions::state(&holder, th, now),
                SessionState::Orphaned | SessionState::Ended
            ),
        };
        if dead {
            out.push(Orphan {
                task_id,
                title,
                claimed_by,
            });
        }
    }
    Ok(out)
}

/// Back to `ready`, without a session, with the change of state and a note saying who released
/// it. History is untouched: the last handoff stays where it was.
pub fn release(
    conn: &mut Connection,
    task_id: &str,
    by: &str,
    source: Source,
    now: DateTime<Utc>,
) -> Result<(), ServiceError> {
    let holder: Option<String> = conn
        .query_row(
            "SELECT claimed_by FROM tasks WHERE id = ?1",
            params![task_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .ok_or_else(|| ServiceError::NotFound(format!("task {task_id} does not exist")))?;
    let holder = holder.ok_or_else(|| {
        ServiceError::Invalid(format!("{task_id} holds no session; there is nothing to release"))
    })?;
    let why = format!("released by {by}");
    let ts = clock::iso(now);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE tasks SET claimed_by = NULL, status = ?1, updated_at = ?2 WHERE id = ?3",
        params![TaskStatus::Ready.as_str(), ts, task_id],
    )?;
    events::emit(
        &tx,
        EventKind::TaskStatus,
        &json!({"to": TaskStatus::Ready.as_str(), "why": why}),
        source,
        None,
        Some(task_id),
        now,
    )?;
    events::emit(
        &tx,
        EventKind::Note,
        &json!({"text": format!("{why}; it was held by session {holder}")}),
        source,
        None,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    Ok(())
}

/// One mechanism, two triggers: the session-end hook calls it for a tidy close, and the
/// session-start hook calls it as a lazy sweep for sessions that died without a hook.
pub fn release_dead(
    conn: &mut Connection,
    repo_root: &str,
    th: &Thresholds,
    now: DateTime<Utc>,
) -> Result<Vec<String>, ServiceError> {
    let dead = orphaned(conn, repo_root, th, now)?;
    let mut released = Vec::new();
    for orphan in dead {
        release(conn, &orphan.task_id, "end of session", Source::Hook, now)?;
        released.push(orphan.task_id);
    }
    Ok(released)
}

pub fn claimed_ids(conn: &Connection, session_id: &str) -> Result<Vec<String>, ServiceError> {
    let mut stmt = conn.prepare(
        "SELECT id FROM tasks WHERE claimed_by = ?1 AND status = ?2 AND archived_at IS NULL ORDER BY id",
    )?;
    let rows = stmt.query_map(params![session_id, TaskStatus::InProgress.as_str()], |r| {
        r.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}
```

Add `pub mod tasks;` to `crates/ratchet/src/services/mod.rs`.

- [ ] **Step 4: Run the tests**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet services:: && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: 7 new unit tests green (17 in `services::` in total).

- [ ] **Step 5: Hand off (no git)**

List the two files and repeat the contract line for group 2 (`task.status` + `note`, same payload keys).

---

### Task 9: `ratchet db migrate | path`

**Files:**
- Create: `crates/ratchet/src/cli/mod.rs`, `crates/ratchet/src/cli/db_cmd.rs`
- Modify: `crates/ratchet/src/main.rs`

**Interfaces:**
- Consumes: `db::{connect, db_path, open_ready, current_version, LATEST_VERSION}`, `config::ratchet_home`.
- Produces:
  - `cli::db_cmd::migrate(env: &HashMap<String, String>) -> i32`
  - `cli::db_cmd::path(env: &HashMap<String, String>) -> i32`
  - `cli::db_cmd::selftest() -> i32` (moved out of `main.rs`)
  - The convention every later face follows: a CLI function returns the process exit code, prints results on stdout and `error: …` on stderr, and never panics.
- Note for Task 11: `src/cli/session_cmd.rs` is added to `cli/mod.rs` there; this task creates the module with only `db_cmd` in it.

- [ ] **Step 1: The failing tests are the scenario tests**

Two scenarios of Task 3 cover this task: `sessions__migrations_apply_once` and `sessions__the_database_path_is_printable`. (`sessions__a_stale_schema_is_reported_not_migrated` drives `session list`, not `db`, so it stays red until Task 11 — do not expect it here.)
```
cd /c/repos/ratchet && cargo test -p ratchet --test spec sessions__migrations sessions__the_database_path
```
Expected: FAIL — `db migrate` and `db path` do not exist.

- [ ] **Step 2: Implement**

`crates/ratchet/src/cli/mod.rs`:
```rust
//! Command-line faces. Thin: parse, call a service, print. No SQL here.

pub mod db_cmd;
```

`crates/ratchet/src/cli/db_cmd.rs`:
```rust
//! `ratchet db migrate | path | selftest`.

use std::collections::HashMap;

use crate::config::ratchet_home;
use crate::db;

/// Applies pending migrations. One of the only two places allowed to migrate (spec §6).
pub fn migrate(env: &HashMap<String, String>) -> i32 {
    let home = ratchet_home(env);
    let mut conn = match db::open(&db::db_path(&home)) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    match db::migrate(&mut conn) {
        Err(e) => fail(e),
        Ok(applied) if applied.is_empty() => {
            println!("already at version {}", db::current_version(&conn));
            0
        }
        Ok(applied) => {
            for (version, name) in applied {
                println!("applied {version:04} {name}");
            }
            println!("now at version {}", db::current_version(&conn));
            0
        }
    }
}

pub fn path(env: &HashMap<String, String>) -> i32 {
    println!("{}", db::db_path(&ratchet_home(env)).display());
    0
}

pub fn selftest() -> i32 {
    match db::selftest() {
        Ok(line) => {
            println!("{line}");
            0
        }
        Err(e) => fail(e),
    }
}

fn fail(e: impl std::fmt::Display) -> i32 {
    eprintln!("error: {e}");
    1
}
```

In `crates/ratchet/src/main.rs`: add `mod cli;` to the `mod` block, extend `DbCmd` and replace the `Cmd::Db` arm:
```rust
#[derive(Subcommand)]
enum DbCmd {
    /// Apply pending migrations to the state database.
    Migrate,
    /// Print the path of the state database.
    Path,
    /// Print one line proving the bundled SQLite is linked and executes SQL.
    Selftest,
}
```
```rust
        Cmd::Db { cmd } => match cmd {
            DbCmd::Migrate => cli::db_cmd::migrate(&env),
            DbCmd::Path => cli::db_cmd::path(&env),
            DbCmd::Selftest => cli::db_cmd::selftest(),
        },
```

- [ ] **Step 3: Run**

```
cd /c/repos/ratchet && cargo test -p ratchet --test spec sessions__migrations sessions__the_database_path && cargo test -p ratchet && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: those two scenario tests green; the rest of the suite no worse than before.

- [ ] **Step 4: Hand off (no git)**

List the three files. Report which scenario tests turned green.

---
### Task 10: The six hooks get bodies

Ported from `C:\repos\ops\ops\cli\hooks.py` (`_dispatch`, `_start`, `_ensure_session`, `_export_session_id`, `_git_branch`). `pre-tool` is not touched: it still resolves the repo, evaluates the rules and returns, without opening anything.

**Files:**
- Modify: `crates/ratchet/src/hooks/dispatch.rs`, `crates/ratchet/src/repo.rs`

**Interfaces:**
- Consumes: `services::sessions::{get, touch, end, upsert_start, StartInput}`, `services::tasks::release_dead`, `db::{connect, open_ready}`, `clock::now`, `model::{EventKind, SessionMode, LaunchedBy}`, `repo::{find_repo, within, normalize}`.
- Produces: `repo::git_branch(cwd: &Path) -> Option<String>`; the `Payload.session_id` field; the six handlers. Nothing outside `hooks/` consumes them.
- **Seam for group 2 (do not lose it):** in `session_start`, the briefing is printed *before* `release_dead` is called. Group 1 leaves a marked, empty line between `export_session_id` and the release; group 2's briefing goes exactly there, with the same `now`. Reversing the order makes an orphaned task appear as `ready` without ever having shown its last handoff (the bug ops fixed in T-0021).

- [ ] **Step 1: Write the failing unit tests**

Append to the existing `#[cfg(test)] mod tests` of `crates/ratchet/src/hooks/dispatch.rs`:
```rust
    #[test]
    fn payload_carries_the_session_id() {
        let p = parse_payload(r#"{"session_id":"abc","cwd":"C:/r"}"#).unwrap();
        assert_eq!(p.session_id, "abc");
    }

    #[test]
    fn identity_prefers_the_payload_then_the_environment() {
        let mut env = HashMap::new();
        let with_id = parse_payload(r#"{"session_id":"from-payload"}"#).unwrap();
        let without = parse_payload("{}").unwrap();
        env.insert("RATCHET_SESSION_ID".to_string(), "from-env".to_string());
        assert_eq!(session_identity(&with_id, &env).as_deref(), Some("from-payload"));
        assert_eq!(session_identity(&without, &env).as_deref(), Some("from-env"));
        env.clear();
        assert_eq!(session_identity(&without, &env), None);
    }

    #[test]
    fn exporting_the_id_appends_one_line_and_survives_a_missing_variable() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("env.sh");
        let mut env = HashMap::new();
        env.insert(
            "CLAUDE_ENV_FILE".to_string(),
            file.to_string_lossy().to_string(),
        );
        export_session_id(&env, "s-1");
        export_session_id(&env, "s-2");
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            vec!["export RATCHET_SESSION_ID=s-1", "export RATCHET_SESSION_ID=s-2"]
        );
        export_session_id(&HashMap::new(), "s-3"); // no variable: silent no-op
    }

    #[test]
    fn a_branch_is_read_from_a_real_repository_or_is_none() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(crate::repo::git_branch(dir.path()), None, "not a repository");
    }
```

- [ ] **Step 2: Run to see them fail**

```
export PATH="$HOME/.cargo/bin:/c/Users/eillanes/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet hooks::
```
Expected: compile errors naming `session_id`, `session_identity`, `export_session_id`, `git_branch`.

- [ ] **Step 3: Add `git_branch` to `repo.rs`**

```rust
/// Branch checked out at `cwd`, when git can tell. A detached HEAD and any failure answer `None`.
/// Never reached from `pre-tool`: only the session hooks call it, once per session start.
pub fn git_branch(cwd: &Path) -> Option<String> {
    let dir = cwd.to_string_lossy().to_string();
    let out = Command::new("git")
        .args(["-C", &dir, "rev-parse", "--abbrev-ref", "HEAD"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        None
    } else {
        Some(branch)
    }
}
```

- [ ] **Step 4: Give the hooks bodies**

In `crates/ratchet/src/hooks/dispatch.rs`, add `session_id` to the payload:
```rust
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Payload {
    pub session_id: String,
    pub tool_name: String,
    pub tool_input: Value,
    pub cwd: Option<PathBuf>,
    pub hook_event_name: Option<String>,
    pub stop_hook_active: bool,
}
```
replace the `_ => Ok(0)` arm of `dispatch`:
```rust
    match event {
        "pre-tool" => pre_tool(payload, env, &cwd, home),
        "session-start" => session_start(&payload, env, &cwd, home),
        "prompt" => heartbeat(&payload, env, &cwd, home, Some(EventKind::SessionPrompt)),
        // The handoff rule of group 2 goes on top of this heartbeat, and only it may return 2.
        "stop" => heartbeat(&payload, env, &cwd, home, Some(EventKind::SessionStop)),
        "subagent-stop" | "pre-compact" => heartbeat(&payload, env, &cwd, home, None),
        "session-end" => session_end(&payload, env, &cwd, home),
        _ => Ok(0),
    }
```
and add the handlers (imports at the top of the file: `use std::io::Write;`, `use chrono::{DateTime, Utc};`, `use rusqlite::Connection;`, `use crate::clock;`, `use crate::db;`, `use crate::model::{EventKind, LaunchedBy, Session, SessionMode};`, `use crate::repo::{find_repo, git_branch, has_venv, normalize, within, Repo};`, `use crate::services::sessions::{self, StartInput};`, `use crate::services::{tasks, ServiceError};`):
```rust
/// The identity the harness fixed: the payload first, then the environment (a headless run sets
/// it before starting the process). ratchet never invents one.
pub fn session_identity(payload: &Payload, env: &HashMap<String, String>) -> Option<String> {
    if !payload.session_id.is_empty() {
        return Some(payload.session_id.clone());
    }
    env.get("RATCHET_SESSION_ID")
        .filter(|s| !s.is_empty())
        .cloned()
}

fn register(
    conn: &mut Connection,
    repo: &Repo,
    session_id: &str,
    cwd: &Path,
    env: &HashMap<String, String>,
    now: DateTime<Utc>,
) -> Result<Session, ServiceError> {
    let cwd_text = cwd.to_string_lossy().to_string();
    let worktree = if within(cwd, &repo.worktrees_dir) {
        Some(cwd_text.clone())
    } else {
        None
    };
    let branch = git_branch(cwd);
    let repo_root = normalize(&repo.main_root).to_string_lossy().to_string();
    let mode = SessionMode::from_db(
        env.get("RATCHET_SESSION_MODE")
            .map(String::as_str)
            .unwrap_or(""),
    );
    let launched_by = LaunchedBy::from_db(
        env.get("RATCHET_LAUNCHED_BY")
            .map(String::as_str)
            .unwrap_or(""),
    );
    sessions::upsert_start(
        conn,
        StartInput {
            session_id,
            repo: &repo.name,
            repo_root: &repo_root,
            cwd: &cwd_text,
            worktree: worktree.as_deref(),
            branch: branch.as_deref(),
            mode,
            launched_by,
        },
        now,
    )
}

fn ensure_session(
    conn: &mut Connection,
    repo: &Repo,
    session_id: &str,
    cwd: &Path,
    env: &HashMap<String, String>,
    now: DateTime<Utc>,
) -> Result<Session, ServiceError> {
    match sessions::get(conn, session_id)? {
        Some(s) => Ok(s),
        None => register(conn, repo, session_id, cwd, env, now),
    }
}

/// The only hook that migrates (spec §6).
pub fn session_start(
    payload: &Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
) -> Result<i32, String> {
    let Some(repo) = find_repo(cwd).map_err(|e| e.to_string())? else {
        return Ok(0);
    };
    let Some(session_id) = session_identity(payload, env) else {
        return Ok(0);
    };
    let now = clock::now(env);
    let mut conn = db::connect(home).map_err(|e| e.to_string())?;
    let session = register(&mut conn, &repo, &session_id, cwd, env, now).map_err(|e| e.to_string())?;
    export_session_id(env, &session.id);

    // ---- group 2: the [ratchet] briefing is printed HERE, with this same `now`, BEFORE the
    // sweep below, so an orphaned task is shown once with its last handoff while it is still
    // claimed. Do not move the sweep above this line. ----

    tasks::release_dead(
        &mut conn,
        &session.repo_root,
        &repo.config.thresholds,
        now,
    )
    .map_err(|e| e.to_string())?;
    Ok(0)
}

fn heartbeat(
    payload: &Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
    kind: Option<EventKind>,
) -> Result<i32, String> {
    let Some(repo) = find_repo(cwd).map_err(|e| e.to_string())? else {
        return Ok(0);
    };
    let Some(session_id) = session_identity(payload, env) else {
        return Ok(0);
    };
    let now = clock::now(env);
    let mut conn = db::open_ready(home).map_err(|e| e.to_string())?;
    ensure_session(&mut conn, &repo, &session_id, cwd, env, now).map_err(|e| e.to_string())?;
    sessions::touch(&mut conn, &session_id, kind, now).map_err(|e| e.to_string())?;
    Ok(0)
}

fn session_end(
    payload: &Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
) -> Result<i32, String> {
    let Some(repo) = find_repo(cwd).map_err(|e| e.to_string())? else {
        return Ok(0);
    };
    let Some(session_id) = session_identity(payload, env) else {
        return Ok(0);
    };
    let now = clock::now(env);
    let mut conn = db::open_ready(home).map_err(|e| e.to_string())?;
    // A session that was never registered has nothing to close: silent success.
    if sessions::get(&conn, &session_id)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        return Ok(0);
    }
    let session = sessions::end(&mut conn, &session_id, now).map_err(|e| e.to_string())?;
    // Tidy close: this session is `ended`, so its work goes back now instead of waiting for the
    // lazy sweep of the next session start.
    tasks::release_dead(&mut conn, &session.repo_root, &repo.config.thresholds, now)
        .map_err(|e| e.to_string())?;
    Ok(0)
}

/// Publishes the identity to the session's shell, so `ratchet` commands run from Bash attribute
/// their writes without `--session`.
pub fn export_session_id(env: &HashMap<String, String>, session_id: &str) {
    let Some(target) = env.get("CLAUDE_ENV_FILE").filter(|s| !s.is_empty()) else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(target)
    {
        let _ = writeln!(file, "export RATCHET_SESSION_ID={session_id}");
    }
}
```

- [ ] **Step 5: Run the unit tests, then the scenarios**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet hooks:: && cargo test -p ratchet --test spec 2>&1 | tail -20
```
Expected: the 4 new unit tests green; every `sessions__` scenario green except the three that need `session list|show` (Task 11): `listing_shows_the_derived_state`, `showing_an_unknown_session_fails_with_a_message`, `showing_without_an_id_resolves_the_session_of_the_directory` — and `a_stale_schema_is_reported_not_migrated`, `a_session_in_a_worktree_records_branch_and_worktree`, `a_headless_session_launched_by_the_platform`, `an_ended_session_stays_ended`, `the_repo_sets_its_own_thresholds` and `a_session_with_no_signal_for_ninety_minutes_is_orphaned`, which read through the CLI too. Group 0's 21 stay green.

- [ ] **Step 6: Prove the hot path did not move**

```
cd /c/repos/ratchet && cargo test -p ratchet --release --test latency -- --nocapture
grep -n "Command::new" crates/ratchet/src/*.rs crates/ratchet/src/*/*.rs
```
Expected: the latency median unchanged within noise, and `Command::new` only in `repo::is_tracked` and `repo::git_branch` — with `git_branch` unreachable from `pre_tool`. State both in the hand-off.

- [ ] **Step 7: Hand off (no git)**

List the two files, the scenario counts and the latency numbers.

---

### Task 11: `ratchet session list | show`, and the README

**Files:**
- Create: `crates/ratchet/src/cli/session_cmd.rs`
- Modify: `crates/ratchet/src/main.rs`, `README.md`

**Interfaces:**
- Consumes: `sessions::{list, get, state, resolve}`, `tasks::claimed_ids`, `db::open_ready`, `config::{ratchet_home, Thresholds}`, `repo::find_repo`, `clock::{now, iso}`.
- Produces: `cli::session_cmd::list(...) -> i32`, `cli::session_cmd::show(...) -> i32`. Group 2 promotes `--json` and `--session` to global options once every face honours them; today they live on these two commands only.

- [ ] **Step 1: The failing tests are the scenario tests**

```
cd /c/repos/ratchet && cargo test -p ratchet --test spec sessions__
```
Expected: the remaining red ones all mention `session list` or `session show`.

- [ ] **Step 2: Implement**

`crates/ratchet/src/cli/session_cmd.rs`:
```rust
//! `ratchet session list | show`. Reads only: no face writes to the database except through a
//! service, and neither of these mutates anything.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::{ratchet_home, Thresholds};
use crate::db;
use crate::model::{Session, SessionState};
use crate::repo::find_repo;
use crate::services::{sessions, tasks};
use crate::clock;

/// Thresholds of the repo the directory belongs to; the defaults outside a marked repo.
fn thresholds_for(cwd: Option<&Path>) -> Thresholds {
    cwd.and_then(|c| find_repo(c).ok().flatten())
        .map(|r| r.config.thresholds)
        .unwrap_or_default()
}

pub fn list(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    repo_filter: Option<&str>,
    live_only: bool,
    json: bool,
) -> i32 {
    let home = ratchet_home(env);
    let conn = match db::open_ready(&home) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let now = clock::now(env);
    let th = thresholds_for(cwd.as_deref());
    let found = match sessions::list(&conn, repo_filter) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let mut rows: Vec<(Session, SessionState, Vec<String>)> = Vec::new();
    for s in found {
        let st = sessions::state(&s, &th, now);
        if live_only && st != SessionState::Live {
            continue;
        }
        let mine = match tasks::claimed_ids(&conn, &s.id) {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        rows.push((s, st, mine));
    }
    if json {
        let payload: Vec<serde_json::Value> = rows
            .iter()
            .map(|(s, st, mine)| {
                let mut v = serde_json::to_value(s).unwrap_or(serde_json::Value::Null);
                if let Some(map) = v.as_object_mut() {
                    map.insert("state".into(), st.as_str().into());
                    map.insert("tasks".into(), serde_json::json!(mine));
                }
                v
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
        return 0;
    }
    for (s, st, mine) in rows {
        let id = s.id.chars().take(8).collect::<String>();
        let branch = s.branch.clone().unwrap_or_else(|| "?".to_string());
        let held = if mine.is_empty() {
            "-".to_string()
        } else {
            mine.join(",")
        };
        println!(
            "{:<8}  {:<9} {:<14} {:<20} {}  {}",
            id,
            st.as_str(),
            s.repo,
            branch,
            held,
            clock::iso(s.last_seen)
        );
    }
    0
}

pub fn show(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    id: Option<&str>,
    json: bool,
) -> i32 {
    let home = ratchet_home(env);
    let conn = match db::open_ready(&home) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let now = clock::now(env);
    let th = thresholds_for(cwd.as_deref());
    let here = cwd.unwrap_or_else(|| PathBuf::from("."));
    let resolved = match sessions::resolve(&conn, id, env, &here, &th, now) {
        Ok(Some(v)) => v,
        Ok(None) => {
            eprintln!(
                "error: no session resolved for this directory; name one or set RATCHET_SESSION_ID"
            );
            return 1;
        }
        Err(e) => return fail(e),
    };
    let session = match sessions::get(&conn, &resolved) {
        Ok(Some(s)) => s,
        Ok(None) => {
            eprintln!("error: session {resolved} does not exist");
            return 1;
        }
        Err(e) => return fail(e),
    };
    let st = sessions::state(&session, &th, now);
    let mine = match tasks::claimed_ids(&conn, &session.id) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    if json {
        let mut v = serde_json::to_value(&session).unwrap_or(serde_json::Value::Null);
        if let Some(map) = v.as_object_mut() {
            map.insert("state".into(), st.as_str().into());
            map.insert("tasks".into(), serde_json::json!(mine));
        }
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return 0;
    }
    println!("{}  {}", session.id, st.as_str());
    println!(
        "repo {} · branch {} · {} · {}",
        session.repo,
        session.branch.clone().unwrap_or_else(|| "?".to_string()),
        session.mode.as_str(),
        session.launched_by.as_str()
    );
    println!("cwd {}", session.cwd);
    if let Some(wt) = &session.worktree {
        println!("worktree {wt}");
    }
    let ended = session
        .ended_at
        .map(|e| format!(" · ended {}", clock::iso(e)))
        .unwrap_or_default();
    println!(
        "started {} · last seen {}{}",
        clock::iso(session.started_at),
        clock::iso(session.last_seen),
        ended
    );
    if !mine.is_empty() {
        println!("tasks: {}", mine.join(", "));
    }
    0
}

/// Every error of a face prints the same shape and exits 1 — never 2, which belongs to blocks.
fn fail(e: impl std::fmt::Display) -> i32 {
    eprintln!("error: {e}");
    1
}
```

In `crates/ratchet/src/main.rs`, add the subcommand:
```rust
    /// Agent sessions: `list`, `show`.
    Session {
        #[command(subcommand)]
        cmd: SessionCmd,
    },
```
```rust
#[derive(Subcommand)]
enum SessionCmd {
    /// One line per session with its derived state.
    List {
        /// Only sessions of this repo (the name in the marker, or the directory name).
        #[arg(long)]
        repo: Option<String>,
        /// Only the live ones.
        #[arg(long)]
        live: bool,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Detail of one session; without an id, the session covering this directory.
    Show {
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
}
```
```rust
        Cmd::Session { cmd } => match cmd {
            SessionCmd::List { repo, live, json } => {
                cli::session_cmd::list(&env, cwd, repo.as_deref(), live, json)
            }
            SessionCmd::Show { id, json } => cli::session_cmd::show(&env, cwd, id.as_deref(), json),
        },
```
and add `pub mod session_cmd;` to `crates/ratchet/src/cli/mod.rs`.

- [ ] **Step 3: Run the whole suite**

```
cd /c/repos/ratchet && cargo test -p ratchet && cargo test -p ratchet --test scenarios && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: everything green — group 0's 21 scenarios, the 24 of this group, every unit test, the checker and the latency test.

- [ ] **Step 4: Extend the README**

Two edits, nothing else. First, correct the stale pointer in the "Opt a repo in" section: the line `Create `ratchet.toml` at the repo root (see `ratchet config init` once group 1 lands):` becomes `Create `ratchet.toml` at the repo root (`ratchet config init` arrives with group 2):`. Then append, before the `## Not here (yet)` section:

```markdown
## State and sessions

Everything ratchet remembers lives in one SQLite file: `~/.ratchet/ratchet.db` (override the
directory with `RATCHET_HOME`). SQLite is compiled into the binary — nothing to install — and
the schema is applied by embedded migrations. The session-start hook migrates automatically;
everywhere else, an out-of-date database says so and asks for `ratchet db migrate`.

    ratchet db path        # where the database is
    ratchet db migrate     # apply pending migrations
    ratchet session list   # one line per session: id, state, repo, branch, tasks, last signal
    ratchet session list --live
    ratchet session show   # the session covering this directory (or name one)

A session is registered by the hooks themselves, with the identifier Claude Code gives them —
ratchet never invents one. It records the repo, the directory, the worktree and branch when
there are any, the mode (`interactive` or `headless`), who launched it (`user` or `platform`),
and its signals. State is derived, never stored: `live` while the last signal is under
`live_minutes`, `idle` until `idle_minutes`, `orphaned` after that, `ended` once the session
closed. Both thresholds come from `[thresholds]` in the repo's `ratchet.toml` (10 and 60 by
default).

When a session dies, its work goes back: a task `in_progress` held by an ended or orphaned
session returns to `ready` without a session, at the end of that session and, for sessions that
died without a hook, at the next session start in that repo. The task keeps its whole history.

Two environment variables let a launcher place a headless session in the registry:
`RATCHET_SESSION_ID` (the identity, also written to the session's shell through
`CLAUDE_ENV_FILE`), `RATCHET_SESSION_MODE` and `RATCHET_LAUNCHED_BY`. `RATCHET_NOW` (RFC 3339)
replaces the clock for one process and exists so the time-dependent behaviour above can be
tested without sleeping.

The `PreToolUse` guardrail hook still opens no database at all: it is the hot path and it stays
at the latency group 0 measured.
```

- [ ] **Step 5: Hand off (no git)**

List the three files, the full test counts and the release latency median. Nothing is committed; the owner publishes from the other account.

---

### Task 12: Group review (reviewer, read-only)

**Files:** none modified. A blocking item is described with `file:line`, never fixed here.

- [ ] **Step 1: Run the gate** from `C:\repos\ratchet`, with the PATH preamble: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p ratchet`, `cargo test -p ratchet --test scenarios`, `cargo test -p ratchet --release --test latency -- --nocapture`. All green, numbers recorded, including the release median against group 0's 46.3 ms and the size of the binary.
- [ ] **Step 2: Contrast against `openspec/specs/sessions/spec.md` and this plan.** Every one of the 24 scenarios has a test that really bites (pick three and break the production line they cover, confirm the test fails, restore). Check the layer rule by grep: `INSERT|UPDATE|DELETE` appears only under `src/services/` and in `src/db/migrations/`; `events` is never the target of an `UPDATE` or `DELETE` anywhere; `db::connect` is called only from `hooks::dispatch::session_start` and `cli::db_cmd::migrate`; `Command::new` only in `repo::is_tracked` and `repo::git_branch`, neither reachable from `pre_tool`.
- [ ] **Step 3: Adversarial probes** with the real binary and a temporary `RATCHET_HOME`:
  - two `session-start` hooks for different session ids launched at the same moment against a database that does not exist yet (both must exit 0 and the schema must be applied exactly once — check `SELECT COUNT(*) FROM schema_version`);
  - a `session-end` hook for a session id that was never registered (exit 0, nothing written);
  - a `prompt` hook whose payload has no `session_id` and no `RATCHET_SESSION_ID` (exit 0, nothing written, at most one log line);
  - `RATCHET_HOME` pointing at a path that cannot be created, e.g. a file (every hook still exits 0);
  - a `ratchet.toml` with `[thresholds] live_minutes = 0` (no panic; the session is never `live`);
  - a task row whose `claimed_by` names a session that is not in the registry (the sweep must release it: "gone" counts as dead);
  - `ratchet session show` from a directory with no marker and no live session (exit 1 with a message, never a panic).
- [ ] **Step 4: Verdict** as a written note: APPROVED, or BLOCKING items with `file:line`. Include the answer to one question explicitly: would group 2 be able to add the briefing, the prompt reminder and the handoff rule without editing anything in `services/sessions.rs`? If not, say what it would have to change.

---

## Owner questions

The spec is silent on these four. Each is decided here so the group does not stall; each is a ruling the owner can reverse with one edit.

- **Ruling G1-R1 — `repo_root` is the key, `repo` is the label.** D-marker removed the central registry of repos, so a repo's name is no longer unique on a machine: two checkouts called `web` would share a namespace. `sessions` and `tasks` therefore carry both `repo` (display name from the marker, what `--repo` filters on) and `repo_root` (the absolute, lower-cased main root, what every lookup and the orphan sweep scope by). Cost if wrong: one column in migration `0001` that group 2 stops using. Cost if omitted and wrong: a migration and a backfill later, with two repos' tasks already mixed.
- **Ruling G1-R2 — the whole v1 schema lands in migration `0001`.** Group 1 only writes `sessions`, `events` and one column of `tasks`, but the orphan sweep must `UPDATE tasks`, and splitting the board tables into a later migration would buy nothing but an `ALTER` for group 2. `0001` therefore creates `tasks`, `checklist_items`, `task_seq`, `sessions` and `events`; group 2 fills the board service. Cost if wrong: an unused table for one group.
- **Ruling G1-R3 — `RATCHET_NOW` ships in the binary.** Derived state (live/idle/orphaned) and the orphan sweep are pure functions of "now"; without a seam the scenarios would need `sleep(90 minutes)` or white-box access. Services always take `now` as a parameter; only the faces read the clock, and they honour `RATCHET_NOW` when it parses. It is documented in the README as a test seam. Cost if wrong: one line in `clock::now` and a different testing strategy.
- **Ruling G1-R4 — no briefing and no `config show|init` in group 1.** Spec §4.2 shows the `SessionStart` hook printing the `[ratchet]` briefing, but §8 assigns the briefing, the prompt reminder, the handoff rule and the output discipline to group 2; §8 governs the split, so group 1's `session-start` prints nothing and leaves a marked insertion point (Task 10). Likewise `ratchet config init` is not in §8's group-1 list; the README line promising it "once group 1 lands" is corrected to group 2 in Task 11 rather than pulling the command forward. Cost if wrong: group 2 moves one line, or `config init` is added as a 20-line task.

## Self-review against the spec

- **Spec coverage (group 1).** §8 group 1 lists five things: embedded migrations (Task 4), `db migrate|path` (Task 9), the sessions service (Task 7), the other six hooks — registry, heartbeat, orphan release, session end (Tasks 8, 10) — and `session list|show` (Task 11). D-runtime/D-state: `rusqlite` bundled, everything under `~/.ratchet/`, `RATCHET_HOME` override (Tasks 1, 4). D-p3: every group-1 hook exits 0, enforced by a Global Constraint, by the scenario `an_unusable_database_does_not_break_the_session` and by Task 12's probes. D-specs-first: Tasks 2 and 3, checker green. D-roles: Task 3 is the `spec-test-author`'s, Tasks 1 and 4-11 the implementer's, Task 12 the reviewer's; no git anywhere. D-english: all content. D-runner-out: `RATCHET_SESSION_MODE`/`RATCHET_LAUNCHED_BY` are read so a headless run registers like an interactive one, and nothing else of the runner appears. §4.1 identity resolution order: `sessions::resolve` (Task 7), used by `session show` (Task 11). §4.3 thresholds: read from the marker (Tasks 7, 11). §5 data flow: hooks → services → database, the no-marker branch opens nothing. §6: automatic migration only on session start, `run ratchet db migrate` elsewhere, invalid marker still exit 0 (group 0's behaviour, untouched).
- **Deliberately deferred.** The briefing, the prompt reminder, the handoff rule, the output discipline (>60 lines to `~/.ratchet/out/`), `--json`/`--session` as global options, the task board CLI and `ratchet config init` — all group 2 per §8. Release binaries and the bootstrap — group 5. Group 4 content already exists in the repo and is not re-planned; the README only gains a section and one corrected line.
- **Placeholder scan.** No "TBD", no "add error handling", no "similar to Task N": every step carries the code or the exact command. The only conditional instruction in the whole plan is Task 1's MSRV branch (if a transitive dependency forces a higher `rust-version`, raise it in the same edit and record it), and it says exactly what to do.
- **Type consistency.** `StartInput` has the same eight fields in Tasks 7 and 10. `sessions::state(&Session, &Thresholds, DateTime<Utc>)` is called identically from `tasks::orphaned`, `session list` and `session show`. `events::emit` keeps its seven parameters in Tasks 6, 7 and 8, and is always given a `&Connection` or a `&Transaction` (which derefs). `Thresholds` is group 0's `config::Thresholds` (`live_minutes`, `idle_minutes`), never redefined. `ServiceError` is constructed only in `services/`. `db::open_ready` is used by every face except `session-start` and `db migrate`, which use `db::connect`. `repo_root` is written by `register` as `normalize(&repo.main_root)` and read back by `tasks::orphaned` and the test helper `repo_root_key`, which produce the same lower-cased string. Event kind strings (`session.start`, `session.prompt`, `session.stop`, `session.end`, `task.status`, `note`) match between `model::EventKind`, the scenario tests and Task 3's Interfaces block.
