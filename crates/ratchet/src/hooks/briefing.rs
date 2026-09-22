//! The `[ratchet]` briefing printed on session start, and (Task 10) the one-line reminder printed
//! on every prompt. Reads only: a briefing that failed must never cost a registration, so every
//! query here falls back to "nothing" instead of propagating an error.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::config::Thresholds;
use crate::model::{Session, Task, TaskStatus};
use crate::output;
use crate::services::tasks;

/// The hard cap of spec §4.2: a briefing is context, not a report.
pub const MAX_LINES: usize = 40;
/// Enough to choose from without turning the briefing into the board.
pub const MAX_READY: usize = 5;
pub const COMMANDS_LINE: &str =
    "Commands: ratchet task show|claim|check|note|handoff  ·  full guide: skill ratchet-tasks";

pub fn build(
    conn: &Connection,
    session: &Session,
    main_root: &Path,
    th: &Thresholds,
    now: DateTime<Utc>,
) -> String {
    let header = format!(
        "[ratchet] repo {} · session {} · branch {}",
        session.repo,
        short(&session.id),
        session.branch.as_deref().unwrap_or("?")
    );
    let rules_line = agents_md_line(main_root);
    let map_line = crate::map::briefing_line(main_root);
    let orphans = tasks::orphaned(conn, &session.repo_root, th, now).unwrap_or_default();
    let mine = tasks::list(
        conn,
        &tasks::Filter {
            repo_root: Some(&session.repo_root),
            statuses: &[TaskStatus::InProgress],
            claimed_by: Some(&session.id),
            ..Default::default()
        },
    )
    .unwrap_or_default();
    let ready: Vec<Task> = tasks::list(
        conn,
        &tasks::Filter {
            repo_root: Some(&session.repo_root),
            statuses: &[TaskStatus::Ready],
            ..Default::default()
        },
    )
    .unwrap_or_default()
    .into_iter()
    .take(MAX_READY)
    .collect();
    if orphans.is_empty() && mine.is_empty() && ready.is_empty() {
        let mut lines = vec![header];
        lines.extend(rules_line.clone());
        lines.extend(map_line.clone());
        return lines.join("\n");
    }
    let mut lines = vec![header];
    lines.extend(rules_line.clone());
    if !mine.is_empty() {
        lines.push("Your tasks in progress:".to_string());
        lines.extend(mine.iter().map(|t| task_line(conn, t, true)));
    }
    if !orphans.is_empty() {
        lines.push("In progress, held by dead sessions (take them or leave them):".to_string());
        for orphan in &orphans {
            match tasks::get(conn, &orphan.task_id) {
                Ok(task) => lines.push(task_line(conn, &task, true)),
                Err(_) => lines.push(format!("  {}  {}", orphan.task_id, orphan.title)),
            }
        }
    }
    if !ready.is_empty() {
        lines.push("Ready to take:".to_string());
        lines.extend(ready.iter().map(|t| task_line(conn, t, false)));
    }
    lines.push(COMMANDS_LINE.to_string());
    // The map line is dropped first (design §4.3): only inserted when the rest already fits.
    // The rules line (just added above, if any) always stays; it sits right after the header,
    // so the map line goes one slot further in when both are present.
    if let Some(l) = &map_line {
        if lines.len() < MAX_LINES {
            let insert_at = if rules_line.is_some() { 2 } else { 1 };
            lines.insert(insert_at, l.clone());
        }
    }
    if lines.len() > MAX_LINES {
        lines.truncate(MAX_LINES - 2);
        lines.push("  … (more in `ratchet task list`)".to_string());
        lines.push(COMMANDS_LINE.to_string());
    }
    lines.join("\n")
}

/// `Some("rules: AGENTS.md")` when the repo has an `AGENTS.md` at its root; `None` otherwise.
/// The briefing points at the file once — it never restates what is in it (T-0012, owner
/// decision: repo rules live in AGENTS.md, nowhere else).
fn agents_md_line(main_root: &Path) -> Option<String> {
    if main_root.join("AGENTS.md").is_file() {
        Some("rules: AGENTS.md".to_string())
    } else {
        None
    }
}

/// One line for the prompt hook, or nothing at all. Deliberately not scoped to the repo: a session
/// that holds a task of another checkout still has to be reminded of it before it starts the next
/// turn.
pub fn prompt_line(conn: &Connection, session: &Session) -> Option<String> {
    let mine = tasks::list(
        conn,
        &tasks::Filter {
            statuses: &[TaskStatus::InProgress],
            claimed_by: Some(&session.id),
            ..Default::default()
        },
    )
    .ok()?;
    // "The first held task" (agent-protocol spec, Task reminder on every prompt): the same
    // `tasks::first_held_id` ordering `hooks::dispatch::subagent_event` uses to attribute a
    // subagent event when it is recorded, so the task this line names and the task a subagent
    // line names can never disagree.
    let held_id = tasks::first_held_id(conn, &session.id).ok().flatten()?;
    let task = mine.iter().find(|t| t.id == held_id)?;
    let progress = tasks::progress(conn, &task.id)
        .ok()
        .flatten()
        .map(|(done, total)| format!(" ({done}/{total})"))
        .unwrap_or_default();
    let handoff = handoff_text(conn, &task.id)
        .map(|text| format!(" · last handoff: {}", quote(&text, 70)))
        .unwrap_or_default();
    let more = if mine.len() > 1 {
        format!(" (+{} more)", mine.len() - 1)
    } else {
        String::new()
    };
    let base = format!(
        "[ratchet] {} {}{progress}{handoff}{more}",
        task.id,
        task.status.as_str()
    );
    let mut lines = vec![base];
    lines.extend(stopped_no_record_lines(conn, &session.id, &task.id));
    lines.extend(running_lines(conn, &session.id, &task.id));
    Some(lines.join("\n"))
}

/// Extra reminder lines for subagents of this session, scoped to the first held task only: the
/// query itself filters on `task_id`, the event's own column, so a line can never be printed
/// under a task other than the one the event was actually recorded against (an event recorded
/// with no task, or against a task that is not `task_id`, is filtered out here and never reaches
/// `out`). Every query here falls back to "nothing": a failed read must never cost the base
/// reminder.
///
/// Kinds are string literals, never `model::EventKind`: the subagent kinds are extended in
/// parallel and this must not couple to them.
fn stopped_no_record_lines(conn: &Connection, session_id: &str, task_id: &str) -> Vec<String> {
    let cutoff = prompt_cutoff(conn, session_id);
    let stops: Vec<(i64, String)> = query_id_payload(
        conn,
        "SELECT id, payload FROM events WHERE session_id = ?1 AND task_id = ?2 AND kind = 'subagent.stop' AND id > ?3 ORDER BY id ASC",
        session_id,
        task_id,
        Some(cutoff),
    );
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (id, payload) in stops {
        let agent = agent_id_of(&payload);
        if !seen.insert(agent.clone()) {
            continue;
        }
        // Any handoff, checklist change, note or status change on this task after the stop
        // counts as a record, whoever wrote it. On a read error assume a record exists.
        let records: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE task_id = ?1 AND id > ?2 AND kind IN ('handoff','checklist.done','checklist.undone','note','task.status')",
                params![task_id, id],
                |r| r.get(0),
            )
            .unwrap_or(1);
        if records == 0 {
            out.push(format!(
                "[ratchet] {task_id} · subagent {} stopped with no record",
                short8(&agent)
            ));
        }
    }
    out
}

/// One line per agent with a `subagent.start` and no later `subagent.stop`, by event id. Scoped
/// to the first held task only, by the event's own `task_id` column (see
/// `stopped_no_record_lines`).
fn running_lines(conn: &Connection, session_id: &str, task_id: &str) -> Vec<String> {
    let starts = query_id_payload(
        conn,
        "SELECT id, payload FROM events WHERE session_id = ?1 AND task_id = ?2 AND kind = 'subagent.start' ORDER BY id ASC",
        session_id,
        task_id,
        None,
    );
    if starts.is_empty() {
        return Vec::new();
    }
    let stops = query_id_payload(
        conn,
        "SELECT id, payload FROM events WHERE session_id = ?1 AND task_id = ?2 AND kind = 'subagent.stop' ORDER BY id ASC",
        session_id,
        task_id,
        None,
    );
    let mut last_start: HashMap<String, i64> = HashMap::new();
    for (id, payload) in &starts {
        last_start.insert(agent_id_of(payload), *id);
    }
    let mut last_stop: HashMap<String, i64> = HashMap::new();
    for (id, payload) in &stops {
        last_stop.insert(agent_id_of(payload), *id);
    }
    let mut running: Vec<(i64, String)> = last_start
        .into_iter()
        .filter(|(agent, start)| *start > last_stop.get(agent).copied().unwrap_or(0))
        .map(|(agent, start)| (start, agent))
        .collect();
    running.sort();
    running
        .into_iter()
        .map(|(_, agent)| format!("[ratchet] {task_id} · subagent {} running", short8(&agent)))
        .collect()
}

/// The window for "since the last prompt" starts at the previous prompt, not the triggering
/// one: dispatch records the `session.prompt` heartbeat before this reminder reads, so the
/// triggering prompt is always the max id and a stop before it would never count otherwise.
/// Mirrors `handoff_rule::penultimate_prompt_id`. Zero when there are fewer than two prompts.
fn prompt_cutoff(conn: &Connection, session_id: &str) -> i64 {
    let mut stmt = match conn.prepare(
        "SELECT id FROM events WHERE session_id = ?1 AND kind = 'session.prompt' ORDER BY id DESC LIMIT 2",
    ) {
        Ok(s) => s,
        Err(_) => return i64::MAX,
    };
    let ids: Vec<i64> = stmt
        .query_map(params![session_id], |r| r.get(0))
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .unwrap_or_default();
    if ids.len() == 2 {
        ids[1]
    } else {
        0
    }
}

/// `(id, payload)` rows for a session- and task-scoped subagent query (`?1` session, `?2` task,
/// `?3` the optional "after this id" bound); empty on any error.
fn query_id_payload(
    conn: &Connection,
    sql: &str,
    session_id: &str,
    task_id: &str,
    after: Option<i64>,
) -> Vec<(i64, String)> {
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mapped = match after {
        Some(id) => stmt.query_map(params![session_id, task_id, id], id_payload_row),
        None => stmt.query_map(params![session_id, task_id], id_payload_row),
    };
    mapped
        .and_then(|rows| rows.collect::<Result<Vec<(i64, String)>, _>>())
        .unwrap_or_default()
}

fn id_payload_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, String)> {
    Ok((r.get(0)?, r.get(1)?))
}

/// The `agent_id` key of a subagent event payload, or `"unknown"` when absent.
fn agent_id_of(payload: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get("agent_id").cloned())
        .and_then(|v| match v {
            serde_json::Value::String(s) if !s.is_empty() => Some(s),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// Eight characters are enough to name a subagent in a reminder; shorter ids stay whole.
fn short8(agent_id: &str) -> String {
    if agent_id.chars().count() > 8 {
        agent_id.chars().take(8).collect()
    } else {
        agent_id.to_string()
    }
}

fn task_line(conn: &Connection, task: &Task, with_handoff: bool) -> String {
    let mut line = format!(
        "  {}",
        output::format_task_line(task, tasks::progress(conn, &task.id).ok().flatten())
    );
    if with_handoff {
        if let Some(text) = handoff_text(conn, &task.id) {
            line.push_str(&format!("   last handoff: {}", quote(&text, 60)));
        }
    }
    line
}

/// The text of the task's most recent handoff, when there is one with something in it.
pub(crate) fn handoff_text(conn: &Connection, task_id: &str) -> Option<String> {
    let event = tasks::last_handoff(conn, task_id).ok().flatten()?;
    let text = event.payload.get("text")?.as_str()?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// One flattened, quoted line of at most `width` characters of text, so a handoff written as a
/// paragraph cannot blow the 40-line cap on its own.
pub(crate) fn quote(text: &str, width: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > width {
        let cut: String = flat.chars().take(width - 1).collect();
        format!("\"{cut}…\"")
    } else {
        format!("\"{flat}\"")
    }
}

/// Session identifiers are long; eight characters are enough to name one in a briefing.
#[allow(dead_code)] // consumed by Task 10
pub(crate) fn short(session_id: &str) -> String {
    if session_id.chars().count() > 8 {
        format!("{}…", session_id.chars().take(8).collect::<String>())
    } else {
        session_id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::model::{LaunchedBy, SessionMode};
    use crate::services::sessions::{self, StartInput};
    use crate::services::tasks::NewTask;

    fn at(s: &str) -> DateTime<Utc> {
        crate::clock::parse(s).unwrap()
    }

    /// A database with one registered session, and the session it registered.
    fn setup(session_id: &str) -> (Connection, Session) {
        let mut conn = db::open_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        let session = sessions::upsert_start(
            &mut conn,
            StartInput {
                session_id,
                repo: "demo",
                repo_root: "root",
                cwd: "root",
                worktree: None,
                branch: Some("main"),
                mode: SessionMode::Interactive,
                launched_by: LaunchedBy::User,
            },
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        (conn, session)
    }

    fn make(conn: &mut Connection, title: &str) -> String {
        tasks::create(
            conn,
            NewTask {
                title,
                body: "",
                repo: "demo",
                repo_root: "root",
                priority: 3,
                parent_id: None,
                tags: &[],
                checklist: &[],
            },
            crate::model::Source::Cli,
            None,
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap()
        .id
    }

    /// A task claimed by `sid`, for the subagent reminder tests.
    fn claim_one(conn: &mut Connection, sid: &str) -> String {
        let id = make(conn, "held");
        tasks::claim(
            conn,
            &id,
            sid,
            &Thresholds::default(),
            crate::model::Source::Cli,
            at("2026-09-16T12:01:00Z"),
        )
        .unwrap();
        id
    }

    /// A raw event row, for exercising the reminder queries directly rather than through a
    /// service call that always sets `task_id` to the current first held task.
    fn ev(conn: &Connection, sid: &str, tid: Option<&str>, kind: &str, payload: &str, ts: &str) {
        conn.execute(
            "INSERT INTO events(ts, session_id, task_id, kind, payload, source) VALUES (?1,?2,?3,?4,?5,'hook')",
            rusqlite::params![ts, sid, tid, kind, payload],
        )
        .unwrap();
    }

    #[test]
    fn a_quiet_repo_with_no_map_adds_the_map_none_line() {
        let (conn, session) = setup("session-abcdef0123");
        let text = build(
            &conn,
            &session,
            Path::new("root"),
            &Thresholds::default(),
            at("2026-09-16T12:01:00Z"),
        );
        assert_eq!(text.lines().count(), 2, "{text}");
        // G2-P7(a): `short("session-abcdef0123")` keeps the hyphen inside its 8-char window, so
        // the short form is "session-…", not "session…" as the brief's own assertion assumed.
        assert!(
            text.starts_with("[ratchet] repo demo · session session-…"),
            "{text}"
        );
        assert!(text.contains("branch main"), "{text}");
        assert!(
            text.contains("map: none — run /ratchet:map for the repo layout"),
            "{text}"
        );
    }

    #[test]
    fn agents_md_adds_exactly_one_rules_line_right_after_the_header() {
        let (conn, session) = setup("session-abcdef0123");
        let d = tempfile::TempDir::new().unwrap();
        std::fs::write(d.path().join("AGENTS.md"), "# doctrine\n").unwrap();
        let text = build(
            &conn,
            &session,
            d.path(),
            &Thresholds::default(),
            at("2026-09-16T12:01:00Z"),
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.get(1), Some(&"rules: AGENTS.md"), "{text}");
        assert_eq!(
            text.matches("AGENTS.md").count(),
            1,
            "the briefing points at the file once, it does not restate it: {text}"
        );
    }

    #[test]
    fn no_agents_md_means_no_rules_line() {
        let (conn, session) = setup("session-abcdef0123");
        let text = build(
            &conn,
            &session,
            Path::new("root"),
            &Thresholds::default(),
            at("2026-09-16T12:01:00Z"),
        );
        assert!(!text.contains("AGENTS.md"), "{text}");
    }

    #[test]
    fn a_quiet_repo_with_a_current_map_prints_exactly_one_line() {
        use std::process::{Command, Stdio};
        let (conn, session) = setup("session-abcdef0123");
        let d = tempfile::TempDir::new().unwrap();
        let git = |args: &[&str]| {
            let st = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(d.path())
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(st.success());
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["commit", "--allow-empty", "-q", "-m", "init"]);
        let generated = crate::map::generate(
            d.path(),
            &crate::config::MapSection::default(),
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        crate::map::write_map(d.path(), &generated).unwrap();

        let text = build(
            &conn,
            &session,
            d.path(),
            &Thresholds::default(),
            at("2026-09-16T12:01:00Z"),
        );
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(
            text.starts_with("[ratchet] repo demo · session session-…"),
            "{text}"
        );
    }

    #[test]
    fn the_briefing_groups_mine_orphans_and_ready() {
        let (mut conn, session) = setup("s-live");
        let now = at("2026-09-16T13:40:00Z");
        let mine = make(&mut conn, "what I am doing");
        tasks::claim(
            &mut conn,
            &mine,
            "s-live",
            &Thresholds::default(),
            crate::model::Source::Cli,
            at("2026-09-16T12:01:00Z"),
        )
        .unwrap();
        let ready = make(&mut conn, "free to take");
        tasks::transition(
            &mut conn,
            &ready,
            TaskStatus::Ready,
            crate::model::Source::Cli,
            None,
            None,
            false,
            at("2026-09-16T12:02:00Z"),
        )
        .unwrap();
        // A task held by a session that was never registered counts as orphaned.
        let lost = make(&mut conn, "left behind");
        tasks::transition(
            &mut conn,
            &lost,
            TaskStatus::Ready,
            crate::model::Source::Cli,
            None,
            None,
            false,
            at("2026-09-16T12:03:00Z"),
        )
        .unwrap();
        tasks::transition(
            &mut conn,
            &lost,
            TaskStatus::InProgress,
            crate::model::Source::Cli,
            None,
            None,
            false,
            at("2026-09-16T12:04:00Z"),
        )
        .unwrap();
        conn.execute(
            "UPDATE tasks SET claimed_by = 's-ghost' WHERE id = ?1",
            rusqlite::params![lost],
        )
        .unwrap();
        tasks::handoff(
            &mut conn,
            &lost,
            "stopped at the parser",
            crate::model::Source::Cli,
            Some("s-ghost"),
            at("2026-09-16T12:05:00Z"),
        )
        .unwrap();

        let text = build(
            &conn,
            &session,
            Path::new("root"),
            &Thresholds::default(),
            now,
        );
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("[ratchet] repo demo"), "{text}");
        let mine_at = lines
            .iter()
            .position(|l| l.contains("Your tasks in progress"))
            .unwrap();
        let orphan_at = lines
            .iter()
            .position(|l| l.contains("dead session"))
            .unwrap();
        let ready_at = lines
            .iter()
            .position(|l| l.contains("Ready to take"))
            .unwrap();
        assert!(mine_at < orphan_at && orphan_at < ready_at, "{text}");
        assert!(text.contains(&mine), "{text}");
        assert!(text.contains("stopped at the parser"), "{text}");
        assert!(text.contains("free to take"), "{text}");
        assert_eq!(*lines.last().unwrap(), COMMANDS_LINE);
    }

    #[test]
    fn at_most_five_ready_tasks_are_listed() {
        let (mut conn, session) = setup("s-1");
        for n in 1..=9 {
            let id = make(&mut conn, &format!("ready {n}"));
            tasks::transition(
                &mut conn,
                &id,
                TaskStatus::Ready,
                crate::model::Source::Cli,
                None,
                None,
                false,
                at("2026-09-16T12:01:00Z"),
            )
            .unwrap();
        }
        let text = build(
            &conn,
            &session,
            Path::new("root"),
            &Thresholds::default(),
            at("2026-09-16T12:02:00Z"),
        );
        // G2-P7(b): `format_task_line`'s `{:<12}` pads the status "ready" with trailing spaces,
        // so `text.matches("ready ").count()` double-counts (the padded status matches too),
        // giving 10, not 5. Assert on the titles directly: exactly the first five (lowest id,
        // since all nine share priority 3 and `list` orders by priority then id) are present.
        for n in 1..=5 {
            assert!(text.contains(&format!("ready {n}")), "{text}");
        }
        for n in 6..=9 {
            assert!(!text.contains(&format!("ready {n}")), "{text}");
        }
    }

    #[test]
    fn a_crowded_board_is_cut_to_forty_lines() {
        let (mut conn, session) = setup("s-1");
        for n in 1..=60 {
            let id = make(&mut conn, &format!("held {n}"));
            tasks::transition(
                &mut conn,
                &id,
                TaskStatus::Ready,
                crate::model::Source::Cli,
                None,
                None,
                false,
                at("2026-09-16T12:01:00Z"),
            )
            .unwrap();
            tasks::transition(
                &mut conn,
                &id,
                TaskStatus::InProgress,
                crate::model::Source::Cli,
                None,
                None,
                false,
                at("2026-09-16T12:02:00Z"),
            )
            .unwrap();
            conn.execute(
                "UPDATE tasks SET claimed_by = 's-ghost' WHERE id = ?1",
                rusqlite::params![id],
            )
            .unwrap();
        }
        let text = build(
            &conn,
            &session,
            Path::new("root"),
            &Thresholds::default(),
            at("2026-09-16T12:03:00Z"),
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), MAX_LINES, "{}", lines.len());
        assert!(
            lines[MAX_LINES - 2].contains("ratchet task list"),
            "{:?}",
            lines[MAX_LINES - 2]
        );
        assert_eq!(lines[MAX_LINES - 1], COMMANDS_LINE);
    }

    #[test]
    fn a_long_handoff_is_quoted_and_cut() {
        let text = quote(&"word ".repeat(40), 60);
        assert!(text.starts_with('"') && text.ends_with("…\""), "{text}");
        assert_eq!(text.chars().count(), 62);
        assert_eq!(quote("  two   spaces  ", 60), "\"two spaces\"");
    }

    #[test]
    fn no_claimed_task_means_no_line() {
        let (conn, session) = setup("s-1");
        assert_eq!(prompt_line(&conn, &session), None);
    }

    #[test]
    fn the_reminder_names_the_task_its_progress_and_its_last_handoff() {
        let (mut conn, session) = setup("s-1");
        let id = tasks::create(
            &mut conn,
            crate::services::tasks::NewTask {
                title: "the claimed one",
                body: "",
                repo: "demo",
                repo_root: "root",
                priority: 3,
                parent_id: None,
                tags: &[],
                checklist: &["a".to_string(), "b".to_string(), "c".to_string()],
            },
            crate::model::Source::Cli,
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
            crate::model::Source::Cli,
            at("2026-09-16T12:02:00Z"),
        )
        .unwrap();
        tasks::check(
            &mut conn,
            &id,
            1,
            crate::model::Source::Cli,
            Some("s-1"),
            at("2026-09-16T12:03:00Z"),
        )
        .unwrap();
        tasks::handoff(
            &mut conn,
            &id,
            "half way; resume at item 2",
            crate::model::Source::Cli,
            Some("s-1"),
            at("2026-09-16T12:04:00Z"),
        )
        .unwrap();
        let line = prompt_line(&conn, &session).unwrap();
        assert_eq!(line.lines().count(), 1, "{line}");
        assert!(line.starts_with("[ratchet] "), "{line}");
        assert!(line.contains(&id), "{line}");
        assert!(line.contains("in_progress"), "{line}");
        assert!(line.contains("(1/3)"), "{line}");
        assert!(line.contains("resume at item 2"), "{line}");
        assert!(!line.contains("more"), "{line}");
    }

    #[test]
    fn holding_more_than_one_task_says_how_many() {
        let (mut conn, session) = setup("s-1");
        for title in ["first", "second", "third"] {
            let id = make(&mut conn, title);
            tasks::claim(
                &mut conn,
                &id,
                "s-1",
                &Thresholds::default(),
                crate::model::Source::Cli,
                at("2026-09-16T12:01:00Z"),
            )
            .unwrap();
        }
        let line = prompt_line(&conn, &session).unwrap();
        assert!(line.contains("(+2 more)"), "{line}");
    }

    #[test]
    fn a_stopped_subagent_with_no_record_adds_a_reminder_line() {
        let (mut c, session) = setup("s-1");
        let id = claim_one(&mut c, "s-1");
        ev(
            &c,
            "s-1",
            None,
            "session.prompt",
            "{}",
            "2026-09-16T12:02:00Z",
        );
        ev(
            &c,
            "s-1",
            Some(&id),
            "subagent.stop",
            r#"{"agent_id":"abcdefgh1234"}"#,
            "2026-09-16T12:03:00Z",
        );
        let line = prompt_line(&c, &session).unwrap();
        assert_eq!(line.lines().count(), 2, "{line}");
        assert!(
            line.contains(&format!(
                "[ratchet] {id} · subagent abcdefgh stopped with no record"
            )),
            "{line}"
        );
    }

    #[test]
    fn a_trigger_prompt_does_not_swallow_the_stop_before_it() {
        let (mut c, session) = setup("s-1");
        let id = claim_one(&mut c, "s-1");
        ev(
            &c,
            "s-1",
            None,
            "session.prompt",
            "{}",
            "2026-09-16T12:02:00Z",
        );
        ev(
            &c,
            "s-1",
            Some(&id),
            "subagent.stop",
            r#"{"agent_id":"abcdefgh1234"}"#,
            "2026-09-16T12:03:00Z",
        );
        // The prompt that triggers this very reminder is itself the max `session.prompt` id;
        // the window has to start at the one before it or the stop above would never count.
        ev(
            &c,
            "s-1",
            None,
            "session.prompt",
            "{}",
            "2026-09-16T12:04:00Z",
        );
        let line = prompt_line(&c, &session).unwrap();
        assert_eq!(line.lines().count(), 2, "{line}");
        assert!(line.contains("stopped with no record"), "{line}");
    }

    #[test]
    fn a_stop_before_the_cutoff_is_silent() {
        let (mut c, session) = setup("s-1");
        let id = claim_one(&mut c, "s-1");
        ev(
            &c,
            "s-1",
            None,
            "session.prompt",
            "{}",
            "2026-09-16T12:02:00Z",
        );
        ev(
            &c,
            "s-1",
            Some(&id),
            "subagent.stop",
            r#"{"agent_id":"abcdefgh1234"}"#,
            "2026-09-16T12:03:00Z",
        );
        ev(
            &c,
            "s-1",
            None,
            "session.prompt",
            "{}",
            "2026-09-16T12:04:00Z",
        );
        ev(
            &c,
            "s-1",
            None,
            "session.prompt",
            "{}",
            "2026-09-16T12:05:00Z",
        );
        let line = prompt_line(&c, &session).unwrap();
        assert_eq!(line.lines().count(), 1, "{line}");
    }

    /// G2-P10 fix: a stop recorded against a task other than the one this reminder is scoped to
    /// (the first held task) is never surfaced under it, however it got its `task_id` — the query
    /// filters on the event's own column, not on whatever "first held task" was true when the
    /// event was recorded.
    #[test]
    fn a_stop_recorded_against_another_task_adds_no_line() {
        let (mut c, session) = setup("s-1");
        let id = claim_one(&mut c, "s-1");
        ev(
            &c,
            "s-1",
            None,
            "session.prompt",
            "{}",
            "2026-09-16T12:02:00Z",
        );
        ev(
            &c,
            "s-1",
            Some("T-elsewhere"),
            "subagent.stop",
            r#"{"agent_id":"abcdefgh1234"}"#,
            "2026-09-16T12:03:00Z",
        );
        let line = prompt_line(&c, &session).unwrap();
        assert_eq!(line.lines().count(), 1, "{line}");
        assert!(line.contains(&id), "{line}");
    }

    #[test]
    fn a_stop_with_a_note_after_it_is_silent() {
        let (mut c, session) = setup("s-1");
        let id = claim_one(&mut c, "s-1");
        ev(
            &c,
            "s-1",
            Some(&id),
            "subagent.stop",
            r#"{"agent_id":"abcdefgh1234"}"#,
            "2026-09-16T12:03:00Z",
        );
        ev(
            &c,
            "s-1",
            Some(&id),
            "note",
            r#"{"text":"recorded"}"#,
            "2026-09-16T12:04:00Z",
        );
        let line = prompt_line(&c, &session).unwrap();
        assert_eq!(line.lines().count(), 1, "{line}");
    }

    #[test]
    fn two_running_subagents_one_with_no_agent_id_both_show() {
        let (mut c, session) = setup("s-1");
        let id = claim_one(&mut c, "s-1");
        ev(
            &c,
            "s-1",
            Some(&id),
            "subagent.start",
            r#"{"agent_id":"zzzzzzzz9999"}"#,
            "2026-09-16T12:03:00Z",
        );
        ev(
            &c,
            "s-1",
            Some(&id),
            "subagent.start",
            r#"{}"#,
            "2026-09-16T12:04:00Z",
        );
        let line = prompt_line(&c, &session).unwrap();
        assert_eq!(line.lines().count(), 3, "{line}");
        assert!(
            line.contains(&format!("[ratchet] {id} · subagent zzzzzzzz running")),
            "{line}"
        );
        assert!(
            line.contains(&format!("[ratchet] {id} · subagent unknown running")),
            "{line}"
        );
    }

    #[test]
    fn a_started_then_stopped_subagent_is_not_running() {
        let (mut c, session) = setup("s-1");
        let id = claim_one(&mut c, "s-1");
        ev(
            &c,
            "s-1",
            Some(&id),
            "subagent.start",
            r#"{"agent_id":"a1"}"#,
            "2026-09-16T12:03:00Z",
        );
        ev(
            &c,
            "s-1",
            Some(&id),
            "subagent.stop",
            r#"{"agent_id":"a1"}"#,
            "2026-09-16T12:04:00Z",
        );
        let line = prompt_line(&c, &session).unwrap();
        assert_eq!(line.lines().count(), 2, "{line}");
        assert!(line.contains("stopped with no record"), "{line}");
        assert!(!line.contains("running"), "{line}");
    }
}
