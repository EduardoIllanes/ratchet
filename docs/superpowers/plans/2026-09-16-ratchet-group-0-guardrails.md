# ratchet — Group 0: plugin skeleton, repo marker and `pre-tool` guardrails

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a usable guardrail-only Claude Code plugin: a Rust binary `ratchet` whose `hook pre-tool` blocks the four built-in guardrails in any repo that opts in with a `ratchet.toml`, in milliseconds, and never breaks a session.

**Architecture:** One Rust crate (`crates/ratchet`) with thin faces (`hook`, `guardrails`) over pure modules (`config`, `repo`, `guardrails::{segment,rules,eval,main_tree}`). Hooks read the Claude Code JSON payload on stdin, resolve the repo from the nearest `ratchet.toml`, evaluate rules and answer with exit 0 (allow) or exit 2 (block, message on stderr). Any internal error is exit 0 plus one line in `~/.ratchet/ratchet.log`. Scenario tests run the real binary as a subprocess; every `#### Scenario` of the ported spec has a test that references it by slug.

**Tech Stack:** Rust 2021 (rust-version 1.79), `clap` 4 (derive), `serde` + `serde_json` + `toml`, `fancy-regex` (lookaround), `chrono` (log timestamps), `dirs` (home dir); dev: `assert_cmd`, `predicates`, `tempfile`.

**Spec:** `docs/superpowers/specs/2026-09-16-ratchet-plugin-design.md` (sections 2 D-runtime, D-regex, D-marker, D-state, D-p3, D-roles; 3; 4.1-4.4; 6; 7; 8 group 0).

## Global Constraints

- **No git commands in `C:\repos\ratchet` on the owner's machine** (spec D-roles). Every task ends with a hand-off listing the files created or changed; the owner commits from another account. Tests may run `git` inside temporary directories only.
- Work directly in `C:\repos\ratchet` (there is no worktree because there is no git flow here; the `main-tree` guardrail of `ops` does not apply to this directory).
- All content, identifiers, messages and docs in English (spec D-english). The stderr prefix of a block is exactly `[ratchet guardrail:<id>]`.
- Hook exit codes: 0 allow / internal error, 2 block. Nothing else, ever (spec D-p3).
- `pre-tool` never opens a database and never spawns a subprocess except `git ls-files` when a write targets the main tree (spec 4.2).
- Latency target: `pre-tool` median under 30 ms in release on Windows; the test ceiling in debug builds is 200 ms (spec 4.2, 7).
- State root: `RATCHET_HOME` or `~/.ratchet` (spec D-state). Group 0 only writes `ratchet.log` there.
- Repo marker file name: `ratchet.toml` at the repo root (spec 4.3). No global list of repos.
- Rust toolchain: stable, `rust-version = "1.79"`. Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- Example patterns in tests and docs use neutral method names (`purge_all`, `write_rows`), never the write methods of a real database driver: the owner's own harness scans written content and blocks those names.

---

## Prerequisite (owner, once): install the Rust toolchain

`cargo` and `rustc` are not installed on the owner's machine (checked 2026-09-16).

- [ ] Run in PowerShell: `winget install --id Rustlang.Rustup -e`
- [ ] Open a new terminal and run `rustup default stable`
- [ ] Verify: `cargo --version` prints `cargo 1.8x.x` or newer, `rustc --version` prints 1.79 or newer.

No task below starts until this passes.

---

## File structure

```
ratchet/
├── Cargo.toml                                workspace: members = ["crates/ratchet"]
├── .gitignore                                target/, bin/ratchet*, *.log
├── README.md                                 what it is, install (group 0 subset), what is not here
├── .claude-plugin/plugin.json                plugin manifest
├── hooks/hooks.json                          seven events → run-hook.cmd
├── hooks/run-hook.cmd                        polyglot wrapper → bin/ratchet
├── bin/.gitkeep                              binary lands here (gitignored)
├── openspec/config.yaml                      spec rules (ported from ops, trimmed)
├── openspec/specs/agent-protocol/spec.md     group-0 requirements, English
└── crates/ratchet/
    ├── Cargo.toml
    ├── src/main.rs                           clap CLI: hook, guardrails {list,test}, version
    ├── src/config.rs                         ratchet.toml + ~/.ratchet/config.toml, RATCHET_HOME
    ├── src/repo.rs                           marker resolution, main root of a worktree, within, has_venv, is_tracked
    ├── src/log.rs                            append one line to ~/.ratchet/ratchet.log
    ├── src/guardrails/mod.rs                 pub mod of the submodules + cli
    ├── src/guardrails/builtin.toml           the four built-in rules (include_str!)
    ├── src/guardrails/segment.rs             split a command outside quotes
    ├── src/guardrails/rules.rs               Rule, Kind, parse, merge, load_rule_set
    ├── src/guardrails/eval.rs                GuardContext, Violation, evaluate
    ├── src/guardrails/main_tree.rs           writes_main_tree
    ├── src/guardrails/cli.rs                 `ratchet guardrails list|test`
    ├── src/hooks/mod.rs                      run() with catch_unwind + log
    ├── src/hooks/dispatch.rs                 payload parsing, event routing, pre_tool
    ├── tests/scenarios.rs                    every #### Scenario has a test (slug check)
    ├── tests/latency.rs                      pre-tool median under the ceiling
    └── tests/spec/main.rs, support.rs, agent_protocol.rs   scenario tests (subprocess)
```

Scenario slug rule (used by `tests/scenarios.rs` and by test names): lowercase the scenario title, replace every run of non-alphanumerics with `_`, trim `_`; the test function is `fn <spec>__<slug>()` where `<spec>` is the spec directory name with `-` → `_`. Example: `#### Scenario: Python outside the venv` in `agent-protocol` → `fn agent_protocol__python_outside_the_venv()`.

---

### Task 1: Workspace, plugin manifest and `ratchet version`

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `bin/.gitkeep`, `.claude-plugin/plugin.json`, `hooks/hooks.json`, `crates/ratchet/Cargo.toml`, `crates/ratchet/src/main.rs`, `README.md`
- Test: `crates/ratchet/tests/cli_version.rs`

**Interfaces:**
- Produces: binary `ratchet` with subcommands `version`, `hook <event>`, `guardrails list|test`. `hook` and `guardrails` are wired to functions defined in Tasks 9 and 10; until then they return exit 0 / print "not implemented" so the crate compiles at every task.

- [ ] **Step 1: Write the failing test**

`crates/ratchet/tests/cli_version.rs`:
```rust
use assert_cmd::Command;

#[test]
fn version_prints_name_and_semver() {
    let out = Command::cargo_bin("ratchet").unwrap().arg("version").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("ratchet 0.1.0"), "got: {text}");
}
```

- [ ] **Step 2: Create the workspace and crate**

`Cargo.toml` (root):
```toml
[workspace]
members = ["crates/ratchet"]
resolver = "2"
```

`.gitignore`:
```
target/
bin/ratchet
bin/ratchet.exe
*.log
```

`bin/.gitkeep`: empty file.

`crates/ratchet/Cargo.toml`:
```toml
[package]
name = "ratchet"
version = "0.1.0"
edition = "2021"
rust-version = "1.79"
description = "Guardrailed, spec-driven harness for Claude Code"
license = "MIT"

[[bin]]
name = "ratchet"
path = "src/main.rs"

[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
fancy-regex = "0.13"
chrono = "0.4"
dirs = "5"

[dev-dependencies]
assert_cmd = "2"
predicates = "3"
tempfile = "3"
```

`crates/ratchet/src/main.rs`:
```rust
//! ratchet — thin CLI face. All logic lives in the modules; main only routes.

mod config;
mod guardrails;
mod hooks;
mod log;
mod repo;

use std::collections::HashMap;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ratchet", version, about = "Guardrailed, spec-driven harness for Claude Code")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a Claude Code hook. Reads the JSON payload on stdin.
    Hook { event: String },
    /// Inspect or dry-run the guardrails that apply in the current directory.
    Guardrails {
        #[command(subcommand)]
        cmd: GuardrailsCmd,
    },
    /// Print the version.
    Version,
}

#[derive(Subcommand)]
enum GuardrailsCmd {
    /// List the active rules (built-in, machine and repo), marking disabled ones.
    List,
    /// Evaluate one tool call: `ratchet guardrails test Bash '{"command":"python x.py"}'`.
    /// Exit 2 when a rule blocks, 0 otherwise.
    Test { tool: String, payload: String },
}

fn main() {
    let cli = Cli::parse();
    let env: HashMap<String, String> = std::env::vars().collect();
    let cwd = std::env::current_dir().ok();
    let code = match cli.cmd {
        Cmd::Version => {
            println!("ratchet {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Cmd::Hook { event } => hooks::run(&event, std::io::stdin().lock(), &env, cwd),
        Cmd::Guardrails { cmd } => match cmd {
            GuardrailsCmd::List => guardrails::cli::list(&env, cwd),
            GuardrailsCmd::Test { tool, payload } => guardrails::cli::test(&tool, &payload, &env, cwd),
        },
    };
    std::process::exit(code);
}
```

Placeholder modules so the crate compiles now (each is replaced by its task):

`crates/ratchet/src/config.rs`, `src/repo.rs`, `src/log.rs`: create with a single line `//! filled in by a later task` (the `mod` declarations need the files to exist).

`crates/ratchet/src/guardrails/mod.rs`:
```rust
pub mod cli;
```
`crates/ratchet/src/guardrails/cli.rs`:
```rust
use std::collections::HashMap;
use std::path::PathBuf;

pub fn list(_env: &HashMap<String, String>, _cwd: Option<PathBuf>) -> i32 {
    eprintln!("not implemented");
    0
}

pub fn test(_tool: &str, _payload: &str, _env: &HashMap<String, String>, _cwd: Option<PathBuf>) -> i32 {
    eprintln!("not implemented");
    0
}
```
`crates/ratchet/src/hooks/mod.rs`:
```rust
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

pub fn run(_event: &str, _stdin: impl Read, _env: &HashMap<String, String>, _cwd: Option<PathBuf>) -> i32 {
    0
}
```

- [ ] **Step 3: Plugin manifest and hooks registration**

`.claude-plugin/plugin.json`:
```json
{
  "name": "ratchet",
  "version": "0.1.0",
  "description": "Guardrails, task board and spec-driven agent roles for Claude Code, enforced by hooks instead of prompts.",
  "author": { "name": "ratchet contributors" },
  "license": "MIT",
  "keywords": ["guardrails", "hooks", "tasks", "spec-driven", "agents"]
}
```

`hooks/hooks.json` (all seven events registered now; events other than `pre-tool` exit 0 until groups 1-2):
```json
{
  "hooks": {
    "SessionStart": [
      { "matcher": "startup|resume|clear|compact",
        "hooks": [ { "type": "command", "command": "\"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd\" hook session-start", "shell": "bash", "timeout": 10 } ] }
    ],
    "UserPromptSubmit": [
      { "hooks": [ { "type": "command", "command": "\"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd\" hook prompt", "shell": "bash", "timeout": 10 } ] }
    ],
    "PreToolUse": [
      { "matcher": "Bash|PowerShell|Edit|Write|NotebookEdit|MultiEdit",
        "hooks": [ { "type": "command", "command": "\"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd\" hook pre-tool", "shell": "bash", "timeout": 10 } ] }
    ],
    "Stop": [
      { "hooks": [ { "type": "command", "command": "\"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd\" hook stop", "shell": "bash", "timeout": 10 } ] }
    ],
    "SubagentStop": [
      { "hooks": [ { "type": "command", "command": "\"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd\" hook subagent-stop", "shell": "bash", "timeout": 10 } ] }
    ],
    "PreCompact": [
      { "hooks": [ { "type": "command", "command": "\"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd\" hook pre-compact", "shell": "bash", "timeout": 10 } ] }
    ],
    "SessionEnd": [
      { "hooks": [ { "type": "command", "command": "\"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd\" hook session-end", "shell": "bash", "timeout": 10 } ] }
    ]
  }
}
```

`README.md` (first version; Task 11 extends it):
```markdown
# ratchet

A Claude Code plugin that turns working rules into things the agent cannot skip from the
prompt: guardrails before every tool call, a task board with checklists and mandatory
handoffs, and separated agent roles (spec-test author, implementer, reviewer).

Status: group 0 — guardrails only. See `docs/superpowers/specs/2026-09-16-ratchet-plugin-design.md`.

## Build from source (until releases exist)

    cargo build --release
    # copy target/release/ratchet (or ratchet.exe) into bin/

## Opt a repo in

Create `ratchet.toml` at the repo root (see `ratchet config init` once group 1 lands):

    [repo]
    default_branch = "main"
    worktrees_dir = ".worktrees"

    [guardrails]
    off = []

Without that file, every hook is a no-op.
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p ratchet --test cli_version`
Expected: PASS (`version_prints_name_and_semver`).

- [ ] **Step 5: Hand off (no git)**

List the files created. Run `cargo fmt` and `cargo clippy --all-targets -- -D warnings`; both must be clean.

---

### Task 2: Ported spec (group-0 requirements) and the scenario checker

**Files:**
- Create: `openspec/config.yaml`, `openspec/specs/agent-protocol/spec.md`, `crates/ratchet/tests/scenarios.rs`

**Interfaces:**
- Produces: the scenario titles below are the contract for Task 3's tests. The checker fails until every scenario has a `fn agent_protocol__<slug>` in `crates/ratchet/tests/spec/*.rs`.

- [ ] **Step 1: Write `openspec/config.yaml`**

```yaml
schema: spec-driven
context: |
  ratchet is a Claude Code plugin: a Rust binary driven by hooks, a task board in SQLite,
  declarative guardrails and agent profiles. Specs describe behaviour, never code.
rules:
  proposal:
    - One page maximum. Why, not how.
  specs:
    - Requirements use SHALL/MUST. Each requirement has at least one "#### Scenario:" with WHEN/THEN.
    - Scenarios name no library, type or function.
    - Every scenario of an active spec has a test in crates/ratchet/tests/spec that references it by slug.
  design:
    - Every decision names at least one discarded alternative and why it lost.
  tasks:
    - Each task fits one agent session and says how it is verified.
    - The scenario-test task of a group comes before or with the implementation task, never after.
```

- [ ] **Step 2: Write `openspec/specs/agent-protocol/spec.md`**

```markdown
# agent-protocol

How a Claude Code session interacts with ratchet through hooks. Group 0 covers repo opt-in,
guardrails and the never-break rule; briefing, reminders and the handoff rule arrive with
groups 1 and 2.

## Purpose

Rules that live in the harness are rules the agent cannot skip from the prompt. Each block
carries the alternative the agent should use instead, so it corrects itself in one attempt.

## Requirements

### Requirement: Repo opt-in by marker
A repo SHALL opt in to ratchet by having a `ratchet.toml` at its root. Every hook SHALL
resolve the repo from the working directory of the tool call by walking up to the nearest
marker; when the working directory is a linked git worktree, the repo root SHALL be the main
checkout that owns it. Without a marker, every hook SHALL exit 0 immediately without reading
any other file. An unreadable or invalid marker SHALL be logged and treated as "no marker".

#### Scenario: No marker, hooks do nothing
- **WHEN** the pre-tool hook receives `python scripts/x.py` from a directory with no `ratchet.toml` above it
- **THEN** the hook exits 0 and prints nothing

#### Scenario: Marker found from a subdirectory
- **WHEN** the working directory is `src/deep` inside a repo that has `.venv` and `ratchet.toml` at its root and the tool call is `python scripts/x.py`
- **THEN** the hook blocks with the python-venv rule

#### Scenario: Invalid marker is logged and ignored
- **WHEN** `ratchet.toml` is not valid TOML and the tool call is `python scripts/x.py`
- **THEN** the hook exits 0 and the log file contains one line naming `ratchet.toml`

### Requirement: Guardrails before the action
In PreToolUse inside an opted-in repo, ratchet SHALL block, with a message that states the
rule and the alternative, prefixed `[ratchet guardrail:<id>]`: (a) `python`, `pip`,
`pytest`, `uvicorn`, `mypy` or `ruff` not run through the repo's virtualenv (`uv run` or the
`.venv` interpreter) when the repo has a `.venv`; (b) `git push --force`, `git reset --hard`,
`git checkout -- .`, `git clean -f` and recursive forced deletion, except a deletion whose
every target is under the session scratchpad; (c) writes to `.env` files; (d) writes to a
tracked file of the main tree of the repo, from any session, including one running in a
worktree. Command rules SHALL evaluate each command segment separately, splitting on `;`,
`&&`, `||`, `|` and newlines only outside quotes. The same rules SHALL apply with the same
message whether the command arrived through `Bash` or `PowerShell`. Rules SHALL be
extensible from a machine-wide file and from a repo file with the same schema; a rule with
the id of an existing one SHALL replace it; a repo SHALL be able to disable a rule by id.

#### Scenario: Python outside the venv
- **WHEN** the tool call is `python scripts/x.py` through `Bash` in a repo with `.venv`
- **THEN** the hook blocks with rule `python-venv` and the message proposes `uv run`

#### Scenario: Python through PowerShell, same message
- **WHEN** the tool call is `python -c "print(1)"` through `PowerShell` in a repo with `.venv`
- **THEN** the hook blocks with exactly the same stderr line as through `Bash`

#### Scenario: Quoted text does not split a command
- **WHEN** the tool call is `uv run ratchet task new -t x -c "done; mypy clean"` in a repo with `.venv`
- **THEN** the hook allows it

#### Scenario: Git destructive
- **WHEN** the tool call is `git reset --hard HEAD~1`
- **THEN** the hook blocks with rule `git-destructive` and the message says the owner runs it

#### Scenario: Recursive delete under the scratchpad is allowed
- **WHEN** the tool call is `rm -rf <scratchpad>/tmp` and the scratchpad directory is known from the environment
- **THEN** the hook allows it, while `rm -rf <repo>/src` is blocked

#### Scenario: Env file write blocked
- **WHEN** a `Write` targets `.env.local` inside the repo
- **THEN** the hook blocks with rule `env-files`

#### Scenario: Write to the main tree blocked
- **WHEN** an `Edit` targets a tracked file of the main tree and the working directory is the main tree
- **THEN** the hook blocks with rule `main-tree` and the message proposes a worktree

#### Scenario: Write to the main tree blocked from a worktree session
- **WHEN** an `Edit` targets a tracked file of the main tree and the working directory is a linked worktree of that repo
- **THEN** the hook blocks with rule `main-tree`

#### Scenario: Write inside a worktree allowed
- **WHEN** an `Edit` targets a file inside the repo's worktrees directory
- **THEN** the hook allows it

#### Scenario: Untracked file in the main tree allowed
- **WHEN** a `Write` targets a path in the main tree that git does not track
- **THEN** the hook allows it

#### Scenario: Rule disabled per repo
- **WHEN** `ratchet.toml` lists `python-venv` under `guardrails.off` and the tool call is `python scripts/x.py`
- **THEN** the hook allows it

#### Scenario: Custom content rule from the repo
- **WHEN** the repo's extra rules file defines a `content` rule blocking `\.purge_all\s*\(` and a `Write` has that text in its content
- **THEN** the hook blocks with the id and message of that rule

#### Scenario: Machine-wide rule overrides a built-in
- **WHEN** the machine rules file redefines `git-destructive` with a different message and the tool call is `git reset --hard`
- **THEN** the hook blocks with the machine message

### Requirement: Hooks never break a session
On any internal error (invalid config, malformed payload, unexpected failure) a hook SHALL
exit 0 without blocking and SHALL append one line to the log file under the state
directory. An unknown event SHALL exit 0. A pre-tool evaluation SHALL answer well under
the Claude Code hook timeout; the target is milliseconds, not seconds.

#### Scenario: Malformed payload exits 0 and is logged
- **WHEN** the pre-tool hook receives stdin that is not JSON
- **THEN** the hook exits 0, prints nothing on stdout, and the log file has one line for the event

#### Scenario: Unknown event exits 0
- **WHEN** the hook is invoked with an event name that does not exist
- **THEN** the hook exits 0

#### Scenario: Pre-tool answers fast
- **WHEN** the pre-tool hook is invoked twenty times on a blocked command in an opted-in repo
- **THEN** the median wall time is under 200 ms in a debug build

### Requirement: Active rules can be listed and dry-run
The active rule set for the current directory SHALL be listable, showing id, kind, tools,
source (built-in, machine or repo) and whether the repo disabled it. One tool call SHALL be
evaluable from the command line, with the same exit code and message a hook would give.

#### Scenario: List shows built-ins and disabled state
- **WHEN** `ratchet guardrails list` runs in a repo whose marker disables `python-venv`
- **THEN** the output has the four built-in ids and marks `python-venv` as off

#### Scenario: Dry-run reproduces the block
- **WHEN** `ratchet guardrails test Bash '{"command":"python x.py"}'` runs in a repo with `.venv`
- **THEN** it exits 2 and prints the same `[ratchet guardrail:python-venv]` line a hook would
```

- [ ] **Step 3: Write the scenario checker**

`crates/ratchet/tests/scenarios.rs`:
```rust
//! Every `#### Scenario:` of an active spec has a test function that references it by slug.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // crates/ratchet → repo root
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
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
    let mut missing = Vec::new();
    for entry in fs::read_dir(&specs_dir).expect("openspec/specs exists") {
        let dir = entry.unwrap().path();
        let spec = dir.join("spec.md");
        if !spec.is_file() {
            continue;
        }
        let prefix = dir.file_name().unwrap().to_string_lossy().replace('-', "_");
        for line in fs::read_to_string(&spec).unwrap().lines() {
            if let Some(title) = line.strip_prefix("#### Scenario:") {
                let name = format!("fn {}__{}(", prefix, slug(title));
                if !tests_text.contains(&name) {
                    missing.push(format!("{}: {} → {}", prefix, title.trim(), name));
                }
            }
        }
    }
    assert!(missing.is_empty(), "scenarios without a test:\n{}", missing.join("\n"));
}
```

- [ ] **Step 4: Run it and confirm it fails for the right reason**

Create the empty directory `crates/ratchet/tests/spec/` (Task 3 fills it).
Run: `cargo test -p ratchet --test scenarios`
Expected: `slug_examples` PASS; `every_scenario_has_a_test` FAIL listing all 21 scenarios of `agent_protocol`.

- [ ] **Step 5: Hand off (no git)**

List the three files. Note for the owner: the checker is red on purpose until Task 3.

---

### Task 3: Scenario tests, red-clean (spec-test-author)

**Files:**
- Create: `crates/ratchet/tests/spec/main.rs`, `crates/ratchet/tests/spec/support.rs`, `crates/ratchet/tests/spec/agent_protocol.rs`

**Interfaces:**
- Consumes: the CLI contract of Task 1 (`ratchet hook <event>` reads JSON on stdin; `ratchet guardrails list|test`), the spec of Task 2, and the environment variables `RATCHET_HOME` (state dir) and `CLAUDE_SCRATCHPAD` (session scratchpad).
- Produces: nothing for later tasks; these tests turn green in Tasks 9 and 10 and must not be edited by the implementer.

The author of this task reads only `openspec/specs/agent-protocol/spec.md`, this task and the README; not the design document.

- [ ] **Step 1: Write the support module**

`crates/ratchet/tests/spec/main.rs`:
```rust
mod agent_protocol;
mod support;
```

`crates/ratchet/tests/spec/support.rs`:
```rust
//! Sandbox: a temporary state dir, a temporary git repo with a marker and one tracked file.
//! Tests run the real binary as a subprocess, exactly as Claude Code would.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};
use tempfile::TempDir;

pub struct Sandbox {
    pub home: TempDir,
    pub repo: TempDir,
    pub scratchpad: TempDir,
}

impl Sandbox {
    pub fn root(&self) -> PathBuf {
        self.repo.path().canonicalize().unwrap()
    }
    pub fn log_text(&self) -> String {
        fs::read_to_string(self.home.path().join("ratchet.log")).unwrap_or_default()
    }
    pub fn write_marker(&self, body: &str) {
        fs::write(self.repo.path().join("ratchet.toml"), body).unwrap();
    }
    pub fn write(&self, rel: &str, body: &str) -> PathBuf {
        let p = self.repo.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, body).unwrap();
        p
    }
}

pub fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git available");
    assert!(st.success(), "git {:?} failed", args);
}

/// Opted-in repo: `.venv/`, `ratchet.toml`, `tracked.txt` committed, `.worktrees/wt` linked worktree.
pub fn sandbox() -> Sandbox {
    let sb = Sandbox { home: TempDir::new().unwrap(), repo: TempDir::new().unwrap(), scratchpad: TempDir::new().unwrap() };
    let root = sb.repo.path();
    fs::create_dir_all(root.join(".venv")).unwrap();
    fs::create_dir_all(root.join("src/deep")).unwrap();
    sb.write_marker("[repo]\ndefault_branch = \"main\"\nworktrees_dir = \".worktrees\"\n");
    fs::write(root.join("tracked.txt"), "hello\n").unwrap();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["add", "tracked.txt", "ratchet.toml"]);
    git(root, &["commit", "-q", "-m", "init"]);
    git(root, &["worktree", "add", "-q", ".worktrees/wt", "-b", "wt"]);
    sb
}

/// Directory with no marker anywhere above it.
pub fn unmanaged_dir() -> TempDir {
    TempDir::new().unwrap()
}

pub fn ratchet_bin() -> PathBuf {
    assert_cmd::cargo::cargo_bin("ratchet")
}

pub fn hook_in(sb: &Sandbox, event: &str, payload: &Value, cwd: &Path) -> Output {
    run_hook(sb.home.path(), Some(sb.scratchpad.path()), event, &payload.to_string(), cwd)
}

pub fn run_hook(home: &Path, scratchpad: Option<&Path>, event: &str, stdin_text: &str, cwd: &Path) -> Output {
    let mut cmd = Command::new(ratchet_bin());
    cmd.args(["hook", event])
        .current_dir(cwd)
        .env("RATCHET_HOME", home)
        .env_remove("CLAUDE_SCRATCHPAD")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(s) = scratchpad {
        cmd.env("CLAUDE_SCRATCHPAD", s);
    }
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin_text.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

pub fn bash(command: &str, cwd: &Path) -> Value {
    json!({ "tool_name": "Bash", "tool_input": { "command": command }, "cwd": cwd.to_string_lossy() })
}

pub fn powershell(command: &str, cwd: &Path) -> Value {
    json!({ "tool_name": "PowerShell", "tool_input": { "command": command }, "cwd": cwd.to_string_lossy() })
}

pub fn edit(file_path: &Path, new_string: &str, cwd: &Path) -> Value {
    json!({ "tool_name": "Edit", "tool_input": { "file_path": file_path.to_string_lossy(), "old_string": "x", "new_string": new_string }, "cwd": cwd.to_string_lossy() })
}

pub fn write(file_path: &Path, content: &str, cwd: &Path) -> Value {
    json!({ "tool_name": "Write", "tool_input": { "file_path": file_path.to_string_lossy(), "content": content }, "cwd": cwd.to_string_lossy() })
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

pub fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

pub fn guardrails(sb: &Sandbox, args: &[&str], cwd: &Path) -> Output {
    Command::new(ratchet_bin())
        .arg("guardrails")
        .args(args)
        .current_dir(cwd)
        .env("RATCHET_HOME", sb.home.path())
        .output()
        .unwrap()
}
```

- [ ] **Step 2: Write the scenario tests**

`crates/ratchet/tests/spec/agent_protocol.rs`:
```rust
//! One test per `#### Scenario` of openspec/specs/agent-protocol/spec.md, named by slug.

use std::fs;
use std::time::Instant;

use crate::support::*;

// --- Requirement: Repo opt-in by marker -------------------------------------------

#[test]
fn agent_protocol__no_marker_hooks_do_nothing() {
    let sb = sandbox();
    let dir = unmanaged_dir();
    let out = hook_in(&sb, "pre-tool", &bash("python scripts/x.py", dir.path()), dir.path());
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), "");
}

#[test]
fn agent_protocol__marker_found_from_a_subdirectory() {
    let sb = sandbox();
    let deep = sb.root().join("src/deep");
    let out = hook_in(&sb, "pre-tool", &bash("python scripts/x.py", &deep), &deep);
    assert_eq!(code(&out), 2, "stderr: {}", stderr(&out));
    assert!(stderr(&out).starts_with("[ratchet guardrail:python-venv]"));
}

#[test]
fn agent_protocol__invalid_marker_is_logged_and_ignored() {
    let sb = sandbox();
    sb.write_marker("[repo\nthis is = not toml");
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &bash("python scripts/x.py", &root), &root);
    assert_eq!(code(&out), 0);
    assert!(sb.log_text().contains("ratchet.toml"), "log: {}", sb.log_text());
}

// --- Requirement: Guardrails before the action -----------------------------------

#[test]
fn agent_protocol__python_outside_the_venv() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &bash("python scripts/x.py", &root), &root);
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(err.starts_with("[ratchet guardrail:python-venv]"), "{err}");
    assert!(err.contains("uv run"), "{err}");
    let ok = hook_in(&sb, "pre-tool", &bash("uv run python scripts/x.py", &root), &root);
    assert_eq!(code(&ok), 0);
}

#[test]
fn agent_protocol__python_through_powershell_same_message() {
    let sb = sandbox();
    let root = sb.root();
    let via_bash = hook_in(&sb, "pre-tool", &bash("python -c \"print(1)\"", &root), &root);
    let via_ps = hook_in(&sb, "pre-tool", &powershell("python -c \"print(1)\"", &root), &root);
    assert_eq!(code(&via_ps), 2);
    assert_eq!(stderr(&via_ps), stderr(&via_bash));
}

#[test]
fn agent_protocol__quoted_text_does_not_split_a_command() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &bash("uv run ratchet task new -t x -c \"done; mypy clean\"", &root), &root);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
}

#[test]
fn agent_protocol__git_destructive() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &bash("git reset --hard HEAD~1", &root), &root);
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(err.starts_with("[ratchet guardrail:git-destructive]"), "{err}");
    assert!(err.to_lowercase().contains("owner"), "{err}");
    let lease = hook_in(&sb, "pre-tool", &bash("git push --force-with-lease", &root), &root);
    assert_eq!(code(&lease), 0);
}

#[test]
fn agent_protocol__recursive_delete_under_the_scratchpad_is_allowed() {
    let sb = sandbox();
    let root = sb.root();
    let inside = sb.scratchpad.path().join("tmp");
    let ok = hook_in(&sb, "pre-tool", &bash(&format!("rm -rf {}", inside.display()), &root), &root);
    assert_eq!(code(&ok), 0, "stderr: {}", stderr(&ok));
    let bad = hook_in(&sb, "pre-tool", &bash(&format!("rm -rf {}", root.join("src").display()), &root), &root);
    assert_eq!(code(&bad), 2);
}

#[test]
fn agent_protocol__env_file_write_blocked() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &write(&root.join(".env.local"), "KEY=1", &root), &root);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).starts_with("[ratchet guardrail:env-files]"));
}

#[test]
fn agent_protocol__write_to_the_main_tree_blocked() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &edit(&root.join("tracked.txt"), "bye", &root), &root);
    assert_eq!(code(&out), 2, "stderr: {}", stderr(&out));
    let err = stderr(&out);
    assert!(err.starts_with("[ratchet guardrail:main-tree]"), "{err}");
    assert!(err.contains("worktree"), "{err}");
}

#[test]
fn agent_protocol__write_to_the_main_tree_blocked_from_a_worktree_session() {
    let sb = sandbox();
    let root = sb.root();
    let wt = root.join(".worktrees/wt");
    let out = hook_in(&sb, "pre-tool", &edit(&root.join("tracked.txt"), "bye", &wt), &wt);
    assert_eq!(code(&out), 2, "stderr: {}", stderr(&out));
    assert!(stderr(&out).starts_with("[ratchet guardrail:main-tree]"));
}

#[test]
fn agent_protocol__write_inside_a_worktree_allowed() {
    let sb = sandbox();
    let root = sb.root();
    let wt = root.join(".worktrees/wt");
    let out = hook_in(&sb, "pre-tool", &edit(&wt.join("tracked.txt"), "bye", &wt), &wt);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
}

#[test]
fn agent_protocol__untracked_file_in_the_main_tree_allowed() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &write(&root.join("notes/new.md"), "draft", &root), &root);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
}

#[test]
fn agent_protocol__rule_disabled_per_repo() {
    let sb = sandbox();
    sb.write_marker("[repo]\nworktrees_dir = \".worktrees\"\n[guardrails]\noff = [\"python-venv\"]\n");
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &bash("python scripts/x.py", &root), &root);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
}

#[test]
fn agent_protocol__custom_content_rule_from_the_repo() {
    let sb = sandbox();
    sb.write_marker("[repo]\nworktrees_dir = \".worktrees\"\n[guardrails]\nextra = \"ratchet/guardrails.toml\"\n");
    sb.write(
        "ratchet/guardrails.toml",
        "[[rules]]\nid = \"db-readonly\"\ntools = [\"Bash\", \"PowerShell\", \"Edit\", \"Write\"]\nkind = \"content\"\npattern = '\\.purge_all\\s*\\('\nmessage = \"The database is read-only.\"\nalternative = \"Read through the data layer.\"\n",
    );
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &write(&root.join("notes/s.py"), "coll.purge_all({})", &root), &root);
    assert_eq!(code(&out), 2, "stderr: {}", stderr(&out));
    let err = stderr(&out);
    assert!(err.starts_with("[ratchet guardrail:db-readonly] The database is read-only."), "{err}");
}

#[test]
fn agent_protocol__machine_wide_rule_overrides_a_built_in() {
    let sb = sandbox();
    fs::create_dir_all(sb.home.path()).unwrap();
    fs::write(
        sb.home.path().join("config.toml"),
        "[guardrails]\nextra = \"guardrails.toml\"\n",
    ).unwrap();
    fs::write(
        sb.home.path().join("guardrails.toml"),
        "[[rules]]\nid = \"git-destructive\"\ntools = [\"Bash\", \"PowerShell\"]\nkind = \"command\"\npattern = 'git\\s+reset\\s+--hard'\nmessage = \"Machine says no.\"\nalternative = \"Use git stash.\"\n",
    ).unwrap();
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &bash("git reset --hard", &root), &root);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).starts_with("[ratchet guardrail:git-destructive] Machine says no."), "{}", stderr(&out));
}

// --- Requirement: Hooks never break a session -------------------------------------

#[test]
fn agent_protocol__malformed_payload_exits_0_and_is_logged() {
    let sb = sandbox();
    let root = sb.root();
    let out = run_hook(sb.home.path(), None, "pre-tool", "this is not json", &root);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "");
    assert!(sb.log_text().contains("pre-tool"), "log: {}", sb.log_text());
}

#[test]
fn agent_protocol__unknown_event_exits_0() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(&sb, "no-such-event", &bash("echo hi", &root), &root);
    assert_eq!(code(&out), 0);
}

#[test]
fn agent_protocol__pre_tool_answers_fast() {
    let sb = sandbox();
    let root = sb.root();
    let payload = bash("python scripts/x.py", &root);
    let mut times = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        let out = hook_in(&sb, "pre-tool", &payload, &root);
        times.push(t.elapsed().as_millis());
        assert_eq!(code(&out), 2);
    }
    times.sort();
    let median = times[times.len() / 2];
    eprintln!("pre-tool median {median} ms (debug build)");
    assert!(median < 200, "median {median} ms");
}

// --- Requirement: Active rules can be listed and dry-run ---------------------------

#[test]
fn agent_protocol__list_shows_built_ins_and_disabled_state() {
    let sb = sandbox();
    sb.write_marker("[repo]\nworktrees_dir = \".worktrees\"\n[guardrails]\noff = [\"python-venv\"]\n");
    let root = sb.root();
    let out = guardrails(&sb, &["list"], &root);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    for id in ["python-venv", "git-destructive", "env-files", "main-tree"] {
        assert!(text.contains(id), "{text}");
    }
    let line = text.lines().find(|l| l.contains("python-venv")).unwrap();
    assert!(line.contains("off"), "{line}");
}

#[test]
fn agent_protocol__dry_run_reproduces_the_block() {
    let sb = sandbox();
    let root = sb.root();
    let out = guardrails(&sb, &["test", "Bash", r#"{"command":"python x.py"}"#], &root);
    assert_eq!(code(&out), 2);
    assert!(stdout(&out).contains("[ratchet guardrail:python-venv]") || stderr(&out).contains("[ratchet guardrail:python-venv]"));
}
```

- [ ] **Step 3: Verify red-clean**

Run: `cargo test -p ratchet --test spec`
Expected: the crate compiles (no import or type errors); every test FAILS on an assertion (exit codes are 0 because the placeholder `hooks::run` returns 0). `pre_tool_answers_fast` fails on `code == 2`.
Run: `cargo test -p ratchet --test scenarios`
Expected: PASS (every scenario now has a test).

- [ ] **Step 4: Hand off (no git)**

List the three files. Do not touch anything under `src/`.

---

### Task 4: `config.rs` and `repo.rs` (marker, main root, path helpers)

**Files:**
- Modify: `crates/ratchet/src/config.rs`, `crates/ratchet/src/repo.rs` (replace placeholders)
- Test: unit tests inside both files

**Interfaces:**
- Produces:
  - `config::MARKER: &str = "ratchet.toml"`
  - `config::RepoConfig { repo: RepoSection { name, default_branch, worktrees_dir }, guardrails: GuardrailsSection { off: Vec<String>, extra: Option<String> }, thresholds }`
  - `config::MachineConfig { guardrails: MachineGuardrails { extra: Option<String> } }`
  - `config::ConfigError { path: PathBuf, message: String }` (Display: `"<path>: <message>"`)
  - `config::parse_repo_config(text: &str, path: &Path) -> Result<RepoConfig, ConfigError>`
  - `config::load_repo_config(path: &Path) -> Result<RepoConfig, ConfigError>`
  - `config::ratchet_home(env: &HashMap<String,String>) -> PathBuf`
  - `config::load_machine_config(home: &Path) -> Result<MachineConfig, ConfigError>` (missing file → default)
  - `repo::Repo { main_root, checkout_root, name, worktrees_dir, config }`
  - `repo::find_repo(cwd: &Path) -> Result<Option<Repo>, ConfigError>`
  - `repo::main_root_of(checkout_root: &Path) -> PathBuf`
  - `repo::normalize(p: &Path) -> PathBuf`, `repo::within(target, root) -> bool`, `repo::has_venv(cwd, main_root) -> bool`, `repo::is_tracked(main_root, target) -> bool`

- [ ] **Step 1: Write the failing unit tests (append to each file after the code, as `#[cfg(test)] mod tests`)**

For `config.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn defaults_when_sections_are_missing() {
        let c = parse_repo_config("", Path::new("ratchet.toml")).unwrap();
        assert_eq!(c.repo.worktrees_dir, None);
        assert!(c.guardrails.off.is_empty());
        assert_eq!(c.thresholds.live_minutes, 10);
        assert_eq!(c.thresholds.idle_minutes, 60);
    }

    #[test]
    fn parses_all_sections() {
        let text = "[repo]\nname = \"x\"\ndefault_branch = \"main\"\nworktrees_dir = \".wt\"\n[guardrails]\noff = [\"python-venv\"]\nextra = \"ratchet/g.toml\"\n[thresholds]\nlive_minutes = 3\n";
        let c = parse_repo_config(text, Path::new("ratchet.toml")).unwrap();
        assert_eq!(c.repo.name.as_deref(), Some("x"));
        assert_eq!(c.repo.worktrees_dir.as_deref(), Some(".wt"));
        assert_eq!(c.guardrails.off, vec!["python-venv"]);
        assert_eq!(c.guardrails.extra.as_deref(), Some("ratchet/g.toml"));
        assert_eq!(c.thresholds.live_minutes, 3);
    }

    #[test]
    fn invalid_toml_names_the_file() {
        let err = parse_repo_config("[repo\nx = ", Path::new("C:/r/ratchet.toml")).unwrap_err();
        assert!(err.to_string().contains("ratchet.toml"), "{err}");
    }

    #[test]
    fn unknown_key_is_an_error() {
        let err = parse_repo_config("[guardrails]\nofff = []\n", Path::new("ratchet.toml")).unwrap_err();
        assert!(err.message.contains("offf"), "{}", err.message);
    }

    #[test]
    fn home_from_env_or_default() {
        let mut env = std::collections::HashMap::new();
        env.insert("RATCHET_HOME".to_string(), "C:/tmp/rh".to_string());
        assert_eq!(ratchet_home(&env), PathBuf::from("C:/tmp/rh"));
        env.clear();
        assert!(ratchet_home(&env).ends_with(".ratchet"));
    }

    #[test]
    fn machine_config_missing_is_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let c = load_machine_config(dir.path()).unwrap();
        assert_eq!(c.guardrails.extra, None);
    }
}
```

For `repo.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn marker(dir: &Path, body: &str) {
        fs::write(dir.join(crate::config::MARKER), body).unwrap();
    }

    #[test]
    fn no_marker_is_none() {
        let d = tempfile::TempDir::new().unwrap();
        assert!(find_repo(d.path()).unwrap().is_none());
    }

    #[test]
    fn nearest_marker_from_subdir_and_defaults() {
        let d = tempfile::TempDir::new().unwrap();
        marker(d.path(), "");
        let deep = d.path().join("a/b");
        fs::create_dir_all(&deep).unwrap();
        let r = find_repo(&deep).unwrap().unwrap();
        assert_eq!(normalize(&r.main_root), normalize(d.path()));
        assert_eq!(normalize(&r.worktrees_dir), normalize(&d.path().join(".worktrees")));
        assert_eq!(r.name, d.path().file_name().unwrap().to_string_lossy());
    }

    #[test]
    fn linked_worktree_resolves_to_main_root() {
        let main = tempfile::TempDir::new().unwrap();
        marker(main.path(), "[repo]\nname = \"m\"\n");
        fs::create_dir_all(main.path().join(".git/worktrees/wt")).unwrap();
        let wt = main.path().join(".worktrees/wt");
        fs::create_dir_all(&wt).unwrap();
        marker(&wt, "");
        fs::write(wt.join(".git"), format!("gitdir: {}\n", main.path().join(".git/worktrees/wt").display())).unwrap();
        let r = find_repo(&wt).unwrap().unwrap();
        assert_eq!(normalize(&r.main_root), normalize(main.path()));
        assert_eq!(normalize(&r.checkout_root), normalize(&wt));
        assert_eq!(r.name, "m");
    }

    #[test]
    fn invalid_marker_is_an_error() {
        let d = tempfile::TempDir::new().unwrap();
        marker(d.path(), "[repo\n");
        assert!(find_repo(d.path()).is_err());
    }

    #[test]
    fn within_is_case_insensitive_and_prefix_safe() {
        let d = tempfile::TempDir::new().unwrap();
        let root = d.path().join("Repo");
        fs::create_dir_all(root.join("src")).unwrap();
        assert!(within(&root.join("src/x.rs"), &root));
        let upper = PathBuf::from(root.to_string_lossy().to_uppercase());
        assert!(within(&root.join("src"), &upper));
        assert!(!within(&d.path().join("Repo2/x"), &root));
    }

    #[test]
    fn has_venv_walks_up_to_main_root() {
        let d = tempfile::TempDir::new().unwrap();
        let deep = d.path().join("a/b");
        fs::create_dir_all(&deep).unwrap();
        assert!(!has_venv(&deep, d.path()));
        fs::create_dir_all(d.path().join(".venv")).unwrap();
        assert!(has_venv(&deep, d.path()));
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p ratchet --lib config repo`
Expected: compile errors (functions undefined).

- [ ] **Step 3: Implement `config.rs`**

```rust
//! Configuration: the repo marker `ratchet.toml` and the machine file `~/.ratchet/config.toml`.
//! Pure parsing; no side effects beyond reading the named file.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const MARKER: &str = "ratchet.toml";

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RepoConfig {
    pub repo: RepoSection,
    pub guardrails: GuardrailsSection,
    pub thresholds: Thresholds,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RepoSection {
    pub name: Option<String>,
    pub default_branch: Option<String>,
    /// Relative to the main root. Default `.worktrees`.
    pub worktrees_dir: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GuardrailsSection {
    /// Ids of rules disabled in this repo.
    pub off: Vec<String>,
    /// Path, relative to the main root, of a rules file with the built-in schema.
    pub extra: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Thresholds {
    pub live_minutes: u64,
    pub idle_minutes: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self { live_minutes: 10, idle_minutes: 60 }
    }
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MachineConfig {
    pub guardrails: MachineGuardrails,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MachineGuardrails {
    /// Path of a rules file; relative paths resolve against the state directory.
    pub extra: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    pub path: PathBuf,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for ConfigError {}

pub fn parse_repo_config(text: &str, path: &Path) -> Result<RepoConfig, ConfigError> {
    toml::from_str(text).map_err(|e| ConfigError { path: path.to_path_buf(), message: e.to_string() })
}

pub fn load_repo_config(path: &Path) -> Result<RepoConfig, ConfigError> {
    let text = fs::read_to_string(path).map_err(|e| ConfigError { path: path.to_path_buf(), message: e.to_string() })?;
    parse_repo_config(&text, path)
}

pub fn ratchet_home(env: &HashMap<String, String>) -> PathBuf {
    if let Some(h) = env.get("RATCHET_HOME").filter(|s| !s.is_empty()) {
        return PathBuf::from(h);
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".ratchet")
}

pub fn load_machine_config(home: &Path) -> Result<MachineConfig, ConfigError> {
    let path = home.join("config.toml");
    if !path.is_file() {
        return Ok(MachineConfig::default());
    }
    let text = fs::read_to_string(&path).map_err(|e| ConfigError { path: path.clone(), message: e.to_string() })?;
    toml::from_str(&text).map_err(|e| ConfigError { path, message: e.to_string() })
}
```

- [ ] **Step 4: Implement `repo.rs`**

```rust
//! Which repo does a working directory belong to? Answered from the nearest `ratchet.toml`,
//! without spawning git: a linked worktree has a `.git` *file* pointing at the main checkout.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{self, ConfigError, RepoConfig, MARKER};

#[derive(Debug, Clone)]
pub struct Repo {
    /// The main checkout (owner of the worktrees).
    pub main_root: PathBuf,
    /// Where the marker nearest to cwd was found (a worktree or the main root).
    pub checkout_root: PathBuf,
    pub name: String,
    pub worktrees_dir: PathBuf,
    pub config: RepoConfig,
}

pub fn find_repo(cwd: &Path) -> Result<Option<Repo>, ConfigError> {
    let Some(checkout_root) = nearest_marker_dir(cwd) else {
        return Ok(None);
    };
    let main_root = main_root_of(&checkout_root);
    let cfg_path = if main_root.join(MARKER).is_file() { main_root.join(MARKER) } else { checkout_root.join(MARKER) };
    let cfg = config::load_repo_config(&cfg_path)?;
    let name = cfg
        .repo
        .name
        .clone()
        .unwrap_or_else(|| main_root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "repo".into()));
    let worktrees_dir = main_root.join(cfg.repo.worktrees_dir.as_deref().unwrap_or(".worktrees"));
    Ok(Some(Repo { main_root, checkout_root, name, worktrees_dir, config: cfg }))
}

fn nearest_marker_dir(start: &Path) -> Option<PathBuf> {
    start.ancestors().find(|d| d.join(MARKER).is_file()).map(Path::to_path_buf)
}

/// `<checkout>/.git` as a file means a linked worktree: `gitdir: <main>/.git/worktrees/<name>`.
pub fn main_root_of(checkout_root: &Path) -> PathBuf {
    let dot_git = checkout_root.join(".git");
    if dot_git.is_file() {
        if let Ok(text) = fs::read_to_string(&dot_git) {
            if let Some(rest) = text.trim().strip_prefix("gitdir:") {
                let gitdir = PathBuf::from(rest.trim());
                let gitdir = if gitdir.is_absolute() { gitdir } else { checkout_root.join(gitdir) };
                // <main>/.git/worktrees/<name> → <main>
                if let Some(main_git) = gitdir.parent().and_then(Path::parent) {
                    if main_git.file_name().map(|n| n == ".git").unwrap_or(false) {
                        if let Some(main) = main_git.parent() {
                            return main.to_path_buf();
                        }
                    }
                }
            }
        }
    }
    checkout_root.to_path_buf()
}

/// Absolute, symlink-resolved when possible, lower-cased, without the Windows `\\?\` prefix.
pub fn normalize(p: &Path) -> PathBuf {
    let abs = p.canonicalize().or_else(|_| std::path::absolute(p)).unwrap_or_else(|_| p.to_path_buf());
    let s = abs.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    PathBuf::from(s.to_lowercase().replace('/', std::path::MAIN_SEPARATOR_STR))
}

pub fn within(target: &Path, root: &Path) -> bool {
    let (t, r) = (normalize(target), normalize(root));
    t == r || t.starts_with(&r)
}

/// `.venv` in cwd or any parent up to (and including) the main root.
pub fn has_venv(cwd: &Path, main_root: &Path) -> bool {
    let stop = normalize(main_root);
    for dir in cwd.ancestors() {
        if dir.join(".venv").exists() {
            return true;
        }
        if normalize(dir) == stop {
            break;
        }
    }
    main_root.join(".venv").exists()
}

/// Is `target` tracked by the main checkout? The only subprocess on the hot path, reached only
/// when a write already points inside the main tree.
pub fn is_tracked(main_root: &Path, target: &Path) -> bool {
    let rel = match pathdiff(target, main_root) {
        Some(r) => r,
        None => return false,
    };
    Command::new("git")
        .args(["-C", &main_root.to_string_lossy(), "ls-files", "--error-unmatch", "--"])
        .arg(rel)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn pathdiff(target: &Path, root: &Path) -> Option<PathBuf> {
    let (t, r) = (normalize(target), normalize(root));
    t.strip_prefix(&r).ok().map(Path::to_path_buf)
}
```

- [ ] **Step 5: Run the unit tests**

Run: `cargo test -p ratchet --lib`
Expected: all `config::tests` and `repo::tests` PASS.

- [ ] **Step 6: Hand off (no git)**

`cargo fmt`, `cargo clippy --all-targets -- -D warnings` clean. List the two files.

---

### Task 5: `guardrails/segment.rs` (split a command outside quotes)

**Files:**
- Create: `crates/ratchet/src/guardrails/segment.rs`
- Modify: `crates/ratchet/src/guardrails/mod.rs` (add `pub mod segment;`)

**Interfaces:**
- Produces: `segment::segments(command: &str) -> Vec<String>` — splits on `;`, `&&`, `||`, `|`, `\n` outside single/double quotes; `\;` escaped outside quotes does not split; an unclosed quote leaves the rest as one segment; empty segments dropped; each trimmed.

- [ ] **Step 1: Write the failing tests (bottom of the new file)**

```rust
#[cfg(test)]
mod tests {
    use super::segments;

    #[test]
    fn respects_quotes() {
        assert_eq!(
            segments(r#"uv run ratchet task new -c "a; mypy b" && echo done"#),
            vec![r#"uv run ratchet task new -c "a; mypy b""#, "echo done"]
        );
    }

    #[test]
    fn splits_on_every_operator_and_newline() {
        assert_eq!(
            segments("uv run a; mypy b | ruff c || pytest d\npython e"),
            vec!["uv run a", "mypy b", "ruff c", "pytest d", "python e"]
        );
    }

    #[test]
    fn unclosed_quote_keeps_the_rest_whole() {
        let cmd = r#"uv run ratchet task note T-1 "left it; pytest green"#;
        assert_eq!(segments(cmd), vec![cmd]);
    }

    #[test]
    fn escaped_semicolon_outside_quotes_does_not_split() {
        assert_eq!(segments(r"echo a\; python b"), vec![r"echo a\; python b"]);
    }

    #[test]
    fn windows_backslashes_inside_quotes_are_fine() {
        assert_eq!(
            segments(r#"uv run python "C:\repos\x\s.py; pytest later""#),
            vec![r#"uv run python "C:\repos\x\s.py; pytest later""#]
        );
    }

    #[test]
    fn single_ampersand_does_not_split() {
        assert_eq!(segments("sleep 1 & echo bg"), vec!["sleep 1 & echo bg"]);
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p ratchet --lib segment`
Expected: compile error, `segments` undefined.

- [ ] **Step 3: Implement**

```rust
//! Split a shell command into segments on `;`, `&&`, `||`, `|` and newlines, only outside
//! quotes. We do not de-quote (shlex would fight Windows paths); we only need to know where to
//! cut, so we scan the raw text carrying the quote state.

pub fn segments(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    let n = chars.len();
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
                } else if ch == ';' || ch == '\n' || ch == '|' {
                    out.push(std::mem::take(&mut buf));
                    if ch == '|' && i + 1 < n && chars[i + 1] == '|' {
                        i += 1;
                    }
                } else if ch == '&' && i + 1 < n && chars[i + 1] == '&' {
                    out.push(std::mem::take(&mut buf));
                    i += 1;
                } else {
                    buf.push(ch);
                }
            }
        }
        i += 1;
    }
    out.push(buf);
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p ratchet --lib segment`
Expected: 6 PASS.

- [ ] **Step 5: Hand off (no git)**

---

### Task 6: `guardrails/rules.rs` and `builtin.toml` (rule schema, built-ins, loading, merging)

**Files:**
- Create: `crates/ratchet/src/guardrails/rules.rs`, `crates/ratchet/src/guardrails/builtin.toml`
- Modify: `crates/ratchet/src/guardrails/mod.rs` (add `pub mod rules;`)

**Interfaces:**
- Consumes: `config::{ConfigError, MachineConfig, load_machine_config}`, `repo::Repo`.
- Produces:
  - `rules::Kind { Command, FilePath, Content, MainTree }` (serde `snake_case`)
  - `rules::Rule { id, tools: Vec<String>, kind, pattern: Option<String>, exempt: Option<String>, requires: Option<String>, message, alternative, source: String }` (`source` not in the file: `builtin` / `machine` / `repo`)
  - `rules::builtin_rules() -> Vec<Rule>`
  - `rules::parse_rules(text: &str, path: &Path, source: &str) -> Result<Vec<Rule>, ConfigError>` (also validates every regex)
  - `rules::merge(base: Vec<Rule>, extra: Vec<Rule>) -> Vec<Rule>` (same id → replaced in place)
  - `rules::load_rule_set(home: &Path, repo: Option<&Repo>) -> Result<RuleSet, ConfigError>` with `RuleSet { rules: Vec<Rule>, off: Vec<String> }`, `RuleSet::active(&self) -> impl Iterator<Item=&Rule>` (excludes `off`) and `RuleSet::is_off(&self, id) -> bool`

- [ ] **Step 1: Write `builtin.toml`**

```toml
# Built-in guardrails. Same schema for ~/.ratchet/guardrails.toml and a repo's extra file.
# kind: command   -> regex over each command segment (split on ; && || | and newlines, outside quotes)
#       file_path -> regex over tool_input.file_path (or notebook_path)
#       content   -> regex over command + content + new_string (whatever will run or be written)
#       main_tree -> write to a tracked file of the main tree, outside worktrees_dir
# exempt: regex that, when it matches THE SAME SEGMENT, cancels the rule.
# requires = "venv" -> rule applies only when the repo has a .venv.
# Patterns are case-insensitive and multiline. Lookaround is supported.

[[rules]]
id = "python-venv"
tools = ["Bash", "PowerShell"]
kind = "command"
pattern = '^\s*(env\s+)?(\w+=\S*\s+)*(python|python3|py|pip|pip3|pytest|uvicorn|mypy|ruff)(\.exe)?(\s|$)'
exempt = '^\s*(env\s+)?(\w+=\S*\s+)*uv\s+run\b|\.venv[\\/](Scripts|bin)[\\/]'
requires = "venv"
message = "Python must run through the repo's virtualenv, not the global interpreter."
alternative = "Prefix the command with `uv run` (e.g. `uv run python scripts/x.py`) or call the venv interpreter (`.venv/Scripts/python` or `.venv/bin/python`)."

[[rules]]
id = "git-destructive"
tools = ["Bash", "PowerShell"]
kind = "command"
pattern = 'git\s+push\b(?![^;&|]*--force-with-lease)[^;&|]*(--force\b|\s-f\b)|git\s+reset\s+--hard|git\s+checkout\s+--\s+\.|git\s+clean\b[^;&|]*\s-[a-zA-Z]*f|^\s*rm(?=(?:.*\s)(?:-[a-zA-Z]*r[a-zA-Z]*|--recursive)(?:\s|$))(?=(?:.*\s)(?:-[a-zA-Z]*f[a-zA-Z]*|--force)(?:\s|$))'
message = "Destructive git or file action: the owner runs it, not an agent."
alternative = "Ask the owner to run it, or use a reversible alternative (git stash, git revert, move the files to the scratchpad)."

[[rules]]
id = "env-files"
tools = ["Edit", "Write", "NotebookEdit", "MultiEdit", "PowerShell"]
kind = "file_path"
pattern = '(^|[\\/])\.env(\.[A-Za-z0-9_.-]+)?$'
message = "`.env` files are never written by agents: they hold credentials."
alternative = "Credentials live outside the repo; if a variable is missing, tell the owner."

[[rules]]
id = "main-tree"
tools = ["Edit", "Write", "NotebookEdit", "MultiEdit"]
kind = "main_tree"
message = "Write to a tracked file in the repo's main tree."
alternative = "Work in a worktree (`git worktree add <worktrees_dir>/<name> -b <branch>`) and edit there."
```

- [ ] **Step 2: Write the failing tests (bottom of `rules.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn builtins_have_the_four_ids_in_order() {
        let ids: Vec<&str> = builtin_rules().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["python-venv", "git-destructive", "env-files", "main-tree"]);
        assert!(builtin_rules().iter().all(|r| r.source == "builtin"));
    }

    #[test]
    fn parse_rejects_bad_regex_with_the_file_named() {
        let text = "[[rules]]\nid = \"x\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = '('\nmessage = \"m\"\nalternative = \"a\"\n";
        let err = parse_rules(text, Path::new("g.toml"), "repo").unwrap_err();
        assert!(err.to_string().contains("g.toml"), "{err}");
        assert!(err.message.contains("x"), "{}", err.message);
    }

    #[test]
    fn parse_rejects_unknown_kind_and_missing_message() {
        let text = "[[rules]]\nid = \"x\"\ntools = [\"Bash\"]\nkind = \"weird\"\nmessage = \"m\"\nalternative = \"a\"\n";
        assert!(parse_rules(text, Path::new("g.toml"), "repo").is_err());
        let text = "[[rules]]\nid = \"x\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = 'a'\nalternative = \"a\"\n";
        assert!(parse_rules(text, Path::new("g.toml"), "repo").is_err());
    }

    #[test]
    fn merge_replaces_same_id_in_place_and_appends_new() {
        let base = builtin_rules();
        let extra = parse_rules(
            "[[rules]]\nid = \"git-destructive\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = 'x'\nmessage = \"new\"\nalternative = \"a\"\n[[rules]]\nid = \"custom\"\ntools = [\"Bash\"]\nkind = \"command\"\npattern = 'y'\nmessage = \"c\"\nalternative = \"a\"\n",
            Path::new("g.toml"),
            "machine",
        )
        .unwrap();
        let merged = merge(base, extra);
        let ids: Vec<&str> = merged.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["python-venv", "git-destructive", "env-files", "main-tree", "custom"]);
        assert_eq!(merged[1].message, "new");
        assert_eq!(merged[1].source, "machine");
    }

    #[test]
    fn rule_set_active_excludes_off() {
        let set = RuleSet { rules: builtin_rules(), off: vec!["python-venv".into()] };
        let ids: Vec<&str> = set.active().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["git-destructive", "env-files", "main-tree"]);
    }
}
```

- [ ] **Step 3: Run to see them fail**

Run: `cargo test -p ratchet --lib rules`
Expected: compile errors.

- [ ] **Step 4: Implement `rules.rs`**

```rust
//! Guardrail rules: schema, built-ins, extension files and merging. Pure; the only I/O is
//! reading the extra files named by the machine and repo configs.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::{self, ConfigError};
use crate::repo::Repo;

pub const BUILTIN_TOML: &str = include_str!("builtin.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Command,
    FilePath,
    Content,
    MainTree,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Command => "command",
            Kind::FilePath => "file_path",
            Kind::Content => "content",
            Kind::MainTree => "main_tree",
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub tools: Vec<String>,
    pub kind: Kind,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub exempt: Option<String>,
    #[serde(default)]
    pub requires: Option<String>,
    pub message: String,
    pub alternative: String,
    /// `builtin`, `machine` or `repo`. Not read from the file.
    #[serde(skip)]
    pub source: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    #[serde(default)]
    rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
    pub off: Vec<String>,
}

impl RuleSet {
    pub fn active(&self) -> impl Iterator<Item = &Rule> {
        self.rules.iter().filter(move |r| !self.off.iter().any(|o| o == &r.id))
    }
    pub fn is_off(&self, id: &str) -> bool {
        self.off.iter().any(|o| o == id)
    }
}

pub fn builtin_rules() -> Vec<Rule> {
    parse_rules(BUILTIN_TOML, Path::new("<builtin>"), "builtin").expect("builtin rules are valid")
}

pub fn parse_rules(text: &str, path: &Path, source: &str) -> Result<Vec<Rule>, ConfigError> {
    let file: RuleFile = toml::from_str(text).map_err(|e| ConfigError { path: path.to_path_buf(), message: e.to_string() })?;
    let mut rules = file.rules;
    for r in &mut rules {
        r.source = source.to_string();
        for (label, pat) in [("pattern", &r.pattern), ("exempt", &r.exempt)] {
            if let Some(p) = pat {
                fancy_regex::Regex::new(&format!("(?im){p}")).map_err(|e| ConfigError {
                    path: path.to_path_buf(),
                    message: format!("rule `{}`: invalid {label} regex: {e}", r.id),
                })?;
            }
        }
        if r.kind != Kind::MainTree && r.pattern.is_none() {
            return Err(ConfigError { path: path.to_path_buf(), message: format!("rule `{}`: `pattern` is required for kind {}", r.id, r.kind.as_str()) });
        }
    }
    Ok(rules)
}

fn load_rules_file(path: &Path, source: &str) -> Result<Vec<Rule>, ConfigError> {
    let text = fs::read_to_string(path).map_err(|e| ConfigError { path: path.to_path_buf(), message: e.to_string() })?;
    parse_rules(&text, path, source)
}

/// Later rules with an existing id replace the earlier rule in place; new ids append.
pub fn merge(mut base: Vec<Rule>, extra: Vec<Rule>) -> Vec<Rule> {
    for r in extra {
        match base.iter_mut().find(|b| b.id == r.id) {
            Some(slot) => *slot = r,
            None => base.push(r),
        }
    }
    base
}

/// built-ins, then the machine file (`~/.ratchet/config.toml` → `guardrails.extra`), then the
/// repo file (`ratchet.toml` → `guardrails.extra`, relative to the main root).
pub fn load_rule_set(home: &Path, repo: Option<&Repo>) -> Result<RuleSet, ConfigError> {
    let mut rules = builtin_rules();
    let machine = config::load_machine_config(home)?;
    if let Some(extra) = machine.guardrails.extra {
        let path = resolve(&extra, home);
        rules = merge(rules, load_rules_file(&path, "machine")?);
    }
    let mut off = Vec::new();
    if let Some(repo) = repo {
        if let Some(extra) = &repo.config.guardrails.extra {
            let path = resolve(extra, &repo.main_root);
            rules = merge(rules, load_rules_file(&path, "repo")?);
        }
        off = repo.config.guardrails.off.clone();
    }
    Ok(RuleSet { rules, off })
}

fn resolve(p: &str, base: &Path) -> PathBuf {
    let path = PathBuf::from(p);
    if path.is_absolute() { path } else { base.join(path) }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p ratchet --lib rules`
Expected: 5 PASS.

- [ ] **Step 6: Hand off (no git)**

---

### Task 7: `guardrails/eval.rs` (evaluate command / file_path / content rules)

**Files:**
- Create: `crates/ratchet/src/guardrails/eval.rs`, `crates/ratchet/src/guardrails/main_tree.rs` (placeholder, real in Task 8)
- Modify: `crates/ratchet/src/guardrails/mod.rs` (add `pub mod eval; pub mod main_tree;`)

**Interfaces:**
- Consumes: `rules::{Rule, Kind}`, `segment::segments`, `repo::{normalize, within}`; `main_tree::writes_main_tree` (placeholder now, real in Task 8).
- Produces:
  - `eval::GuardContext { main_root: Option<PathBuf>, worktrees_dir: Option<PathBuf>, cwd: PathBuf, has_venv: bool, scratchpad: Option<PathBuf> }`
  - `eval::Violation { rule_id, message, alternative }` with `render() -> String` = `"[ratchet guardrail:{id}] {message} {alternative}"`
  - `eval::evaluate(rules: &[&Rule], tool_name: &str, tool_input: &serde_json::Value, ctx: &GuardContext) -> Option<Violation>` — first matching rule wins, in order
  - `eval::scratchpad_from_env(env: &HashMap<String,String>) -> Option<PathBuf>` (`CLAUDE_SCRATCHPAD`, else `%LOCALAPPDATA%\Temp\claude` when `LOCALAPPDATA` is set)
  - `eval::SCAN_CAP: usize = 262_144` (text longer than this is scanned only up to the cap)
  - `main_tree::writes_main_tree(tool_name: &str, tool_input: &Value, ctx: &GuardContext) -> bool`

- [ ] **Step 1: Write the failing tests (bottom of `eval.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::guardrails::rules::builtin_rules;
    use serde_json::json;
    use std::path::PathBuf;

    fn ctx(has_venv: bool, scratchpad: Option<PathBuf>) -> GuardContext {
        GuardContext { main_root: Some(PathBuf::from("C:/r")), worktrees_dir: Some(PathBuf::from("C:/r/.worktrees")), cwd: PathBuf::from("C:/r"), has_venv, scratchpad }
    }

    fn bash(cmd: &str, c: &GuardContext) -> Option<String> {
        let rules = builtin_rules();
        let refs: Vec<&Rule> = rules.iter().collect();
        evaluate(&refs, "Bash", &json!({ "command": cmd }), c).map(|v| v.rule_id)
    }

    #[test]
    fn python_venv_blocks_and_exempts() {
        let c = ctx(true, None);
        assert_eq!(bash("python scripts/x.py", &c).as_deref(), Some("python-venv"));
        assert_eq!(bash("PYTHONPATH=. python x.py", &c).as_deref(), Some("python-venv"));
        assert_eq!(bash("uv run pytest -q", &c), None);
        assert_eq!(bash(r".venv\Scripts\python.exe x.py", &c), None);
        assert_eq!(bash("echo ok && python x.py", &c).as_deref(), Some("python-venv"));
        assert_eq!(bash(r#"uv run x -c "a; mypy b""#, &c), None);
    }

    #[test]
    fn python_venv_needs_a_venv() {
        assert_eq!(bash("python scripts/x.py", &ctx(false, None)), None);
    }

    #[test]
    fn git_destructive_cases() {
        let c = ctx(false, None);
        for cmd in ["git reset --hard HEAD~1", "git push -f origin main", "git push origin main --force", "git checkout -- .", "git clean -fd", "rm -rf build", "rm -r -f build", "rm --recursive --force build"] {
            assert_eq!(bash(cmd, &c).as_deref(), Some("git-destructive"), "{cmd}");
        }
        for cmd in ["git push --force-with-lease", "git reset --soft HEAD~1", "rm -r build", "rm build.txt", "git clean -n"] {
            assert_eq!(bash(cmd, &c), None, "{cmd}");
        }
    }

    #[test]
    fn rm_under_scratchpad_is_allowed_only_when_all_targets_are_inside() {
        let scratch = tempfile::TempDir::new().unwrap();
        let c = ctx(false, Some(scratch.path().to_path_buf()));
        let inside = scratch.path().join("tmp");
        assert_eq!(bash(&format!("rm -rf {}", inside.display()), &c), None);
        assert_eq!(bash(&format!("rm -rf {} C:/r/src", inside.display()), &c).as_deref(), Some("git-destructive"));
        assert_eq!(bash("rm -rf C:/r/src", &c).as_deref(), Some("git-destructive"));
    }

    #[test]
    fn env_files_by_path() {
        let rules = builtin_rules();
        let refs: Vec<&Rule> = rules.iter().collect();
        let c = ctx(false, None);
        let hit = evaluate(&refs, "Write", &json!({ "file_path": "C:/r/.env.local", "content": "" }), &c);
        assert_eq!(hit.map(|v| v.rule_id).as_deref(), Some("env-files"));
        let ok = evaluate(&refs, "Write", &json!({ "file_path": "C:/r/environment.md", "content": "" }), &c);
        assert!(ok.is_none());
        let ps = evaluate(&refs, "PowerShell", &json!({ "command": "Set-Content .env x" }), &c);
        assert!(ps.is_none(), "file_path rules look at file_path, not commands");
    }

    #[test]
    fn content_rule_scans_command_content_and_new_string() {
        let custom = Rule { id: "db".into(), tools: vec!["Bash".into(), "Write".into(), "Edit".into()], kind: Kind::Content, pattern: Some(r"\.purge_all\s*\(".into()), exempt: None, requires: None, message: "m".into(), alternative: "a".into(), source: "repo".into() };
        let refs = vec![&custom];
        let c = ctx(false, None);
        assert!(evaluate(&refs, "Write", &json!({ "file_path": "x.py", "content": "c.purge_all({})" }), &c).is_some());
        assert!(evaluate(&refs, "Edit", &json!({ "file_path": "x.py", "new_string": "c.purge_all (" }), &c).is_some());
        assert!(evaluate(&refs, "Bash", &json!({ "command": "uv run python -c 'c.purge_all({})'" }), &c).is_some());
        assert!(evaluate(&refs, "Write", &json!({ "file_path": "x.py", "content": "c.find({})" }), &c).is_none());
    }

    #[test]
    fn tool_filter_and_order() {
        let rules = builtin_rules();
        let refs: Vec<&Rule> = rules.iter().collect();
        let c = ctx(true, None);
        assert!(evaluate(&refs, "Read", &json!({ "command": "python x.py" }), &c).is_none());
        let v = evaluate(&refs, "Bash", &json!({ "command": "python x.py; git reset --hard" }), &c).unwrap();
        assert_eq!(v.rule_id, "python-venv");
        assert_eq!(v.render(), format!("[ratchet guardrail:python-venv] {} {}", v.message, v.alternative));
    }

    #[test]
    fn scratchpad_from_env_prefers_claude_var() {
        let mut env = std::collections::HashMap::new();
        env.insert("LOCALAPPDATA".to_string(), "C:/u/l".to_string());
        assert_eq!(scratchpad_from_env(&env), Some(PathBuf::from("C:/u/l").join("Temp").join("claude")));
        env.insert("CLAUDE_SCRATCHPAD".to_string(), "C:/s".to_string());
        assert_eq!(scratchpad_from_env(&env), Some(PathBuf::from("C:/s")));
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p ratchet --lib eval`
Expected: compile errors.

- [ ] **Step 3: Implement `eval.rs`** (and the placeholder `main_tree.rs`)

`crates/ratchet/src/guardrails/main_tree.rs` (placeholder, replaced in Task 8):
```rust
use serde_json::Value;

use super::eval::GuardContext;

pub fn writes_main_tree(_tool_name: &str, _tool_input: &Value, _ctx: &GuardContext) -> bool {
    false
}
```

`crates/ratchet/src/guardrails/eval.rs`:
```rust
//! Pure evaluation of a tool call against a rule list. No I/O except the tracked-file check
//! delegated to `main_tree` (which only runs when a write already points into the main tree).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use fancy_regex::Regex;
use serde_json::Value;

use super::main_tree::writes_main_tree;
use super::rules::{Kind, Rule};
use super::segment::segments;
use crate::repo::within;

/// Longest text a rule scans; longer inputs are scanned up to this many bytes.
pub const SCAN_CAP: usize = 262_144;

#[derive(Debug, Clone)]
pub struct GuardContext {
    pub main_root: Option<PathBuf>,
    pub worktrees_dir: Option<PathBuf>,
    pub cwd: PathBuf,
    pub has_venv: bool,
    pub scratchpad: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub rule_id: String,
    pub message: String,
    pub alternative: String,
}

impl Violation {
    pub fn render(&self) -> String {
        format!("[ratchet guardrail:{}] {} {}", self.rule_id, self.message, self.alternative)
    }
}

pub fn evaluate(rules: &[&Rule], tool_name: &str, tool_input: &Value, ctx: &GuardContext) -> Option<Violation> {
    for rule in rules {
        if !rule.tools.iter().any(|t| t == tool_name) {
            continue;
        }
        if rule.requires.as_deref() == Some("venv") && !ctx.has_venv {
            continue;
        }
        if matches(rule, tool_name, tool_input, ctx) {
            return Some(Violation { rule_id: rule.id.clone(), message: rule.message.clone(), alternative: rule.alternative.clone() });
        }
    }
    None
}

pub fn scratchpad_from_env(env: &HashMap<String, String>) -> Option<PathBuf> {
    if let Some(s) = env.get("CLAUDE_SCRATCHPAD").filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(s));
    }
    env.get("LOCALAPPDATA").filter(|s| !s.is_empty()).map(|l| PathBuf::from(l).join("Temp").join("claude"))
}

fn matches(rule: &Rule, tool_name: &str, tool_input: &Value, ctx: &GuardContext) -> bool {
    if rule.kind == Kind::MainTree {
        return writes_main_tree(tool_name, tool_input, ctx);
    }
    let text = text_for(rule.kind, tool_input);
    let text = cap(&text);
    let Some(pattern) = rule.pattern.as_deref().and_then(compile) else {
        return false;
    };
    let exempt = rule.exempt.as_deref().and_then(compile);
    if rule.kind != Kind::Command {
        return is_match(&pattern, text) && !exempt.as_ref().map(|e| is_match(e, text)).unwrap_or(false);
    }
    for segment in segments(text) {
        if !is_match(&pattern, &segment) {
            continue;
        }
        if exempt.as_ref().map(|e| is_match(e, &segment)).unwrap_or(false) {
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
            if fp.is_empty() { get("notebook_path").to_string() } else { fp.to_string() }
        }
        Kind::Content | Kind::MainTree => ["command", "content", "new_string", "new_source"]
            .iter()
            .map(|k| get(k))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// True when the segment is an `rm …` whose every non-flag argument is under the scratchpad.
/// A `git …` destructive command sharing the segment stays blocked.
fn only_rm_under_scratchpad(segment: &str, scratchpad: Option<&Path>) -> bool {
    let Some(scratch) = scratchpad else { return false };
    if !is_match(&compile(r"^\s*rm\s+").unwrap(), segment) {
        return false;
    }
    if is_match(&compile(r"git\s+(push|reset|checkout|clean)").unwrap(), segment) {
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
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p ratchet --lib eval`
Expected: 8 PASS. If `git_destructive_cases` fails on a specific command, the regex was mis-copied from `builtin.toml`; compare character by character with Task 6 Step 1 rather than changing the test.

- [ ] **Step 5: Hand off (no git)**

---

### Task 8: `guardrails/main_tree.rs` (writes to a tracked file of the main tree)

**Files:**
- Modify: `crates/ratchet/src/guardrails/main_tree.rs` (replace placeholder)

**Interfaces:**
- Consumes: `eval::GuardContext`, `repo::{within, is_tracked}`.
- Produces: `main_tree::writes_main_tree(tool_name, tool_input, ctx) -> bool`, true only when: the tool is one of `Edit|Write|NotebookEdit|MultiEdit`, `ctx.main_root` is set, the target (absolute, or relative to `ctx.cwd`) is within `main_root`, not within `worktrees_dir`, and `is_tracked(main_root, target)`.

- [ ] **Step 1: Write the failing tests (bottom of the file)**

```rust
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
            .env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").env_remove("GIT_INDEX_FILE")
            .stdout(Stdio::null()).stderr(Stdio::null())
            .status().unwrap();
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
        assert!(writes_main_tree("Edit", &edit(&d.path().join("tracked.txt")), &ctx));
    }

    #[test]
    fn relative_path_resolves_against_cwd() {
        let (_d, ctx) = repo_with_tracked_file();
        assert!(writes_main_tree("Write", &serde_json::json!({ "file_path": "tracked.txt", "content": "" }), &ctx));
    }

    #[test]
    fn untracked_worktree_and_outside_are_not() {
        let (d, ctx) = repo_with_tracked_file();
        assert!(!writes_main_tree("Edit", &edit(&d.path().join("new.txt")), &ctx));
        assert!(!writes_main_tree("Edit", &edit(&d.path().join(".worktrees/wt/tracked.txt")), &ctx));
        assert!(!writes_main_tree("Edit", &edit(&PathBuf::from("C:/elsewhere/tracked.txt")), &ctx));
    }

    #[test]
    fn only_write_tools_and_only_with_a_repo() {
        let (d, mut ctx) = repo_with_tracked_file();
        assert!(!writes_main_tree("Bash", &serde_json::json!({ "command": "echo" }), &ctx));
        assert!(!writes_main_tree("Read", &edit(&d.path().join("tracked.txt")), &ctx));
        ctx.main_root = None;
        assert!(!writes_main_tree("Edit", &edit(&d.path().join("tracked.txt")), &ctx));
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p ratchet --lib main_tree`
Expected: `tracked_file_in_main_tree_is_a_write` and `relative_path_resolves_against_cwd` FAIL (placeholder returns false).

- [ ] **Step 3: Implement**

```rust
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
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p ratchet --lib`
Expected: all unit tests PASS (config, repo, segment, rules, eval, main_tree).

- [ ] **Step 5: Hand off (no git)**

---

### Task 9: `hook pre-tool` end to end (dispatch, log, never-break)

**Files:**
- Modify: `crates/ratchet/src/log.rs`, `crates/ratchet/src/hooks/mod.rs` (replace placeholders)
- Create: `crates/ratchet/src/hooks/dispatch.rs`

**Interfaces:**
- Consumes: `config::ratchet_home`, `repo::{find_repo, has_venv}`, `guardrails::rules::load_rule_set`, `guardrails::eval::{evaluate, GuardContext, scratchpad_from_env}`.
- Produces:
  - `log::append(home: &Path, line: &str)` — creates `home`, appends `"<RFC3339 local> <line>\n"` to `home/ratchet.log`, swallows errors.
  - `hooks::run(event, stdin, env, process_cwd) -> i32` — catches every error and panic → log → 0.
  - `hooks::dispatch::Payload { tool_name: String, tool_input: Value, cwd: Option<PathBuf>, hook_event_name: Option<String>, stop_hook_active: bool }` and `dispatch::parse_payload(text: &str) -> Result<Payload, String>` (empty stdin → default payload).
  - `dispatch::dispatch(event, payload, env, process_cwd, home) -> Result<i32, String>`; `dispatch::pre_tool(payload, env, cwd, home) -> Result<i32, String>`; other known events → `Ok(0)`; unknown → `Err`.
  - `dispatch::is_known_event(event) -> bool`, `hooks::BLOCK: i32 = 2`.

- [ ] **Step 1: Write the failing unit tests (bottom of `dispatch.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_payload() {
        let p = parse_payload(r#"{"tool_name":"Bash","tool_input":{"command":"ls"},"cwd":"C:/r","hook_event_name":"PreToolUse","stop_hook_active":true}"#).unwrap();
        assert_eq!(p.tool_name, "Bash");
        assert_eq!(p.tool_input["command"], "ls");
        assert_eq!(p.cwd.as_deref(), Some(std::path::Path::new("C:/r")));
        assert!(p.stop_hook_active);
    }

    #[test]
    fn empty_stdin_is_a_default_payload() {
        let p = parse_payload("   ").unwrap();
        assert_eq!(p.tool_name, "");
        assert!(p.cwd.is_none());
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(parse_payload("not json").is_err());
    }

    #[test]
    fn known_events_are_recognised() {
        for e in ["session-start", "prompt", "pre-tool", "stop", "subagent-stop", "pre-compact", "session-end"] {
            assert!(is_known_event(e), "{e}");
        }
        assert!(!is_known_event("no-such-event"));
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p ratchet --lib dispatch`
Expected: compile errors.

- [ ] **Step 3: Implement `log.rs`**

```rust
//! One-line append to `<home>/ratchet.log`. Never fails: a hook must not die because the log did.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub fn append(home: &Path, line: &str) {
    let _ = fs::create_dir_all(home);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(home.join("ratchet.log")) {
        let stamp = chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let _ = writeln!(f, "{stamp} {line}");
    }
}
```

- [ ] **Step 4: Implement `hooks/dispatch.rs`**

```rust
//! Hook payload parsing and per-event handlers. Errors bubble up as `String`; `hooks::run`
//! turns them into exit 0 + log line.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::guardrails::eval::{evaluate, scratchpad_from_env, GuardContext};
use crate::guardrails::rules::load_rule_set;
use crate::repo::{find_repo, has_venv};

pub const KNOWN_EVENTS: [&str; 7] = ["session-start", "prompt", "pre-tool", "stop", "subagent-stop", "pre-compact", "session-end"];

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Payload {
    pub tool_name: String,
    pub tool_input: Value,
    pub cwd: Option<PathBuf>,
    pub hook_event_name: Option<String>,
    pub stop_hook_active: bool,
}

pub fn parse_payload(text: &str) -> Result<Payload, String> {
    if text.trim().is_empty() {
        return Ok(Payload::default());
    }
    serde_json::from_str(text).map_err(|e| format!("invalid payload: {e}"))
}

pub fn is_known_event(event: &str) -> bool {
    KNOWN_EVENTS.contains(&event)
}

/// Returns the exit code. `Err` means an internal error the caller logs and maps to 0.
pub fn dispatch(event: &str, payload: Payload, env: &HashMap<String, String>, process_cwd: Option<PathBuf>, home: &Path) -> Result<i32, String> {
    if !is_known_event(event) {
        return Err(format!("unknown event `{event}`"));
    }
    let cwd = payload.cwd.clone().or(process_cwd).ok_or("no cwd")?;
    match event {
        "pre-tool" => pre_tool(payload, env, &cwd, home),
        // Groups 1 and 2 fill these in; until then they are no-ops by design.
        _ => Ok(0),
    }
}

pub fn pre_tool(payload: Payload, env: &HashMap<String, String>, cwd: &Path, home: &Path) -> Result<i32, String> {
    let Some(repo) = find_repo(cwd).map_err(|e| e.to_string())? else {
        return Ok(0);
    };
    let set = load_rule_set(home, Some(&repo)).map_err(|e| e.to_string())?;
    let ctx = GuardContext {
        main_root: Some(repo.main_root.clone()),
        worktrees_dir: Some(repo.worktrees_dir.clone()),
        cwd: cwd.to_path_buf(),
        has_venv: has_venv(cwd, &repo.main_root),
        scratchpad: scratchpad_from_env(env),
    };
    let rules: Vec<_> = set.active().collect();
    match evaluate(&rules, &payload.tool_name, &payload.tool_input, &ctx) {
        Some(v) => {
            eprintln!("{}", v.render());
            Ok(super::BLOCK)
        }
        None => Ok(0),
    }
}
```

- [ ] **Step 5: Implement `hooks/mod.rs`**

```rust
//! Entry point of every hook. Principle: a hook never breaks a session. Any error or panic is
//! exit 0 plus one log line; the only non-zero exit is a deliberate block (2).

pub mod dispatch;

use std::collections::HashMap;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use crate::config::ratchet_home;
use crate::log;

pub const BLOCK: i32 = 2;

pub fn run(event: &str, mut stdin: impl Read, env: &HashMap<String, String>, process_cwd: Option<PathBuf>) -> i32 {
    // A panic must not print a backtrace into the session.
    std::panic::set_hook(Box::new(|_| {}));
    let home = ratchet_home(env);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut text = String::new();
        stdin.read_to_string(&mut text).map_err(|e| format!("stdin: {e}"))?;
        let payload = dispatch::parse_payload(&text)?;
        dispatch::dispatch(event, payload, env, process_cwd, &home)
    }));
    match result {
        Ok(Ok(code)) => code,
        Ok(Err(msg)) => {
            log::append(&home, &format!("hook {event}: {msg}"));
            0
        }
        Err(_) => {
            log::append(&home, &format!("hook {event}: panic"));
            0
        }
    }
}
```

- [ ] **Step 6: Run unit tests, then the scenario tests**

Run: `cargo test -p ratchet --lib dispatch`
Expected: 4 PASS.
Run: `cargo test -p ratchet --test spec`
Expected: every `agent_protocol__*` test PASSES except `list_shows_built_ins_and_disabled_state` and `dry_run_reproduces_the_block` (Task 10). If `write_to_the_main_tree_blocked_from_a_worktree_session` fails, check `repo::main_root_of` against the `.git` file that `git worktree add` wrote in the sandbox (`gitdir: <abs>/.git/worktrees/wt`).

- [ ] **Step 7: Hand off (no git)**

`cargo fmt`, `cargo clippy --all-targets -- -D warnings` clean. List files.

---

### Task 10: `ratchet guardrails list|test`

**Files:**
- Modify: `crates/ratchet/src/guardrails/cli.rs` (replace placeholder)

**Interfaces:**
- Consumes: `repo::find_repo`, `rules::load_rule_set`, `eval::*`, `config::ratchet_home`.
- Produces: `cli::list(env, cwd) -> i32` prints one line per rule: `<id>  <kind>  <tools joined by ,>  <source>  [off]`, then a line `repo: <main_root>` or `repo: (none - no ratchet.toml above cwd; hooks are no-ops here)`; config errors print `error: <path>: <message>` and exit 1. `cli::test(tool, payload_json, env, cwd) -> i32` builds the tool call from `tool` and the JSON payload as `tool_input`, evaluates like `pre-tool`, prints the render line on stdout, exits 2 on block, 0 on allow, 1 on bad JSON.

- [ ] **Step 1: Write the failing test** — the two scenario tests from Task 3 (`list_shows_built_ins_and_disabled_state`, `dry_run_reproduces_the_block`) are the tests. Run: `cargo test -p ratchet --test spec list dry_run` → FAIL.

- [ ] **Step 2: Implement**

```rust
//! `ratchet guardrails list|test`: see and dry-run what a hook would do here.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::config::ratchet_home;
use crate::guardrails::eval::{evaluate, scratchpad_from_env, GuardContext};
use crate::guardrails::rules::load_rule_set;
use crate::repo::{find_repo, has_venv, Repo};

fn resolve(env: &HashMap<String, String>, cwd: Option<PathBuf>) -> Result<(PathBuf, Option<Repo>, PathBuf), String> {
    let cwd = cwd.ok_or("no cwd")?;
    let home = ratchet_home(env);
    let repo = find_repo(&cwd).map_err(|e| e.to_string())?;
    Ok((cwd, repo, home))
}

pub fn list(env: &HashMap<String, String>, cwd: Option<PathBuf>) -> i32 {
    let (_cwd, repo, home) = match resolve(env, cwd) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let set = match load_rule_set(&home, repo.as_ref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    for r in &set.rules {
        let off = if set.is_off(&r.id) { "  off" } else { "" };
        println!("{:<18} {:<10} {:<40} {}{}", r.id, r.kind.as_str(), r.tools.join(","), r.source, off);
    }
    match repo {
        Some(r) => println!("repo: {}", r.main_root.display()),
        None => println!("repo: (none - no ratchet.toml above cwd; hooks are no-ops here)"),
    }
    0
}

pub fn test(tool: &str, payload: &str, env: &HashMap<String, String>, cwd: Option<PathBuf>) -> i32 {
    let tool_input: Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: payload is not JSON: {e}");
            return 1;
        }
    };
    let (cwd, repo, home) = match resolve(env, cwd) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let set = match load_rule_set(&home, repo.as_ref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if repo.is_none() {
        println!("allowed (no ratchet.toml above cwd; hooks are no-ops here)");
        return 0;
    }
    let ctx = GuardContext {
        main_root: repo.as_ref().map(|r| r.main_root.clone()),
        worktrees_dir: repo.as_ref().map(|r| r.worktrees_dir.clone()),
        has_venv: repo.as_ref().map(|r| has_venv(&cwd, &r.main_root)).unwrap_or(false),
        cwd,
        scratchpad: scratchpad_from_env(env),
    };
    let rules: Vec<_> = set.active().collect();
    match evaluate(&rules, tool, &tool_input, &ctx) {
        Some(v) => {
            println!("{}", v.render());
            2
        }
        None => {
            println!("allowed");
            0
        }
    }
}
```

- [ ] **Step 3: Run the whole suite**

Run: `cargo test -p ratchet`
Expected: everything PASS, including `scenarios::every_scenario_has_a_test` and all 21 `agent_protocol__*`.

- [ ] **Step 4: Hand off (no git)**

---

### Task 11: `run-hook.cmd` wrapper, README install section, latency report

**Files:**
- Create: `hooks/run-hook.cmd`, `crates/ratchet/tests/latency.rs`
- Modify: `README.md`

**Interfaces:**
- Produces: the wrapper contract. Resolution order: `$RATCHET_BIN` if set and executable; else `<plugin root>/bin/ratchet` or `bin/ratchet.exe`; else one stderr line and exit 0. Stdin passes through untouched (`exec`).

- [ ] **Step 1: Write the wrapper**

`hooks/run-hook.cmd` (polyglot: cmd.exe runs the batch block; bash skips it via the here-doc no-op):
```
: << 'CMDBLOCK'
@echo off
REM Cross-platform wrapper. Claude Code invokes this with shell=bash on every platform;
REM cmd.exe only reaches the batch part if someone runs it by hand on Windows.
set "HOOK_DIR=%~dp0"
if exist "C:\Program Files\Git\bin\bash.exe" (
    "C:\Program Files\Git\bin\bash.exe" "%HOOK_DIR%run-hook.cmd" %*
    exit /b %ERRORLEVEL%
)
where bash >nul 2>nul
if %ERRORLEVEL% equ 0 (
    bash "%HOOK_DIR%run-hook.cmd" %*
    exit /b %ERRORLEVEL%
)
exit /b 0
CMDBLOCK

# bash from here on. Find the binary and exec it with the same args and stdin.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [ -n "$RATCHET_BIN" ] && [ -x "$RATCHET_BIN" ]; then
    exec "$RATCHET_BIN" "$@"
fi
for candidate in "$ROOT/bin/ratchet" "$ROOT/bin/ratchet.exe"; do
    if [ -x "$candidate" ]; then
        exec "$candidate" "$@"
    fi
done
echo "[ratchet] binary not found. Build it (cargo build --release) and copy target/release/ratchet[.exe] into $ROOT/bin/, or set RATCHET_BIN." >&2
exit 0
```

- [ ] **Step 2: Write the latency test**

`crates/ratchet/tests/latency.rs`:
```rust
//! Reports pre-tool latency and fails above the ceiling. Release numbers are what the README
//! quotes; run `cargo test --release --test latency -- --nocapture` to see them.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;

#[test]
fn pre_tool_median_under_ceiling() {
    let home = tempfile::TempDir::new().unwrap();
    let repo = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(repo.path().join(".venv")).unwrap();
    std::fs::write(repo.path().join("ratchet.toml"), "").unwrap();
    let payload = format!(
        r#"{{"tool_name":"Bash","tool_input":{{"command":"python x.py"}},"cwd":{}}}"#,
        serde_json::to_string(&repo.path().to_string_lossy()).unwrap()
    );
    let bin = assert_cmd::cargo::cargo_bin("ratchet");
    let mut times = Vec::new();
    for _ in 0..30 {
        let t = Instant::now();
        let mut child = Command::new(&bin)
            .args(["hook", "pre-tool"])
            .current_dir(repo.path())
            .env("RATCHET_HOME", home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
        let st = child.wait().unwrap();
        times.push(t.elapsed().as_micros());
        assert_eq!(st.code(), Some(2));
    }
    times.sort();
    let median_ms = times[times.len() / 2] as f64 / 1000.0;
    let p90_ms = times[times.len() * 9 / 10] as f64 / 1000.0;
    let build = if cfg!(debug_assertions) { "debug" } else { "release" };
    eprintln!("pre-tool latency: median {median_ms:.1} ms, p90 {p90_ms:.1} ms ({build})");
    let ceiling = if cfg!(debug_assertions) { 200.0 } else { 60.0 };
    assert!(median_ms < ceiling, "median {median_ms:.1} ms over {ceiling} ms");
}
```

- [ ] **Step 3: Run both builds**

Run: `cargo test -p ratchet --test latency -- --nocapture`
Expected: PASS, prints the debug numbers.
Run: `cargo test -p ratchet --release --test latency -- --nocapture`
Expected: PASS, median well under 60 ms on Windows. Record both numbers for the README.

- [ ] **Step 4: Extend the README**

Append to `README.md`:
```markdown
## Install (group 0, from source)

1. `cargo build --release`
2. Copy `target/release/ratchet` (Windows: `ratchet.exe`) into `bin/` of this plugin directory,
   or export `RATCHET_BIN=<path>`.
3. Install the plugin in Claude Code from this directory (marketplace entry or
   `claude plugin add <path>`), then restart the session.
4. In a repo you want governed: create `ratchet.toml` at its root (see above).
5. Check: `ratchet guardrails list` from that repo, and
   `ratchet guardrails test Bash '{"command":"python x.py"}'` should exit 2 when the repo has a `.venv`.

Measured `pre-tool` latency on Windows 11 (release build): median <fill from Step 3> ms, p90 <fill> ms.
The same check in the Python-based harness this replaces cost ~830 ms.

## Guardrails

Built-in: `python-venv`, `git-destructive`, `env-files`, `main-tree`. Disable per repo with
`[guardrails] off = ["id"]`. Add your own with the same schema, machine-wide in
`~/.ratchet/config.toml` → `[guardrails] extra = "guardrails.toml"`, or per repo in
`ratchet.toml` → `[guardrails] extra = "ratchet/guardrails.toml"`. A rule with an existing id
replaces it. Example of a custom `content` rule that keeps a database read-only (use the
write-method names of your own driver in the pattern):

    [[rules]]
    id = "db-readonly"
    tools = ["Bash", "PowerShell", "Edit", "Write", "NotebookEdit", "MultiEdit"]
    kind = "content"
    pattern = '\.(write_rows|purge_all)\s*\('
    message = "The database is read-only for agents."
    alternative = "Read through the data layer; if a write is really needed, the owner does it by hand."

Known behaviour, by design: command rules split on `;` only outside quotes, so a quoted
`"done; mypy clean"` does not trip `python-venv`; but a `content` rule scans what will be
written, so quoting a blocked pattern in documentation blocks that write too.

## Not here (yet)

Task board, session registry, briefing and handoff rule (groups 1-2), `ratchet fetch`
(group 3), agent profiles and skills (group 4), release binaries and bootstrap (group 5).
```
Replace the two `<fill>` placeholders with the numbers from Step 3 before handing off.

- [ ] **Step 5: Manual smoke on the owner's machine (owner or orchestrator, not the implementer)**

In PowerShell, from a temporary repo with `ratchet.toml` and `.venv`:
```
$env:RATCHET_BIN = "C:\repos\ratchet\target\release\ratchet.exe"
'{"tool_name":"Bash","tool_input":{"command":"python x.py"},"cwd":"<that repo>"}' | bash C:\repos\ratchet\hooks\run-hook.cmd hook pre-tool; echo "exit $LASTEXITCODE"
```
Expected: `[ratchet guardrail:python-venv] …` on stderr and `exit 2`. From `C:\repos\ops` (no marker): `exit 0`, nothing printed.

- [ ] **Step 6: Hand off (no git)**

---

### Task 12: Group review (reviewer, read-only)

**Files:** none modified.

- [ ] **Step 1:** Run the gate from `C:\repos\ratchet`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p ratchet`, `cargo test -p ratchet --release --test latency -- --nocapture`. All green, numbers recorded.
- [ ] **Step 2:** Contrast against `openspec/specs/agent-protocol/spec.md` and this plan: every scenario has a test, every built-in rule's message contains its alternative, `pre-tool` opens no database and spawns nothing except `git ls-files` on main-tree writes (grep `Command::new` in `src/`: only `repo::is_tracked`).
- [ ] **Step 3:** Adversarial probes with the real binary: a `Write` whose `content` is 1 MB (must answer under the ceiling and not panic); a `ratchet.toml` with an unknown key (hook exits 0, log names the key); `rm -rf` with a target that starts with the scratchpad path but is a sibling (`<scratchpad>2/x` must be blocked); a `.git` file with a relative `gitdir:`.
- [ ] **Step 4:** Verdict as a note: APPROVED, or BLOCKING items with `file:line`. A blocking item is described, not fixed.

---

## Self-review against the spec

- **Spec coverage (group 0):** D-runtime (Task 1 crate), D-regex (Task 6 validates with `fancy-regex`, Task 7 uses it), D-marker (Task 4 `find_repo`, Task 9 no-marker exit 0), D-state (`ratchet_home`, `log.rs`), D-p3 (Task 9 `run`), D-english (all content), D-specs-first (Tasks 2-3, `scenarios.rs`), D-roles (no git; author/implementer/reviewer split across Tasks 3, 4-11, 12), §4.2 hook table for `pre-tool` and matcher with PowerShell (Task 1 `hooks.json`), §4.3 marker schema (Task 4), §4.4 rules and extension (Tasks 6-7, README), §6 error handling for missing binary / invalid marker / malformed payload (Tasks 9, 11), §7 latency test (Tasks 3, 11), §8 group 0 deliverable (Task 11 smoke).
- **Deferred on purpose:** bootstrap download (group 5), `ratchet config init` (group 1, needs the CLI scaffolding for state), the six non-pre-tool hooks (groups 1-2; registered now, no-ops by design).
- **Type consistency checked:** `GuardContext` fields identical in Tasks 7, 8, 9, 10; `evaluate` takes `&[&Rule]` everywhere; `RuleSet::active()` returns a `&Rule` iterator consumed via `collect::<Vec<_>>()`; `ConfigError` has `path` + `message` used by tests in Tasks 4 and 6; `Repo` fields `main_root`, `checkout_root`, `worktrees_dir`, `config` used consistently; `hooks::BLOCK` referenced from `dispatch` as `super::BLOCK`; `Kind::as_str` used by `rules.rs` and `cli.rs`.
