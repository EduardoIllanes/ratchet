//! Pure evaluation of a tool call against a rule list. No I/O except the tracked-file check
//! delegated to `main_tree` (which only runs when a write already points into the main tree).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use fancy_regex::Regex;
use serde_json::Value;

use super::big_read::blocks_big_read;
use super::main_tree::writes_main_tree;
use super::rules::{Kind, Rule};
use super::segment::segments;
use crate::repo::within;

/// Longest text a rule scans; longer inputs are scanned up to this many bytes.
#[allow(dead_code)] // consumed by Task 9/10
pub const SCAN_CAP: usize = 262_144;

// Fields consumed by Task 9 (hooks::dispatch) when building the real context.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct GuardContext {
    pub main_root: Option<PathBuf>,
    pub worktrees_dir: Option<PathBuf>,
    pub cwd: PathBuf,
    pub has_venv: bool,
    pub scratchpad: Option<PathBuf>,
    /// `[guardrails] big_read_lines` (default `config::DEFAULT_BIG_READ_LINES`): the line-count
    /// threshold the `big-read` rule blocks over.
    pub big_read_lines: usize,
}

// render() is consumed by Task 9 (hooks::dispatch) to print the block message.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub rule_id: String,
    pub message: String,
    pub alternative: String,
}

impl Violation {
    /// A repo-declared inline rule (`[[guardrails.rules]]`, T-0012) has no separate
    /// `alternative` — its `message` already states it — so `alternative` is the empty string
    /// there; this omits the trailing space that a naive `"{} {}"` would otherwise leave.
    #[allow(dead_code)] // consumed by Task 9 (hooks::dispatch)
    pub fn render(&self) -> String {
        if self.alternative.is_empty() {
            format!("[ratchet guardrail:{}] {}", self.rule_id, self.message)
        } else {
            format!(
                "[ratchet guardrail:{}] {} {}",
                self.rule_id, self.message, self.alternative
            )
        }
    }
}

// Consumed by Task 9 (hooks::dispatch) and Task 10 (guardrails::cli).
#[allow(dead_code)]
pub fn evaluate(
    rules: &[&Rule],
    tool_name: &str,
    tool_input: &Value,
    ctx: &GuardContext,
) -> Option<Violation> {
    for rule in rules {
        if !rule.tools.iter().any(|t| t == tool_name) {
            continue;
        }
        if rule.requires.as_deref() == Some("venv") && !ctx.has_venv {
            continue;
        }
        if matches(rule, tool_name, tool_input, ctx) {
            return Some(Violation {
                rule_id: rule.id.clone(),
                message: rule.message.clone(),
                alternative: rule.alternative.clone(),
            });
        }
    }
    None
}

// Consumed by Task 9 (hooks::dispatch) to build GuardContext.scratchpad.
#[allow(dead_code)]
pub fn scratchpad_from_env(env: &HashMap<String, String>) -> Option<PathBuf> {
    if let Some(s) = env.get("CLAUDE_SCRATCHPAD").filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(s));
    }
    env.get("LOCALAPPDATA")
        .filter(|s| !s.is_empty())
        .map(|l| PathBuf::from(l).join("Temp").join("claude"))
}

fn matches(rule: &Rule, tool_name: &str, tool_input: &Value, ctx: &GuardContext) -> bool {
    if rule.kind == Kind::MainTree {
        return writes_main_tree(tool_name, tool_input, ctx);
    }
    if rule.kind == Kind::BigRead {
        return blocks_big_read(tool_name, tool_input, ctx);
    }
    let text = text_for(rule.kind, tool_input);
    let text = cap(&text);
    let Some(pattern) = rule.pattern.as_deref().and_then(compile) else {
        return false;
    };
    let exempt = rule.exempt.as_deref().and_then(compile);
    if rule.kind != Kind::Command {
        return is_match(&pattern, text)
            && !exempt.as_ref().map(|e| is_match(e, text)).unwrap_or(false);
    }
    for segment in segments(text) {
        if !is_match(&pattern, &segment) {
            continue;
        }
        if exempt
            .as_ref()
            .map(|e| is_match(e, &segment))
            .unwrap_or(false)
        {
            continue;
        }
        if only_rm_under_scratchpad(&segment, ctx.scratchpad.as_deref()) {
            continue;
        }
        return true;
    }
    false
}

fn compile(pattern: &str) -> Option<Regex> {
    Regex::new(&format!("(?im){pattern}")).ok()
}

fn is_match(re: &Regex, text: &str) -> bool {
    re.is_match(text).unwrap_or(false)
}

fn cap(text: &str) -> &str {
    if text.len() <= SCAN_CAP {
        return text;
    }
    let mut end = SCAN_CAP;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn text_for(kind: Kind, tool_input: &Value) -> String {
    let get = |k: &str| tool_input.get(k).and_then(Value::as_str).unwrap_or("");
    match kind {
        Kind::Command => get("command").to_string(),
        Kind::FilePath => {
            let fp = get("file_path");
            if fp.is_empty() {
                get("notebook_path").to_string()
            } else {
                fp.to_string()
            }
        }
        // MainTree/BigRead never reach here: `matches()` dispatches both to their own module
        // before falling through to this pattern-based path.
        Kind::Content | Kind::MainTree | Kind::BigRead => {
            ["command", "content", "new_string", "new_source"]
                .iter()
                .map(|k| get(k))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

/// True when the segment is an `rm …` whose every non-flag argument is under the scratchpad.
/// A `git …` destructive command sharing the segment stays blocked.
fn only_rm_under_scratchpad(segment: &str, scratchpad: Option<&Path>) -> bool {
    let Some(scratch) = scratchpad else {
        return false;
    };
    if !is_match(&compile(r"^\s*rm\s+").unwrap(), segment) {
        return false;
    }
    if is_match(
        &compile(r"git\s+(push|reset|checkout|clean)").unwrap(),
        segment,
    ) {
        return false;
    }
    let targets: Vec<&str> = segment
        .split_whitespace()
        .skip(1)
        .filter(|tok| !tok.starts_with('-'))
        .map(|t| t.trim_matches(|c| c == '"' || c == '\''))
        .collect();
    if targets.is_empty() {
        return false;
    }
    targets.iter().all(|t| within(Path::new(t), scratch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guardrails::rules::builtin_rules;
    use serde_json::json;
    use std::path::PathBuf;

    fn ctx(has_venv: bool, scratchpad: Option<PathBuf>) -> GuardContext {
        GuardContext {
            main_root: Some(PathBuf::from("C:/r")),
            worktrees_dir: Some(PathBuf::from("C:/r/.worktrees")),
            cwd: PathBuf::from("C:/r"),
            has_venv,
            scratchpad,
            big_read_lines: crate::config::DEFAULT_BIG_READ_LINES,
        }
    }

    fn bash(cmd: &str, c: &GuardContext) -> Option<String> {
        let rules = builtin_rules();
        let refs: Vec<&Rule> = rules.iter().collect();
        evaluate(&refs, "Bash", &json!({ "command": cmd }), c).map(|v| v.rule_id)
    }

    #[test]
    fn python_venv_blocks_and_exempts() {
        let c = ctx(true, None);
        assert_eq!(
            bash("python scripts/x.py", &c).as_deref(),
            Some("python-venv")
        );
        assert_eq!(
            bash("PYTHONPATH=. python x.py", &c).as_deref(),
            Some("python-venv")
        );
        assert_eq!(bash("uv run pytest -q", &c), None);
        assert_eq!(bash(r".venv\Scripts\python.exe x.py", &c), None);
        assert_eq!(
            bash("echo ok && python x.py", &c).as_deref(),
            Some("python-venv")
        );
        assert_eq!(bash(r#"uv run x -c "a; mypy b""#, &c), None);
    }

    #[test]
    fn python_venv_needs_a_venv() {
        assert_eq!(bash("python scripts/x.py", &ctx(false, None)), None);
    }

    #[test]
    fn git_destructive_cases() {
        let c = ctx(false, None);
        for cmd in [
            "git reset --hard HEAD~1",
            "git push -f origin main",
            "git push origin main --force",
            "git checkout -- .",
            "git clean -fd",
            "rm -rf build",
            "rm -r -f build",
            "rm --recursive --force build",
        ] {
            assert_eq!(bash(cmd, &c).as_deref(), Some("git-destructive"), "{cmd}");
        }
        for cmd in [
            "git push --force-with-lease",
            "git reset --soft HEAD~1",
            "rm -r build",
            "rm build.txt",
            "git clean -n",
        ] {
            assert_eq!(bash(cmd, &c), None, "{cmd}");
        }
    }

    #[test]
    fn rm_under_scratchpad_is_allowed_only_when_all_targets_are_inside() {
        let scratch = tempfile::TempDir::new().unwrap();
        let c = ctx(false, Some(scratch.path().to_path_buf()));
        let inside = scratch.path().join("tmp");
        assert_eq!(bash(&format!("rm -rf {}", inside.display()), &c), None);
        assert_eq!(
            bash(&format!("rm -rf {} C:/r/src", inside.display()), &c).as_deref(),
            Some("git-destructive")
        );
        assert_eq!(
            bash("rm -rf C:/r/src", &c).as_deref(),
            Some("git-destructive")
        );
    }

    #[test]
    fn env_files_by_path() {
        let rules = builtin_rules();
        let refs: Vec<&Rule> = rules.iter().collect();
        let c = ctx(false, None);
        let hit = evaluate(
            &refs,
            "Write",
            &json!({ "file_path": "C:/r/.env.local", "content": "" }),
            &c,
        );
        assert_eq!(hit.map(|v| v.rule_id).as_deref(), Some("env-files"));
        let ok = evaluate(
            &refs,
            "Write",
            &json!({ "file_path": "C:/r/environment.md", "content": "" }),
            &c,
        );
        assert!(ok.is_none());
        let ps = evaluate(
            &refs,
            "PowerShell",
            &json!({ "command": "Set-Content .env x" }),
            &c,
        );
        assert!(
            ps.is_none(),
            "file_path rules look at file_path, not commands"
        );
    }

    #[test]
    fn content_rule_scans_command_content_and_new_string() {
        let custom = Rule {
            id: "db".into(),
            tools: vec!["Bash".into(), "Write".into(), "Edit".into()],
            kind: Kind::Content,
            pattern: Some(r"\.purge_all\s*\(".into()),
            exempt: None,
            requires: None,
            message: "m".into(),
            alternative: "a".into(),
            source: "repo".into(),
        };
        let refs = vec![&custom];
        let c = ctx(false, None);
        assert!(evaluate(
            &refs,
            "Write",
            &json!({ "file_path": "x.py", "content": "c.purge_all({})" }),
            &c
        )
        .is_some());
        assert!(evaluate(
            &refs,
            "Edit",
            &json!({ "file_path": "x.py", "new_string": "c.purge_all (" }),
            &c
        )
        .is_some());
        assert!(evaluate(
            &refs,
            "Bash",
            &json!({ "command": "uv run python -c 'c.purge_all({})'" }),
            &c
        )
        .is_some());
        assert!(evaluate(
            &refs,
            "Write",
            &json!({ "file_path": "x.py", "content": "c.find({})" }),
            &c
        )
        .is_none());
    }

    #[test]
    fn tool_filter_and_order() {
        let rules = builtin_rules();
        let refs: Vec<&Rule> = rules.iter().collect();
        let c = ctx(true, None);
        assert!(evaluate(&refs, "Read", &json!({ "command": "python x.py" }), &c).is_none());
        let v = evaluate(
            &refs,
            "Bash",
            &json!({ "command": "python x.py; git reset --hard" }),
            &c,
        )
        .unwrap();
        assert_eq!(v.rule_id, "python-venv");
        assert_eq!(
            v.render(),
            format!(
                "[ratchet guardrail:python-venv] {} {}",
                v.message, v.alternative
            )
        );
    }

    #[test]
    fn render_omits_the_trailing_space_when_alternative_is_empty() {
        let v = Violation {
            rule_id: "no-curl".into(),
            message: "Use the repo's fetch script instead.".into(),
            alternative: String::new(),
        };
        assert_eq!(
            v.render(),
            "[ratchet guardrail:no-curl] Use the repo's fetch script instead."
        );
    }

    #[test]
    fn scratchpad_from_env_prefers_claude_var() {
        let mut env = std::collections::HashMap::new();
        env.insert("LOCALAPPDATA".to_string(), "C:/u/l".to_string());
        assert_eq!(
            scratchpad_from_env(&env),
            Some(PathBuf::from("C:/u/l").join("Temp").join("claude"))
        );
        env.insert("CLAUDE_SCRATCHPAD".to_string(), "C:/s".to_string());
        assert_eq!(scratchpad_from_env(&env), Some(PathBuf::from("C:/s")));
    }
}
