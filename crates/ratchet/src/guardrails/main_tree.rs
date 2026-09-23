//! The `main_tree` rule: a write to a tracked file of the main checkout, from any session,
//! including one whose cwd is a worktree (that was the documented bug this rule exists for).
//!
//! A write can arrive as an `Edit`/`Write`/`NotebookEdit`/`MultiEdit` call naming the file
//! directly, or as a `Bash`/`PowerShell` command whose shape writes to a file: a `>`/`>>`
//! redirection, `tee`, `sed -i`, the destination of `cp`/`mv`/`rsync`, or the paths given to
//! `git checkout --`/`git restore`. A command that writes through an interpreter (`python`,
//! `node`, a heredoc script) is not a recognised shape and is not caught here — see
//! `guardrail.main_tree_write`, emitted after the fact by the post-tool hook.

use std::path::Path;

use serde_json::Value;

use super::eval::GuardContext;
use super::paths::resolve_tool_path;
use super::rules::Rule;
use super::segment::{scan_token, segments, tokenize};
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
    let target = resolve_tool_path(raw, &ctx.cwd, cfg!(windows));
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
/// Not a shell parser: tokenizes with `segment::tokenize`, a small quote-aware scanner that
/// keeps a quoted argument containing a space as one token and strips its quotes (`cp /tmp/x
/// "src/my dir/file.rs"` resolves the real destination instead of splitting on the internal
/// space) without needing to fully parse the shell grammar.
fn shape_targets(segment: &str) -> Vec<String> {
    let mut out = redirection_targets(segment);
    let tokens = tokenize(segment);
    match tokens.first().map(String::as_str) {
        Some("tee") => out.extend(tokens[1..].iter().filter(|t| !t.starts_with('-')).cloned()),
        Some("sed") => {
            if tokens.iter().any(|t| t == "-i" || t.starts_with("-i")) {
                if let Some(last) = tokens.last() {
                    out.push(last.clone());
                }
            }
        }
        Some("cp") | Some("mv") | Some("rsync") => {
            // At least a source and a destination; the destination is the last argument.
            if tokens.len() >= 3 {
                if let Some(last) = tokens.last() {
                    out.push(last.clone());
                }
            }
        }
        Some("git") => match tokens.get(1).map(String::as_str) {
            Some("checkout") => {
                if let Some(pos) = tokens.iter().position(|t| t.as_str() == "--") {
                    out.extend(tokens[pos + 1..].iter().cloned());
                }
            }
            Some("restore") => {
                out.extend(tokens[2..].iter().filter(|t| !t.starts_with('-')).cloned())
            }
            _ => {}
        },
        _ => {}
    }
    out.into_iter().filter(|t| !t.is_empty()).collect()
}

/// Targets of `>`/`>>` redirections (a prefix such as the `2` in `2>` or the `&` in `&>` is not
/// significant here — only the arrow itself is located), skipping `N>&M` file-descriptor
/// duplication (the character right after the arrow(s) is `&`, not a path). A `>` inside a
/// quoted argument is not an operator. The destination is parsed with the same quote-aware
/// `segment::scan_token` scanner `shape_targets` uses, whether or not whitespace separates it
/// from the arrow (`>"path"`, `>> 'path'`), so a quoted destination containing a space resolves
/// to one token instead of being cut at the space.
fn redirection_targets(segment: &str) -> Vec<String> {
    let chars: Vec<char> = segment.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < n {
        let ch = chars[i];
        if let Some(q) = quote {
            if ch == '\\' && q == '"' && i + 1 < n {
                i += 2;
                continue;
            }
            if ch == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if ch == '\\' && i + 1 < n {
            i += 2;
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            i += 1;
            continue;
        }
        if ch != '>' {
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
            if let Some((tok, next)) = scan_token(&chars, j) {
                if !tok.is_empty() {
                    out.push(tok);
                }
                i = next;
                continue;
            }
        }
        i = j.max(i + 1);
    }
    out
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
            big_read_lines: crate::config::DEFAULT_BIG_READ_LINES,
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

    /// A repo with a tracked file, an untracked file, and a worktree-shadowed file all sharing a
    /// directory whose name has a space, to exercise quote-aware target resolution.
    fn repo_with_spaced_paths() -> (tempfile::TempDir, GuardContext) {
        let d = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(d.path().join("src/my dir")).unwrap();
        fs::write(d.path().join("src/my dir/file.rs"), "x").unwrap();
        git(d.path(), &["init", "-q"]);
        git(d.path(), &["add", "src/my dir/file.rs"]);
        git(d.path(), &["commit", "-q", "-m", "i"]);
        fs::write(d.path().join("src/my dir/untracked.rs"), "y").unwrap();
        let wt_dir = d.path().join(".worktrees");
        fs::create_dir_all(wt_dir.join("wt/src/my dir")).unwrap();
        fs::write(wt_dir.join("wt/src/my dir/file.rs"), "z").unwrap();
        let ctx = GuardContext {
            main_root: Some(d.path().to_path_buf()),
            worktrees_dir: Some(wt_dir),
            cwd: d.path().to_path_buf(),
            has_venv: false,
            scratchpad: None,
            big_read_lines: crate::config::DEFAULT_BIG_READ_LINES,
        };
        (d, ctx)
    }

    #[test]
    fn quoted_destination_with_a_space_is_still_blocked() {
        let (_d, ctx) = repo_with_spaced_paths();
        let shapes = [
            r#"cp /tmp/x "src/my dir/file.rs""#,
            r#"mv /tmp/x "src/my dir/file.rs""#,
            r#"printf y >> "src/my dir/file.rs""#,
            r#"git restore "src/my dir/file.rs""#,
            r#"git checkout -- "src/my dir/file.rs""#,
        ];
        for shape in shapes {
            assert!(
                writes_main_tree("Bash", &serde_json::json!({ "command": shape }), &ctx),
                "expected a block for: {shape}"
            );
        }
    }

    #[test]
    fn redirection_target_quoted_with_no_space_before_the_operator_is_still_blocked() {
        let (_d, ctx) = repo_with_spaced_paths();
        assert!(writes_main_tree(
            "Bash",
            &serde_json::json!({ "command": r#"echo y >"src/my dir/file.rs""# }),
            &ctx
        ));
        assert!(writes_main_tree(
            "Bash",
            &serde_json::json!({ "command": r#"echo y >>'src/my dir/file.rs'"# }),
            &ctx
        ));
    }

    #[test]
    fn quoted_destination_untracked_or_inside_a_worktree_is_allowed() {
        let (_d, ctx) = repo_with_spaced_paths();
        assert!(!writes_main_tree(
            "Bash",
            &serde_json::json!({ "command": r#"cp /tmp/x "src/my dir/untracked.rs""# }),
            &ctx
        ));
        assert!(!writes_main_tree(
            "Bash",
            &serde_json::json!({ "command": r#"cp /tmp/x ".worktrees/wt/src/my dir/file.rs""# }),
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
