//! `ratchet config init`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The repo marker template (spec §4.3), with the two optional keys commented out.
const TEMPLATE: &str = r#"# ratchet repo marker. Its presence opts this repo in; without it every hook is a no-op.

[repo]
# name = "myproject"          # optional; defaults to the directory name
default_branch = "main"
worktrees_dir = ".worktrees"  # relative to the repo root; writes here are never "main tree"

[guardrails]
off = []                       # ids of built-in rules to disable in this repo
# extra = "ratchet/guardrails.toml"   # optional: repo-specific rules, same schema as built-ins

[thresholds]                   # optional, defaults shown
live_minutes = 10
idle_minutes = 60
"#;

/// Writes a commented `ratchet.toml` at the root of the git repository containing `cwd`.
///
/// `repo::find_repo` resolves a repo from the nearest existing marker, which would be
/// circular here: this command's job is to create that marker where none exists yet. So the
/// root is resolved by shelling out to git (`git rev-parse --show-toplevel`), the same way
/// `repo::git_branch` and `tests/spec/support.rs::git` do.
pub fn init(force: bool, cwd: &Path) -> i32 {
    let root = match git_toplevel(cwd) {
        Some(r) => r,
        None => {
            eprintln!("not a git repository: {}", cwd.display());
            return 1;
        }
    };
    let path = root.join("ratchet.toml");
    if path.is_file() && !force {
        eprintln!(
            "ratchet.toml already exists at {}; use --force to overwrite",
            path.display()
        );
        return 1;
    }
    if let Err(e) = std::fs::write(&path, TEMPLATE) {
        eprintln!("error: {e}");
        return 1;
    }
    println!("wrote {}", path.display());
    0
}

fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .args(["-C", &cwd.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        return None;
    }
    Some(PathBuf::from(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RepoConfig;

    #[test]
    fn template_parses_with_default_branch_main() {
        let cfg: RepoConfig = toml::from_str(TEMPLATE).unwrap();
        assert_eq!(cfg.repo.default_branch.as_deref(), Some("main"));
    }
}
