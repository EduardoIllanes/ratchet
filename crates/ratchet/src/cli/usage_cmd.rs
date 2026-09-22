//! `ratchet usage`: token cost per task, role and model, read from the transcripts Claude Code
//! already writes to disk and joined with the sessions/task events ratchet already records.
//! Read-only except `--note`, which appends the one-task summary through the same
//! `services::tasks::note` `ratchet task note` calls: parse, attribute, print (and, only with
//! `--note`, write one note). No SQL beyond the existing `services::{sessions, tasks, events}`
//! reads, and `db::open_ready` only.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::clock;
use crate::config::{self, ModelWeights};
use crate::db;
use crate::model::{Event, Source};
use crate::output;
use crate::repo::find_repo;
use crate::services::{events, sessions, tasks};
use crate::usage::attribute::{
    self, AttributedCall, Bucket, SessionEvent, SessionEventKind, SubagentEvent, Totals,
};
use crate::usage::transcript::{self, Call, Meta};

/// One task's identity for a report row, plus what `rounds`/`touched` need: `touched_at` is the
/// timestamp of its most recent event, any kind (Assumption 2); `first_claim`/`review_entries`
/// are read once here so every render stays free of the database.
pub(crate) struct TaskRow {
    pub id: String,
    pub title: String,
    pub status: String,
    pub touched_at: DateTime<Utc>,
    pub first_claim: DateTime<Utc>,
    pub review_entries: Vec<DateTime<Utc>>,
}

/// One session's orientation total and the tasks it ever held — the unit Requirement 4's
/// "average over the task's sessions" / "average per session" average over.
pub(crate) struct SessionOrientation {
    // Kept for Task 5 / future by-session orientation queries; today's renders key off
    // `held_tasks` only.
    #[allow(dead_code)]
    pub session: String,
    pub held_tasks: Vec<String>,
    pub totals: Totals,
}

/// Everything one report is built from, gathered once by `collect`. `calls` is every parsed call
/// with its task resolved (or not); `buckets` is `calls` grouped by `(session, role, model)`
/// across the whole scan (weights applied when the machine config has any); `skipped`/`partial`
/// are the record-level tallies Requirement 2's trailer line names; `notices` is one line per
/// session whose transcript file is missing; `orientation` is per-session, not per-task, so a
/// render can average it either way.
pub(crate) struct Collected {
    pub tasks: Vec<TaskRow>,
    pub calls: Vec<AttributedCall>,
    // The whole window's (session, role, model) buckets. Per-task renders recompute
    // `buckets_of` on task-filtered calls instead (a `Bucket` carries no task dimension to
    // filter this one by), so this whole-window rollup is kept for Task 5 / a future
    // whole-window `--json` consumer, not read by anything in this task yet.
    #[allow(dead_code)]
    pub buckets: Vec<Bucket>,
    pub skipped: u64,
    pub partial: u64,
    pub max_version: Option<String>,
    pub notices: Vec<String>,
    pub orientation: Vec<SessionOrientation>,
    pub weights: Option<HashMap<String, ModelWeights>>,
}

fn fail(e: impl std::fmt::Display) -> i32 {
    eprintln!("error: {e}");
    1
}

/// `RATCHET_CLAUDE_PROJECTS` when set and non-empty, else `~/.claude/projects`. Checked before
/// the database is ever opened (R1: a missing directory fails naming the path, exit 1).
fn projects_dir(env: &HashMap<String, String>) -> Result<PathBuf, String> {
    let dir = match env.get("RATCHET_CLAUDE_PROJECTS").filter(|s| !s.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claude")
            .join("projects"),
    };
    if !dir.is_dir() {
        return Err(format!("no such projects directory: {}", dir.display()));
    }
    Ok(dir)
}

fn session_event_of(e: &Event) -> Option<SessionEvent> {
    match e.kind.as_str() {
        "task.claimed" => {
            let task = e.task_id.clone()?;
            Some(SessionEvent {
                ts: e.ts,
                kind: SessionEventKind::Claimed { task },
            })
        }
        "task.status" => {
            let task = e.task_id.clone()?;
            let to = e.payload.get("to").and_then(Value::as_str)?.to_string();
            Some(SessionEvent {
                ts: e.ts,
                kind: SessionEventKind::Status { task, to },
            })
        }
        _ => None,
    }
}

/// The `subagent.start`/`.stop` event for this session naming `agent_id`, read from the row's own
/// `task_id` column (Assumption 1: never from the payload, which has no `task_id` field).
fn subagent_event_of(raw_events: &[Event], agent_id: &str) -> Option<SubagentEvent> {
    raw_events
        .iter()
        .find(|e| {
            matches!(e.kind.as_str(), "subagent.start" | "subagent.stop")
                && e.payload.get("agent_id").and_then(Value::as_str) == Some(agent_id)
        })
        .map(|e| SubagentEvent {
            agent_id: agent_id.to_string(),
            agent_type: e
                .payload
                .get("agent_type")
                .and_then(Value::as_str)
                .map(str::to_string),
            description: e
                .payload
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string),
            task: e.task_id.clone(),
        })
}

/// `<id>` from `agent-<id>.jsonl`.
fn agent_id_from(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    stem.strip_prefix("agent-").map(str::to_string)
}

/// Coarse numeric-dot version compare, same rule as `transcript::parse`'s own (private) one —
/// duplicated here in the few lines it takes rather than exporting a seam Task 2 never needed.
fn bump_version(current: &mut Option<String>, candidate: &Option<String>) {
    let Some(v) = candidate else { return };
    let newer = match current.as_ref() {
        None => true,
        Some(cur) => version_key(v) > version_key(cur),
    };
    if newer {
        *current = Some(v.clone());
    }
}

fn version_key(v: &str) -> Vec<u64> {
    v.split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .collect()
}

/// The collection pass: resolve the projects dir, open the database read-only, walk every
/// session's transcript (and its subagents'), attribute every call to a task or `None`, and
/// gather the task rows the window filters render from. Resolution order matches the brief
/// exactly: projects dir before the database, sessions before transcripts, holds before calls.
pub(crate) fn collect(
    env: &HashMap<String, String>,
    cwd: Option<&Path>,
    all_repos: bool,
) -> Result<Collected, String> {
    let projects = projects_dir(env)?;
    let home = config::ratchet_home(env);
    let conn = db::open_ready(&home).map_err(|e| e.to_string())?;
    let here = cwd
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let repo_name = if all_repos {
        None
    } else {
        match find_repo(&here).map_err(|e| e.to_string())? {
            Some(r) => Some(r.name),
            None => {
                return Err(format!(
                    "not a ratchet-managed repo (no ratchet.toml found above {}); pass --all-repos to see every repo",
                    here.display()
                ))
            }
        }
    };
    let machine = config::load_machine_config(&home).map_err(|e| e.to_string())?;
    let weights = if machine.usage.weights.is_empty() {
        None
    } else {
        Some(machine.usage.weights)
    };

    let found_sessions = sessions::list(&conn, repo_name.as_deref()).map_err(|e| e.to_string())?;

    let mut all_calls: Vec<AttributedCall> = Vec::new();
    let mut skipped: u64 = 0;
    let mut partial: u64 = 0;
    let mut max_version: Option<String> = None;
    let mut notices: Vec<String> = Vec::new();
    let mut orientation: Vec<SessionOrientation> = Vec::new();
    let mut task_ids: BTreeSet<String> = BTreeSet::new();

    for s in &found_sessions {
        let raw_events = events::for_session(&conn, &s.id, i64::MAX).map_err(|e| e.to_string())?;
        let session_events: Vec<SessionEvent> =
            raw_events.iter().filter_map(session_event_of).collect();
        for ev in &session_events {
            match &ev.kind {
                SessionEventKind::Claimed { task } => {
                    task_ids.insert(task.clone());
                }
                SessionEventKind::Status { task, .. } => {
                    task_ids.insert(task.clone());
                }
            }
        }
        let holds = attribute::holds(&session_events, s.ended_at);
        let first_claim = session_events
            .iter()
            .filter_map(|e| match &e.kind {
                SessionEventKind::Claimed { .. } => Some(e.ts),
                SessionEventKind::Status { .. } => None,
            })
            .min();
        let mut held_tasks: Vec<String> = holds.iter().map(|h| h.task.clone()).collect();
        held_tasks.sort();
        held_tasks.dedup();

        // A missing main transcript is R1's "no transcript" row, not an error, and — critically —
        // does not skip this session's subagent transcripts below: a subagent scenario writes no
        // parent transcript at all (fix round: three subagent scenarios were losing their calls
        // entirely because an early `continue` here never reached the subagent scan).
        let path = transcript::transcript_path(&projects, &s.cwd, &s.id);
        let main_calls: Vec<Call> = match std::fs::read(&path) {
            Ok(bytes) => {
                let parsed = transcript::parse(&bytes);
                skipped += parsed.skipped;
                partial += parsed.partial;
                bump_version(&mut max_version, &parsed.max_version);
                parsed.calls
            }
            Err(_) => {
                notices.push(format!("session {}: no transcript", s.id));
                Vec::new()
            }
        };

        let mut sorted_calls: Vec<&Call> = main_calls.iter().collect();
        sorted_calls.sort_by_key(|c| c.ts);
        let orient_totals = attribute::orientation(&sorted_calls, first_claim);
        orientation.push(SessionOrientation {
            session: s.id.clone(),
            held_tasks,
            totals: orient_totals,
        });

        let mut parent_agent_calls: Vec<(String, DateTime<Utc>)> = Vec::new();
        for c in &main_calls {
            for (name, tool_id) in c.tools.iter().zip(c.tool_use_ids.iter()) {
                if name == "Agent" {
                    parent_agent_calls.push((tool_id.clone(), c.ts));
                }
            }
        }

        for c in &main_calls {
            let task = attribute::task_at(&holds, c.ts).map(str::to_string);
            all_calls.push(AttributedCall {
                task,
                session: s.id.clone(),
                role: "orchestrator".to_string(),
                model: c.model.clone(),
                ts: c.ts,
                usage: c.usage,
            });
        }

        let sub_dir = transcript::subagents_dir(&projects, &s.cwd, &s.id);
        if let Ok(entries) = std::fs::read_dir(&sub_dir) {
            let mut files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().and_then(|x| x.to_str()) == Some("jsonl")
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with("agent-"))
                            .unwrap_or(false)
                })
                .collect();
            files.sort();
            for jf in files {
                let Some(agent_id) = agent_id_from(&jf) else {
                    continue;
                };
                let bytes = match std::fs::read(&jf) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let sub_parsed = transcript::parse(&bytes);
                skipped += sub_parsed.skipped;
                partial += sub_parsed.partial;
                bump_version(&mut max_version, &sub_parsed.max_version);

                let meta_path = jf.with_file_name(format!("agent-{agent_id}.meta.json"));
                let meta: Option<Meta> = std::fs::read(&meta_path)
                    .ok()
                    .and_then(|b| transcript::read_meta(&b));

                let event = subagent_event_of(&raw_events, &agent_id);
                let description = event
                    .as_ref()
                    .and_then(|e| e.description.clone())
                    .filter(|d| !d.is_empty())
                    .or_else(|| meta.as_ref().and_then(|m| m.description.clone()));

                let first_ts = sub_parsed.calls.iter().map(|c| c.ts).min();
                let (task, role) = attribute::subagent_task_and_role(
                    event.as_ref(),
                    meta.as_ref(),
                    &holds,
                    &parent_agent_calls,
                    first_ts,
                );
                if let Some(t) = &task {
                    task_ids.insert(t.clone());
                }
                let display = attribute::display_role(&role, description.as_deref());

                for c in &sub_parsed.calls {
                    all_calls.push(AttributedCall {
                        task: task.clone(),
                        session: s.id.clone(),
                        role: display.clone(),
                        model: c.model.clone(),
                        ts: c.ts,
                        usage: c.usage,
                    });
                }
            }
        }
    }

    let mut tasks_out: Vec<TaskRow> = Vec::new();
    for id in &task_ids {
        let Ok(t) = tasks::get(&conn, id) else {
            continue;
        };
        let task_events = events::for_task(&conn, id, i64::MAX).map_err(|e| e.to_string())?;
        let touched_at = task_events.last().map(|e| e.ts).unwrap_or(t.updated_at);
        let first_claim = task_events
            .iter()
            .find(|e| e.kind == "task.claimed")
            .map(|e| e.ts)
            .unwrap_or(t.created_at);
        let review_entries: Vec<DateTime<Utc>> = task_events
            .iter()
            .filter(|e| {
                e.kind == "task.status"
                    && e.payload.get("to").and_then(Value::as_str) == Some("review")
            })
            .map(|e| e.ts)
            .collect();
        tasks_out.push(TaskRow {
            id: t.id,
            title: t.title,
            status: t.status.as_str().to_string(),
            touched_at,
            first_claim,
            review_entries,
        });
    }

    let buckets = attribute::buckets_of(&all_calls, weights.as_ref());

    Ok(Collected {
        tasks: tasks_out,
        calls: all_calls,
        buckets,
        skipped,
        partial,
        max_version,
        notices,
        orientation,
        weights,
    })
}

/// One task's numbers, gathered once so both the text detail view and `one_task_summary` build
/// from the same figures.
struct TaskMetrics {
    id: String,
    title: String,
    status: String,
    role_model: BTreeMap<(String, String), Totals>,
    totals: Totals,
    orientation: Totals,
    rounds_totals: Vec<Totals>,
    orch_share: f64,
    cache_eff: f64,
    cost: Option<f64>,
}

fn task_metrics(collected: &Collected, task_id: &str) -> Option<TaskMetrics> {
    let row = collected.tasks.iter().find(|t| t.id == task_id)?;
    let calls: Vec<AttributedCall> = collected
        .calls
        .iter()
        .filter(|c| c.task.as_deref() == Some(task_id))
        .cloned()
        .collect();
    let role_model = attribute::totals_by_role_model(&calls);
    let mut totals = Totals::default();
    for c in &calls {
        totals.add(&c.usage);
    }
    let buckets = attribute::buckets_of(&calls, collected.weights.as_ref());
    let orient_sessions: Vec<Totals> = collected
        .orientation
        .iter()
        .filter(|so| so.held_tasks.iter().any(|t| t == task_id))
        .map(|so| so.totals)
        .collect();
    let orientation = attribute::average_totals(&orient_sessions);
    let rounds_totals = attribute::rounds_tokens(&calls, row.first_claim, &row.review_entries);
    let orch_share = attribute::orchestrator_share(&buckets);
    let cache_eff = attribute::cache_efficiency(&totals);
    let cost = attribute::task_cost(&buckets);
    Some(TaskMetrics {
        id: row.id.clone(),
        title: row.title.clone(),
        status: row.status.clone(),
        role_model,
        totals,
        orientation,
        rounds_totals,
        orch_share,
        cache_eff,
        cost,
    })
}

fn tokens_line(prefix: &str, t: &Totals) -> String {
    format!(
        "{prefix} in {} cache_w {} cache_r {} out {}",
        attribute::abbreviate(t.input),
        attribute::abbreviate(t.cache_write),
        attribute::abbreviate(t.cache_read),
        attribute::abbreviate(t.output)
    )
}

fn trailer(collected: &Collected) -> Option<String> {
    if collected.skipped == 0 && collected.partial == 0 {
        return None;
    }
    Some(format!(
        "skipped {} partial {} version {}",
        collected.skipped,
        collected.partial,
        collected.max_version.as_deref().unwrap_or("-")
    ))
}

/// The one-task summary Requirement 10's `--note` appends (prefixed with `usage: ` by
/// `write_note`), on one paragraph — no embedded newlines. Shared with `render_task`'s own
/// figures (both build from `task_metrics`) so the note and the `<id>` render can never drift.
pub(crate) fn one_task_summary(collected: &Collected, task_id: &str) -> String {
    let Some(m) = task_metrics(collected, task_id) else {
        return format!("{task_id}: no usage data");
    };
    let mut parts = vec![
        format!("{} {}", m.id, m.title),
        format!("rounds {}", m.rounds_totals.len()),
        tokens_line("tokens", &m.totals),
        tokens_line("orientation", &m.orientation),
    ];
    if let Some(c) = m.cost {
        parts.push(format!("cost {c:.2}"));
    }
    parts.join(" · ")
}

/// `--note`'s write path (Requirement 10): opens its own connection — `collect`'s is long since
/// closed by the time a render would run — resolves the session exactly as `task_cmd::face`/
/// `attributed` do (repo thresholds when there is a repo, else defaults), and writes through the
/// SAME `services::tasks::note` `ratchet task note` calls. A missing task fails inside that one
/// call (`tasks::get` under the transaction), so the error text is identical by construction and
/// no event row survives the rolled-back transaction — nothing here needs to special-case it.
fn write_note(
    env: &HashMap<String, String>,
    cwd: Option<&Path>,
    session: Option<&str>,
    collected: &Collected,
    task_id: &str,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let home = config::ratchet_home(env);
    let mut conn = db::open_ready(&home).map_err(|e| e.to_string())?;
    let here = cwd
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let th = find_repo(&here)
        .map_err(|e| e.to_string())?
        .map(|r| r.config.thresholds)
        .unwrap_or_default();
    let session_id = sessions::resolve(&conn, session, env, &here, &th, now)
        .ok()
        .flatten();
    let text = format!("usage: {}", one_task_summary(collected, task_id));
    tasks::note(
        &mut conn,
        task_id,
        &text,
        Source::Cli,
        session_id.as_deref(),
        now,
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn render_task(
    home: &Path,
    collected: &Collected,
    task_id: &str,
    json_out: bool,
    now: DateTime<Utc>,
) -> i32 {
    let Some(m) = task_metrics(collected, task_id) else {
        eprintln!("error: task {task_id} does not exist");
        return 1;
    };
    if json_out {
        let calls: Vec<AttributedCall> = collected
            .calls
            .iter()
            .filter(|c| c.task.as_deref() == Some(task_id))
            .cloned()
            .collect();
        let buckets = attribute::buckets_of(&calls, collected.weights.as_ref());
        let payload = json!({
            "tasks": [json!({
                "id": m.id,
                "title": m.title,
                "status": m.status,
                "rounds": m.rounds_totals.len(),
                "orientation": m.orientation,
                "buckets": buckets,
            })],
            "skipped": collected.skipped,
            "partial": collected.partial,
            "version": collected.max_version,
        });
        output::emit_json(home, &format!("usage-{task_id}"), &payload, now);
        return 0;
    }
    let mut lines = vec![
        format!("{}  {}", m.id, m.title),
        format!("status {}", m.status),
    ];
    for ((role, model), t) in &m.role_model {
        lines.push(tokens_line(&format!("{role}  {model}"), t));
    }
    lines.push(tokens_line("orientation", &m.orientation));
    lines.push(format!("rounds {}", m.rounds_totals.len()));
    for (i, t) in m.rounds_totals.iter().enumerate() {
        lines.push(tokens_line(&format!("round {}", i + 1), t));
    }
    lines.push(format!("orch share {:.1}%", m.orch_share * 100.0));
    lines.push(format!("cache eff {:.1}%", m.cache_eff * 100.0));
    if let Some(c) = m.cost {
        lines.push(format!("cost {c:.2}"));
    }
    if let Some(t) = trailer(collected) {
        lines.push(t);
    }
    output::emit(home, &format!("usage-{task_id}"), &lines, now);
    0
}

fn render_listing(
    home: &Path,
    collected: &Collected,
    cutoff: DateTime<Utc>,
    json_out: bool,
    now: DateTime<Utc>,
) -> i32 {
    let rows: Vec<&TaskRow> = collected
        .tasks
        .iter()
        .filter(|t| t.touched_at >= cutoff)
        .collect();
    if json_out {
        let mut tasks_json: Vec<Value> = Vec::new();
        for row in &rows {
            let calls: Vec<AttributedCall> = collected
                .calls
                .iter()
                .filter(|c| c.task.as_deref() == Some(row.id.as_str()))
                .cloned()
                .collect();
            let buckets = attribute::buckets_of(&calls, collected.weights.as_ref());
            let orient_sessions: Vec<Totals> = collected
                .orientation
                .iter()
                .filter(|so| so.held_tasks.iter().any(|t| t == &row.id))
                .map(|so| so.totals)
                .collect();
            let orientation = attribute::average_totals(&orient_sessions);
            tasks_json.push(json!({
                "id": row.id,
                "title": row.title,
                "status": row.status,
                "rounds": row.review_entries.len(),
                "orientation": orientation,
                "buckets": buckets,
            }));
        }
        let payload = json!({
            "tasks": tasks_json,
            "skipped": collected.skipped,
            "partial": collected.partial,
            "version": collected.max_version,
        });
        output::emit_json(home, "usage", &payload, now);
        return 0;
    }
    let mut lines = Vec::new();
    for row in &rows {
        let calls: Vec<AttributedCall> = collected
            .calls
            .iter()
            .filter(|c| c.task.as_deref() == Some(row.id.as_str()))
            .cloned()
            .collect();
        let mut t = Totals::default();
        for c in &calls {
            t.add(&c.usage);
        }
        let buckets = attribute::buckets_of(&calls, collected.weights.as_ref());
        let cost = attribute::task_cost(&buckets);
        let mut line = format!(
            "{}  {:<12} rounds {}  {}  {}",
            row.id,
            row.status,
            row.review_entries.len(),
            tokens_line("tokens", &t),
            row.title
        );
        if let Some(c) = cost {
            line.push_str(&format!("  cost {c:.2}"));
        }
        lines.push(line);
    }
    for n in &collected.notices {
        lines.push(n.clone());
    }
    if let Some(t) = trailer(collected) {
        lines.push(t);
    }
    output::emit(home, "usage", &lines, now);
    0
}

fn render_by(
    home: &Path,
    collected: &Collected,
    by: &str,
    cutoff: DateTime<Utc>,
    json_out: bool,
    now: DateTime<Utc>,
) -> i32 {
    let in_window: BTreeSet<&str> = collected
        .tasks
        .iter()
        .filter(|t| t.touched_at >= cutoff)
        .map(|t| t.id.as_str())
        .collect();
    let owned_calls: Vec<AttributedCall> = collected
        .calls
        .iter()
        .filter(|c| {
            c.task
                .as_deref()
                .map(|t| in_window.contains(t))
                .unwrap_or(true)
        })
        .cloned()
        .collect();
    let grouped = match by {
        "task" => attribute::group_totals(&owned_calls, |c| {
            c.task.clone().unwrap_or_else(|| "unassigned".to_string())
        }),
        "role" => attribute::group_totals(&owned_calls, |c| c.role.clone()),
        "model" => attribute::group_totals(&owned_calls, |c| c.model.clone()),
        "session" => attribute::group_totals(&owned_calls, |c| {
            format!(
                "{} {}",
                c.session,
                c.task.clone().unwrap_or_else(|| "unassigned".to_string())
            )
        }),
        _ => unreachable!("validated in run()"),
    };
    if json_out {
        let payload: Vec<Value> = grouped
            .iter()
            .map(|(k, t)| json!({ "key": k, "tokens": t }))
            .collect();
        output::emit_json(home, "usage-by", &json!(payload), now);
        return 0;
    }
    let mut lines: Vec<String> = grouped.iter().map(|(k, t)| tokens_line(k, t)).collect();
    if by == "role" {
        let task_count = in_window.len().max(1);
        let total_rounds: usize = collected
            .tasks
            .iter()
            .filter(|t| in_window.contains(t.id.as_str()))
            .map(|t| t.review_entries.len())
            .sum();
        lines.push(format!(
            "review rounds per task {:.1}",
            total_rounds as f64 / task_count as f64
        ));
        let sess_totals: Vec<Totals> = collected.orientation.iter().map(|s| s.totals).collect();
        let avg = attribute::average_totals(&sess_totals);
        lines.push(tokens_line("orientation per session", &avg));
    }
    if let Some(t) = trailer(collected) {
        lines.push(t);
    }
    output::emit(home, "usage-by", &lines, now);
    0
}

/// `ratchet usage` / `ratchet usage <id>` / `--by`/`--since`/`--all-repos`/`--json`/`--note`.
/// `--note` (Requirement 10) requires `<id>` and changes no render: it only adds the note write
/// before the same `render_task` call the plain `usage <id>` path already makes.
#[allow(clippy::too_many_arguments)]
pub fn run(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: Option<&str>,
    by: Option<&str>,
    since: Option<&str>,
    all_repos: bool,
    json_out: bool,
    note: bool,
) -> i32 {
    if note && id.is_none() {
        return fail("--note requires a task id: ratchet usage <id> --note");
    }
    let by = match by {
        None => None,
        Some(raw) => match raw {
            "task" | "role" | "model" | "session" => Some(raw),
            other => {
                return fail(format!(
                    "invalid --by: {other}; expected task, role, model or session"
                ))
            }
        },
    };
    let home = config::ratchet_home(env);
    let now = clock::now(env);
    let since_spec = since.unwrap_or("7d");
    let cutoff = match attribute::since_cutoff(since_spec, now) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let collected = match collect(env, cwd.as_deref(), all_repos) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    if let Some(task_id) = id {
        if note {
            if let Err(e) = write_note(env, cwd.as_deref(), session, &collected, task_id, now) {
                return fail(e);
            }
        }
        return render_task(&home, &collected, task_id, json_out, now);
    }
    if let Some(by) = by {
        return render_by(&home, &collected, by, cutoff, json_out, now);
    }
    render_listing(&home, &collected, cutoff, json_out, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn totals(input: u64) -> Totals {
        Totals {
            input,
            cache_write: 0,
            cache_read: 0,
            output: 0,
            thinking: 0,
        }
    }

    fn empty_task_row(id: &str) -> TaskRow {
        let ts = clock::parse("2026-09-16T12:00:00Z").unwrap();
        TaskRow {
            id: id.to_string(),
            title: "t".to_string(),
            status: "in_progress".to_string(),
            touched_at: ts,
            first_claim: ts,
            review_entries: Vec::new(),
        }
    }

    /// Requirement 4: orientation is shown per task as the AVERAGE over the task's sessions, never
    /// their sum. Two sessions with different orientation totals held the same task; the average
    /// (20) must appear, not the sum (40).
    #[test]
    fn orientation_for_a_task_averages_across_its_sessions_not_sums() {
        let collected = Collected {
            tasks: vec![empty_task_row("T-0001")],
            calls: Vec::new(),
            buckets: Vec::new(),
            skipped: 0,
            partial: 0,
            max_version: None,
            notices: Vec::new(),
            orientation: vec![
                SessionOrientation {
                    session: "s-1".to_string(),
                    held_tasks: vec!["T-0001".to_string()],
                    totals: totals(10),
                },
                SessionOrientation {
                    session: "s-2".to_string(),
                    held_tasks: vec!["T-0001".to_string()],
                    totals: totals(30),
                },
            ],
            weights: None,
        };
        let m = task_metrics(&collected, "T-0001").expect("task present");
        assert_eq!(
            m.orientation.input, 20,
            "average of 10 and 30 is 20, never their sum 40"
        );
    }

    #[test]
    fn a_task_absent_from_collected_has_no_metrics() {
        let collected = Collected {
            tasks: Vec::new(),
            calls: Vec::new(),
            buckets: Vec::new(),
            skipped: 0,
            partial: 0,
            max_version: None,
            notices: Vec::new(),
            orientation: Vec::new(),
            weights: None,
        };
        assert!(task_metrics(&collected, "T-9999").is_none());
    }
}
