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
use crate::services::{events, pending_calls, sessions, tasks};
use crate::usage::transcript;

const VALID_STATUSES: &str = "backlog, ready, in_progress, blocked, review, done";
const VALID_VERDICTS: &str = "approve, changes";

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

/// `Err` here is never "no session" (that is `Ok(None)`) — it is `sessions::resolve` refusing a
/// `--session`/`RATCHET_SESSION_ID` value outright (T-0016: one shaped like a pair), and every
/// caller must surface it as the command's own failure, not silently fall back to no session.
fn session_of(f: &Face, explicit: Option<&str>) -> Result<Option<String>, String> {
    sessions::resolve(&f.conn, explicit, &f.env, &f.cwd, &f.th, f.now).map_err(|e| e.to_string())
}

/// A write nobody can be attributed to still happens — the board would lose the note otherwise —
/// but it says so once on stderr.
fn attributed(f: &Face, explicit: Option<&str>) -> Result<Option<String>, String> {
    let session = session_of(f, explicit)?;
    if session.is_none() {
        eprintln!(
            "warning: no session resolved (use --session or RATCHET_SESSION_ID); recording with no session"
        );
    }
    Ok(session)
}

/// Resolves the subagent (if any) T-0016's `pending_calls::resolve` attributes this board write
/// to: `session_id` is what `session_of`/`attributed` already resolved, `subcommand` is the
/// write's own CLI word (`"claim"`, `"check"`, `"note"`, `"handoff"`, `"review"` or `"status"`),
/// and `verdict` is `Some` only for a review. `None` with no session at all — there is nothing to
/// search — or on any database error (never fails the write itself over an attribution lookup).
fn agent_of(
    f: &Face,
    session_id: Option<&str>,
    task_id: &str,
    subcommand: &str,
    verdict: Option<&str>,
) -> Option<(String, String)> {
    let session_id = session_id?;
    pending_calls::resolve(&f.conn, session_id, task_id, subcommand, verdict)
        .ok()
        .flatten()
}

/// Borrows out of `agent_of`'s owned pair, for the `Option<(&str, &str)>` every attributed
/// service function takes.
fn agent_ref(agent: &Option<(String, String)>) -> Option<(&str, &str)> {
    agent.as_ref().map(|(id, ty)| (id.as_str(), ty.as_str()))
}

/// Whether a task's checklist evidence is already in place for a `done` move — the same test
/// `require_done_evidence` runs inside `transition`, read-only, so `require_reviewer_transcript`
/// below only spends effort on H3 when the move has a real chance of otherwise succeeding (an
/// incomplete checklist should be refused for THAT reason, not an unrelated transcript message).
fn done_evidence_ready(f: &Face, task_id: &str, why: Option<&str>) -> bool {
    match tasks::progress(&f.conn, task_id) {
        Ok(Some((done, total))) => done == total,
        Ok(None) => why.map(|w| !w.trim().is_empty()).unwrap_or(false),
        Err(_) => false,
    }
}

/// T-0016's H3: before a `done` move actually happens, the approving identity's own Claude Code
/// transcript must contain the review command it claims to have run — otherwise a forged
/// pending-call match (crafted command text), or a hand-registered session with no real Claude
/// Code process behind it at all, could approve without ever having reviewed anything. Applies to
/// every approving identity, paired or bare (see the review that flagged the bare-session gap on
/// T-0016's board): a paired identity's proof lives in that subagent's own `agent-<id>.jsonl`
/// (`transcript::read_identity_transcript` with `Some(agent_id)`, as before); a bare identity's
/// proof lives in the session's own `<session>.jsonl` (`None`) — `identity_transcript_path`
/// already picks the right file for either case, so only the early return that used to skip a
/// bare identity entirely needed to go.
///
/// Every early exit here is deliberately `Ok(())`, never a refusal of its own: this function only
/// ever ADDS a refusal on top of what `transition` would already decide: if `reviewer_identity_
/// for_done` itself errs, or there is no projects dir, or no session row at all, this falls
/// through silently and lets `transition`'s own `require_independent_review` raise the
/// authoritative, better-worded error a moment later. This is a preview, not a second source of
/// truth. Once an identity is resolved and its session found, though, a missing or non-matching
/// transcript IS the refusal this function exists to raise — that is the bare-session bypass H3
/// closes, so it cannot fall through silently the way the exits above do.
fn require_reviewer_transcript(f: &Face, task_id: &str) -> Result<(), String> {
    let identity = match tasks::reviewer_identity_for_done(&f.conn, task_id) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let session = match sessions::get(&f.conn, &identity.session_id) {
        Ok(Some(s)) => s,
        _ => return Ok(()),
    };
    let Ok(projects) = transcript::projects_dir(&f.env) else {
        return Ok(());
    };
    let Ok(projects_canon) = projects.canonicalize() else {
        return Ok(());
    };
    let needle = format!("task review {task_id}");
    let found = transcript::read_identity_transcript(
        &projects,
        &projects_canon,
        &session.cwd,
        &identity.session_id,
        identity.agent_id.as_deref(),
    )
    .map(|bytes| transcript::contains_bash_command(&bytes, &needle))
    .unwrap_or(false);
    if found {
        Ok(())
    } else {
        Err(format!(
            "{task_id}: the approving identity's transcript carries no matching review command \
             — expected a Bash tool call containing `task review {task_id}`"
        ))
    }
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

/// Only two words are ever stored, so this stays a plain match rather than another `db_enum!`.
fn parse_verdict(text: &str) -> Result<&'static str, String> {
    match text {
        "approve" => Ok("approve"),
        "changes" => Ok("changes"),
        _ => Err(format!("unknown verdict `{text}`; valid: {VALID_VERDICTS}")),
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
            Ok(Some(id)) => Some(id),
            Ok(None) => {
                return fail("--mine needs a session (use --session or RATCHET_SESSION_ID)")
            }
            Err(e) => return fail(e),
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
                "  {}  {:<16} {}{}",
                clock::iso(ev.ts),
                ev.kind,
                summary(&ev.payload),
                agent_suffix(&ev.agent_id, &ev.payload)
            ));
        }
    }
    output::emit(&f.home, &format!("task-{id}"), &lines, f.now);
    0
}

/// T-0016: `" · <agent type> <first 8 chars of the agent id>"` on an event attributed to a
/// subagent, empty for the bare session — exactly what `task show`'s history is asked to add
/// alongside the session on an attributed event. `agent_type` rides in the payload (`events::
/// emit_attributed` put it there); a plain first-8-chars, not `short_session`'s "…"-truncated
/// style, since the spec asks for the identifier's first eight characters specifically.
fn agent_suffix(agent_id: &Option<String>, payload: &Value) -> String {
    let Some(agent_id) = agent_id.as_deref() else {
        return String::new();
    };
    let agent_type = payload
        .get("agent_type")
        .and_then(Value::as_str)
        .unwrap_or("agent");
    let short: String = agent_id.chars().take(8).collect();
    format!(" · {agent_type} {short}")
}

/// One short line for an event payload: its text when it has one, else its non-null keys.
/// A `review.verdict` payload carries both `verdict` and `text`; the verdict word leads the line
/// so `approve` and `changes` are told apart at a glance, not swallowed by the generic `text` case
/// below.
fn summary(payload: &Value) -> String {
    if let Some(verdict) = payload.get("verdict").and_then(|v| v.as_str()) {
        let text = payload
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        return if text.is_empty() {
            verdict.to_string()
        } else {
            format!("{verdict}: {text}")
        };
    }
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
    let session_id = match attributed(&f, session) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
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
    let session_id = match session_of(&f, session) {
        Ok(Some(v)) => v,
        Ok(None) => return fail("claiming needs a session (use --session or RATCHET_SESSION_ID)"),
        Err(e) => return fail(e),
    };
    let agent = agent_of(&f, Some(&session_id), id, "claim", None);
    match tasks::claim_attributed(
        &mut f.conn,
        id,
        &session_id,
        agent_ref(&agent),
        &f.th,
        Source::Cli,
        f.now,
    ) {
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

#[allow(clippy::too_many_arguments)]
pub fn status(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    to: &str,
    why: Option<&str>,
    unreviewed: bool,
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
    let session_id = match attributed(&f, session) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    if to == TaskStatus::Done && !unreviewed && done_evidence_ready(&f, id, why) {
        if let Err(e) = require_reviewer_transcript(&f, id) {
            return fail(e);
        }
    }
    let agent = agent_of(&f, session_id.as_deref(), id, "status", None);
    match tasks::transition_attributed(
        &mut f.conn,
        id,
        to,
        Source::Cli,
        session_id.as_deref(),
        agent_ref(&agent),
        why,
        unreviewed,
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

/// `positions` holds one or more item numbers, in the order given on the command line; every one
/// is validated before anything is written (T-0010's batching requirement — see
/// `tasks::check_many`/`uncheck_many`). A single position behaves exactly as before.
pub fn check(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    positions: &[i64],
    undo: bool,
    json_out: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let session_id = match attributed(&f, session) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let agent = agent_of(&f, session_id.as_deref(), id, "check", None);
    let result = if undo {
        tasks::uncheck_many_attributed(
            &mut f.conn,
            id,
            positions,
            Source::Cli,
            session_id.as_deref(),
            agent_ref(&agent),
            f.now,
        )
    } else {
        tasks::check_many_attributed(
            &mut f.conn,
            id,
            positions,
            Source::Cli,
            session_id.as_deref(),
            agent_ref(&agent),
            f.now,
        )
    };
    match result {
        Err(e) => fail(e),
        Ok(items) => {
            if json_out {
                output::emit_json(&f.home, "task-check", &json!(items), f.now);
            } else {
                let progress = tasks::progress(&f.conn, id)
                    .ok()
                    .flatten()
                    .map(|(d, t)| format!("  ({d}/{t})"))
                    .unwrap_or_default();
                for item in &items {
                    println!(
                        "{id} [{}] {}. {}{progress}",
                        if item.done { "x" } else { " " },
                        item.position,
                        item.text
                    );
                }
            }
            0
        }
    }
}

/// Records an independent review verdict. Never touches status: `done` is what reads this back.
#[allow(clippy::too_many_arguments)]
pub fn review(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    verdict: &str,
    text: &str,
    json_out: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let verdict = match parse_verdict(verdict) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let session_id = match attributed(&f, session) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let agent = agent_of(&f, session_id.as_deref(), id, "review", Some(verdict));
    match tasks::review_attributed(
        &mut f.conn,
        id,
        verdict,
        text,
        Source::Cli,
        session_id.as_deref(),
        agent_ref(&agent),
        f.now,
    ) {
        Err(e) => fail(e),
        Ok(ev) => {
            if json_out {
                output::emit_json(&f.home, "task-review", &json!(ev), f.now);
            } else {
                println!("{id} review recorded: {verdict}");
            }
            0
        }
    }
}

/// `texts` holds one or more note texts, in the order given on the command line, each its own
/// `note` event (T-0010). A single text behaves exactly as before.
pub fn note(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    texts: &[String],
    json_out: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let session_id = match attributed(&f, session) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let agent = agent_of(&f, session_id.as_deref(), id, "note", None);
    match tasks::note_many_attributed(
        &mut f.conn,
        id,
        texts,
        Source::Cli,
        session_id.as_deref(),
        agent_ref(&agent),
        f.now,
    ) {
        Err(e) => fail(e),
        Ok(events) => {
            if json_out {
                output::emit_json(&f.home, "task-record", &json!(events), f.now);
            } else {
                for _ in &events {
                    println!("{id} note recorded");
                }
            }
            0
        }
    }
}

/// Records the handoff, then — when `status` is given — attempts the same transition
/// `ratchet task status <id> <status>` would, forwarding `why` and `unreviewed`. The handoff is
/// recorded regardless of what the transition does; a refused transition exits with that
/// transition's own error (T-0010). With no `status`, behaves exactly as before this requirement
/// existed.
#[allow(clippy::too_many_arguments)]
pub fn handoff(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    text: &str,
    status: Option<&str>,
    why: Option<&str>,
    unreviewed: bool,
    json_out: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let session_id = match attributed(&f, session) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let handoff_agent = agent_of(&f, session_id.as_deref(), id, "handoff", None);
    let handoff_ev = match tasks::handoff_attributed(
        &mut f.conn,
        id,
        text,
        Source::Cli,
        session_id.as_deref(),
        agent_ref(&handoff_agent),
        f.now,
    ) {
        Err(e) => return fail(e),
        Ok(ev) => ev,
    };
    let Some(status) = status else {
        if json_out {
            output::emit_json(&f.home, "task-record", &json!(handoff_ev), f.now);
        } else {
            println!("{id} handoff recorded");
        }
        return 0;
    };
    let to = match parse_status(status) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    if to == TaskStatus::Done && !unreviewed && done_evidence_ready(&f, id, why) {
        if let Err(e) = require_reviewer_transcript(&f, id) {
            return fail(e);
        }
    }
    let status_agent = agent_of(&f, session_id.as_deref(), id, "status", None);
    match tasks::transition_attributed(
        &mut f.conn,
        id,
        to,
        Source::Cli,
        session_id.as_deref(),
        agent_ref(&status_agent),
        why,
        unreviewed,
        f.now,
    ) {
        // The handoff above already committed; this exits with the transition's own error, as
        // `ratchet task status` would for the same move.
        Err(e) => fail(e),
        Ok(task) => {
            if json_out {
                output::emit_json(
                    &f.home,
                    "task-record",
                    &json!({ "handoff": handoff_ev, "task": task }),
                    f.now,
                );
            } else {
                println!("{id} handoff recorded");
                println!("{} → {}", task.id, task.status.as_str());
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
    let session_id = match session_of(&f, session) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
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
