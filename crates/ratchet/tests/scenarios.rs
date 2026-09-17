//! Every `#### Scenario:` of an active spec has a test function that references it by slug.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // crates/ratchet → repo root
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

pub fn slug(title: &str) -> String {
    let mut out = String::new();
    let mut prev_us = false;
    for ch in title.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_us = false;
        } else if !prev_us && !out.is_empty() {
            out.push('_');
            prev_us = true;
        }
    }
    out.trim_end_matches('_').to_string()
}

#[test]
fn slug_examples() {
    assert_eq!(slug("Python outside the venv"), "python_outside_the_venv");
    assert_eq!(slug("Git destructive"), "git_destructive");
    assert_eq!(slug("Rule disabled per repo"), "rule_disabled_per_repo");
}

/// True for a line that opens or closes a fenced code block: optionally indented, optionally
/// behind a `//` or `///` comment marker (so a Rust doc-comment fence counts too), then a
/// ` ``` ` at the start of what remains. Deliberately narrower than "contains a ``` anywhere",
/// so prose that merely mentions backticks mid-line can't flip fence state.
fn is_fence_line(line: &str) -> bool {
    let rest = line.trim_start();
    let rest = rest
        .strip_prefix("///")
        .or_else(|| rest.strip_prefix("//"))
        .unwrap_or(rest);
    rest.trim_start().starts_with("```")
}

/// Strips the content of fenced code blocks from `text`, so a `#### Scenario:` heading or a
/// `fn name(` shown only as an example inside a fence is not read as the real thing. Fence
/// delimiter lines themselves are dropped too; everything outside fences is kept as-is.
fn strip_fenced_blocks(text: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in text.lines() {
        if is_fence_line(line) {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Extracts `#### Scenario:` titles from spec markdown, ignoring any that sit inside a fenced
/// code block (e.g. a spec that shows example spec syntax for illustration).
fn scenario_titles(spec_text: &str) -> Vec<String> {
    strip_fenced_blocks(spec_text)
        .lines()
        .filter_map(|line| {
            line.strip_prefix("#### Scenario:")
                .map(|t| t.trim().to_string())
        })
        .collect()
}

#[test]
fn scenario_inside_fence_is_ignored() {
    let spec = "\
#### Scenario: Real scenario

Some example of spec syntax:

```
#### Scenario: Fake scenario shown only as an example
```

#### Scenario: Another real one
";
    assert_eq!(
        scenario_titles(spec),
        vec!["Real scenario".to_string(), "Another real one".to_string()]
    );
}

#[test]
fn every_scenario_has_a_test() {
    let root = repo_root();
    let specs_dir = root.join("openspec/specs");
    let tests_dir = root.join("crates/ratchet/tests/spec");
    let mut tests_text = String::new();
    for entry in fs::read_dir(&tests_dir).expect("tests/spec exists") {
        let p = entry.unwrap().path();
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            tests_text.push_str(&fs::read_to_string(&p).unwrap());
        }
    }
    // Ignore anything shown only inside a fenced example (e.g. a doc-comment code block) when
    // searching test sources for the referenced function name, so an illustrative snippet can't
    // be mistaken for a real test.
    let tests_text = strip_fenced_blocks(&tests_text);
    let mut missing = Vec::new();
    for entry in fs::read_dir(&specs_dir).expect("openspec/specs exists") {
        let dir = entry.unwrap().path();
        let spec = dir.join("spec.md");
        if !spec.is_file() {
            continue;
        }
        let prefix = dir.file_name().unwrap().to_string_lossy().replace('-', "_");
        for title in scenario_titles(&fs::read_to_string(&spec).unwrap()) {
            let name = format!("fn {}__{}(", prefix, slug(&title));
            if !tests_text.contains(&name) {
                missing.push(format!("{}: {} → {}", prefix, title, name));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "scenarios without a test:\n{}",
        missing.join("\n")
    );
}
