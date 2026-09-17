//! `ratchet session list | show`. Reads only: no face writes to the database except through a
//! service, and neither of these mutates anything.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::clock;
use crate::config::{ratchet_home, Thresholds};
use crate::db;
use crate::model::{Session, SessionState};
use crate::repo::find_repo;
use crate::services::{sessions, tasks};

/// Thresholds of the repo the directory belongs to; the defaults outside a marked repo.
fn thresholds_for(cwd: Option<&Path>) -> Thresholds {
    cwd.and_then(|c| find_repo(c).ok().flatten())
        .map(|r| r.config.thresholds)
        .unwrap_or_default()
}

pub fn list(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    repo_filter: Option<&str>,
    live_only: bool,
    json: bool,
) -> i32 {
    let home = ratchet_home(env);
    let conn = match db::open_ready(&home) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let now = clock::now(env);
    let th = thresholds_for(cwd.as_deref());
    let found = match sessions::list(&conn, repo_filter) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let mut rows: Vec<(Session, SessionState, Vec<String>)> = Vec::new();
    for s in found {
        let st = sessions::state(&s, &th, now);
        if live_only && st != SessionState::Live {
            continue;
        }
        let mine = match tasks::claimed_ids(&conn, &s.id) {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        rows.push((s, st, mine));
    }
    if json {
        let payload: Vec<serde_json::Value> = rows
            .iter()
            .map(|(s, st, mine)| {
                let mut v = serde_json::to_value(s).unwrap_or(serde_json::Value::Null);
                if let Some(map) = v.as_object_mut() {
                    map.insert("state".into(), st.as_str().into());
                    map.insert("tasks".into(), serde_json::json!(mine));
                }
                v
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return 0;
    }
    for (s, st, mine) in rows {
        let id = s.id.chars().take(8).collect::<String>();
        let branch = s.branch.clone().unwrap_or_else(|| "?".to_string());
        let held = if mine.is_empty() {
            "-".to_string()
        } else {
            mine.join(",")
        };
        println!(
            "{:<8}  {:<9} {:<14} {:<20} {}  {}",
            id,
            st.as_str(),
            s.repo,
            branch,
            held,
            clock::iso(s.last_seen)
        );
    }
    0
}

pub fn show(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    id: Option<&str>,
    json: bool,
) -> i32 {
    let home = ratchet_home(env);
    let conn = match db::open_ready(&home) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let now = clock::now(env);
    let th = thresholds_for(cwd.as_deref());
    let here = cwd.unwrap_or_else(|| PathBuf::from("."));
    let resolved = match sessions::resolve(&conn, id, env, &here, &th, now) {
        Ok(Some(v)) => v,
        Ok(None) => {
            eprintln!(
                "error: no session resolved for this directory; name one or set RATCHET_SESSION_ID"
            );
            return 1;
        }
        Err(e) => return fail(e),
    };
    let session = match sessions::get(&conn, &resolved) {
        Ok(Some(s)) => s,
        Ok(None) => {
            eprintln!("error: session {resolved} does not exist");
            return 1;
        }
        Err(e) => return fail(e),
    };
    let st = sessions::state(&session, &th, now);
    let mine = match tasks::claimed_ids(&conn, &session.id) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    if json {
        let mut v = serde_json::to_value(&session).unwrap_or(serde_json::Value::Null);
        if let Some(map) = v.as_object_mut() {
            map.insert("state".into(), st.as_str().into());
            map.insert("tasks".into(), serde_json::json!(mine));
        }
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return 0;
    }
    println!("{}  {}", session.id, st.as_str());
    println!(
        "repo {} · branch {} · {} · {}",
        session.repo,
        session.branch.clone().unwrap_or_else(|| "?".to_string()),
        session.mode.as_str(),
        session.launched_by.as_str()
    );
    println!("cwd {}", session.cwd);
    if let Some(wt) = &session.worktree {
        println!("worktree {wt}");
    }
    let ended = session
        .ended_at
        .map(|e| format!(" · ended {}", clock::iso(e)))
        .unwrap_or_default();
    println!(
        "started {} · last seen {}{}",
        clock::iso(session.started_at),
        clock::iso(session.last_seen),
        ended
    );
    if !mine.is_empty() {
        println!("tasks: {}", mine.join(", "));
    }
    0
}

/// Every error of a face prints the same shape and exits 1 — never 2, which belongs to blocks.
fn fail(e: impl std::fmt::Display) -> i32 {
    eprintln!("error: {e}");
    1
}
