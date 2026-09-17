//! Append-only log. INSERT only: no UPDATE, no DELETE, in this module or any other.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde_json::Value;

use super::ServiceError;
use crate::clock;
use crate::model::{Event, EventKind, Source};

// Consumed by services::sessions (Task 7), services::tasks (Task 8) and hooks::dispatch
// (Task 10) to append one event per mutation.
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

// Consumed by cli::session_cmd (Task 11) to print a session's event history.
#[allow(dead_code)]
pub fn for_session(
    conn: &Connection,
    session_id: &str,
    limit: i64,
) -> Result<Vec<Event>, ServiceError> {
    collect(
        conn,
        "SELECT * FROM events WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2",
        session_id,
        limit,
    )
}

// Consumed by cli::task_cmd (Task 8/11) to print a task's event history.
#[allow(dead_code)]
pub fn for_task(conn: &Connection, task_id: &str, limit: i64) -> Result<Vec<Event>, ServiceError> {
    collect(
        conn,
        "SELECT * FROM events WHERE task_id = ?1 ORDER BY id DESC LIMIT ?2",
        task_id,
        limit,
    )
}

/// Reads the newest `limit` rows and returns them oldest first, so callers read a timeline.
fn collect(
    conn: &Connection,
    sql: &str,
    key: &str,
    limit: i64,
) -> Result<Vec<Event>, ServiceError> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![key, limit], Event::from_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    out.reverse();
    Ok(out)
}

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
        for (i, kind) in [EventKind::SessionStart, EventKind::SessionPrompt]
            .iter()
            .enumerate()
        {
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
            emit(
                &tx,
                EventKind::Note,
                &json!({}),
                Source::Cli,
                None,
                None,
                at("2026-09-16T12:00:00Z"),
            )
            .unwrap();
            // dropped without commit
        }
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
}
