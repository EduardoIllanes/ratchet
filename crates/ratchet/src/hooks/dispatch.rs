//! Hook payload parsing and per-event handlers. Errors bubble up as `String`; `hooks::run`
//! turns them into exit 0 + log line.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::Value;

use super::briefing;
use super::handoff_rule;
use crate::clock;
use crate::db;
use crate::guardrails::eval::{evaluate, scratchpad_from_env, GuardContext};
use crate::guardrails::main_tree;
use crate::guardrails::rules::load_rule_set;
use crate::model::{EventKind, LaunchedBy, Session, SessionMode, Source};
use crate::repo::{find_repo, git_branch, has_venv, normalize, status_paths, within, Repo};
use crate::services::events;
use crate::services::sessions::{self, StartInput};
use crate::services::{tasks, ServiceError};

pub const KNOWN_EVENTS: [&str; 8] = [
    "session-start",
    "prompt",
    "pre-tool",
    "post-tool",
    "stop",
    "subagent-stop",
    "pre-compact",
    "session-end",
];

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

pub fn parse_payload(text: &str) -> Result<Payload, String> {
    if text.trim().is_empty() {
        return Ok(Payload::default());
    }
    serde_json::from_str(text).map_err(|e| format!("invalid payload: {e}"))
}

pub fn is_known_event(event: &str) -> bool {
    KNOWN_EVENTS.contains(&event)
}

/// Returns the exit code. `Err` means an internal error the caller logs and maps to 0.
pub fn dispatch(
    event: &str,
    payload: Payload,
    env: &HashMap<String, String>,
    process_cwd: Option<PathBuf>,
    home: &Path,
) -> Result<i32, String> {
    if !is_known_event(event) {
        return Err(format!("unknown event `{event}`"));
    }
    let cwd = payload.cwd.clone().or(process_cwd).ok_or("no cwd")?;
    match event {
        "pre-tool" => pre_tool(payload, env, &cwd, home),
        "post-tool" => post_tool(&payload, env, &cwd, home),
        "session-start" => session_start(&payload, env, &cwd, home),
        "prompt" => prompt(&payload, env, &cwd, home),
        // The handoff rule of group 2 goes on top of this heartbeat, and only it may return 2.
        "stop" => stop(&payload, env, &cwd, home),
        "subagent-stop" | "pre-compact" => heartbeat(&payload, env, &cwd, home, None),
        "session-end" => session_end(&payload, env, &cwd, home),
        _ => Ok(0),
    }
}

pub fn pre_tool(
    payload: Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
) -> Result<i32, String> {
    let Some(repo) = find_repo(cwd).map_err(|e| e.to_string())? else {
        return Ok(0);
    };
    let set = load_rule_set(home, Some(&repo)).map_err(|e| e.to_string())?;
    let ctx = GuardContext {
        main_root: Some(repo.main_root.clone()),
        worktrees_dir: Some(repo.worktrees_dir.clone()),
        cwd: cwd.to_path_buf(),
        has_venv: has_venv(cwd, &repo.main_root),
        scratchpad: scratchpad_from_env(env),
    };
    let rules: Vec<_> = set.active().collect();
    match evaluate(&rules, &payload.tool_name, &payload.tool_input, &ctx) {
        Some(v) => {
            eprintln!("{}", v.render());
            Ok(super::BLOCK)
        }
        None => {
            // The "before" half of the main-tree post-check: only a command that is actually
            // going to run is worth a snapshot, and only when there is a session to key it by.
            if is_command_tool(&payload.tool_name) {
                if let Some(session_id) = session_identity(&payload, env) {
                    record_snapshot(home, &session_id, &repo.main_root);
                }
            }
            Ok(0)
        }
    }
}

fn is_command_tool(tool_name: &str) -> bool {
    tool_name == "Bash" || tool_name == "PowerShell"
}

/// One file per session under `<home>/main-tree-snapshots/`, holding the tracked-file paths
/// `git status --porcelain --untracked-files=no` showed as changed right before a `Bash`/
/// `PowerShell` call. A file is the simplest thing that survives across the pre-tool and
/// post-tool process invocations without a database migration.
fn snapshot_dir(home: &Path) -> PathBuf {
    home.join("main-tree-snapshots")
}

fn snapshot_path(home: &Path, session_id: &str) -> PathBuf {
    let safe: String = session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    snapshot_dir(home).join(format!("{safe}.txt"))
}

/// Best-effort: a write failure here only costs the after-the-fact report, never the hook
/// itself (the "hooks never break a session" rule).
fn record_snapshot(home: &Path, session_id: &str, main_root: &Path) {
    let paths = status_paths(main_root);
    if fs::create_dir_all(snapshot_dir(home)).is_ok() {
        let _ = fs::write(snapshot_path(home, session_id), paths.join("\n"));
    }
}

/// Reads and removes (consumes) the snapshot recorded for `session_id`. `None` means no
/// PreToolUse ever recorded one for this session's command, or it was already consumed.
fn take_snapshot(home: &Path, session_id: &str) -> Option<Vec<String>> {
    let path = snapshot_path(home, session_id);
    let text = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(&path);
    Some(
        text.lines()
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

/// Main-tree writes detected after the fact: never blocks, stays silent unless a tracked file
/// of the main tree changed during the command.
fn post_tool(
    payload: &Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
) -> Result<i32, String> {
    if !is_command_tool(&payload.tool_name) {
        return Ok(0);
    }
    let Some(repo) = find_repo(cwd).map_err(|e| e.to_string())? else {
        return Ok(0);
    };
    let Some(session_id) = session_identity(payload, env) else {
        return Ok(0);
    };
    let Some(before) = take_snapshot(home, &session_id) else {
        return Ok(0);
    };
    let after = status_paths(&repo.main_root); // the hook's one `git status`
    let changed = main_tree::newly_changed(&before, &after);
    if changed.is_empty() {
        return Ok(0);
    }
    let set = load_rule_set(home, Some(&repo)).map_err(|e| e.to_string())?;
    // A repo may disable `main-tree` (or replace it); honour that the same way the pre-tool
    // check does rather than reporting against a rule the repo turned off.
    let Some(rule) = set.active().find(|r| r.id == "main-tree") else {
        return Ok(0);
    };
    let now = clock::now(env);
    let conn = db::open_ready(home).map_err(|e| e.to_string())?;
    let command = payload
        .tool_input
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("");
    let ev_payload = serde_json::json!({ "command": command, "files": changed });
    events::emit(
        &conn,
        EventKind::GuardrailMainTreeWrite,
        &ev_payload,
        Source::Hook,
        Some(session_id.as_str()),
        None,
        now,
    )
    .map_err(|e| e.to_string())?;
    eprintln!("{}", main_tree::render_post_check(rule, &changed));
    Ok(0)
}

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
    let session =
        register(&mut conn, &repo, &session_id, cwd, env, now).map_err(|e| e.to_string())?;
    export_session_id(env, &session.id);

    // The briefing is built HERE, with this same `now`, BEFORE the sweep below, so an orphaned
    // task is shown once with its last handoff while it is still claimed. Do not move the sweep
    // above this line.
    println!(
        "{}",
        briefing::build(&conn, &session, &repo.config.thresholds, now)
    );

    tasks::release_dead(
        &mut conn,
        &session.repo_root,
        &repo.config.thresholds,
        "the start of the next session",
        now,
    )
    .map_err(|e| e.to_string())?;
    Ok(0)
}

/// What a hook has after the heartbeat: the open database and the session it just refreshed.
struct Beat {
    conn: Connection,
    session: Session,
}

/// The heartbeat every hook that is not `session-start` performs. `Ok(None)` means there was
/// nothing to do — no marker above `cwd`, or no identity the harness fixed — and the caller exits
/// 0 without opening anything else.
fn beat(
    payload: &Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
    kind: Option<EventKind>,
) -> Result<Option<Beat>, String> {
    let Some(repo) = find_repo(cwd).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let Some(session_id) = session_identity(payload, env) else {
        return Ok(None);
    };
    let now = clock::now(env);
    let mut conn = db::open_ready(home).map_err(|e| e.to_string())?;
    ensure_session(&mut conn, &repo, &session_id, cwd, env, now).map_err(|e| e.to_string())?;
    let session = sessions::touch(&mut conn, &session_id, kind, now).map_err(|e| e.to_string())?;
    Ok(Some(Beat { conn, session }))
}

fn heartbeat(
    payload: &Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
    kind: Option<EventKind>,
) -> Result<i32, String> {
    beat(payload, env, cwd, home, kind)?;
    Ok(0)
}

/// Heartbeat, then the reminder. Printing nothing is the normal case.
fn prompt(
    payload: &Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
) -> Result<i32, String> {
    let Some(b) = beat(payload, env, cwd, home, Some(EventKind::SessionPrompt))? else {
        return Ok(0);
    };
    if let Some(line) = briefing::prompt_line(&b.conn, &b.session) {
        println!("{line}");
    }
    Ok(0)
}

/// Heartbeat, then the handoff rule. The only place in this group that returns a non-zero code.
/// The order matters: the heartbeat writes `session.stop` **before** the rule reads, and the
/// rule's window is the last `session.prompt`, so the heartbeat cannot exempt the session from
/// its own rule. An `Err` from the heartbeat (an unusable database) propagates to `hooks::run`,
/// which logs it and exits 0 — a broken database never blocks a close.
fn stop(
    payload: &Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
) -> Result<i32, String> {
    let Some(b) = beat(payload, env, cwd, home, Some(EventKind::SessionStop))? else {
        return Ok(0);
    };
    match handoff_rule::should_block_stop(
        &b.conn,
        &b.session.id,
        payload.stop_hook_active,
        b.session.mode,
    ) {
        Some(message) => {
            eprintln!("[ratchet] {message}");
            Ok(super::BLOCK)
        }
        None => Ok(0),
    }
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
    tasks::release_dead(
        &mut conn,
        &session.repo_root,
        &repo.config.thresholds,
        "end of session",
        now,
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_payload() {
        let p = parse_payload(r#"{"tool_name":"Bash","tool_input":{"command":"ls"},"cwd":"C:/r","hook_event_name":"PreToolUse","stop_hook_active":true}"#).unwrap();
        assert_eq!(p.tool_name, "Bash");
        assert_eq!(p.tool_input["command"], "ls");
        assert_eq!(p.cwd.as_deref(), Some(std::path::Path::new("C:/r")));
        assert!(p.stop_hook_active);
    }

    #[test]
    fn empty_stdin_is_a_default_payload() {
        let p = parse_payload("   ").unwrap();
        assert_eq!(p.tool_name, "");
        assert!(p.cwd.is_none());
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(parse_payload("not json").is_err());
    }

    #[test]
    fn known_events_are_recognised() {
        for e in [
            "session-start",
            "prompt",
            "pre-tool",
            "post-tool",
            "stop",
            "subagent-stop",
            "pre-compact",
            "session-end",
        ] {
            assert!(is_known_event(e), "{e}");
        }
        assert!(!is_known_event("no-such-event"));
    }

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
        assert_eq!(
            session_identity(&with_id, &env).as_deref(),
            Some("from-payload")
        );
        assert_eq!(
            session_identity(&without, &env).as_deref(),
            Some("from-env")
        );
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
            vec![
                "export RATCHET_SESSION_ID=s-1",
                "export RATCHET_SESSION_ID=s-2"
            ]
        );
        export_session_id(&HashMap::new(), "s-3"); // no variable: silent no-op
    }

    #[test]
    fn a_branch_is_read_from_a_real_repository_or_is_none() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(
            crate::repo::git_branch(dir.path()),
            None,
            "not a repository"
        );
    }
}
