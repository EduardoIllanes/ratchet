//! The `big-read` rule: a whole-file `Read` (no `offset`/`limit`), or a `cat`/`head`/`tail`/
//! `less`/`more` `Bash`/`PowerShell` segment with no pipe, over a regular file of more than
//! `[guardrails] big_read_lines` lines sends that whole file into the orchestrator's own
//! context. The rule exempts a file under the repo's worktrees directory (that is where
//! implementers read whole files on purpose), a file that does not exist, and — for the
//! command shape — a segment that pipes into something else (`cat big.rs | grep fn` already
//! filters before anything reaches the transcript).
//!
//! Line counting always gives the true count — no shortcut here is allowed to under-block a
//! file that really is over the threshold. The byte size already read via `fs::metadata` (cheap,
//! needed anyway to confirm this is a regular file) still buys a real latency win: a file with
//! fewer bytes than the threshold cannot possibly have that many lines (a line costs at least
//! one byte, its own newline), so the read is skipped outright for anything that small. Above
//! that floor, the read stops the moment it has seen more than the threshold's worth of lines,
//! so a huge file never costs more than a bounded prefix.

use std::fs;
use std::io::BufRead;
use std::path::Path;

use serde_json::Value;

use super::eval::GuardContext;
use super::paths::resolve_tool_path;
use super::segment::tokenize;
use crate::repo::within;

const READER_COMMANDS: [&str; 5] = ["cat", "head", "tail", "less", "more"];

pub fn blocks_big_read(tool_name: &str, tool_input: &Value, ctx: &GuardContext) -> bool {
    match tool_name {
        "Read" => blocks_read(tool_input, ctx),
        "Bash" | "PowerShell" => blocks_command(tool_input, ctx),
        _ => false,
    }
}

fn blocks_read(tool_input: &Value, ctx: &GuardContext) -> bool {
    if tool_input.get("offset").is_some() || tool_input.get("limit").is_some() {
        return false;
    }
    let raw = tool_input
        .get("file_path")
        .and_then(Value::as_str)
        .unwrap_or("");
    if raw.is_empty() {
        return false;
    }
    is_big_outside_worktree(raw, ctx)
}

fn blocks_command(tool_input: &Value, ctx: &GuardContext) -> bool {
    let command = tool_input
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("");
    if command.is_empty() {
        return false;
    }
    for (segment, piped) in pipe_aware_segments(command) {
        if piped {
            continue;
        }
        for raw in reader_command_targets(&segment) {
            if is_big_outside_worktree(&raw, ctx) {
                return true;
            }
        }
    }
    false
}

/// Non-flag arguments of a segment whose command is one of `READER_COMMANDS`; empty when the
/// segment's command is something else. Tokenizes with the shared `segment::tokenize` scanner
/// (quote-aware, not a full shell parser) so a quoted argument containing a space — `cat "src/my
/// dir/big file.rs"` — resolves as one token instead of being cut at the space.
fn reader_command_targets(segment: &str) -> Vec<String> {
    let tokens = tokenize(segment);
    let Some(first) = tokens.first() else {
        return Vec::new();
    };
    let name = first.to_ascii_lowercase();
    let name = name.trim_end_matches(".exe");
    if !READER_COMMANDS.contains(&name) {
        return Vec::new();
    }
    tokens[1..]
        .iter()
        .filter(|t| !t.starts_with('-'))
        .filter(|t| !t.is_empty())
        .cloned()
        .collect()
}

fn is_big_outside_worktree(raw: &str, ctx: &GuardContext) -> bool {
    let target = resolve_tool_path(raw, &ctx.cwd, cfg!(windows));
    if let Some(wt) = ctx.worktrees_dir.as_deref() {
        if within(&target, wt) {
            return false;
        }
    }
    exceeds_line_threshold(&target, ctx.big_read_lines)
}

/// True when `path` is an existing regular file with more than `big_read_lines` lines.
fn exceeds_line_threshold(path: &Path, big_read_lines: usize) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    if meta.len() <= big_read_lines as u64 {
        // Fewer bytes than the threshold: even a file of nothing but blank lines could not
        // reach it, so the read below is skipped outright.
        return false;
    }
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let mut count = 0usize;
    for line in std::io::BufReader::new(file).lines() {
        if line.is_err() {
            return false;
        }
        count += 1;
        if count > big_read_lines {
            return true;
        }
    }
    false
}

/// Split a command into top-level pieces on `;`, `&&`, `||` and newlines only — unlike
/// `segment::segments`, a lone `|` is kept INSIDE its piece rather than splitting it, and its
/// presence is reported as `true`, so a caller can tell "this reads a whole file into the
/// transcript" from "this reads a whole file into another command" (`cat big.rs | grep fn`).
fn pipe_aware_segments(command: &str) -> Vec<(String, bool)> {
    let chars: Vec<char> = command.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut has_pipe = false;
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < n {
        let ch = chars[i];
        match quote {
            Some(q) => {
                if ch == '\\' && q == '"' && i + 1 < n {
                    buf.push(ch);
                    buf.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if ch == q {
                    quote = None;
                }
                buf.push(ch);
            }
            None => {
                if ch == '\\' && i + 1 < n {
                    buf.push(ch);
                    buf.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                    buf.push(ch);
                } else if ch == ';' || ch == '\n' {
                    out.push((std::mem::take(&mut buf), has_pipe));
                    has_pipe = false;
                } else if ch == '|' {
                    if i + 1 < n && chars[i + 1] == '|' {
                        out.push((std::mem::take(&mut buf), has_pipe));
                        has_pipe = false;
                        i += 1;
                    } else {
                        has_pipe = true;
                        buf.push(ch);
                    }
                } else if ch == '&' && i + 1 < n && chars[i + 1] == '&' {
                    out.push((std::mem::take(&mut buf), has_pipe));
                    has_pipe = false;
                    i += 1;
                } else {
                    buf.push(ch);
                }
            }
        }
        i += 1;
    }
    out.push((buf, has_pipe));
    out.into_iter()
        .map(|(s, p)| (s.trim().to_string(), p))
        .filter(|(s, _)| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn ctx(root: &Path, cwd: &Path, big_read_lines: usize) -> GuardContext {
        GuardContext {
            main_root: Some(root.to_path_buf()),
            worktrees_dir: Some(root.join(".worktrees")),
            cwd: cwd.to_path_buf(),
            has_venv: false,
            scratchpad: None,
            big_read_lines,
        }
    }

    fn write_lines(path: &Path, n: usize) {
        let mut body = String::new();
        for i in 1..=n {
            body.push_str(&format!("// line {i}\n"));
        }
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn pipe_aware_segments_keeps_a_lone_pipe_inside_its_piece() {
        assert_eq!(
            pipe_aware_segments("cat big.rs | grep fn"),
            vec![("cat big.rs | grep fn".to_string(), true)]
        );
        assert_eq!(
            pipe_aware_segments("head -40 big.rs | cat"),
            vec![("head -40 big.rs | cat".to_string(), true)]
        );
    }

    #[test]
    fn pipe_aware_segments_splits_on_the_control_operators_not_on_a_pipe() {
        assert_eq!(
            pipe_aware_segments("cat a.rs; cat b.rs && cat c.rs || cat d.rs\ncat e.rs"),
            vec![
                ("cat a.rs".to_string(), false),
                ("cat b.rs".to_string(), false),
                ("cat c.rs".to_string(), false),
                ("cat d.rs".to_string(), false),
                ("cat e.rs".to_string(), false),
            ]
        );
    }

    #[test]
    fn pipe_aware_segments_ignores_a_pipe_inside_quotes() {
        assert_eq!(
            pipe_aware_segments(r#"echo "a|b""#),
            vec![(r#"echo "a|b""#.to_string(), false)]
        );
    }

    #[test]
    fn reader_command_targets_filters_flags_and_recognises_the_five_commands() {
        assert_eq!(
            reader_command_targets("cat src/big.rs"),
            vec!["src/big.rs".to_string()]
        );
        assert_eq!(
            reader_command_targets("head -n 40 src/big.rs"),
            vec!["40".to_string(), "src/big.rs".to_string()]
        );
        assert_eq!(
            reader_command_targets("grep fn src/big.rs"),
            Vec::<String>::new()
        );
        assert_eq!(
            reader_command_targets("CAT.EXE src/big.rs"),
            vec!["src/big.rs".to_string()]
        );
    }

    #[test]
    fn exceeds_line_threshold_true_line_count_not_a_byte_guess() {
        let d = tempfile::TempDir::new().unwrap();
        // Deliberately short lines: total bytes stay under `big_read_lines * 16`, proving the
        // byte floor below never causes a false "small" verdict for a file that really is big.
        let big = d.path().join("big.rs");
        write_lines(&big, 400);
        assert!(fs::metadata(&big).unwrap().len() < 350 * 16);
        assert!(exceeds_line_threshold(&big, 350));

        let small = d.path().join("small.rs");
        write_lines(&small, 349);
        assert!(!exceeds_line_threshold(&small, 350));
    }

    #[test]
    fn exceeds_line_threshold_skips_the_read_for_a_file_smaller_than_the_line_floor() {
        let d = tempfile::TempDir::new().unwrap();
        let tiny = d.path().join("tiny.rs");
        fs::write(&tiny, "x").unwrap(); // 1 byte, threshold 350: can never have 350 lines
        assert!(!exceeds_line_threshold(&tiny, 350));
    }

    #[test]
    fn exceeds_line_threshold_false_for_missing_file_or_a_directory() {
        let d = tempfile::TempDir::new().unwrap();
        assert!(!exceeds_line_threshold(&d.path().join("nope.rs"), 1));
        assert!(!exceeds_line_threshold(d.path(), 1));
    }

    #[test]
    fn blocks_read_respects_offset_and_limit_and_the_worktree_exemption() {
        let d = tempfile::TempDir::new().unwrap();
        let root = d.path();
        let big = root.join("big.rs");
        write_lines(&big, 400);
        let c = ctx(root, root, 350);

        assert!(blocks_big_read(
            "Read",
            &json!({ "file_path": big.to_string_lossy() }),
            &c
        ));
        assert!(!blocks_big_read(
            "Read",
            &json!({ "file_path": big.to_string_lossy(), "limit": 80 }),
            &c
        ));
        assert!(!blocks_big_read(
            "Read",
            &json!({ "file_path": big.to_string_lossy(), "offset": 10 }),
            &c
        ));

        let in_wt = root.join(".worktrees/wt/big.rs");
        write_lines(&in_wt, 400);
        assert!(!blocks_big_read(
            "Read",
            &json!({ "file_path": in_wt.to_string_lossy() }),
            &c
        ));
    }

    #[test]
    fn blocks_command_cat_blocked_piped_cat_allowed_powershell_too() {
        let d = tempfile::TempDir::new().unwrap();
        let root = d.path();
        write_lines(&root.join("src/big.rs"), 400);
        let c = ctx(root, root, 350);

        assert!(blocks_big_read(
            "Bash",
            &json!({ "command": "cat src/big.rs" }),
            &c
        ));
        assert!(blocks_big_read(
            "PowerShell",
            &json!({ "command": "cat src/big.rs" }),
            &c
        ));
        assert!(!blocks_big_read(
            "Bash",
            &json!({ "command": "cat src/big.rs | grep fn" }),
            &c
        ));
        assert!(!blocks_big_read(
            "Bash",
            &json!({ "command": "head -40 src/big.rs | cat" }),
            &c
        ));
        assert!(!blocks_big_read(
            "Bash",
            &json!({ "command": "grep fn src/big.rs" }),
            &c
        ));
    }

    #[test]
    fn quoted_command_target_with_a_space_is_still_blocked() {
        let d = tempfile::TempDir::new().unwrap();
        let root = d.path();
        write_lines(&root.join("src/my dir/big file.rs"), 400);
        let c = ctx(root, root, 350);

        assert!(blocks_big_read(
            "Bash",
            &json!({ "command": r#"cat "src/my dir/big file.rs""# }),
            &c
        ));
        assert!(blocks_big_read(
            "PowerShell",
            &json!({ "command": r#"cat "src/my dir/big file.rs""# }),
            &c
        ));
    }

    #[test]
    fn quoted_command_target_untracked_small_or_in_a_worktree_is_allowed() {
        let d = tempfile::TempDir::new().unwrap();
        let root = d.path();
        write_lines(&root.join("src/my dir/small file.rs"), 10);
        write_lines(&root.join(".worktrees/wt/src/my dir/big file.rs"), 400);
        let c = ctx(root, root, 350);

        assert!(!blocks_big_read(
            "Bash",
            &json!({ "command": r#"cat "src/my dir/small file.rs""# }),
            &c
        ));
        assert!(!blocks_big_read(
            "Bash",
            &json!({ "command": r#"cat ".worktrees/wt/src/my dir/big file.rs""# }),
            &c
        ));
    }

    #[test]
    fn blocks_read_false_for_a_file_that_does_not_exist() {
        let d = tempfile::TempDir::new().unwrap();
        let root = d.path();
        let c = ctx(root, root, 350);
        assert!(!blocks_big_read(
            "Read",
            &json!({ "file_path": root.join("nope.rs").to_string_lossy() }),
            &c
        ));
    }
}
