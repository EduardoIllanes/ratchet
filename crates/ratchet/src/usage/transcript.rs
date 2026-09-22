//! Pure JSONL transcript parsing. No I/O: callers (`cli::usage_cmd`, Task 4) read the file and
//! hand this module bytes. Tolerant per D-usage-tolerant: nothing here ever panics or fails on a
//! record it does not understand.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::clock;

/// The four token classes plus thinking, all defaulting to zero for a record that lacks them.
// Consumed by usage::attribute (Task 3) and cli::usage_cmd (Task 4).
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
    content: Option<Vec<Value>>,
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
#[allow(dead_code)]
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
        if raw_usage.is_none() {
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
        let (tools, tool_use_ids) =
            extract_tools(raw.message.as_ref().and_then(|m| m.content.as_ref()));
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
fn newer(current: &Option<String>, candidate: &str) -> bool {
    match current {
        None => true,
        Some(cur) => version_key(candidate) > version_key(cur),
    }
}

fn version_key(v: &str) -> Vec<u64> {
    v.split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .collect()
}

/// `<slug>` per R1: the session's recorded `cwd` with every path separator replaced by `-`.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn slug_for(cwd: &str) -> String {
    cwd.replace(['/', '\\'], "-")
}

/// `<projects>/<slug>/<session id>.jsonl`.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn transcript_path(projects_dir: &Path, cwd: &str, session_id: &str) -> PathBuf {
    projects_dir
        .join(slug_for(cwd))
        .join(format!("{session_id}.jsonl"))
}

/// `<projects>/<slug>/<session id>/subagents/`.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn subagents_dir(projects_dir: &Path, cwd: &str, session_id: &str) -> PathBuf {
    projects_dir
        .join(slug_for(cwd))
        .join(session_id)
        .join("subagents")
}

/// The sibling `agent-<id>.meta.json` of a subagent transcript.
// Consumed by usage::attribute (Task 3) and cli::usage_cmd (Task 4).
#[allow(dead_code)]
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
#[allow(dead_code)]
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
    fn slug_replaces_every_separator() {
        assert_eq!(slug_for("/Users/e/repo"), "-Users-e-repo");
        // R1: "every path separator replaced by `-`" -- `:` is not a path separator, so it is
        // left alone. (Fixed from the brief's verbatim expectation, "-C--repos-demo", which does
        // not match what `/` and `\` alone, replaced with `-`, produce for this input: 13
        // characters in, 13 out, not 14.)
        assert_eq!(slug_for(r"C:\repos\demo"), "C:-repos-demo");
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
