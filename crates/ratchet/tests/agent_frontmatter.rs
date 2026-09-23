//! Every `agents/*.md` frontmatter names a `model` and, where present, an `effort`, from the
//! allowed set. Written and made green in the same task that adds `mapper.md` (group 6, Task 5)
//! — a version of this test written before `mapper.md` existed would have started green, not
//! red, so it does not belong with the scenario-test-author's red-clean contract.
//!
//! Owner ruling (2026-09-21, before execution of the group 6 plan): `model` accepts one of the
//! short aliases below, `inherit`, or a full model id (`claude-...`); `effort` accepts one of
//! the levels below. This is wider than group 6's own plan draft, which only allowed
//! `opus`/`sonnet`/`haiku` and `low`/`medium`/`high` — the plan's own agents (`refactorer`,
//! `reviewer`) already use `effort: high`, so the draft's narrower set would have failed on
//! files that predate this test.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

const ALLOWED_MODEL_ALIASES: &[&str] = &["sonnet", "opus", "haiku", "fable", "inherit"];
const ALLOWED_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

fn is_allowed_model(model: &str) -> bool {
    ALLOWED_MODEL_ALIASES.contains(&model) || model.starts_with("claude-")
}

fn frontmatter(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

fn field<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}:");
    block
        .lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .map(|v| v.trim().trim_matches('"'))
}

#[test]
fn every_agent_frontmatter_has_a_valid_model_and_effort() {
    let dir = repo_root().join("agents");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).expect("agents/ exists") {
        let path = entry.unwrap().path();
        if path.extension().map(|e| e == "md").unwrap_or(false) {
            checked += 1;
            let text = fs::read_to_string(&path).unwrap();
            let block = frontmatter(&text).unwrap_or_else(|| panic!("{path:?}: no frontmatter"));
            let name = field(block, "name").unwrap_or_else(|| panic!("{path:?}: no name"));
            assert!(!name.is_empty(), "{path:?}: empty name");
            let model = field(block, "model").unwrap_or_else(|| panic!("{path:?}: no model"));
            assert!(
                is_allowed_model(model),
                "{path:?}: model {model:?} not in {ALLOWED_MODEL_ALIASES:?} and not a full model id"
            );
            if let Some(effort) = field(block, "effort") {
                assert!(
                    ALLOWED_EFFORTS.contains(&effort),
                    "{path:?}: effort {effort:?} not in {ALLOWED_EFFORTS:?}"
                );
            }
        }
    }
    assert!(
        checked >= 7,
        "expected at least 7 agent files (6 pre-group-6 + mapper), found {checked}"
    );
}
