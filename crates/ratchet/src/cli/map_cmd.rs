//! `ratchet map` (bare) and `ratchet map status`. `note`/`--missing` (Task 3) and the `--wire`
//! branch's body (Task 5) extend this file; the shapes below already accept their parameters so
//! neither later task edits a call site in `main.rs`.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::clock;
use crate::repo::find_repo;

fn fail(e: impl std::fmt::Display) -> i32 {
    eprintln!("error: {e}");
    1
}

fn not_a_repo(here: &std::path::Path) -> i32 {
    eprintln!(
        "error: not a ratchet-managed repo (no ratchet.toml found above {})",
        here.display()
    );
    1
}

/// `ratchet map` / `ratchet map --wire`. When `wire` is set, wiring runs first, so a refusal
/// (a symlinked `CLAUDE.md` or `.gitignore`) leaves the map itself unwritten too.
pub fn generate(wire: bool, env: &HashMap<String, String>, cwd: Option<PathBuf>) -> i32 {
    let here = cwd.unwrap_or_else(|| PathBuf::from("."));
    let repo = match find_repo(&here) {
        Ok(Some(r)) => r,
        Ok(None) => return not_a_repo(&here),
        Err(e) => return fail(e),
    };
    if wire {
        match crate::map::wire(&repo.main_root) {
            Ok(r) if !r.claude_md_changed && !r.gitignore_changed => {
                println!("wire: already wired (CLAUDE.md, .gitignore)");
            }
            Ok(r) => {
                if r.claude_md_changed {
                    println!("wired: CLAUDE.md now imports .ratchet/map.md");
                }
                if r.gitignore_changed {
                    println!("wired: .gitignore now covers .ratchet/");
                }
            }
            Err(e) => return fail(e),
        }
    }
    let now = clock::now(env);
    let generated = match crate::map::generate(&repo.main_root, &repo.config.map, now) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    let path = match crate::map::write_map(&repo.main_root, &generated) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    println!(
        "wrote {} ({} lines, {} modules without a description)",
        path.display(),
        generated.text.lines().count(),
        generated.without
    );
    for hint in &generated.hints {
        println!("{hint}");
    }
    0
}

/// Exit 0 always; prints nothing outside a marker repo (design §4.1 — safe to run unconditionally).
pub fn status(cwd: Option<PathBuf>) -> i32 {
    let here = cwd.unwrap_or_else(|| PathBuf::from("."));
    match find_repo(&here) {
        Ok(Some(repo)) => {
            println!("{}", crate::map::status_line(&repo.main_root));
            0
        }
        Ok(None) => 0,
        Err(_) => 0,
    }
}

pub fn note(path: &str, sentence: &str, cwd: Option<PathBuf>) -> i32 {
    let here = cwd.unwrap_or_else(|| PathBuf::from("."));
    let repo = match find_repo(&here) {
        Ok(Some(r)) => r,
        Ok(None) => return not_a_repo(&here),
        Err(e) => return fail(e),
    };
    let mut target = PathBuf::from(path);
    if !target.is_absolute() {
        target = here.join(target);
    }
    match crate::map::note(&repo.main_root, &target, sentence) {
        Ok(()) => 0,
        Err(e) => fail(e),
    }
}

pub fn missing(all: bool, cwd: Option<PathBuf>) -> i32 {
    let here = cwd.unwrap_or_else(|| PathBuf::from("."));
    let repo = match find_repo(&here) {
        Ok(Some(r)) => r,
        Ok(None) => return not_a_repo(&here),
        Err(e) => return fail(e),
    };
    match crate::map::missing(&repo.main_root, &repo.config.map, all) {
        Ok(files) => {
            for f in files {
                println!("{f}");
            }
            0
        }
        Err(e) => fail(e),
    }
}
