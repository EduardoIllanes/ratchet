//! The `main_tree` rule: a write to a tracked file of the main checkout, from any session,
//! including one whose cwd is a worktree (that was the documented bug this rule exists for).

use std::path::PathBuf;

use serde_json::Value;

use super::eval::GuardContext;
use crate::repo::{is_tracked, within};

const WRITE_TOOLS: [&str; 4] = ["Edit", "Write", "NotebookEdit", "MultiEdit"];

pub fn writes_main_tree(tool_name: &str, tool_input: &Value, ctx: &GuardContext) -> bool {
    if !WRITE_TOOLS.contains(&tool_name) {
        return false;
    }
    let Some(main_root) = ctx.main_root.as_deref() else {
        return false;
    };
    let raw = tool_input
        .get("file_path")
        .or_else(|| tool_input.get("notebook_path"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if raw.is_empty() {
        return false;
    }
    let mut target = PathBuf::from(raw);
    if !target.is_absolute() {
        target = ctx.cwd.join(target);
    }
    if !within(&target, main_root) {
        return false;
    }
    if let Some(wt) = ctx.worktrees_dir.as_deref() {
        if within(&target, wt) {
            return false;
        }
    }
    is_tracked(main_root, &target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    fn git(dir: &std::path::Path, args: &[&str]) {
        let st = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(st.success());
    }

    fn repo_with_tracked_file() -> (tempfile::TempDir, GuardContext) {
        let d = tempfile::TempDir::new().unwrap();
        fs::write(d.path().join("tracked.txt"), "x").unwrap();
        git(d.path(), &["init", "-q"]);
        git(d.path(), &["add", "tracked.txt"]);
        git(d.path(), &["commit", "-q", "-m", "i"]);
        let ctx = GuardContext {
            main_root: Some(d.path().to_path_buf()),
            worktrees_dir: Some(d.path().join(".worktrees")),
            cwd: d.path().to_path_buf(),
            has_venv: false,
            scratchpad: None,
        };
        (d, ctx)
    }

    fn edit(path: &std::path::Path) -> serde_json::Value {
        serde_json::json!({ "file_path": path.to_string_lossy(), "old_string": "x", "new_string": "y" })
    }

    #[test]
    fn tracked_file_in_main_tree_is_a_write() {
        let (d, ctx) = repo_with_tracked_file();
        assert!(writes_main_tree(
            "Edit",
            &edit(&d.path().join("tracked.txt")),
            &ctx
        ));
    }

    #[test]
    fn relative_path_resolves_against_cwd() {
        let (_d, ctx) = repo_with_tracked_file();
        assert!(writes_main_tree(
            "Write",
            &serde_json::json!({ "file_path": "tracked.txt", "content": "" }),
            &ctx
        ));
    }

    #[test]
    fn untracked_worktree_and_outside_are_not() {
        let (d, ctx) = repo_with_tracked_file();
        assert!(!writes_main_tree(
            "Edit",
            &edit(&d.path().join("new.txt")),
            &ctx
        ));
        assert!(!writes_main_tree(
            "Edit",
            &edit(&d.path().join(".worktrees/wt/tracked.txt")),
            &ctx
        ));
        assert!(!writes_main_tree(
            "Edit",
            &edit(&PathBuf::from("C:/elsewhere/tracked.txt")),
            &ctx
        ));
    }

    #[test]
    fn only_write_tools_and_only_with_a_repo() {
        let (d, mut ctx) = repo_with_tracked_file();
        assert!(!writes_main_tree(
            "Bash",
            &serde_json::json!({ "command": "echo" }),
            &ctx
        ));
        assert!(!writes_main_tree(
            "Read",
            &edit(&d.path().join("tracked.txt")),
            &ctx
        ));
        ctx.main_root = None;
        assert!(!writes_main_tree(
            "Edit",
            &edit(&d.path().join("tracked.txt")),
            &ctx
        ));
    }
}
