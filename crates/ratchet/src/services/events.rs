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
    emit_attributed(conn, kind, payload, source, session_id, None, task_id, now)
}

/// Same as `emit`, plus the subagent identity (T-0016) a board write resolved through
/// `pending_calls::resolve`: `agent` is `(agent_id, agent_type)`. `agent_id` lands in its own
/// nullable column, so every existing `session_id` query (usage's `attribute.rs`, the briefing,
/// the done gate) keeps working unmodified; `agent_type` has no column of its own and is folded
/// into the JSON payload instead, the only place anything reads it back for display. `agent:
/// None` behaves exactly like plain `emit`: no column, no payload keys, bare session as before
/// this requirement existed.
#[allow(clippy::too_many_arguments)]
pub fn emit_attributed(
    conn: &Connection,
    kind: EventKind,
    payload: &Value,
    source: Source,
    session_id: Option<&str>,
    agent: Option<(&str, &str)>,
    task_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Event, ServiceError> {
    let mut payload = payload.clone();
    let agent_id = agent.map(|(id, _)| id);
    if let (Some(obj), Some((id, ty))) = (payload.as_object_mut(), agent) {
        obj.insert("agent_id".to_string(), Value::String(id.to_string()));
        obj.insert("agent_type".to_string(), Value::String(ty.to_string()));
    }
    conn.execute(
        "INSERT INTO events(ts, session_id, agent_id, task_id, kind, payload, source) \
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            clock::iso(now),
            session_id,
            agent_id,
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
        agent_id: agent_id.map(str::to_string),
        task_id: task_id.map(str::to_string),
        kind: kind.as_str().to_string(),
        payload,
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
    fn emit_attributed_stores_the_agent_id_and_folds_the_type_into_the_payload() {
        let c = conn();
        let ev = emit_attributed(
            &c,
            EventKind::ChecklistDone,
            &json!({"position": 1}),
            Source::Cli,
            Some("s-1"),
            Some(("agent-abc", "ratchet:implementer")),
            Some("T-0001"),
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        assert_eq!(ev.agent_id.as_deref(), Some("agent-abc"));
        assert_eq!(ev.payload["agent_id"], "agent-abc");
        assert_eq!(ev.payload["agent_type"], "ratchet:implementer");
        let (stored_agent, stored_payload): (Option<String>, String) = c
            .query_row(
                "SELECT agent_id, payload FROM events WHERE id = ?1",
                [ev.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored_agent.as_deref(), Some("agent-abc"));
        assert!(stored_payload.contains("agent-abc"));
    }

    #[test]
    fn plain_emit_leaves_the_agent_column_and_payload_untouched() {
        let c = conn();
        let ev = emit(
            &c,
            EventKind::Note,
            &json!({"text": "hi"}),
            Source::Cli,
            Some("s-1"),
            None,
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        assert_eq!(ev.agent_id, None);
        assert!(!ev.payload.to_string().contains("agent_id"));
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
