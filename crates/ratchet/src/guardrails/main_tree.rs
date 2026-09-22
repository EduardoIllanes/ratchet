//! The `main_tree` rule: a write to a tracked file of the main checkout, from any session,
//! including one whose cwd is a worktree (that was the documented bug this rule exists for).
//!
//! A write can arrive as an `Edit`/`Write`/`NotebookEdit`/`MultiEdit` call naming the file
//! directly, or as a `Bash`/`PowerShell` command whose shape writes to a file: a `>`/`>>`
//! redirection, `tee`, `sed -i`, the destination of `cp`/`mv`/`rsync`, or the paths given to
//! `git checkout --`/`git restore`. A command that writes through an interpreter (`python`,
//! `node`, a heredoc script) is not a recognised shape and is not caught here — see
//! `guardrail.main_tree_write`, emitted after the fact by the post-tool hook.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::eval::GuardContext;
use super::rules::Rule;
use super::segment::segments;
use crate::repo::{is_tracked, within};

const WRITE_TOOLS: [&str; 4] = ["Edit", "Write", "NotebookEdit", "MultiEdit"];
const COMMAND_TOOLS: [&str; 2] = ["Bash", "PowerShell"];

pub fn writes_main_tree(tool_name: &str, tool_input: &Value, ctx: &GuardContext) -> bool {
    let Some(main_root) = ctx.main_root.as_deref() else {
        return false;
    };
    if WRITE_TOOLS.contains(&tool_name) {
        let raw = tool_input
            .get("file_path")
            .or_else(|| tool_input.get("notebook_path"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if raw.is_empty() {
            return false;
        }
        return resolves_to_tracked_main_tree(raw, ctx, main_root);
    }
    if COMMAND_TOOLS.contains(&tool_name) {
        let command = tool_input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("");
        if command.is_empty() {
            return false;
        }
        for segment in segments(command) {
            for raw in shape_targets(&segment) {
                if resolves_to_tracked_main_tree(&raw, ctx, main_root) {
                    return true;
                }
            }
        }
        return false;
    }
    false
}

fn resolves_to_tracked_main_tree(raw: &str, ctx: &GuardContext, main_root: &Path) -> bool {
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

/// Candidate write targets of one already-split command segment, for the recognised shapes only.
/// Not a shell parser: a simple token scan is enough to catch the shapes the spec lists without
/// mis-splitting quoted Windows paths the way a real tokenizer would need to.
fn shape_targets(segment: &str) -> Vec<String> {
    let mut out = redirection_targets(segment);
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    match tokens.first().copied() {
        Some("tee") => out.extend(
            tokens[1..]
                .iter()
                .filter(|t| !t.starts_with('-'))
                .map(|t| t.to_string()),
        ),
        Some("sed") => {
            if tokens.iter().any(|t| *t == "-i" || t.starts_with("-i")) {
                if let Some(last) = tokens.last() {
                    out.push((*last).to_string());
                }
            }
        }
        Some("cp") | Some("mv") | Some("rsync") => {
            // At least a source and a destination; the destination is the last argument.
            if tokens.len() >= 3 {
                if let Some(last) = tokens.last() {
                    out.push((*last).to_string());
                }
            }
        }
        Some("git") => match tokens.get(1).copied() {
            Some("checkout") => {
                if let Some(pos) = tokens.iter().position(|t| *t == "--") {
                    out.extend(tokens[pos + 1..].iter().map(|t| t.to_string()));
                }
            }
            Some("restore") => out.extend(
                tokens[2..]
                    .iter()
                    .filter(|t| !t.starts_with('-'))
                    .map(|t| t.to_string()),
            ),
            _ => {}
        },
        _ => {}
    }
    out.into_iter()
        .map(|t| strip_quotes(&t))
        .filter(|t| !t.is_empty())
        .collect()
}

/// Targets of unquoted `>`/`>>` redirections. Skips `N>&M` file-descriptor duplication (the
/// character right after the arrow(s) is `&`, not a path).
fn redirection_targets(segment: &str) -> Vec<String> {
    let chars: Vec<char> = segment.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if chars[i] != '>' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        if j < n && chars[j] == '>' {
            j += 1;
        }
        while j < n && chars[j].is_whitespace() {
            j += 1;
        }
        if j < n && chars[j] != '&' {
            let start = j;
            while j < n && !chars[j].is_whitespace() {
                j += 1;
            }
            out.push(chars[start..j].iter().collect());
        }
        i = j.max(i + 1);
    }
    out
}

fn strip_quotes(raw: &str) -> String {
    let s = raw.trim();
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

/// Paths present in `after` but not in `before` — both are `git status --porcelain
/// --untracked-files=no` paths at the main root, so any path here is tracked and freshly
/// touched during whatever ran between the two snapshots.
pub fn newly_changed(before: &[String], after: &[String]) -> Vec<String> {
    after
        .iter()
        .filter(|p| !before.contains(p))
        .cloned()
        .collect()
}

/// The post-tool's stderr line: the same `[ratchet guardrail:main-tree]` prefix and rule text an
/// `Edit` block gets, with the changed file(s) named in between.
pub fn render_post_check(rule: &Rule, files: &[String]) -> String {
    format!(
        "[ratchet guardrail:{}] {} ({}) {}",
        rule.id,
        rule.message,
        files.join(", "),
        rule.alternative
    )
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
