//! Pending-call bookkeeping (T-0016): one open row per in-flight `Bash`/`PowerShell` tool call,
//! from the moment its pre-tool hook fires to the moment its post-tool hook (or a sweep) clears
//! it. A board write resolves the subagent that made it, if any, by matching this session's own
//! open rows against its own task id and subcommand word — `resolve` below, the one function
//! every write calls.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};

use super::ServiceError;
use crate::clock;

/// How long an unclosed pending call survives before the sweep removes it.
pub const TTL_MINUTES: i64 = 15;

/// Records one pending call, replacing any previous row of the same `tool_use_id` (so a harness
/// that somehow reused one still leaves exactly one row, never two). Also sweeps every row past
/// its TTL first, which is the cheapest place to do it: there is no separate background job here,
/// only the hooks that already run on every real tool call.
#[allow(clippy::too_many_arguments)]
pub fn record(
    conn: &Connection,
    tool_use_id: &str,
    session_id: &str,
    agent_id: Option<&str>,
    agent_type: Option<&str>,
    command: &str,
    now: DateTime<Utc>,
) -> Result<(), ServiceError> {
    sweep_ttl(conn, now)?;
    conn.execute(
        "INSERT OR REPLACE INTO pending_calls(tool_use_id, session_id, agent_id, agent_type, command, created_at) \
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            tool_use_id,
            session_id,
            agent_id,
            agent_type,
            command,
            clock::iso(now)
        ],
    )?;
    Ok(())
}

/// Clears one pending call by its tool call identifier — the post-tool hook's own join key.
pub fn clear(conn: &Connection, tool_use_id: &str) -> Result<(), ServiceError> {
    conn.execute(
        "DELETE FROM pending_calls WHERE tool_use_id = ?1",
        params![tool_use_id],
    )?;
    Ok(())
}

/// Clears every pending call a subagent left open — a subagent-stop's sweep of its own rows.
pub fn clear_for_agent(
    conn: &Connection,
    session_id: &str,
    agent_id: &str,
) -> Result<(), ServiceError> {
    conn.execute(
        "DELETE FROM pending_calls WHERE session_id = ?1 AND agent_id = ?2",
        params![session_id, agent_id],
    )?;
    Ok(())
}

/// Clears every pending call of a session — a session-end's sweep of what is left.
pub fn clear_for_session(conn: &Connection, session_id: &str) -> Result<(), ServiceError> {
    conn.execute(
        "DELETE FROM pending_calls WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

/// Removes every row older than `TTL_MINUTES`, regardless of session — the periodic sweep the
/// design calls for, run opportunistically on every `record` rather than on a timer.
pub fn sweep_ttl(conn: &Connection, now: DateTime<Utc>) -> Result<usize, ServiceError> {
    let cutoff = clock::iso(now - Duration::minutes(TTL_MINUTES));
    Ok(conn.execute(
        "DELETE FROM pending_calls WHERE created_at < ?1",
        params![cutoff],
    )?)
}

struct Pending {
    agent_id: Option<String>,
    agent_type: Option<String>,
    command: String,
}

/// A board write's identity, resolved from this session's own open pending calls: the ones whose
/// command text contains `task_id` and `subcommand` — and, for a review, `verdict` too — all as
/// plain substrings, anywhere in the text. Exactly one such call, carrying an agent identifier,
/// gives `Some((agent_id, agent_type))`; one with no agent identifier, none at all, or more than
/// one (ambiguous) give `None` — the bare session, exactly as before this requirement existed. A
/// match is never consumed: the same open call can still attribute more than one board write (a
/// compound `check && note` runs two `ratchet` processes against the one call still open).
/// `agent_type` defaults to the empty string when a matching call somehow carries an identifier
/// with no type — display-only, and never what decides whether a match attributes.
pub fn resolve(
    conn: &Connection,
    session_id: &str,
    task_id: &str,
    subcommand: &str,
    verdict: Option<&str>,
) -> Result<Option<(String, String)>, ServiceError> {
    let mut stmt = conn
        .prepare("SELECT agent_id, agent_type, command FROM pending_calls WHERE session_id = ?1")?;
    let rows = stmt.query_map(params![session_id], |r| {
        Ok(Pending {
            agent_id: r.get(0)?,
            agent_type: r.get(1)?,
            command: r.get(2)?,
        })
    })?;
    let mut matches = Vec::new();
    for row in rows {
        let row = row?;
        if row.command.contains(task_id)
            && row.command.contains(subcommand)
            && verdict.map(|v| row.command.contains(v)).unwrap_or(true)
        {
            matches.push(row);
        }
    }
    if matches.len() != 1 {
        return Ok(None);
    }
    let only = matches.into_iter().next().unwrap();
    match only.agent_id {
        Some(id) => Ok(Some((id, only.agent_type.unwrap_or_default()))),
        None => Ok(None),
    }
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

    #[test]
    fn record_then_resolve_attributes_the_single_matching_call() {
        let c = conn();
        record(
            &c,
            "tu-1",
            "s-1",
            Some("agent-1"),
            Some("ratchet:implementer"),
            "ratchet task check T-0001 1",
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        let found = resolve(&c, "s-1", "T-0001", "check", None).unwrap();
        assert_eq!(
            found,
            Some(("agent-1".to_string(), "ratchet:implementer".to_string()))
        );
    }

    #[test]
    fn no_match_or_no_agent_resolves_to_bare() {
        let c = conn();
        record(
            &c,
            "tu-1",
            "s-1",
            None,
            None,
            "ratchet task check T-0001 1",
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        assert_eq!(resolve(&c, "s-1", "T-0001", "check", None).unwrap(), None);
        assert_eq!(resolve(&c, "s-1", "T-9999", "check", None).unwrap(), None);
    }

    #[test]
    fn two_matches_are_ambiguous_and_resolve_to_bare() {
        let c = conn();
        record(
            &c,
            "tu-1",
            "s-1",
            Some("agent-1"),
            Some("ratchet:implementer"),
            "ratchet task check T-0001 1",
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        record(
            &c,
            "tu-2",
            "s-1",
            Some("agent-2"),
            Some("ratchet:reviewer"),
            "ratchet task check T-0001 1",
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        assert_eq!(resolve(&c, "s-1", "T-0001", "check", None).unwrap(), None);
    }

    #[test]
    fn a_review_also_matches_the_verdict_word() {
        let c = conn();
        record(
            &c,
            "tu-1",
            "s-1",
            Some("agent-1"),
            Some("ratchet:reviewer"),
            "ratchet task review T-0001 approve \"fine\"",
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        assert_eq!(
            resolve(&c, "s-1", "T-0001", "review", Some("approve")).unwrap(),
            Some(("agent-1".to_string(), "ratchet:reviewer".to_string()))
        );
        assert_eq!(
            resolve(&c, "s-1", "T-0001", "review", Some("changes")).unwrap(),
            None
        );
    }

    #[test]
    fn clearing_by_tool_use_id_removes_only_that_row() {
        let c = conn();
        record(
            &c,
            "tu-1",
            "s-1",
            Some("agent-1"),
            Some("t"),
            "ratchet task check T-0001 1",
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        record(
            &c,
            "tu-2",
            "s-1",
            Some("agent-2"),
            Some("t"),
            "ratchet task note T-0001 hi",
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        clear(&c, "tu-1").unwrap();
        assert_eq!(resolve(&c, "s-1", "T-0001", "check", None).unwrap(), None);
        assert_eq!(
            resolve(&c, "s-1", "T-0001", "note", None).unwrap(),
            Some(("agent-2".to_string(), "t".to_string()))
        );
    }

    #[test]
    fn clear_for_agent_and_for_session_remove_their_own_rows() {
        let c = conn();
        record(
            &c,
            "tu-1",
            "s-1",
            Some("a1"),
            Some("t"),
            "x",
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        record(
            &c,
            "tu-2",
            "s-1",
            Some("a2"),
            Some("t"),
            "y",
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        record(
            &c,
            "tu-3",
            "s-2",
            None,
            None,
            "z",
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        clear_for_agent(&c, "s-1", "a1").unwrap();
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM pending_calls", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
        clear_for_session(&c, "s-1").unwrap();
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM pending_calls", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn sweep_ttl_removes_only_rows_past_the_window() {
        let c = conn();
        record(
            &c,
            "tu-old",
            "s-1",
            None,
            None,
            "x",
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        record(
            &c,
            "tu-new",
            "s-1",
            None,
            None,
            "y",
            at("2026-09-16T12:20:00Z"),
        )
        .unwrap();
        // `record`'s own sweep at T+20min already drops the T+0 row (older than the 15-minute
        // TTL); a direct `sweep_ttl` a moment later confirms the survivor stays.
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM pending_calls", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        sweep_ttl(&c, at("2026-09-16T12:20:01Z")).unwrap();
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM pending_calls", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
}
