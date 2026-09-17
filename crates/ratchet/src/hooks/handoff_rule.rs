//! The handoff rule on `Stop`: a session that holds a task in progress and left no record of this
//! turn is blocked once and asked for a handoff. Every branch here is a bug somebody hit; the
//! comments say which.

use rusqlite::{params, Connection};

use crate::model::{EventKind, SessionMode, TaskStatus};
use crate::services::{events, tasks};

/// What counts as "this session left a record on this task".
const ACTIVITY: [&str; 5] = [
    "handoff",
    "checklist.done",
    "checklist.undone",
    "note",
    "task.status",
];

pub fn should_block_stop(
    conn: &Connection,
    session_id: &str,
    stop_hook_active: bool,
    mode: SessionMode,
) -> Option<String> {
    // Nobody is on the other side of a headless run: blocking would only hold it open until its
    // launcher kills it, and the launcher records the outcome itself.
    if mode == SessionMode::Headless {
        return None;
    }
    // Never twice: the harness says the close was already blocked once in this response.
    if stop_hook_active {
        return None;
    }
    let mine = tasks::list(
        conn,
        &tasks::Filter {
            statuses: &[TaskStatus::InProgress],
            claimed_by: Some(session_id),
            ..Default::default()
        },
    )
    .ok()?;
    if mine.is_empty() {
        return None;
    }
    let cutoff = cutoff_id(conn, session_id);
    let stale: Vec<String> = mine
        .iter()
        .filter(|t| {
            !has_activity_since(conn, &t.id, session_id, cutoff)
                && !handoff_since_claim(conn, &t.id, session_id)
        })
        .map(|t| t.id.clone())
        .collect();
    let first = stale.first()?.clone();
    let verb = if stale.len() == 1 { "is" } else { "are" };
    Some(format!(
        "{} {verb} still in_progress with nothing recorded this turn. \
         Run `ratchet task handoff {first} \"what is left and how to resume\"` \
         (or `ratchet task check|note|status {first} …`) before you finish.",
        stale.join(", ")
    ))
}

/// The id of this session's last prompt; with no prompts at all (hooks installed while the session
/// was already open) its `session.start` — never 0, which would accept any historical event.
fn cutoff_id(conn: &Connection, session_id: &str) -> i64 {
    for kind in [EventKind::SessionPrompt, EventKind::SessionStart] {
        let found: Option<i64> = conn
            .query_row(
                "SELECT MAX(id) FROM events WHERE session_id = ?1 AND kind = ?2",
                params![session_id, kind.as_str()],
                |r| r.get(0),
            )
            .unwrap_or(None);
        if let Some(id) = found.filter(|id| *id > 0) {
            return id;
        }
    }
    0
}

/// Something this session did to this task after the cutoff. Spec §4.2, the `Stop` row, says the
/// block applies when there is no record of this turn "other than your own claim" — so everything
/// `claim` writes about itself is exempt, and nothing else is. `claim` writes three kinds of
/// bookkeeping, and the guard exempts exactly those three:
///
/// 1. `task.status` with `to == "in_progress"` — the hop that takes the task.
/// 2. `task.status` with `(from, to) == ("backlog", "ready")` — a claim from `backlog` writes TWO
///    hops, `backlog → ready` then `ready → in_progress`, before `task.claimed`. Exempting only
///    the `in_progress` hop would let the `ready` hop of the session's own claim pass for work
///    (ruling G2-P6). For a task this session HOLDS, a `backlog → ready` hop can only have come
///    from that claim: nothing else moves a held task out of `backlog`, because holding it means
///    it is already `in_progress`.
/// 3. `note` carrying `"claim": true` — the note `claim` writes when it takes a task from a dead
///    holder (ruling G2-P19). That transfer is the worst case of the three: the task was ALREADY
///    `in_progress`, so no status hop is written at all and the note is the window's only event.
///    Counting it would let a session take someone else's work and close having done nothing.
///
/// `task.claimed` needs no exemption: it is not in `ACTIVITY` at all.
fn has_activity_since(conn: &Connection, task_id: &str, session_id: &str, after_id: i64) -> bool {
    let Ok(history) = events::for_task(conn, task_id, 50) else {
        return false;
    };
    for event in history.iter().rev() {
        if event.id <= after_id {
            break;
        }
        if event.session_id.as_deref() != Some(session_id)
            || !ACTIVITY.contains(&event.kind.as_str())
        {
            continue;
        }
        if is_own_claim_bookkeeping(&event.kind, &event.payload) {
            continue;
        }
        return true;
    }
    false
}

/// The bookkeeping `claim` writes about itself, and only that: the two `task.status` hops of a
/// claim (ruling G2-P6) and the transfer `note` marked `"claim": true` (ruling G2-P19). The marker
/// is a payload key written by `services::tasks::claim`, never a match on the note's text.
fn is_own_claim_bookkeeping(kind: &str, payload: &serde_json::Value) -> bool {
    if kind == EventKind::Note.as_str() {
        return payload.get("claim").and_then(|v| v.as_bool()) == Some(true);
    }
    if kind != EventKind::TaskStatus.as_str() {
        return false;
    }
    let field = |name: &str| payload.get(name).and_then(|v| v.as_str());
    let (from, to) = (field("from"), field("to"));
    to == Some("in_progress") || (from == Some("backlog") && to == Some("ready"))
}

/// A handoff of this session, written during this holding, counts even when a prompt has since
/// moved the window: the agent writes the handoff and the user's next prompt arrives seconds
/// later, and the Stop of that turn used to see nothing (the 2026-08-28 demo). The lower bound is
/// not only the claim — that would exempt a long holding with one early handoff for ever — but the
/// *second to last* prompt, so the grace lasts exactly one turn.
fn handoff_since_claim(conn: &Connection, task_id: &str, session_id: &str) -> bool {
    let claim_id: Option<i64> = conn
        .query_row(
            "SELECT MAX(id) FROM events WHERE task_id = ?1 AND session_id = ?2 AND kind = ?3",
            params![task_id, session_id, EventKind::TaskClaimed.as_str()],
            |r| r.get(0),
        )
        .unwrap_or(None);
    let Some(claim_id) = claim_id.filter(|id| *id > 0) else {
        return false;
    };
    let cutoff = claim_id.max(penultimate_prompt_id(conn, session_id));
    let handoff_id: Option<i64> = conn
        .query_row(
            "SELECT MAX(id) FROM events WHERE task_id = ?1 AND session_id = ?2 AND kind = ?3",
            params![task_id, session_id, EventKind::Handoff.as_str()],
            |r| r.get(0),
        )
        .unwrap_or(None);
    handoff_id.map(|id| id > cutoff).unwrap_or(false)
}

/// The id of this session's second-to-last prompt, or 0 when there are fewer than two: with a
/// single prompt there is no previous turn of grace to close.
fn penultimate_prompt_id(conn: &Connection, session_id: &str) -> i64 {
    let Ok(mut stmt) = conn.prepare(
        "SELECT id FROM events WHERE session_id = ?1 AND kind = ?2 ORDER BY id DESC LIMIT 2",
    ) else {
        return 0;
    };
    let Ok(rows) = stmt.query_map(
        params![session_id, EventKind::SessionPrompt.as_str()],
        |r| r.get::<_, i64>(0),
    ) else {
        return 0;
    };
    let ids: Vec<i64> = rows.filter_map(|r| r.ok()).collect();
    if ids.len() < 2 {
        0
    } else {
        ids[1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    use crate::config::Thresholds;
    use crate::db;
    use crate::model::EventKind;
    use crate::model::{LaunchedBy, SessionMode, Source};
    use crate::services::events;
    use crate::services::sessions::{self, StartInput};
    use crate::services::tasks::{self, NewTask};

    fn at(s: &str) -> DateTime<Utc> {
        crate::clock::parse(s).unwrap()
    }

    /// A session holding one task with a checklist, and no prompt yet.
    fn holding() -> (rusqlite::Connection, String) {
        let mut conn = db::open_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        sessions::upsert_start(
            &mut conn,
            StartInput {
                session_id: "s-1",
                repo: "demo",
                repo_root: "root",
                cwd: "root",
                worktree: None,
                branch: None,
                mode: SessionMode::Interactive,
                launched_by: LaunchedBy::User,
            },
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        let id = tasks::create(
            &mut conn,
            NewTask {
                title: "the work",
                body: "",
                repo: "demo",
                repo_root: "root",
                priority: 3,
                parent_id: None,
                tags: &[],
                checklist: &["one".to_string()],
            },
            Source::Cli,
            None,
            at("2026-09-16T12:01:00Z"),
        )
        .unwrap()
        .id;
        tasks::claim(
            &mut conn,
            &id,
            "s-1",
            &Thresholds::default(),
            Source::Cli,
            at("2026-09-16T12:02:00Z"),
        )
        .unwrap();
        (conn, id)
    }

    fn prompt(conn: &rusqlite::Connection, when: &str) {
        events::emit(
            conn,
            EventKind::SessionPrompt,
            &serde_json::json!({}),
            Source::Hook,
            Some("s-1"),
            None,
            at(when),
        )
        .unwrap();
    }

    #[test]
    fn a_session_holding_nothing_closes() {
        let mut conn = db::open_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        assert_eq!(
            should_block_stop(&conn, "s-1", false, SessionMode::Interactive),
            None
        );
    }

    #[test]
    fn a_claim_and_nothing_else_is_blocked_once() {
        let (conn, id) = holding();
        prompt(&conn, "2026-09-16T12:03:00Z");
        let message = should_block_stop(&conn, "s-1", false, SessionMode::Interactive).unwrap();
        assert!(message.contains(&id), "{message}");
        assert!(message.contains("ratchet task handoff"), "{message}");
        assert!(message.contains("check|note|status"), "{message}");
        // The retry never blocks.
        assert_eq!(
            should_block_stop(&conn, "s-1", true, SessionMode::Interactive),
            None
        );
        // Neither does a session nobody is watching.
        assert_eq!(
            should_block_stop(&conn, "s-1", false, SessionMode::Headless),
            None
        );
    }

    #[test]
    fn any_record_of_this_turn_lets_the_session_close() {
        for (label, record) in [("check", 0), ("note", 1), ("status", 2), ("handoff", 3)] {
            let (mut conn, id) = holding();
            prompt(&conn, "2026-09-16T12:03:00Z");
            match record {
                0 => {
                    tasks::check(
                        &mut conn,
                        &id,
                        1,
                        Source::Cli,
                        Some("s-1"),
                        at("2026-09-16T12:04:00Z"),
                    )
                    .unwrap();
                }
                1 => {
                    tasks::note(
                        &mut conn,
                        &id,
                        "found X",
                        Source::Cli,
                        Some("s-1"),
                        at("2026-09-16T12:04:00Z"),
                    )
                    .unwrap();
                }
                2 => {
                    tasks::transition(
                        &mut conn,
                        &id,
                        crate::model::TaskStatus::Blocked,
                        Source::Cli,
                        Some("s-1"),
                        Some("waiting"),
                        at("2026-09-16T12:04:00Z"),
                    )
                    .unwrap();
                }
                _ => {
                    tasks::handoff(
                        &mut conn,
                        &id,
                        "half done",
                        Source::Cli,
                        Some("s-1"),
                        at("2026-09-16T12:04:00Z"),
                    )
                    .unwrap();
                }
            }
            assert_eq!(
                should_block_stop(&conn, "s-1", false, SessionMode::Interactive),
                None,
                "a {label} did not count as a record"
            );
        }
    }

    #[test]
    fn a_record_from_another_session_does_not_count() {
        let (mut conn, id) = holding();
        prompt(&conn, "2026-09-16T12:03:00Z");
        tasks::note(
            &mut conn,
            &id,
            "passing by",
            Source::Cli,
            Some("s-other"),
            at("2026-09-16T12:04:00Z"),
        )
        .unwrap();
        assert!(should_block_stop(&conn, "s-1", false, SessionMode::Interactive).is_some());
    }

    #[test]
    fn a_handoff_survives_one_prompt_but_not_two() {
        let (mut conn, id) = holding();
        prompt(&conn, "2026-09-16T12:03:00Z");
        tasks::handoff(
            &mut conn,
            &id,
            "wrote it before the next prompt",
            Source::Cli,
            Some("s-1"),
            at("2026-09-16T12:04:00Z"),
        )
        .unwrap();
        // The user's next prompt arrives seconds later and moves the window: the handoff still
        // counts for this one turn of grace (the race of the 2026-08-28 demo).
        prompt(&conn, "2026-09-16T12:05:00Z");
        assert_eq!(
            should_block_stop(&conn, "s-1", false, SessionMode::Interactive),
            None
        );
        // One more turn with no work at all, and the exemption is over.
        prompt(&conn, "2026-09-16T12:09:00Z");
        assert!(should_block_stop(&conn, "s-1", false, SessionMode::Interactive).is_some());
    }

    /// Taking a task from a dead holder is still only a claim (ruling G2-P19). `claim` writes a
    /// `note` saying it transferred the task, and because the task was ALREADY `in_progress` it
    /// writes no `task.status` hop at all — so that note is the only record of the window. It must
    /// not stand in for work: spec §4.2, the `Stop` row, blocks when there is nothing "other than
    /// your own claim".
    #[test]
    fn taking_a_task_from_a_dead_session_is_still_only_a_claim() {
        let mut conn = db::open_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        let th = Thresholds::default();
        let start = |conn: &mut rusqlite::Connection, id: &'static str, when: &str| {
            sessions::upsert_start(
                conn,
                StartInput {
                    session_id: id,
                    repo: "demo",
                    repo_root: "root",
                    cwd: "root",
                    worktree: None,
                    branch: None,
                    mode: SessionMode::Interactive,
                    launched_by: LaunchedBy::User,
                },
                at(when),
            )
            .unwrap();
        };
        // The holder starts, claims, and is never heard from again.
        start(&mut conn, "s-dead", "2026-09-16T12:00:00Z");
        let id = tasks::create(
            &mut conn,
            NewTask {
                title: "abandoned",
                body: "",
                repo: "demo",
                repo_root: "root",
                priority: 3,
                parent_id: None,
                tags: &[],
                checklist: &["one".to_string()],
            },
            Source::Cli,
            None,
            at("2026-09-16T12:01:00Z"),
        )
        .unwrap()
        .id;
        tasks::claim(
            &mut conn,
            &id,
            "s-dead",
            &th,
            Source::Cli,
            at("2026-09-16T12:02:00Z"),
        )
        .unwrap();
        // Two hours on, well past `idle_minutes`, this session takes it over and does nothing.
        start(&mut conn, "s-1", "2026-09-16T14:00:00Z");
        tasks::claim(
            &mut conn,
            &id,
            "s-1",
            &th,
            Source::Cli,
            at("2026-09-16T14:01:00Z"),
        )
        .unwrap();
        // The transfer really did happen, and it really did leave a `note` marked as the claim's.
        let notes: Vec<_> = events::for_task(&conn, &id, 50)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == EventKind::Note.as_str())
            .collect();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].payload["claim"], serde_json::json!(true));
        assert!(
            notes[0].payload["text"]
                .as_str()
                .unwrap_or_default()
                .contains("s-dead"),
            "{:?}",
            notes[0].payload
        );
        assert!(should_block_stop(&conn, "s-1", false, SessionMode::Interactive).is_some());
    }

    #[test]
    fn without_any_prompt_the_window_starts_at_the_session_start() {
        let (conn, _id) = holding();
        // Hooks installed with the session already open: there is no prompt event, and the claim
        // alone must still block. A cutoff of 0 would accept any historical event.
        assert!(should_block_stop(&conn, "s-1", false, SessionMode::Interactive).is_some());
    }
}
