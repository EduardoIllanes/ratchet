//! Pure JSONL transcript parsing. No I/O: callers (`cli::usage_cmd`, Task 4) read the file and
//! hand this module bytes. Tolerant per D-usage-tolerant: nothing here ever panics or fails on a
//! record it does not understand.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::clock;
use crate::hooks::dispatch::sanitize_id;

/// `RATCHET_CLAUDE_PROJECTS` when set and non-empty, else `~/.claude/projects`. Checked before
/// the database is ever opened (R1: a missing directory fails naming the path, exit 1).
// Consumed by cli::usage_cmd (Task 4) and services::tasks' H3 transcript check (T-0016), through
// cli::task_cmd -- both need the same env resolution and the same "must already exist" rule.
pub fn projects_dir(env: &HashMap<String, String>) -> Result<PathBuf, String> {
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

/// Outcome of confining one transcript/subagent path to the canonicalized projects dir, checked
/// right before that path is ever read (Blocking finding 1, T-0007: a session id of
/// `../../outside-secret` used to walk `ratchet usage --by session` two directories above the
/// projects dir and fold its tokens in).
#[derive(Debug, PartialEq, Eq)]
pub enum Confinement {
    /// Nothing at all is on disk at this path — the ordinary "this session/agent never wrote
    /// here" case (Requirement 1's "no transcript" row), not a refusal.
    Missing,
    /// Something is on disk here, but either it is not the confinement's own file/directory type
    /// (a symlink planted at the path itself, `symlink_metadata`'s own type, is refused
    /// unconditionally, wherever it points) or its parent does not canonicalize to somewhere
    /// inside the projects dir (including a canonicalize failure). Never read.
    Refused,
    /// Confirmed present, of the expected type, and confined. Safe to read.
    Present,
}

/// Two independent layers against a crafted or planted path, mirroring
/// `hooks::dispatch::subagent_meta`'s own two-layer defense: `session_id` (and, via the
/// filenames built under it, `agent_id`) is already run through `hooks::dispatch::sanitize_id`
/// before `path` is ever built (`transcript_path`/`subagents_dir`), which keeps a crafted id from
/// introducing a `..` segment or an absolute path of its own; this function is the second,
/// independent layer, checked right before the read. `symlink_metadata` (which does NOT follow a
/// symlink) on `path` itself must show the expected type -- `want_dir` for the subagents
/// directory, a plain file for everything else -- so a symlink planted at a transcript's own path
/// is refused wherever it points, never followed; and `path`'s parent must canonicalize to
/// somewhere inside `projects_canon` (a canonicalize failure, including a parent that does not
/// exist, refuses the path -- it never falls back to reading the raw one).
// Consumed by cli::usage_cmd (Task 4/T-0007) and, for T-0016's H3 review-transcript check, by
// cli::task_cmd through `read_identity_transcript` below -- the one path-confinement rule shared
// by every reader of a Claude Code transcript.
pub fn confine(path: &Path, projects_canon: &Path, want_dir: bool) -> Confinement {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return Confinement::Missing,
    };
    let kind_ok = if want_dir {
        meta.file_type().is_dir()
    } else {
        meta.file_type().is_file()
    };
    if !kind_ok {
        return Confinement::Refused;
    }
    let Some(parent) = path.parent() else {
        return Confinement::Refused;
    };
    match parent.canonicalize() {
        Ok(canon) if canon.starts_with(projects_canon) => Confinement::Present,
        _ => Confinement::Refused,
    }
}

/// The four token classes plus thinking, all defaulting to zero for a record that lacks them.
// Consumed by usage::attribute (Task 3) and cli::usage_cmd (Task 4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub thinking: u64,
}

/// One understood `"type": "assistant"` record.
// Consumed by usage::attribute (Task 3) and cli::usage_cmd (Task 4).
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub ts: DateTime<Utc>,
    pub model: String,
    pub usage: Usage,
    /// `name` of every `tool_use` block in `message.content`, in order.
    pub tools: Vec<String>,
    /// `id` of every `tool_use` block in `message.content`, in the same order as `tools`.
    pub tool_use_ids: Vec<String>,
}

/// Everything one transcript file's bytes turn into: the understood calls, plus the two counts
/// and the highest `version` D-usage-tolerant requires every report to surface.
// Consumed by cli::usage_cmd (Task 4).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParseResult {
    pub calls: Vec<Call>,
    pub skipped: u64,
    pub partial: u64,
    pub max_version: Option<String>,
}

#[derive(Deserialize)]
struct RawRecord {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    version: Option<String>,
    message: Option<RawMessage>,
}

#[derive(Deserialize)]
struct RawMessage {
    model: Option<String>,
    usage: Option<RawUsage>,
    // T-0014: kept as a bare `Value`, not `Option<Vec<Value>>` -- a real record's `content` is
    // normally an array of blocks, but a record with valid usage and STRING content used to fail
    // `RawMessage`'s whole deserialization (a JSON string cannot deserialize into
    // `Option<Vec<Value>>`), which lost the record's tokens by counting it `skipped` instead of
    // `partial`. Kept as `Value` so any shape parses; `parse()` below checks `is_array()` itself
    // and treats a present-but-non-array value as the degenerate case R2 already has a name for:
    // `partial`, tokens kept.
    content: Option<Value>,
}

#[derive(Deserialize)]
struct RawUsage {
    input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    output_tokens_details: Option<RawThinking>,
}

#[derive(Deserialize)]
struct RawThinking {
    thinking_tokens: Option<u64>,
}

/// Parses one transcript's raw bytes, line by line. R2: a line that is not valid JSON counts as
/// `skipped`; a well-formed record whose `type` is not `"assistant"` is silently ignored (not
/// counted anywhere — R2 only names garbage and no-usage records as countable); an `assistant`
/// record with no `message.usage` becomes a `Call` with every class at zero and counts as
/// `partial`; a transcript that ends with zero understood (assistant) records adds exactly one to
/// `skipped`, once, regardless of how many garbage lines it had.
// Consumed by cli::usage_cmd (Task 4).
pub fn parse(bytes: &[u8]) -> ParseResult {
    let mut out = ParseResult::default();
    let text = String::from_utf8_lossy(bytes);
    let mut understood_any = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let raw: RawRecord = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(_) => {
                out.skipped += 1;
                continue;
            }
        };
        if let Some(v) = &raw.version {
            if newer(&out.max_version, v) {
                out.max_version = Some(v.clone());
            }
        }
        if raw.kind.as_deref() != Some("assistant") {
            continue;
        }
        understood_any = true;
        // Lenient like every DB row read in `model.rs`: an unparseable timestamp falls back to
        // the epoch rather than dropping the call, so one bad field never loses real tokens.
        let ts = raw
            .timestamp
            .as_deref()
            .and_then(clock::parse)
            .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"));
        let model = raw
            .message
            .as_ref()
            .and_then(|m| m.model.clone())
            .unwrap_or_default();
        let raw_usage = raw.message.as_ref().and_then(|m| m.usage.as_ref());
        let raw_content = raw.message.as_ref().and_then(|m| m.content.as_ref());
        // T-0014: a present, non-array `content` (e.g. a plain string) can't be read for
        // `tool_use` blocks -- degenerate the same way a missing `usage` already is: counted
        // `partial`, never `skipped`, and the call's tokens (from `usage`, independent of
        // `content`) are kept.
        let content_is_array = raw_content.map(Value::is_array).unwrap_or(true);
        if raw_usage.is_none() || !content_is_array {
            out.partial += 1;
        }
        let usage = raw_usage
            .map(|u| Usage {
                input: u.input_tokens.unwrap_or(0),
                cache_write: u.cache_creation_input_tokens.unwrap_or(0),
                cache_read: u.cache_read_input_tokens.unwrap_or(0),
                output: u.output_tokens.unwrap_or(0),
                thinking: u
                    .output_tokens_details
                    .as_ref()
                    .and_then(|t| t.thinking_tokens)
                    .unwrap_or(0),
            })
            .unwrap_or_default();
        let (tools, tool_use_ids) = extract_tools(raw_content.and_then(Value::as_array));
        out.calls.push(Call {
            ts,
            model,
            usage,
            tools,
            tool_use_ids,
        });
    }
    if !understood_any {
        out.skipped += 1;
    }
    out
}

fn extract_tools(content: Option<&Vec<Value>>) -> (Vec<String>, Vec<String>) {
    let mut tools = Vec::new();
    let mut ids = Vec::new();
    if let Some(items) = content {
        for item in items {
            if item.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            if let Some(name) = item.get("name").and_then(Value::as_str) {
                tools.push(name.to_string());
            }
            if let Some(id) = item.get("id").and_then(Value::as_str) {
                ids.push(id.to_string());
            }
        }
    }
    (tools, ids)
}

/// Coarse semver-ish compare: numeric dot components, left to right; a non-numeric component
/// compares as 0. Good enough to pick "the highest version seen" among the small, well-formed
/// version strings Claude Code writes (D-usage-tolerant only needs "highest", not a full semver
/// implementation).
// `pub(crate)`: `cli::usage_cmd`'s own `bump_version` calls this instead of carrying a second
// copy of the same numeric-dot compare (fix round).
pub(crate) fn newer(current: &Option<String>, candidate: &str) -> bool {
    match current {
        None => true,
        Some(cur) => version_key(candidate) > version_key(cur),
    }
}

pub(crate) fn version_key(v: &str) -> Vec<u64> {
    v.split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .collect()
}

/// `<slug>` per R1: the session's recorded `cwd` with every character that is not an ASCII
/// letter or digit replaced by `-` -- matching how Claude Code itself names a project directory
/// (verified against real `~/.claude/projects` directories: e.g. a `cwd` of
/// `/Users/e/.claude-mem/observer-sessions` becomes `-Users-e--claude-mem-observer-sessions`,
/// the dot included).
// Consumed by cli::usage_cmd (Task 4).
pub fn slug_for(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// `<projects>/<slug>/<session id>.jsonl`. `session_id` comes from the `sessions` table, which
/// hook input populates and is therefore untrusted the same way a hook's own `session_id` is
/// (fix round: a session id of `../../outside-secret` used to walk `ratchet usage` two
/// directories above the projects dir) -- run through the same `[A-Za-z0-9_-]` rule
/// `hooks::dispatch::sanitize_id` applies to hook-supplied identifiers before it ever touches a
/// path, so no crafted id can introduce a path separator or a `..` segment here either.
// Consumed by cli::usage_cmd (Task 4).
pub fn transcript_path(projects_dir: &Path, cwd: &str, session_id: &str) -> PathBuf {
    projects_dir
        .join(slug_for(cwd))
        .join(format!("{}.jsonl", sanitize_id(session_id)))
}

/// `<projects>/<slug>/<session id>/subagents/`. Same `session_id` sanitization as
/// `transcript_path`, and for the same reason.
// Consumed by cli::usage_cmd (Task 4).
pub fn subagents_dir(projects_dir: &Path, cwd: &str, session_id: &str) -> PathBuf {
    projects_dir
        .join(slug_for(cwd))
        .join(sanitize_id(session_id))
        .join("subagents")
}

/// The transcript path for one identity (T-0016): the session's own transcript for a bare
/// identity, or that subagent's `agent-<id>.jsonl` for a paired one.
// Consumed by cli::task_cmd's H3 review-transcript check.
pub fn identity_transcript_path(
    projects_dir: &Path,
    cwd: &str,
    session_id: &str,
    agent_id: Option<&str>,
) -> PathBuf {
    match agent_id {
        None => transcript_path(projects_dir, cwd, session_id),
        Some(a) => subagents_dir(projects_dir, cwd, session_id)
            .join(format!("agent-{}.jsonl", sanitize_id(a))),
    }
}

/// Reads one identity's transcript, confined to `projects_canon` exactly as `ratchet usage`
/// confines every transcript it reads (T-0007's fix: never trust a path built from a hook-
/// supplied identifier without checking it stayed inside the projects dir). `None` for anything
/// not confirmed present, of the right type, and confined — never a partial or refused read.
// Consumed by cli::task_cmd's H3 review-transcript check (T-0016).
pub fn read_identity_transcript(
    projects_dir: &Path,
    projects_canon: &Path,
    cwd: &str,
    session_id: &str,
    agent_id: Option<&str>,
) -> Option<Vec<u8>> {
    let path = identity_transcript_path(projects_dir, cwd, session_id, agent_id);
    match confine(&path, projects_canon, false) {
        Confinement::Present => std::fs::read(&path).ok(),
        Confinement::Missing | Confinement::Refused => None,
    }
}

/// Whether any `Bash` `tool_use` block among this transcript's assistant records carries a
/// command containing `needle` (T-0016's H3: does this identity's transcript actually run the
/// review it claims to have made). `parse`/`Call` above do not keep a tool's `input` — nothing
/// before this needed the command text itself — so this reads `content` directly instead of
/// going through `parse`.
// Consumed by cli::task_cmd's H3 review-transcript check.
pub fn contains_bash_command(bytes: &[u8], needle: &str) -> bool {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(content) = v.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for item in content {
            if item.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            if item.get("name").and_then(Value::as_str) != Some("Bash") {
                continue;
            }
            if let Some(cmd) = item.pointer("/input/command").and_then(Value::as_str) {
                if cmd.contains(needle) {
                    return true;
                }
            }
        }
    }
    false
}

/// The sibling `agent-<id>.meta.json` of a subagent transcript.
// Consumed by usage::attribute (Task 3) and cli::usage_cmd (Task 4).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meta {
    pub agent_type: Option<String>,
    pub description: Option<String>,
    // Part of the parsed shape per design §2; not read by usage::attribute (Task 3) -- each Call
    // already carries its own model from the transcript record itself, which is what every
    // bucket keys on. Same precedent as pdf.rs's Run.ms/Run.pages: parsed and kept, not (yet)
    // consumed.
    #[allow(dead_code)]
    pub model: Option<String>,
    pub tool_use_id: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawMeta {
    #[serde(rename = "agentType")]
    agent_type: Option<String>,
    description: Option<String>,
    model: Option<String>,
    #[serde(rename = "toolUseId")]
    tool_use_id: Option<String>,
}

/// `None` for anything that is not valid JSON — a missing meta file is normal (D-usage-subagents'
/// third fallback), so this is not an error type, just an `Option`.
// Consumed by cli::usage_cmd (Task 4).
pub fn read_meta(bytes: &[u8]) -> Option<Meta> {
    let raw: RawMeta = serde_json::from_slice(bytes).ok()?;
    Some(Meta {
        agent_type: raw.agent_type,
        description: raw.description,
        model: raw.model,
        tool_use_id: raw.tool_use_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_replaces_every_non_alphanumeric_character() {
        // R1 (fix round 1): matches how Claude Code itself names a project directory -- every
        // character that is not an ASCII letter or digit becomes `-`, not only path separators.
        // A dot in a directory name (e.g. `.worktrees`, `my.repo`) becomes `-` too, verified
        // against real `~/.claude/projects` entries on this machine.
        assert_eq!(slug_for("/Users/e/repo"), "-Users-e-repo");
        assert_eq!(slug_for("/Users/x/my.repo"), "-Users-x-my-repo");
        assert_eq!(slug_for("/r/.worktrees/t-0001"), "-r--worktrees-t-0001");
        assert_eq!(slug_for(r"C:\repos\demo"), "C--repos-demo");
    }

    #[test]
    fn transcript_and_subagents_paths_are_built_from_the_slug() {
        let root = Path::new("/home/e/.claude/projects");
        assert_eq!(
            transcript_path(root, "/repo", "s-1"),
            root.join("-repo").join("s-1.jsonl")
        );
        assert_eq!(
            subagents_dir(root, "/repo", "s-1"),
            root.join("-repo").join("s-1").join("subagents")
        );
    }

    #[test]
    fn a_traversal_session_id_yields_a_path_inside_the_projects_dir() {
        // Blocking finding, live repro: a session with id `../../outside-secret` made
        // `ratchet usage --by session` read a file two directories above the projects dir and
        // fold its tokens in. `session_id` is sanitized the same way a hook-supplied identifier
        // is (`hooks::dispatch::sanitize_id`), so every `.`/`/` in it becomes `_` and the joined
        // path can never leave `root.join("-repo")`.
        let root = Path::new("/home/e/.claude/projects");
        let evil = "../../outside-secret";

        let t = transcript_path(root, "/repo", evil);
        assert!(t.starts_with(root.join("-repo")), "{}", t.display());
        assert_eq!(t, root.join("-repo").join("______outside-secret.jsonl"));

        let d = subagents_dir(root, "/repo", evil);
        assert!(d.starts_with(root.join("-repo")), "{}", d.display());
        assert_eq!(
            d,
            root.join("-repo")
                .join("______outside-secret")
                .join("subagents")
        );
    }

    #[test]
    fn a_well_formed_call_parses_its_four_classes_and_tools() {
        let line = r#"{"type":"assistant","timestamp":"2026-09-16T12:00:00Z","version":"1.2.3","message":{"model":"claude-sonnet-5","usage":{"input_tokens":10,"cache_creation_input_tokens":1,"cache_read_input_tokens":2,"output_tokens":5,"output_tokens_details":{"thinking_tokens":3}},"content":[{"type":"tool_use","id":"tu-1","name":"Agent"}]}}"#;
        let r = parse(line.as_bytes());
        assert_eq!(r.calls.len(), 1);
        assert_eq!(r.skipped, 0);
        assert_eq!(r.partial, 0);
        let c = &r.calls[0];
        assert_eq!(c.model, "claude-sonnet-5");
        assert_eq!(
            c.usage,
            Usage {
                input: 10,
                cache_write: 1,
                cache_read: 2,
                output: 5,
                thinking: 3
            }
        );
        assert_eq!(c.tools, vec!["Agent".to_string()]);
        assert_eq!(c.tool_use_ids, vec!["tu-1".to_string()]);
        assert_eq!(r.max_version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn a_non_assistant_record_is_ignored_not_skipped() {
        let r = parse(br#"{"type":"user","timestamp":"2026-09-16T12:00:00Z"}"#);
        assert_eq!(r.calls.len(), 0);
        assert_eq!(r.skipped, 1, "zero understood records adds one, once");
        assert_eq!(r.partial, 0);
    }

    #[test]
    fn non_array_content_is_partial_but_keeps_its_tokens() {
        // T-0014: `RawMessage.content` used to be `Option<Vec<Value>>`, so a record with valid
        // `message.usage` but string `content` failed to deserialize at all and was counted
        // `skipped` -- losing its tokens. It now parses fine: `partial` (content couldn't be
        // read for tool_use blocks) but every token class from `usage` is kept.
        let line = br#"{"type":"assistant","timestamp":"2026-09-16T12:00:00Z","message":{"model":"m","usage":{"input_tokens":10,"output_tokens":5},"content":"a plain string, not an array"}}"#;
        let r = parse(line);
        assert_eq!(r.calls.len(), 1);
        assert_eq!(r.skipped, 0, "must not be counted as skipped");
        assert_eq!(
            r.partial, 1,
            "non-array content is degenerate, counted partial"
        );
        let c = &r.calls[0];
        assert_eq!(c.usage.input, 10, "tokens are kept, not zeroed");
        assert_eq!(c.usage.output, 5);
        assert!(c.tools.is_empty());
    }

    #[test]
    fn garbage_and_missing_usage_are_counted_separately_from_a_good_record() {
        let bytes = b"not json\n{\"type\":\"assistant\",\"timestamp\":\"2026-09-16T12:00:00Z\",\"message\":{\"model\":\"m\"}}\n{\"type\":\"assistant\",\"timestamp\":\"2026-09-16T12:00:01Z\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":5}}}\n";
        let r = parse(bytes);
        assert_eq!(r.skipped, 1);
        assert_eq!(r.partial, 1);
        assert_eq!(
            r.calls.len(),
            2,
            "the partial record is still a call, at zero"
        );
        assert_eq!(r.calls[0].usage, Usage::default());
        assert_eq!(r.calls[1].usage.input, 5);
    }

    #[test]
    fn the_highest_version_seen_uses_numeric_comparison_not_lexicographic() {
        let bytes = b"{\"type\":\"assistant\",\"timestamp\":\"2026-09-16T12:00:00Z\",\"version\":\"1.9.0\",\"message\":{\"model\":\"m\"}}\n{\"type\":\"assistant\",\"timestamp\":\"2026-09-16T12:00:01Z\",\"version\":\"1.10.0\",\"message\":{\"model\":\"m\"}}\n";
        let r = parse(bytes);
        assert_eq!(
            r.max_version.as_deref(),
            Some("1.10.0"),
            "lexicographic compare would wrongly pick 1.9.0"
        );
    }

    #[test]
    fn identity_transcript_path_picks_the_main_or_the_subagent_file() {
        let root = Path::new("/home/e/.claude/projects");
        assert_eq!(
            identity_transcript_path(root, "/repo", "s-1", None),
            root.join("-repo").join("s-1.jsonl")
        );
        assert_eq!(
            identity_transcript_path(root, "/repo", "s-1", Some("agent-1")),
            root.join("-repo")
                .join("s-1")
                .join("subagents")
                .join("agent-agent-1.jsonl")
        );
    }

    #[test]
    fn read_identity_transcript_is_none_for_anything_not_confined_and_present() {
        let dir = tempfile::TempDir::new().unwrap();
        let projects = dir.path().join("projects");
        std::fs::create_dir_all(projects.join("-repo")).unwrap();
        std::fs::write(projects.join("-repo").join("s-1.jsonl"), "{}").unwrap();
        let canon = projects.canonicalize().unwrap();
        assert!(read_identity_transcript(&projects, &canon, "/repo", "s-1", None).is_some());
        assert!(read_identity_transcript(&projects, &canon, "/repo", "s-ghost", None).is_none());
    }

    #[test]
    fn contains_bash_command_finds_a_matching_tool_use_and_ignores_others() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ratchet task review T-0016 approve \"ok\""}}]}}"#;
        assert!(contains_bash_command(line.as_bytes(), "task review T-0016"));
        assert!(!contains_bash_command(
            line.as_bytes(),
            "task review T-9999"
        ));
        let other_tool = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"command":"task review T-0016"}}]}}"#;
        assert!(!contains_bash_command(
            other_tool.as_bytes(),
            "task review T-0016"
        ));
        let non_assistant = r#"{"type":"user","message":{"content":"task review T-0016"}}"#;
        assert!(!contains_bash_command(
            non_assistant.as_bytes(),
            "task review T-0016"
        ));
        assert!(!contains_bash_command(b"not json", "anything"));
    }

    #[test]
    fn meta_reads_its_fields_and_tolerates_a_missing_one() {
        let m =
            read_meta(br#"{"agentType":"ratchet:reviewer","description":"d","toolUseId":"tu-9"}"#)
                .unwrap();
        assert_eq!(m.agent_type.as_deref(), Some("ratchet:reviewer"));
        assert_eq!(m.description.as_deref(), Some("d"));
        assert_eq!(m.tool_use_id.as_deref(), Some("tu-9"));
        assert_eq!(m.model, None);
        assert!(read_meta(b"not json").is_none());
    }
}
