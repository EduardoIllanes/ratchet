//! `ratchet task …`: parse, resolve the session, call a service, print. No SQL here, and no
//! decision that belongs to the board — the service owns every rule.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::clock;
use crate::config::{ratchet_home, Thresholds};
use crate::db;
use crate::model::{Source, TaskStatus};
use crate::output;
use crate::repo::{find_repo, normalize};
use crate::services::{events, sessions, tasks};

const VALID_STATUSES: &str = "backlog, ready, in_progress, blocked, review, done";

/// What every subcommand needs, opened once: the database, the repo of this directory when there
/// is one, its thresholds, and the instant this command runs at.
struct Face {
    env: HashMap<String, String>,
    home: PathBuf,
    cwd: PathBuf,
    conn: Connection,
    repo_name: Option<String>,
    repo_root: Option<String>,
    th: Thresholds,
    now: DateTime<Utc>,
}

fn face(env: &HashMap<String, String>, cwd: Option<PathBuf>) -> Result<Face, String> {
    let home = ratchet_home(env);
    let conn = db::open_ready(&home).map_err(|e| e.to_string())?;
    let cwd = cwd.unwrap_or_else(|| PathBuf::from("."));
    let repo = find_repo(&cwd).map_err(|e| e.to_string())?;
    let th = repo
        .as_ref()
        .map(|r| r.config.thresholds.clone())
        .unwrap_or_default();
    let repo_name = repo.as_ref().map(|r| r.name.clone());
    let repo_root = repo
        .as_ref()
        .map(|r| normalize(&r.main_root).to_string_lossy().to_string());
    Ok(Face {
        env: env.clone(),
        home,
        cwd,
        conn,
        repo_name,
        repo_root,
        th,
        now: clock::now(env),
    })
}

fn fail(e: impl std::fmt::Display) -> i32 {
    eprintln!("error: {e}");
    1
}

fn session_of(f: &Face, explicit: Option<&str>) -> Option<String> {
    sessions::resolve(&f.conn, explicit, &f.env, &f.cwd, &f.th, f.now)
        .ok()
        .flatten()
}

/// A write nobody can be attributed to still happens — the board would lose the note otherwise —
/// but it says so once on stderr.
fn attributed(f: &Face, explicit: Option<&str>) -> Option<String> {
    let session = session_of(f, explicit);
    if session.is_none() {
        eprintln!(
            "warning: no session resolved (use --session or RATCHET_SESSION_ID); recording with no session"
        );
    }
    session
}

/// `TaskStatus::from_db` is lenient by design, so a name it does not know comes back as `backlog`.
/// Here that would silently move a task: compare the round trip and refuse instead.
fn parse_status(text: &str) -> Result<TaskStatus, String> {
    let status = TaskStatus::from_db(text);
    if status.as_str() == text {
        Ok(status)
    } else {
        Err(format!("unknown status `{text}`; valid: {VALID_STATUSES}"))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn list(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    repo: Option<&str>,
    statuses: &[String],
    mine: bool,
    tag: Option<&str>,
    all: bool,
    json_out: bool,
) -> i32 {
    let f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let mut parsed = Vec::new();
    for text in statuses {
        match parse_status(text) {
            Ok(s) => parsed.push(s),
            Err(e) => return fail(e),
        }
    }
    let claimed = if mine {
        match session_of(&f, session) {
            Some(id) => Some(id),
            None => return fail("--mine needs a session (use --session or RATCHET_SESSION_ID)"),
        }
    } else {
        None
    };
    // Inside a repo the board is that repo's; from outside one, or with an explicit --repo, the
    // listing spans the machine.
    let scope = if repo.is_some() {
        None
    } else {
        f.repo_root.clone()
    };
    let filter = tasks::Filter {
        repo_root: scope.as_deref(),
        repo,
        statuses: &parsed,
        claimed_by: claimed.as_deref(),
        tag,
        include_archived: all,
    };
    let found = match tasks::list(&f.conn, &filter) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    if json_out {
        let payload: Vec<Value> = found
            .iter()
            .map(|t| {
                let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
                if let Some(map) = v.as_object_mut() {
                    map.insert(
                        "progress".into(),
                        json!(tasks::progress(&f.conn, &t.id).ok().flatten()),
                    );
                }
                v
            })
            .collect();
        output::emit_json(&f.home, "task-list", &json!(payload), f.now);
        return 0;
    }
    let lines: Vec<String> = found
        .iter()
        .map(|t| output::format_task_line(t, tasks::progress(&f.conn, &t.id).ok().flatten()))
        .collect();
    output::emit(&f.home, "task-list", &lines, f.now);
    0
}

pub fn show(env: &HashMap<String, String>, cwd: Option<PathBuf>, id: &str, json_out: bool) -> i32 {
    let f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let task = match tasks::get(&f.conn, id) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let items = match tasks::checklist(&f.conn, id) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let progress = tasks::progress(&f.conn, id).ok().flatten();
    let last = tasks::last_handoff(&f.conn, id).ok().flatten();
    // The detail view shows the last ten events and no more (spec §4.1).
    let history = events::for_task(&f.conn, id, 10).unwrap_or_default();
    if json_out {
        let payload = json!({
            "task": task,
            "checklist": items,
            "progress": progress,
            "last_handoff": last,
            "events": history,
        });
        output::emit_json(&f.home, &format!("task-{id}"), &payload, f.now);
        return 0;
    }
    let mut lines = vec![
        format!("{}  {}", task.id, task.title),
        format!(
            "status {} · repo {} · priority p{}{}{}{}",
            task.status.as_str(),
            task.repo,
            task.priority,
            if task.archived_at.is_some() {
                " · archived"
            } else {
                ""
            },
            task.claimed_by
                .as_ref()
                .map(|s| format!(" · session {s}"))
                .unwrap_or_default(),
            if task.tags.is_empty() {
                String::new()
            } else {
                format!(" · tags {}", task.tags.join(", "))
            }
        ),
    ];
    if let Some((done, total)) = progress {
        lines.push(format!("progress {done}/{total}"));
    }
    if !task.body.trim().is_empty() {
        lines.push(String::new());
        lines.extend(task.body.trim_end().lines().map(str::to_string));
    }
    if !items.is_empty() {
        lines.push(String::new());
        for item in &items {
            lines.push(format!(
                "  {}. [{}] {}",
                item.position,
                if item.done { "x" } else { " " },
                item.text
            ));
        }
    }
    if let Some(ev) = &last {
        lines.push(String::new());
        lines.push(format!(
            "last handoff ({}): {}",
            clock::iso(ev.ts),
            ev.payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        ));
    }
    if !history.is_empty() {
        lines.push(String::new());
        lines.push("events:".to_string());
        for ev in &history {
            lines.push(format!(
                "  {}  {:<16} {}",
                clock::iso(ev.ts),
                ev.kind,
                summary(&ev.payload)
            ));
        }
    }
    output::emit(&f.home, &format!("task-{id}"), &lines, f.now);
    0
}

/// One short line for an event payload: its text when it has one, else its non-null keys.
fn summary(payload: &Value) -> String {
    if let Some(text) = payload.get("text").and_then(|v| v.as_str()) {
        return text.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    match payload.as_object() {
        None => String::new(),
        Some(map) => map
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| format!("{k}={}", v.to_string().trim_matches('"')))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn new(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    title: &str,
    body: Option<&str>,
    body_file: Option<PathBuf>,
    checks: &[String],
    priority: i64,
    tags: &[String],
    parent: Option<&str>,
    json_out: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let (Some(repo_name), Some(repo_root)) = (f.repo_name.clone(), f.repo_root.clone()) else {
        // There is no registry of repos (spec D-marker): a task belongs to the marker above it.
        return fail(
            "a task belongs to a repo: run this from inside a repo with a ratchet.toml at its root",
        );
    };
    let body_text = match (body, body_file) {
        (_, Some(path)) => match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => return fail(format!("{}: {e}", path.display())),
        },
        (Some(t), None) => t.to_string(),
        (None, None) => String::new(),
    };
    let session_id = attributed(&f, session);
    let created = tasks::create(
        &mut f.conn,
        tasks::NewTask {
            title,
            body: &body_text,
            repo: &repo_name,
            repo_root: &repo_root,
            priority,
            parent_id: parent,
            tags,
            checklist: checks,
        },
        Source::Cli,
        session_id.as_deref(),
        f.now,
    );
    match created {
        Err(e) => fail(e),
        Ok(task) => {
            if json_out {
                output::emit_json(&f.home, "task-new", &json!(task), f.now);
            } else {
                println!("{}  {}  (backlog)", task.id, task.title);
            }
            0
        }
    }
}

pub fn claim(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    json_out: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let Some(session_id) = session_of(&f, session) else {
        return fail("claiming needs a session (use --session or RATCHET_SESSION_ID)");
    };
    match tasks::claim(&mut f.conn, id, &session_id, &f.th, Source::Cli, f.now) {
        Err(e) => fail(e),
        Ok(task) => {
            if json_out {
                output::emit_json(&f.home, "task-claim", &json!(task), f.now);
            } else {
                println!(
                    "{} claimed by {session_id} ({})",
                    task.id,
                    task.status.as_str()
                );
            }
            0
        }
    }
}

pub fn status(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    to: &str,
    why: Option<&str>,
    json_out: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let to = match parse_status(to) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let session_id = attributed(&f, session);
    match tasks::transition(
        &mut f.conn,
        id,
        to,
        Source::Cli,
        session_id.as_deref(),
        why,
        f.now,
    ) {
        Err(e) => fail(e),
        Ok(task) => {
            if json_out {
                output::emit_json(&f.home, "task-status", &json!(task), f.now);
            } else {
                println!("{} → {}", task.id, task.status.as_str());
            }
            0
        }
    }
}

pub fn check(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    position: i64,
    undo: bool,
    json_out: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let session_id = attributed(&f, session);
    let result = if undo {
        tasks::uncheck(
            &mut f.conn,
            id,
            position,
            Source::Cli,
            session_id.as_deref(),
            f.now,
        )
    } else {
        tasks::check(
            &mut f.conn,
            id,
            position,
            Source::Cli,
            session_id.as_deref(),
            f.now,
        )
    };
    match result {
        Err(e) => fail(e),
        Ok(item) => {
            if json_out {
                output::emit_json(&f.home, "task-check", &json!(item), f.now);
            } else {
                let progress = tasks::progress(&f.conn, id)
                    .ok()
                    .flatten()
                    .map(|(d, t)| format!("  ({d}/{t})"))
                    .unwrap_or_default();
                println!(
                    "{id} [{}] {}. {}{progress}",
                    if item.done { "x" } else { " " },
                    item.position,
                    item.text
                );
            }
            0
        }
    }
}

pub fn note(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    text: &str,
    json_out: bool,
) -> i32 {
    record(env, cwd, session, id, text, json_out, false)
}

pub fn handoff(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    text: &str,
    json_out: bool,
) -> i32 {
    record(env, cwd, session, id, text, json_out, true)
}

fn record(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    text: &str,
    json_out: bool,
    is_handoff: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let session_id = attributed(&f, session);
    let result = if is_handoff {
        tasks::handoff(
            &mut f.conn,
            id,
            text,
            Source::Cli,
            session_id.as_deref(),
            f.now,
        )
    } else {
        tasks::note(
            &mut f.conn,
            id,
            text,
            Source::Cli,
            session_id.as_deref(),
            f.now,
        )
    };
    match result {
        Err(e) => fail(e),
        Ok(ev) => {
            if json_out {
                output::emit_json(&f.home, "task-record", &json!(ev), f.now);
            } else if is_handoff {
                println!("{id} handoff recorded");
            } else {
                println!("{id} note recorded");
            }
            0
        }
    }
}

pub fn archive(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    json_out: bool,
) -> i32 {
    shelve(env, cwd, session, id, json_out, true)
}

pub fn unarchive(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    json_out: bool,
) -> i32 {
    shelve(env, cwd, session, id, json_out, false)
}

fn shelve(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    json_out: bool,
    hide: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let session_id = session_of(&f, session);
    let result = if hide {
        tasks::archive(&mut f.conn, id, Source::Cli, session_id.as_deref(), f.now)
    } else {
        tasks::unarchive(&mut f.conn, id, Source::Cli, session_id.as_deref(), f.now)
    };
    match result {
        Err(e) => fail(e),
        Ok(task) => {
            if json_out {
                output::emit_json(&f.home, "task-archive", &json!(task), f.now);
            } else if hide {
                println!(
                    "{} archived (back with: ratchet task unarchive {})",
                    task.id, task.id
                );
            } else {
                println!("{} unarchived ({})", task.id, task.status.as_str());
            }
            0
        }
    }
}
