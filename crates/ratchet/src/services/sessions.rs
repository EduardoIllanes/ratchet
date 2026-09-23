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

// Consumed by cli::session_cmd (Task 11) to show one session and by sessions::resolve/touch/end
// below to reload the row after a write.
#[allow(dead_code)]
pub fn get(conn: &Connection, session_id: &str) -> Result<Option<Session>, ServiceError> {
    let mut stmt = conn.prepare("SELECT * FROM sessions WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![session_id], Session::from_row)?;
    match rows.next() {
        None => Ok(None),
        Some(row) => Ok(Some(row?)),
    }
}

// Consumed by cli::session_cmd (Task 11, `session list`) and by sessions::resolve below.
#[allow(dead_code)]
pub fn list(conn: &Connection, repo_name: Option<&str>) -> Result<Vec<Session>, ServiceError> {
    let mut out = Vec::new();
    match repo_name {
        None => {
            let mut stmt = conn.prepare("SELECT * FROM sessions ORDER BY last_seen DESC")?;
            for row in stmt.query_map([], Session::from_row)? {
                out.push(row?);
            }
        }
        Some(name) => {
            let mut stmt =
                conn.prepare("SELECT * FROM sessions WHERE repo = ?1 ORDER BY last_seen DESC")?;
            for row in stmt.query_map(params![name], Session::from_row)? {
                out.push(row?);
            }
        }
    }
    Ok(out)
}

/// Registers the session, or refreshes the one already registered with that identifier. Both
/// paths append one `session.start` event, with `resumed` saying which happened.
// Consumed by hooks::dispatch (Task 10, `session-start`).
#[allow(dead_code)]
pub fn upsert_start(
    conn: &mut Connection,
    input: StartInput<'_>,
    now: DateTime<Utc>,
) -> Result<Session, ServiceError> {
    let ts = clock::iso(now);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let resumed = get(&tx, input.session_id)?.is_some();
    if resumed {
        tx.execute(
            "UPDATE sessions SET last_seen = ?1, cwd = ?2, worktree = COALESCE(?3, worktree), \
             branch = COALESCE(?4, branch), ended_at = NULL WHERE id = ?5",
            params![
                ts,
                input.cwd,
                input.worktree,
                input.branch,
                input.session_id
            ],
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

// Consumed by hooks::dispatch (Task 10, `user-prompt-submit`/`stop`).
#[allow(dead_code)]
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
        events::emit(
            &tx,
            k,
            &json!({}),
            Source::Hook,
            Some(session_id),
            None,
            now,
        )?;
    }
    tx.commit()?;
    load(conn, session_id)
}

// Consumed by hooks::dispatch (Task 10, `session-end`).
#[allow(dead_code)]
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
// Consumed by cli::session_cmd (Task 11) and by sessions::resolve below.
#[allow(dead_code)]
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
// Consumed by the future `--session` global option and every board write (group 2, per the
// brief's hand-off note).
#[allow(dead_code)]
pub fn resolve(
    conn: &Connection,
    explicit: Option<&str>,
    env: &HashMap<String, String>,
    cwd: &Path,
    th: &Thresholds,
    now: DateTime<Utc>,
) -> Result<Option<String>, ServiceError> {
    if let Some(id) = explicit.filter(|s| !s.is_empty()) {
        reject_agent_shaped(id)?;
        return Ok(Some(id.to_string()));
    }
    if let Some(id) = env.get("RATCHET_SESSION_ID").filter(|s| !s.is_empty()) {
        reject_agent_shaped(id)?;
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

/// Neither `--session` nor `RATCHET_SESSION_ID` can name an agent identity directly (T-0016): a
/// value containing `/` is refused before any registration lookup or attribution runs and before
/// anything is written. Distinct from, and checked before, "session … is not registered" — the
/// pair identity `(session, agent)` this requirement introduces is the one shape nobody can type,
/// because it never comes from a string on the command line, only from what the harness reported
/// to a pre-tool hook.
fn reject_agent_shaped(id: &str) -> Result<(), ServiceError> {
    if id.contains('/') {
        return Err(ServiceError::Invalid(format!(
            "session identifier cannot contain '/': {id}"
        )));
    }
    Ok(())
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
        upsert_start(
            &mut c,
            input("s-1", "c:\\repos\\demo"),
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
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
        let events = crate::services::events::for_session(&c, "s-1", 10).unwrap();
        assert_eq!(events.len(), 2);
        // Oldest first: the first call inserted (not resumed), the second refreshed
        // (resumed) the same row rather than raising a UNIQUE error on a duplicate insert.
        assert_eq!(events[0].payload["resumed"], false);
        assert_eq!(events[1].payload["resumed"], true);
    }

    #[test]
    fn touch_moves_the_signal_and_optionally_records_an_event() {
        let mut c = conn();
        upsert_start(&mut c, input("s-2", "x"), at("2026-09-16T12:00:00Z")).unwrap();
        touch(
            &mut c,
            "s-2",
            Some(EventKind::SessionPrompt),
            at("2026-09-16T12:03:00Z"),
        )
        .unwrap();
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
        assert_eq!(
            s.ended_at.map(clock::iso).as_deref(),
            Some("2026-09-16T12:10:00Z")
        );
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
        assert_eq!(
            state(&s, &th, at("2026-09-16T12:05:00Z")),
            SessionState::Live
        );
        assert_eq!(
            state(&s, &th, at("2026-09-16T12:30:00Z")),
            SessionState::Idle
        );
        assert_eq!(
            state(&s, &th, at("2026-09-16T13:30:00Z")),
            SessionState::Orphaned
        );
        let tight = Thresholds {
            live_minutes: 1,
            idle_minutes: 2,
        };
        assert_eq!(
            state(&s, &tight, at("2026-09-16T12:01:30Z")),
            SessionState::Idle
        );
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
            resolve(&c, Some("explicit"), &env, &deep, &th, now)
                .unwrap()
                .as_deref(),
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
    fn resolve_refuses_an_explicit_or_environment_value_shaped_like_a_pair() {
        let c = conn();
        let dir = tempfile::TempDir::new().unwrap();
        let th = Thresholds::default();
        let now = at("2026-09-16T12:00:00Z");
        let err = resolve(
            &c,
            Some("s-1/agent-x"),
            &HashMap::new(),
            dir.path(),
            &th,
            now,
        )
        .unwrap_err();
        assert!(matches!(err, ServiceError::Invalid(_)));
        assert!(err.to_string().contains("cannot contain"), "{err}");

        let mut env = HashMap::new();
        env.insert("RATCHET_SESSION_ID".to_string(), "s-1/agent-x".to_string());
        let err = resolve(&c, None, &env, dir.path(), &th, now).unwrap_err();
        assert!(err.to_string().contains("cannot contain"), "{err}");
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
            resolve(
                &c,
                None,
                &HashMap::new(),
                &wt,
                &th,
                at("2026-09-16T12:01:00Z")
            )
            .unwrap()
            .as_deref(),
            Some("s-inner"),
            "the deeper cover wins even though it signalled earlier"
        );
    }
}
