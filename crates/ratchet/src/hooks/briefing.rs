//! The `[ratchet]` briefing printed on session start, and (Task 10) the one-line reminder printed
//! on every prompt. Reads only: a briefing that failed must never cost a registration, so every
//! query here falls back to "nothing" instead of propagating an error.

use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::Connection;

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
        return match &map_line {
            Some(l) => format!("{header}\n{l}"),
            None => header,
        };
    }
    let mut lines = vec![header];
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
    if let Some(l) = &map_line {
        if lines.len() < MAX_LINES {
            lines.insert(1, l.clone());
        }
    }
    if lines.len() > MAX_LINES {
        lines.truncate(MAX_LINES - 2);
        lines.push("  … (more in `ratchet task list`)".to_string());
        lines.push(COMMANDS_LINE.to_string());
    }
    lines.join("\n")
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
    let task = mine.first()?;
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
    Some(format!(
        "[ratchet] {} {}{progress}{handoff}{more}",
        task.id,
        task.status.as_str()
    ))
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
}
