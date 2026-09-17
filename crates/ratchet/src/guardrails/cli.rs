//! `ratchet guardrails list|test`: see and dry-run what a hook would do here.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::config::ratchet_home;
use crate::guardrails::eval::{evaluate, scratchpad_from_env, GuardContext};
use crate::guardrails::rules::load_rule_set;
use crate::repo::{find_repo, has_venv, Repo};

fn resolve(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
) -> Result<(PathBuf, Option<Repo>, PathBuf), String> {
    let cwd = cwd.ok_or("no cwd")?;
    let home = ratchet_home(env);
    let repo = find_repo(&cwd).map_err(|e| e.to_string())?;
    Ok((cwd, repo, home))
}

pub fn list(env: &HashMap<String, String>, cwd: Option<PathBuf>) -> i32 {
    let (_cwd, repo, home) = match resolve(env, cwd) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let set = match load_rule_set(&home, repo.as_ref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    for r in &set.rules {
        let off = if set.is_off(&r.id) { "  off" } else { "" };
        println!(
            "{:<18} {:<10} {:<40} {}{}",
            r.id,
            r.kind.as_str(),
            r.tools.join(","),
            r.source,
            off
        );
    }
    match repo {
        Some(r) => println!("repo: {}", r.main_root.display()),
        None => println!("repo: (none - no ratchet.toml above cwd; hooks are no-ops here)"),
    }
    0
}

pub fn test(tool: &str, payload: &str, env: &HashMap<String, String>, cwd: Option<PathBuf>) -> i32 {
    let tool_input: Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: payload is not JSON: {e}");
            return 1;
        }
    };
    let (cwd, repo, home) = match resolve(env, cwd) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let set = match load_rule_set(&home, repo.as_ref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if repo.is_none() {
        println!("allowed (no ratchet.toml above cwd; hooks are no-ops here)");
        return 0;
    }
    let ctx = GuardContext {
        main_root: repo.as_ref().map(|r| r.main_root.clone()),
        worktrees_dir: repo.as_ref().map(|r| r.worktrees_dir.clone()),
        has_venv: repo
            .as_ref()
            .map(|r| has_venv(&cwd, &r.main_root))
            .unwrap_or(false),
        cwd,
        scratchpad: scratchpad_from_env(env),
    };
    let rules: Vec<_> = set.active().collect();
    match evaluate(&rules, tool, &tool_input, &ctx) {
        Some(v) => {
            println!("{}", v.render());
            2
        }
        None => {
            println!("allowed");
            0
        }
    }
}
