# ratchet — Group 7: `ratchet usage` (token cost per task, role and model)

> **Assumptions (2026-09-22, plan author, awaiting owner confirmation):**
> 1. **`subagent.start`/`subagent.stop` shape — SETTLED 2026-09-22 (T-0006 merged).** The
>    event row has `kind = 'subagent.start'` (or `'subagent.stop'`), `session_id` = the parent
>    session, the row's own `task_id` column set to the first task that session holds (NULL when
>    it holds none), and a `payload` JSON object with string fields `agent_id`, `agent_type`,
>    `description`, `transcript_path` (`exit_status` additionally on stops) — and NO `task_id` in
>    the payload. `usage::attribute` reads the task from the row's `task_id` column;
>    `SubagentEvent.task` is filled from the row, never parsed from the payload. Task 1's fixture
>    inserts the row directly with exactly this shape.
> 2. **"Touched", for the default/`--since` listing (Requirement 8), means the timestamp of the
>    task's most recent event** (any kind — `events::for_task(conn, id, 1)`), not
>    `tasks.updated_at` (which `note`/`handoff` do not bump — see `services/tasks.rs::note`). This
>    reuses an existing read path with no new query. If the owner intended session activity or
>    something else, only Task 4's `touched_at` helper changes.
> 3. **`review` does not close a holding window — SETTLED 2026-09-22 (owner).** A hold opens on
>    `task.claimed` and closes only on a `task.status` event to `done`, `blocked` or `ready`, or
>    at session end. Board evidence: fix rounds after a CHANGES NEEDED verdict happen with the
>    task still in `review`, with no reclaim and no status change, so the earlier literal rule
>    (close on anything leaving `in_progress`) sent exactly those tokens to `unassigned`. A bare
>    `status <id> in_progress` is a no-op for holds (the hold never closed); a fresh claim while
>    already held opens a second hold on the same task, harmless under "most recently claimed
>    wins". Requirement 3 of `openspec/specs/usage/spec.md` and D-usage-join were amended to say
>    this; scenario "A fix round after a review verdict still belongs to the task" covers it.
> 4. **A task-level aggregate `cost` (the bare listing's per-task row, and `--by task/session`)
>    is shown only when every bucket folded into that row has a matching weight**; if any one
>    bucket's model matches no configured prefix, the aggregate shows no `cost` at all rather than
>    a partial sum. Requirement 7's scenarios only cover a single-model case; this rule is this
>    plan's extension to the multi-model aggregate case.
> 5. **Scenario "A general-purpose subagent shows its description" (openspec/specs/usage/spec.md,
>    line 95) is off by one character from its own rule.** The requirement text says "the first 40
>    characters"; the scenario's own example description, first-40-characters-clipped, is 41
>    characters (`find every caller of parse_repo_config in` — counted by hand, see the report
>    handed back for this plan). Task 1's test does not reproduce that exact example string; it
>    picks its own description sized so the 40-character boundary is unambiguous, and implements
>    the literal rule (`.chars().take(40)`). Flagged for the owner to fix the spec's example, not
>    treated as authoritative over the rule's own sentence.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `ratchet usage`: reads the session transcripts Claude Code already writes to
`~/.claude/projects`, joins them with the sessions and task events ratchet already records in its
own database, and reports token cost per task, per role (orchestrator vs. subagent) and per model
— with an optional cost column from owner-configured weights, `--by`/`--since`/`--all-repos`
aggregation, `--json`, and `--note` to paste the one-task summary onto the board. Read-only except
for the explicit `--note`; no network; no new DB tables; git never consulted.

**Architecture:** Three pure modules under `crates/ratchet/src/usage/` — `transcript` (parses one
JSONL file's bytes into calls, tolerant of anything it doesn't understand), `weights` (matches a
model to a configured weight by longest prefix and turns four token counts into a cost), and
`attribute` (joins parsed calls with the sessions/events ratchet already has, and derives every
number Requirement 3 through 8 define — holding windows, orientation, rounds, buckets) — plus a
thin `crates/ratchet/src/cli/usage_cmd.rs` face that resolves the repo, loads sessions and events
through the existing `services::{sessions,tasks,events}` read paths, locates and reads transcript
files from disk, calls the pure modules, and prints through `output.rs`. Same split as `pdf.rs` /
`cli/pdf_cmd.rs` (pure module, thin face) and the same "services own every DB read" convention
`cli/task_cmd.rs` already follows.

**Tech Stack:** Rust 2021 (rust-version 1.79), existing deps only: `serde`/`serde_json` (transcript
and meta JSON), `chrono` (timestamps), `toml` (already used by `config.rs` for
`[usage.weights."<prefix>"]`), `clap` (the new `usage` subcommand). No new crate anywhere in this
group.

**Spec:** `docs/superpowers/specs/2026-09-21-ratchet-usage-design.md` (the whole document; most
directly §2 the facts this design rests on, §3 decisions D-usage-transcripts through
D-usage-read-only, §4 commands and column format, §5 components, §6 error handling, §7 testing, §8
delivery plan) and `openspec/specs/usage/spec.md` (10 requirements, 27 scenarios — already written
and merged; this plan does not create it, only tests against it). Also extends
`docs/superpowers/specs/2026-09-16-ratchet-plugin-design.md` for pre-existing conventions this plan
reuses (D-english, D-specs-first, D-roles).

## Global Constraints

- **Work in the worktree `/Users/eduardoillanes/Documents/ratchet/.worktrees/t-0003`, branch
  `usage-spec`, which already exists.** Never touch the main checkout at
  `/Users/eduardoillanes/Documents/ratchet` — the `main-tree` guardrail blocks tracked-file edits
  there, and unlike group 6's `ratchet map` this feature has no reason to ever run against the
  main tree specifically, so there is no Task-6-style exception here. Commit per task, on top of
  whatever `HEAD` is when work starts. Never force-push. Never commit to `main` directly.
- Every shell that runs cargo starts with:
  ```bash
  export PATH="$HOME/.cargo/bin:$PATH"
  cd /Users/eduardoillanes/Documents/ratchet/.worktrees/t-0003
  ```
- The crate is **bin-only**: unit tests run with `cargo test -p ratchet --bin ratchet`. Never
  `--lib`; this plan adds no `[lib]` target. Scenario tests run with
  `cargo test -p ratchet --test spec`.
- Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p ratchet`.
  **The full gate is expected to be RED from the end of Task 1 through the end of Task 3.** Task 1
  writes all 27 `usage__*` scenario tests against the *existing* `openspec/specs/usage/spec.md`
  before any of `Cmd::Usage` exists in `main.rs`; every one of them fails with clap's "unrecognized
  subcommand" (exit 2) until Task 4 wires the CLI, exactly as group 3's pdf plan and group 6's map
  plan did it. Tasks 2 and 3 add pure modules with their own unit tests and their own green
  criterion (`cargo test -p ratchet --bin ratchet usage::`); they do not move any scenario test.
  Task 4 turns scenarios 1–23 and 27 green (everything except the three `--note` scenarios); Task 5 turns
  24–26 green and the full gate clean. Task 6 re-runs everything to confirm, not to finish it.
  Nobody weakens a test to turn the gate green early.
- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are **not** expected to go
  red at any point. If clippy complains about a `pub` item nothing calls yet, add
  `#[allow(dead_code)]` item-by-item with a comment naming which later task reaches it (the pattern
  already used throughout `config.rs`/`pdf.rs`/`repo.rs`), removed once the item is reachable.
- All content, identifiers, messages and docs in English (spec D-english).
- **The clock seam.** Every timestamp a face reads comes from `clock::now(env)`
  (`RATCHET_NOW` in tests), never `Utc::now()` directly. The pure modules (`usage::transcript`,
  `usage::attribute`, `usage::weights`) never read the clock at all — `now` is always a parameter
  where one is needed (e.g. `attribute::since_cutoff`).
- **Git is not consulted anywhere in this group.** No `git -C … ` shell-out, unlike
  `ratchet map`'s use of `repo::git_branch`/`is_tracked`.
- **No new DB tables, no new columns, no new migration.** Every read goes through
  `services::{sessions, tasks, events}` as they exist today, reused with an unbounded `LIMIT`
  (`i64::MAX`) where a full history is needed instead of the small `limit` those functions were
  written for — see Task 4. `--note` writes through the existing `services::tasks::note`, exactly
  like `ratchet task note`.
- **`usage::transcript`, `usage::attribute` and `usage::weights` are pure**: no `std::fs`, no
  `std::process`, no `rusqlite::Connection`, no `crate::clock::now`. They take bytes/rows already
  read and return data; `cli/usage_cmd.rs` (and, in Task 1, the test fixture) is the only place
  that touches a filesystem path under `~/.claude/projects` or `RATCHET_CLAUDE_PROJECTS`.
- **Numbers**: every token count in this group is abbreviated with `usage::attribute::abbreviate`
  (`k`/`M`, one decimal, defined in Task 3) before it reaches a terminal; `--json` always prints
  the raw `u64`.
- **Long output follows `output.rs`**: over 60 lines goes to a file under `<home>/out/`, the
  terminal gets the head — see `crate::output::{emit, emit_json, MAX_STDOUT_LINES, HEAD_LINES}`,
  already used by `cli/task_cmd.rs` and reused as-is here, not reimplemented.

---

## File structure

```
openspec/specs/usage/spec.md            already exists — 10 requirements, 27 scenarios (not this plan's job)
crates/ratchet/tests/spec/
├── main.rs                             Task 1 — + `mod usage;` (alphabetical, after `tasks`)
├── support.rs                          Task 1 — append only: TranscriptBuilder, Usage, ToolUse,
│                                        seed_subagent_event(), usage() cli helper
└── usage.rs                            Task 1 — 27 scenario tests
crates/ratchet/src/
├── main.rs                             Task 4 (+ Cmd::Usage, dispatch); Task 2 adds `mod usage;`
├── config.rs                           Task 2 — + ModelWeights, UsageSettings, MachineConfig.usage
├── usage/
│   ├── mod.rs                          Task 2 (`pub mod transcript; pub mod weights;`), extended
│   │                                    by Task 3 (`pub mod attribute;`)
│   ├── transcript.rs                   Task 2
│   ├── weights.rs                      Task 2
│   └── attribute.rs                    Task 3
└── cli/
    ├── mod.rs                          Task 4 — + `pub mod usage_cmd;`
    └── usage_cmd.rs                    Task 4 (report, --by, --since, --all-repos, --json),
                                         extended by Task 5 (--note)
README.md                               Task 5 — new "## Usage" section
docs/agent-doctrine.md                  Task 5 — one new line
```

No parallelism between Tasks 2–5: each extends what the previous task created, strictly
sequential, same branch. Task 1 (spec-test-author) and Task 6 (reviewer) bookend it and touch no
production file.

---

## Scenario → task allocation

Every scenario is written once, in Task 1, against the existing `openspec/specs/usage/spec.md`.
This table says which task is expected to turn each one green.

| # | Scenario (test function name, prefix `usage__`) | Turns green in |
|---|---|---|
| 1 | `a_worktree_session_is_found_under_its_own_slug` | Task 4 |
| 2 | `a_session_without_a_transcript_is_reported_not_an_error` | Task 4 |
| 3 | `missing_projects_directory_fails_naming_the_path` | Task 4 |
| 4 | `garbage_lines_and_a_record_without_usage_are_counted` | Task 4 |
| 5 | `a_transcript_with_nothing_understood_is_skipped_once` | Task 4 |
| 6 | `one_task_one_session_orchestrator_only` | Task 4 |
| 7 | `two_tasks_held_in_sequence_split_the_session` | Task 4 |
| 8 | `calls_outside_any_held_task_are_unassigned` | Task 4 |
| 27 | `a_fix_round_after_a_review_verdict_still_belongs_to_the_task` | Task 4 |
| 9 | `a_subagent_is_attributed_through_its_start_event` | Task 4 |
| 10 | `a_subagent_is_attributed_through_tooluseid_when_no_event_exists` | Task 4 |
| 11 | `a_subagent_is_attributed_by_first_timestamp_when_both_are_missing` | Task 4 |
| 12 | `a_general_purpose_subagent_shows_its_description` | Task 4 |
| 13 | `calls_before_the_first_claim_are_orientation` | Task 4 |
| 14 | `a_dispatch_ends_orientation_without_a_claim` | Task 4 |
| 15 | `two_review_rounds_and_tokens_per_round` | Task 4 |
| 16 | `weights_present_add_cost` | Task 4 |
| 17 | `no_weights_no_cost` | Task 4 |
| 18 | `the_longest_matching_prefix_wins` | Task 4 |
| 19 | `default_listing_shows_tasks_of_the_last_seven_days` | Task 4 |
| 20 | `since_widens_the_window` | Task 4 |
| 21 | `by_role_aggregates_across_tasks` | Task 4 |
| 22 | `numbers_are_abbreviated` | Task 4 |
| 23 | `json_shape` | Task 4 |
| 24 | `note_appends_the_summary` | Task 5 |
| 25 | `note_on_a_missing_task_is_refused` | Task 5 |
| 26 | `without_note_nothing_is_written` | Task 5 |

Task 4 turns 23 green (1–23); Task 5 turns the remaining 3 green (24–26) plus README/doctrine
additions (no test). Task 6 adds no new scenario; it re-runs the full suite and must find all 27
green already, gate clean.

---

### Task 1: scenario tests + transcript fixture builder (spec-test-author)

**Files:**
- Create: `crates/ratchet/tests/spec/usage.rs`
- Modify: `crates/ratchet/tests/spec/main.rs` (one `mod` line), `crates/ratchet/tests/spec/support.rs`
  (append only — every existing helper, including `Sandbox`, `sandbox()`, `board()`, `task()`,
  `new_task()`, `T0`/`at()`, `db()`/`count()`, `cli()`, is untouched and reused, not replaced)

**Interfaces:**
- Consumes: `crate::support::{Sandbox, board, sandbox, task, new_task, T0, at, code, stdout,
  stderr, db, count, out_files, cli}` (all already in the repo — see
  `crates/ratchet/tests/spec/support.rs`).
- Produces: the 27 scenario titles and their exact `fn usage__<slug>()` names (table above — the
  slugs are mechanical: `crates/ratchet/tests/scenarios.rs::slug()` already enforces
  `fn usage__<slug>(` exists for every `#### Scenario:` of `openspec/specs/usage/spec.md`, and
  that check is *currently red* on this branch — `openspec/specs/usage/spec.md` exists but no
  `usage.rs` does — so Task 1 is also what turns `scenarios::every_scenario_has_a_test` green);
  `support::{TranscriptBuilder, Usage, usage, seed_subagent_event}` — the frozen contract every
  later task's CLI and pure-module code is judged against.

The author of this task reads only `openspec/specs/usage/spec.md`, this task, and the Assumptions
block at the top of this plan (which is exactly the information a spec-test-author would otherwise
have no way to know, since it concerns a database shape group T-0006 owns) — not the design
document beyond what is quoted here, and not Tasks 2–6's planned code.

**Stable substrings** (the implementation guarantees these appear, verbatim, in the relevant
report — asserted with `.contains(...)`, never full-line equality, matching every existing spec
test file in this crate): `no transcript` · `skipped ` · `partial ` · `orchestrator` ·
`orientation` · `rounds ` · `unassigned` · `cost` · `review rounds per task` ·
`orientation per session` · the `k`/`M` abbreviation suffixes.

- [ ] **Step 1: Add the module to the test binary**

Edit `crates/ratchet/tests/spec/main.rs`:

```rust
// Scenario test names are `<spec>__<slug>` by convention (checked by tests/scenarios.rs);
// the double underscore trips the snake_case lint, so it is disabled crate-wide here.
#![allow(non_snake_case)]

mod agent_protocol;
mod board;
mod bootstrap;
mod pdf;
mod sessions;
mod support;
mod tasks;
mod usage;
```

- [ ] **Step 2: Append the transcript fixture builder to `support.rs`**

Append to the end of `crates/ratchet/tests/spec/support.rs` (everything above this line in that
file is untouched):

```rust
// --- group 7: usage --------------------------------------------------------------------------
//
// `TranscriptBuilder` writes JSONL transcripts under a temp "projects" root, matching exactly
// the fields design §2 (`docs/superpowers/specs/2026-09-21-ratchet-usage-design.md`) verified on
// real Claude Code output: `type`, `timestamp`, `sessionId`, `cwd`, `gitBranch`, `version`,
// `message.model`, `message.usage.{input_tokens,cache_creation_input_tokens,
// cache_read_input_tokens,output_tokens,output_tokens_details.thinking_tokens}`,
// `message.content` holding `tool_use` blocks with `id`/`name`. Point `RATCHET_CLAUDE_PROJECTS`
// at `.root` (see `usage()` below). Every record uses a fixed `version` ("1.2.3") and
// `gitBranch` ("main") — no scenario in this plan needs either to vary; a scenario that does can
// still reach into `.root` directly and write its own record.

/// Four token classes plus optional thinking, in the units the fixture's callers already think
/// in (plain `u64`, not yet abbreviated — that only happens on the way to a terminal).
pub struct Usage {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub thinking: Option<u64>,
}

pub fn usage(input: u64, cache_write: u64, cache_read: u64, output: u64) -> Usage {
    Usage {
        input,
        cache_write,
        cache_read,
        output,
        thinking: None,
    }
}

pub struct TranscriptBuilder {
    pub root: TempDir,
}

impl TranscriptBuilder {
    pub fn new() -> Self {
        TranscriptBuilder {
            root: TempDir::new().unwrap(),
        }
    }

    fn slug(cwd: &Path) -> String {
        cwd.to_string_lossy().replace(['/', '\\'], "-")
    }

    fn transcript_path(&self, cwd: &Path, session_id: &str) -> PathBuf {
        self.root
            .path()
            .join(Self::slug(cwd))
            .join(format!("{session_id}.jsonl"))
    }

    fn subagents_dir(&self, cwd: &Path, session_id: &str) -> PathBuf {
        self.root
            .path()
            .join(Self::slug(cwd))
            .join(session_id)
            .join("subagents")
    }

    fn append(path: &Path, record: &Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(f, "{record}").unwrap();
    }

    fn append_raw(path: &Path, line: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(f, "{line}").unwrap();
    }

    fn record(ts: &str, session_id: &str, cwd: &Path, model: &str, u: &Usage, content: Value) -> Value {
        let mut message = json!({ "model": model, "content": content });
        message["usage"] = json!({
            "input_tokens": u.input,
            "cache_creation_input_tokens": u.cache_write,
            "cache_read_input_tokens": u.cache_read,
            "output_tokens": u.output,
        });
        if let Some(t) = u.thinking {
            message["usage"]["output_tokens_details"] = json!({ "thinking_tokens": t });
        }
        json!({
            "type": "assistant",
            "timestamp": ts,
            "sessionId": session_id,
            "cwd": cwd.to_string_lossy(),
            "gitBranch": "main",
            "version": "1.2.3",
            "message": message,
        })
    }

    /// One assistant call, no `tool_use` blocks.
    pub fn call(&self, cwd: &Path, session_id: &str, ts: &str, model: &str, u: &Usage) -> &Self {
        let rec = Self::record(ts, session_id, cwd, model, u, json!([]));
        Self::append(&self.transcript_path(cwd, session_id), &rec);
        self
    }

    /// One assistant call whose content includes one `tool_use` block — used for an `Agent`
    /// dispatch (ends orientation, and its `id` is what a subagent's `toolUseId` matches) and for
    /// `Edit`/`Write`/`NotebookEdit` (also ends orientation, no `toolUseId` match needed).
    pub fn call_with_tool(
        &self,
        cwd: &Path,
        session_id: &str,
        ts: &str,
        model: &str,
        u: &Usage,
        tool_name: &str,
        tool_use_id: &str,
    ) -> &Self {
        let content = json!([{ "type": "tool_use", "id": tool_use_id, "name": tool_name }]);
        let rec = Self::record(ts, session_id, cwd, model, u, content);
        Self::append(&self.transcript_path(cwd, session_id), &rec);
        self
    }

    /// An assistant record with no `message.usage` key at all — R2's "partial" case.
    pub fn call_no_usage(&self, cwd: &Path, session_id: &str, ts: &str, model: &str) -> &Self {
        let rec = json!({
            "type": "assistant",
            "timestamp": ts,
            "sessionId": session_id,
            "cwd": cwd.to_string_lossy(),
            "gitBranch": "main",
            "version": "1.2.3",
            "message": { "model": model, "content": [] },
        });
        Self::append(&self.transcript_path(cwd, session_id), &rec);
        self
    }

    /// A line that is not valid JSON — R2's "skipped" case.
    pub fn garbage(&self, cwd: &Path, session_id: &str) -> &Self {
        Self::append_raw(&self.transcript_path(cwd, session_id), "not json at all {{{");
        self
    }

    /// A well-formed record whose `type` is not `"assistant"` — ignored, never counted anywhere.
    pub fn non_assistant(&self, cwd: &Path, session_id: &str) -> &Self {
        let rec = json!({ "type": "user", "timestamp": "2026-01-01T00:00:00Z" });
        Self::append(&self.transcript_path(cwd, session_id), &rec);
        self
    }

    /// One call in a subagent's own transcript,
    /// `<slug>/<session_id>/subagents/agent-<agent_id>.jsonl`.
    pub fn subagent(
        &self,
        cwd: &Path,
        session_id: &str,
        agent_id: &str,
        ts: &str,
        model: &str,
        u: &Usage,
    ) -> &Self {
        let rec = Self::record(ts, session_id, cwd, model, u, json!([]));
        let path = self
            .subagents_dir(cwd, session_id)
            .join(format!("agent-{agent_id}.jsonl"));
        Self::append(&path, &rec);
        self
    }

    /// `agent-<agent_id>.meta.json` next to that subagent transcript.
    pub fn meta(
        &self,
        cwd: &Path,
        session_id: &str,
        agent_id: &str,
        agent_type: &str,
        description: &str,
        model: &str,
        tool_use_id: Option<&str>,
    ) -> &Self {
        let mut m = json!({ "agentType": agent_type, "description": description, "model": model });
        if let Some(id) = tool_use_id {
            m["toolUseId"] = json!(id);
        }
        let path = self
            .subagents_dir(cwd, session_id)
            .join(format!("agent-{agent_id}.meta.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, m.to_string()).unwrap();
        self
    }
}

/// Run `ratchet usage <args>` against the sandbox, with `RATCHET_CLAUDE_PROJECTS` pointed at the
/// fixture builder's root. Every `usage__*` scenario test goes through this helper.
pub fn usage(sb: &Sandbox, tb: &TranscriptBuilder, args: &[&str], cwd: &Path, extra: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(ratchet_bin());
    cmd.arg("usage")
        .args(args)
        .current_dir(cwd)
        .env("RATCHET_HOME", sb.home.path())
        .env("RATCHET_CLAUDE_PROJECTS", tb.root.path())
        .env_remove("RATCHET_SESSION_ID")
        .env_remove("RATCHET_NOW")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

/// Inserts a `subagent.start`/`subagent.stop` row directly, bypassing the CLI — the exact shape
/// group T-0006 ships (see Assumption 1): the row's `task_id` column carries the task; the
/// payload carries `agent_id`, `agent_type`, `description` only.
pub fn seed_subagent_event(
    sb: &Sandbox,
    kind: &str,
    session_id: &str,
    agent_id: &str,
    agent_type: &str,
    description: &str,
    task_id: &str,
    ts: &str,
) {
    let conn = db(sb);
    conn.execute(
        "INSERT INTO events(ts, session_id, task_id, kind, payload, source) VALUES (?1,?2,?3,?4,?5,'hook')",
        rusqlite::params![
            ts,
            session_id,
            task_id,
            kind,
            json!({
                "agent_id": agent_id,
                "agent_type": agent_type,
                "description": description,
            })
            .to_string(),
        ],
    )
    .unwrap();
}
```

This needs one new import at the top of `support.rs`: add `std::process::Output` if not already
imported (it already is, via `use std::process::{Command, Output, Stdio};` at the top of the
file) — no import changes are needed; `TempDir`, `Value`/`json!`, `fs`, `Path`/`PathBuf`,
`Command`, `Write` are all already imported at the top of `support.rs` for the existing helpers.

- [ ] **Step 3: Run `cargo build` to confirm `support.rs` compiles on its own**

Run: `cargo test -p ratchet --test spec --no-run`
Expected: compiles clean (no `usage.rs` exists yet, so nothing calls the new helpers yet — this
step exists only to catch a typo in Step 2 before Step 4 adds 27 call sites at once).

- [ ] **Step 4: Write `crates/ratchet/tests/spec/usage.rs`**

```rust
//! One test per `#### Scenario` of openspec/specs/usage/spec.md, named by slug. `RATCHET_HOME`
//! isolates the database (as every other spec test file does); `RATCHET_CLAUDE_PROJECTS` (set by
//! `support::usage`) isolates the transcript tree in the same way.

use crate::support::*;
use serde_json::Value;

// --- Requirement: Transcripts are located per session from its working directory -----------

#[test]
fn usage__a_worktree_session_is_found_under_its_own_slug() {
    let sb = sandbox();
    let wt = worktree(&sb);
    let out = start_session(&sb, "s-1", &wt, T0);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let id = new_task(&sb, "wt task", &[], "s-1", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-1", 2)), 0);

    let tb = TranscriptBuilder::new();
    tb.call(&wt, "s-1", &at(3), "claude-sonnet-5", &usage(100, 0, 0, 50));

    let out = usage(&sb, &tb, &[], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!stdout(&out).contains("no transcript"), "{}", stdout(&out));
    assert!(stdout(&out).contains(&id), "{}", stdout(&out));

    let detail = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&detail), 0, "{}", stderr(&detail));
    assert!(stdout(&detail).contains("orchestrator"), "{}", stdout(&detail));
    assert!(!stdout(&detail).contains("no transcript"), "{}", stdout(&detail));
}

#[test]
fn usage__a_session_without_a_transcript_is_reported_not_an_error() {
    let sb = board("s-2");
    let id = new_task(&sb, "no transcript task", &[], "s-2", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-2", 2)), 0);
    let tb = TranscriptBuilder::new(); // nothing ever written for s-2
    let out = usage(&sb, &tb, &[], &sb.root(), &[("RATCHET_NOW", &at(3))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("no transcript"), "{}", stdout(&out));
    assert!(stdout(&out).contains("s-2"), "{}", stdout(&out));
}

#[test]
fn usage__missing_projects_directory_fails_naming_the_path() {
    let sb = board("s-3");
    let missing = sb.scratchpad.path().join("does-not-exist");
    let out = cli(
        &sb,
        &["usage"],
        &sb.root(),
        &[("RATCHET_CLAUDE_PROJECTS", &missing.to_string_lossy())],
    );
    assert_eq!(code(&out), 1, "{}", stdout(&out));
    assert!(
        stderr(&out).contains(&missing.to_string_lossy().to_string()),
        "{}",
        stderr(&out)
    );
}

// --- Requirement: Reading is tolerant and reports what it did not understand ----------------

#[test]
fn usage__garbage_lines_and_a_record_without_usage_are_counted() {
    let sb = board("s-4");
    let id = new_task(&sb, "tolerant", &[], "s-4", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-4", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-4", &at(3), "claude-sonnet-5", &usage(100, 0, 0, 50))
        .garbage(&sb.root(), "s-4")
        .call_no_usage(&sb.root(), "s-4", &at(4), "claude-sonnet-5")
        .call(&sb.root(), "s-4", &at(5), "claude-sonnet-5", &usage(200, 0, 0, 60));

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(6))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("skipped 1"), "{text}");
    assert!(text.contains("partial 1"), "{text}");
    assert!(text.contains("1.2.3"), "{text}"); // the highest version seen
    assert!(text.contains("300"), "{text}"); // 100 + 200, the two understood calls
}

#[test]
fn usage__a_transcript_with_nothing_understood_is_skipped_once() {
    let sb = board("s-5");
    let id = new_task(&sb, "nothing understood", &[], "s-5", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-5", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.non_assistant(&sb.root(), "s-5");
    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(3))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("skipped 1"), "{}", stdout(&out));
}

// --- Requirement: A call belongs to the task its session held at that instant ---------------

#[test]
fn usage__one_task_one_session_orchestrator_only() {
    let sb = board("s-6");
    let id = new_task(&sb, "one task", &[], "s-6", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-6", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-6", &at(3), "claude-sonnet-5", &usage(100, 0, 0, 50))
        .call(&sb.root(), "s-6", &at(4), "claude-sonnet-5", &usage(100, 0, 0, 50))
        .call(&sb.root(), "s-6", &at(5), "claude-sonnet-5", &usage(100, 0, 0, 50));
    assert_eq!(code(&task(&sb, &["status", &id, "review"], "s-6", 6)), 0);

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(7))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("orchestrator"), "{text}");
    assert!(text.contains("300"), "{text}"); // 3 x 100 input, summed
    assert_eq!(
        text.matches("orchestrator").count(),
        1,
        "one row per model, not one per call: {text}"
    );
}

#[test]
fn usage__two_tasks_held_in_sequence_split_the_session() {
    let sb = board("s-7");
    let t1 = new_task(&sb, "first", &[], "s-7", 1);
    let t2 = new_task(&sb, "second", &[], "s-7", 2);
    assert_eq!(code(&task(&sb, &["claim", &t1], "s-7", 3)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-7", &at(4), "claude-sonnet-5", &usage(100, 0, 0, 10))
        .call(&sb.root(), "s-7", &at(5), "claude-sonnet-5", &usage(100, 0, 0, 10));
    assert_eq!(
        code(&task(&sb, &["status", &t1, "done", "--why", "closing for the test"], "s-7", 6)),
        0
    );
    assert_eq!(code(&task(&sb, &["claim", &t2], "s-7", 7)), 0);
    tb.call(&sb.root(), "s-7", &at(8), "claude-sonnet-5", &usage(50, 0, 0, 5));

    let d1 = usage(&sb, &tb, &[&t1], &sb.root(), &[("RATCHET_NOW", &at(9))]);
    assert_eq!(code(&d1), 0, "{}", stderr(&d1));
    assert!(stdout(&d1).contains("200"), "{}", stdout(&d1)); // 100 + 100
    assert!(!stdout(&d1).contains("250"), "{}", stdout(&d1)); // never the sum of both tasks

    let d2 = usage(&sb, &tb, &[&t2], &sb.root(), &[("RATCHET_NOW", &at(9))]);
    assert_eq!(code(&d2), 0, "{}", stderr(&d2));
    assert!(stdout(&d2).contains("50"), "{}", stdout(&d2));
    assert!(!stdout(&d2).contains("150"), "{}", stdout(&d2));
}

#[test]
fn usage__calls_outside_any_held_task_are_unassigned() {
    let sb = board("s-8");
    let id = new_task(&sb, "unassigned test", &[], "s-8", 1);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-8", &at(2), "claude-sonnet-5", &usage(10, 0, 0, 1))
        .call(&sb.root(), "s-8", &at(3), "claude-sonnet-5", &usage(10, 0, 0, 1));
    assert_eq!(code(&task(&sb, &["claim", &id], "s-8", 4)), 0);
    tb.call(&sb.root(), "s-8", &at(5), "claude-sonnet-5", &usage(100, 0, 0, 10));
    assert_eq!(
        code(&task(&sb, &["status", &id, "done", "--why", "closing"], "s-8", 6)),
        0
    );
    tb.call(&sb.root(), "s-8", &at(7), "claude-sonnet-5", &usage(20, 0, 0, 2));

    let out = usage(&sb, &tb, &["--by", "session"], &sb.root(), &[("RATCHET_NOW", &at(8))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("unassigned"), "{text}");
    assert!(text.contains("40"), "{text}"); // 10 + 10 + 20, outside the hold
}

// --- Requirement: Subagent calls are attributed through ratchet's own events first ----------

#[test]
fn usage__a_subagent_is_attributed_through_its_start_event() {
    let sb = board("s-9");
    let id = new_task(&sb, "subagent via event", &[], "s-9", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-9", 2)), 0);
    seed_subagent_event(&sb, "subagent.start", "s-9", "a1", "ratchet:reviewer", "", &id, &at(3));
    let tb = TranscriptBuilder::new();
    // No agent-a1.meta.json at all -- the event alone must be enough.
    tb.subagent(&sb.root(), "s-9", "a1", &at(3), "claude-opus-4", &usage(500, 0, 0, 80));

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("ratchet:reviewer"), "{}", stdout(&out));
    assert!(stdout(&out).contains("500"), "{}", stdout(&out));
}

#[test]
fn usage__a_subagent_is_attributed_through_tooluseid_when_no_event_exists() {
    let sb = board("s-10");
    let id = new_task(&sb, "subagent via tool use id", &[], "s-10", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-10", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call_with_tool(&sb.root(), "s-10", &at(3), "claude-sonnet-5", &usage(20, 0, 0, 5), "Agent", "tu-9")
        .subagent(&sb.root(), "s-10", "a2", &at(3), "claude-sonnet-5", &usage(300, 0, 0, 40));
    tb.meta(&sb.root(), "s-10", "a2", "ratchet:implementer", "irrelevant", "claude-sonnet-5", Some("tu-9"));
    // No subagent.start/stop event for "a2" at all.

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("ratchet:implementer"), "{}", stdout(&out));
    assert!(stdout(&out).contains("300"), "{}", stdout(&out));
}

#[test]
fn usage__a_subagent_is_attributed_by_first_timestamp_when_both_are_missing() {
    let sb = board("s-11");
    let id = new_task(&sb, "subagent by timestamp", &[], "s-11", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-11", 2)), 0);
    let tb = TranscriptBuilder::new();
    // No parent Agent tool_use, no meta file, no event -- only the subagent transcript exists,
    // and its first record's timestamp falls while the task is held.
    tb.subagent(&sb.root(), "s-11", "a3", &at(3), "claude-sonnet-5", &usage(70, 0, 0, 9));

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("subagent"), "{}", stdout(&out));
    assert!(stdout(&out).contains("70"), "{}", stdout(&out));
}

#[test]
fn usage__a_general_purpose_subagent_shows_its_description() {
    let sb = board("s-12");
    let id = new_task(&sb, "general purpose subagent", &[], "s-12", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-12", 2)), 0);
    let description = "search the whole repository for every remaining caller of the old helper";
    seed_subagent_event(&sb, "subagent.start", "s-12", "a4", "general-purpose", description, &id, &at(3));
    let tb = TranscriptBuilder::new();
    tb.subagent(&sb.root(), "s-12", "a4", &at(3), "claude-sonnet-5", &usage(40, 0, 0, 5));

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    // Computed by the same rule production code uses ("first 40 characters"), not hand-counted —
    // see Assumption 5 at the top of this plan for why the spec's own prose example is not
    // reproduced verbatim here.
    let clipped: String = description.chars().take(40).collect();
    assert!(
        stdout(&out).contains(&format!("general-purpose: {clipped}")),
        "{}",
        stdout(&out)
    );
    assert!(!stdout(&out).contains("caller of the old helper"), "{}", stdout(&out));
}

// --- Requirement: Orientation is the orchestrator's cost before real work starts ------------

#[test]
fn usage__calls_before_the_first_claim_are_orientation() {
    let sb = board("s-13");
    let id = new_task(&sb, "orientation via claim", &[], "s-13", 1);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-13", &at(2), "claude-sonnet-5", &usage(30, 0, 0, 4))
        .call(&sb.root(), "s-13", &at(3), "claude-sonnet-5", &usage(30, 0, 0, 4));
    assert_eq!(code(&task(&sb, &["claim", &id], "s-13", 4)), 0);
    tb.call(&sb.root(), "s-13", &at(5), "claude-sonnet-5", &usage(90, 0, 0, 10))
        .call(&sb.root(), "s-13", &at(6), "claude-sonnet-5", &usage(90, 0, 0, 10))
        .call(&sb.root(), "s-13", &at(7), "claude-sonnet-5", &usage(90, 0, 0, 10));

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(8))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("orientation"), "{text}");
    assert!(text.contains("60"), "{text}"); // 30 + 30, the two calls before the claim
}

#[test]
fn usage__a_dispatch_ends_orientation_without_a_claim() {
    let sb = board("s-14");
    let id = new_task(&sb, "orientation via dispatch", &[], "s-14", 1);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-14", &at(2), "claude-sonnet-5", &usage(25, 0, 0, 3))
        .call_with_tool(&sb.root(), "s-14", &at(3), "claude-sonnet-5", &usage(25, 0, 0, 3), "Agent", "tu-1")
        .call(&sb.root(), "s-14", &at(4), "claude-sonnet-5", &usage(80, 0, 0, 9))
        .call(&sb.root(), "s-14", &at(5), "claude-sonnet-5", &usage(80, 0, 0, 9));
    assert_eq!(code(&task(&sb, &["claim", &id], "s-14", 6)), 0);

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(7))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("orientation"), "{text}");
    assert!(text.contains("50"), "{text}"); // 25 + 25 -- the dispatching call itself is included
}

#[test]
fn usage__a_fix_round_after_a_review_verdict_still_belongs_to_the_task() {
    let sb = board("s-27");
    let id = new_task(&sb, "fix round", &[], "s-27", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-27", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-27", &at(3), "claude-sonnet-5", &usage(10, 0, 0, 1));
    assert_eq!(code(&task(&sb, &["status", &id, "review"], "s-27", 4)), 0);
    // No reclaim, no status change: this is how fix rounds actually happen on the board.
    tb.call(&sb.root(), "s-27", &at(5), "claude-sonnet-5", &usage(30, 0, 0, 3))
        .call(&sb.root(), "s-27", &at(6), "claude-sonnet-5", &usage(30, 0, 0, 3));

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(7))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("70"), "{text}"); // 10 + 30 + 30, all on the task
    assert!(!text.contains("unassigned"), "{text}");
}

// --- Requirement: Review rounds are counted from status events ------------------------------

#[test]
fn usage__two_review_rounds_and_tokens_per_round() {
    let sb = board("s-15");
    let id = new_task(&sb, "rounds", &[], "s-15", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-15", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-15", &at(3), "claude-sonnet-5", &usage(40, 0, 0, 4));
    assert_eq!(code(&task(&sb, &["status", &id, "review"], "s-15", 4)), 0);
    // The fix round runs with the task still in `review`: no reclaim, no status change
    // (Assumption 3 -- `review` does not close the hold). The bare `status ... in_progress`
    // afterwards only exists so the second `status ... review` is a real transition.
    tb.call(&sb.root(), "s-15", &at(5), "claude-sonnet-5", &usage(70, 0, 0, 7))
        .call(&sb.root(), "s-15", &at(6), "claude-sonnet-5", &usage(70, 0, 0, 7));
    assert_eq!(code(&task(&sb, &["status", &id, "in_progress"], "s-15", 7)), 0);
    assert_eq!(code(&task(&sb, &["status", &id, "review"], "s-15", 8)), 0);

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(9))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("rounds 2"), "{text}");
    assert!(text.contains("140"), "{text}"); // 70 + 70, round 2's tokens
}

// --- Requirement: Weights add a cost column, and only then ----------------------------------

#[test]
fn usage__weights_present_add_cost() {
    let sb = board("s-16");
    let id = new_task(&sb, "weighted", &[], "s-16", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-16", 2)), 0);
    std::fs::write(
        sb.home.path().join("config.toml"),
        "[usage.weights.\"claude-sonnet\"]\ninput = 3.0\ncache_write = 3.75\ncache_read = 0.3\noutput = 15.0\n",
    )
    .unwrap();
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-16", &at(3), "claude-sonnet-5", &usage(1_000_000, 0, 0, 1_000_000));

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    // 1M input @ $3/M + 1M output @ $15/M = $18.00
    assert!(stdout(&out).contains("cost"), "{}", stdout(&out));
    assert!(stdout(&out).contains("18"), "{}", stdout(&out));
}

#[test]
fn usage__no_weights_no_cost() {
    let sb = board("s-17");
    let id = new_task(&sb, "unweighted", &[], "s-17", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-17", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-17", &at(3), "claude-sonnet-5", &usage(100, 0, 0, 10));

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!stdout(&out).to_lowercase().contains("cost"), "{}", stdout(&out));
}

#[test]
fn usage__the_longest_matching_prefix_wins() {
    let sb = board("s-18");
    let id = new_task(&sb, "prefix match", &[], "s-18", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-18", 2)), 0);
    std::fs::write(
        sb.home.path().join("config.toml"),
        "[usage.weights.\"claude\"]\ninput = 1.0\ncache_write = 1.0\ncache_read = 1.0\noutput = 1.0\n\
         [usage.weights.\"claude-opus\"]\ninput = 100.0\ncache_write = 0\ncache_read = 0\noutput = 0\n",
    )
    .unwrap();
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-18", &at(3), "claude-opus-5", &usage(1_000_000, 0, 0, 0));

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    // "claude" ($1/M) would give $1.00; "claude-opus" ($100/M) gives $100.00.
    assert!(stdout(&out).contains("100"), "{}", stdout(&out));
}

// --- Requirement: Reports come in four shapes and a time window -----------------------------

#[test]
fn usage__default_listing_shows_tasks_of_the_last_seven_days() {
    let sb = board("s-19");
    let recent = new_task(&sb, "recent", &[], "s-19", 1);
    assert_eq!(code(&task(&sb, &["claim", &recent], "s-19", 2)), 0);
    let old = new_task(&sb, "old", &[], "s-19", 3);
    assert_eq!(code(&task(&sb, &["claim", &old], "s-19", 4)), 0);
    // Backdating the old task's only events is simpler than fighting the CLI's own clock to make
    // "20 days ago" happen for real.
    db(&sb)
        .execute(
            "UPDATE events SET ts = ?1 WHERE task_id = ?2",
            rusqlite::params!["2026-08-27T12:00:00Z", old],
        )
        .unwrap();
    let tb = TranscriptBuilder::new();
    let out = usage(&sb, &tb, &[], &sb.root(), &[("RATCHET_NOW", &at(5))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains(&recent), "{text}");
    assert!(!text.contains(&old), "{text}");
}

#[test]
fn usage__since_widens_the_window() {
    let sb = board("s-20");
    let recent = new_task(&sb, "recent2", &[], "s-20", 1);
    assert_eq!(code(&task(&sb, &["claim", &recent], "s-20", 2)), 0);
    let old = new_task(&sb, "old2", &[], "s-20", 3);
    assert_eq!(code(&task(&sb, &["claim", &old], "s-20", 4)), 0);
    db(&sb)
        .execute(
            "UPDATE events SET ts = ?1 WHERE task_id = ?2",
            rusqlite::params!["2026-08-27T12:00:00Z", old],
        )
        .unwrap();
    let tb = TranscriptBuilder::new();
    let narrow = usage(&sb, &tb, &[], &sb.root(), &[("RATCHET_NOW", &at(5))]);
    assert!(!stdout(&narrow).contains(&old), "{}", stdout(&narrow));
    let wide = usage(&sb, &tb, &["--since", "30d"], &sb.root(), &[("RATCHET_NOW", &at(5))]);
    assert_eq!(code(&wide), 0, "{}", stderr(&wide));
    assert!(stdout(&wide).contains(&recent), "{}", stdout(&wide));
    assert!(stdout(&wide).contains(&old), "{}", stdout(&wide));
}

#[test]
fn usage__by_role_aggregates_across_tasks() {
    let sb = board("s-21");
    let t1 = new_task(&sb, "role agg 1", &[], "s-21", 1);
    let t2 = new_task(&sb, "role agg 2", &[], "s-21", 2);
    assert_eq!(code(&task(&sb, &["claim", &t1], "s-21", 3)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-21", &at(4), "claude-sonnet-5", &usage(50, 0, 0, 5));
    seed_subagent_event(&sb, "subagent.start", "s-21", "b1", "ratchet:reviewer", "", &t1, &at(4));
    tb.subagent(&sb.root(), "s-21", "b1", &at(4), "claude-sonnet-5", &usage(60, 0, 0, 6));
    assert_eq!(
        code(&task(&sb, &["status", &t1, "done", "--why", "closing"], "s-21", 5)),
        0
    );

    assert_eq!(code(&task(&sb, &["claim", &t2], "s-21", 6)), 0);
    tb.call(&sb.root(), "s-21", &at(7), "claude-sonnet-5", &usage(70, 0, 0, 7));
    seed_subagent_event(&sb, "subagent.start", "s-21", "b2", "ratchet:reviewer", "", &t2, &at(7));
    tb.subagent(&sb.root(), "s-21", "b2", &at(7), "claude-sonnet-5", &usage(80, 0, 0, 8));

    let out = usage(&sb, &tb, &["--by", "role"], &sb.root(), &[("RATCHET_NOW", &at(8))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("orchestrator"), "{text}");
    assert!(text.contains("120"), "{text}"); // 50 + 70
    assert!(text.contains("ratchet:reviewer"), "{text}");
    assert!(text.contains("140"), "{text}"); // 60 + 80
    assert!(text.contains("review rounds per task"), "{text}");
    assert!(text.contains("orientation per session"), "{text}");
}

#[test]
fn usage__numbers_are_abbreviated() {
    let sb = board("s-22");
    let id = new_task(&sb, "abbreviated", &[], "s-22", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-22", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-22", &at(3), "claude-sonnet-5", &usage(1234, 0, 2_500_000, 1));

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("1.2k"), "{text}");
    assert!(text.contains("2.5M"), "{text}");
}

// --- Requirement: `--json` exposes the same data for scripts ---------------------------------

#[test]
fn usage__json_shape() {
    let sb = board("s-23");
    let id = new_task(&sb, "json", &[], "s-23", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-23", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-23", &at(3), "claude-sonnet-5", &usage(10, 0, 0, 1));

    let out = usage(&sb, &tb, &[&id, "--json"], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["tasks"][0]["id"], id);
    assert!(v["tasks"][0]["buckets"][0]["tokens"]["input"].is_number());
    assert!(v["tasks"][0]["buckets"][0].get("cost").is_none());
    assert!(v.get("skipped").is_some());
    assert!(v.get("partial").is_some());
}

// --- Requirement: `--note` writes the one-task summary to the board and nothing else --------

#[test]
fn usage__note_appends_the_summary() {
    let sb = board("s-24");
    let id = new_task(&sb, "note me", &[], "s-24", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-24", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-24", &at(3), "claude-sonnet-5", &usage(100, 0, 0, 10));

    let out = usage(
        &sb,
        &tb,
        &[&id, "--note"],
        &sb.root(),
        &[("RATCHET_SESSION_ID", "s-24"), ("RATCHET_NOW", &at(4))],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let shown = stdout(&task(&sb, &["show", &id], "s-24", 5));
    assert!(shown.contains("usage:"), "{shown}");
    assert!(shown.contains("100"), "{shown}");
    assert!(shown.contains("rounds"), "{shown}");
}

#[test]
fn usage__note_on_a_missing_task_is_refused() {
    let sb = board("s-25");
    let tb = TranscriptBuilder::new();
    let expected = task(&sb, &["note", "T-9999", "x"], "s-25", 1);
    let out = usage(
        &sb,
        &tb,
        &["T-9999", "--note"],
        &sb.root(),
        &[("RATCHET_SESSION_ID", "s-25"), ("RATCHET_NOW", &at(2))],
    );
    assert_eq!(code(&out), 1, "{}", stdout(&out));
    assert_eq!(
        stderr(&out),
        stderr(&expected),
        "must refuse exactly like `ratchet task note` does"
    );
    assert_eq!(
        count(&sb, "SELECT COUNT(*) FROM events WHERE task_id = 'T-9999'", &[]),
        0
    );
}

#[test]
fn usage__without_note_nothing_is_written() {
    let sb = board("s-26");
    let id = new_task(&sb, "no note", &[], "s-26", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-26", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(&sb.root(), "s-26", &at(3), "claude-sonnet-5", &usage(10, 0, 0, 1));
    let before: i64 = count(&sb, "SELECT COUNT(*) FROM events", &[]);

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        count(&sb, "SELECT COUNT(*) FROM events", &[]),
        before,
        "no event was written"
    );
}
```

- [ ] **Step 5: Confirm the crate still compiles and the tests are red for the right reason**

Run: `cargo test -p ratchet --test spec --no-run`
Expected: compiles clean.

Run: `cargo test -p ratchet --test spec usage::`
Expected: all 27 `usage__*` tests FAIL. Every failure's stderr contains clap's own
`error: unrecognized subcommand 'usage'` (exit code 2) — never a panic, never a compile error.
Reading a handful of the failures is enough to confirm this; running the full 27 is only to prove
none of them panics for an unrelated reason (a typo in a JSON field name, a wrong path, etc.).

Run: `cargo test -p ratchet --test scenarios`
Expected: PASS — `every_scenario_has_a_test` is satisfied now that `usage.rs` names all 27
functions `openspec/specs/usage/spec.md` requires; this test does not care whether `ratchet usage`
itself exists yet.

- [ ] **Step 6: Commit**

```bash
git add crates/ratchet/tests/spec/main.rs crates/ratchet/tests/spec/support.rs crates/ratchet/tests/spec/usage.rs
git commit -m "test(usage): scenario tests and transcript fixture builder"
```

---

### Task 2: `usage::transcript` and `usage::weights` (implementer)

**Files:**
- Create: `crates/ratchet/src/usage/mod.rs`, `crates/ratchet/src/usage/transcript.rs`,
  `crates/ratchet/src/usage/weights.rs`
- Modify: `crates/ratchet/src/config.rs` (append `ModelWeights`/`UsageSettings`, extend
  `MachineConfig`, append unit tests to the existing `mod tests`), `crates/ratchet/src/main.rs`
  (one `mod usage;` line — the `Cmd::Usage` variant and its dispatch arm are Task 4's, not this
  task's)

**Interfaces:**
- Consumes: `crate::clock::parse` (already public), `crate::config::{MachineConfig,
  load_machine_config}` (already public).
- Produces (frozen for Task 3 and Task 4):
  `usage::transcript::{Usage, Call, ParseResult, Meta, parse, read_meta, slug_for,
  transcript_path, subagents_dir}`; `usage::weights::{matching, cost}`;
  `config::{ModelWeights, UsageSettings}` and `MachineConfig.usage: UsageSettings`.

**Green criterion for this task** (the scenario tests do **not** move yet — `Cmd::Usage` does not
exist until Task 4, so every `usage__*` scenario still fails with clap's "unrecognized
subcommand"; that is expected and correct here):

```bash
cargo test -p ratchet --bin ratchet config::
cargo test -p ratchet --bin ratchet usage::
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 1: Extend `config.rs` with the weights shape**

Insert the following just above the existing `MachineConfig` struct (i.e. right after
`PdfSettings`'s `impl Default` block, before the `// Consumed by Task 6...` comment that precedes
`MachineConfig` today):

```rust
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ModelWeights {
    /// Per million tokens, in whatever unit the owner likes. Unset fields default to zero.
    pub input: f64,
    pub cache_write: f64,
    pub cache_read: f64,
    pub output: f64,
}

// Consumed by usage::weights (this task) and cli::usage_cmd (Task 4).
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UsageSettings {
    /// Keyed by model prefix, e.g. `"claude-sonnet"`; matched by longest prefix
    /// (`usage::weights::matching`). Empty when `[usage.weights]` is absent from the file.
    pub weights: HashMap<String, ModelWeights>,
}
```

Then change the existing `MachineConfig` struct (its `#[derive]`/`#[serde]` attributes and the
`guardrails`/`pdf` fields are unchanged — only the new field is added):

```rust
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MachineConfig {
    pub guardrails: MachineGuardrails,
    pub pdf: PdfSettings,
    pub usage: UsageSettings,
}
```

- [ ] **Step 2: Append unit tests to `config.rs`'s existing `#[cfg(test)] mod tests`**

Add these three functions just before the closing `}` of the existing `mod tests` block (every
existing test in that block — `defaults_when_sections_are_missing` through
`pdf_unknown_key_is_an_error_like_other_tables` — is untouched):

```rust
    #[test]
    fn usage_settings_default_is_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let c = load_machine_config(dir.path()).unwrap();
        assert!(c.usage.weights.is_empty());
    }

    #[test]
    fn usage_weights_parse_a_quoted_prefix_table() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[usage.weights.\"claude-sonnet\"]\ninput = 3.0\ncache_write = 3.75\ncache_read = 0.3\noutput = 15.0\n",
        )
        .unwrap();
        let c = load_machine_config(dir.path()).unwrap();
        let w = c.usage.weights.get("claude-sonnet").unwrap();
        assert_eq!(w.input, 3.0);
        assert_eq!(w.output, 15.0);
    }

    #[test]
    fn usage_weights_unknown_key_is_an_error_like_other_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("config.toml"), "[usage.weights.\"x\"]\nbogus = 1\n").unwrap();
        let err = load_machine_config(dir.path()).unwrap_err();
        assert!(err.message.contains("bogus"), "{}", err.message);
    }
```

- [ ] **Step 3: Run the config tests**

Run: `cargo test -p ratchet --bin ratchet config::`
Expected: PASS, including the three new ones.

- [ ] **Step 4: Create `crates/ratchet/src/usage/mod.rs`**

```rust
//! Token cost per task, role and model, read from the transcripts Claude Code already writes to
//! disk. Every module here except the CLI face (`crate::cli::usage_cmd`, Task 4) is pure: no
//! filesystem, no clock, no database. See
//! `docs/superpowers/specs/2026-09-21-ratchet-usage-design.md`.

pub mod transcript;
pub mod weights;
```

(Task 3 adds one more line, `pub mod attribute;`, to this same file.)

- [ ] **Step 5: Create `crates/ratchet/src/usage/transcript.rs`**

```rust
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
    v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect()
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
        assert_eq!(slug_for(r"C:\repos\demo"), "-C--repos-demo");
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
            Usage { input: 10, cache_write: 1, cache_read: 2, output: 5, thinking: 3 }
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
        assert_eq!(r.calls.len(), 2, "the partial record is still a call, at zero");
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
        let m = read_meta(br#"{"agentType":"ratchet:reviewer","description":"d","toolUseId":"tu-9"}"#).unwrap();
        assert_eq!(m.agent_type.as_deref(), Some("ratchet:reviewer"));
        assert_eq!(m.description.as_deref(), Some("d"));
        assert_eq!(m.tool_use_id.as_deref(), Some("tu-9"));
        assert_eq!(m.model, None);
        assert!(read_meta(b"not json").is_none());
    }
}
```

- [ ] **Step 6: Create `crates/ratchet/src/usage/weights.rs`**

```rust
//! Cost is optional and comes only from configured weights (D-usage-weights): pure matching and
//! arithmetic, no TOML parsing here — `config.rs` already owns deserializing
//! `[usage.weights."<prefix>"]` into `HashMap<String, ModelWeights>`. Takes plain token counts
//! rather than a `usage::attribute::Totals`, so this module has no dependency on
//! `usage::attribute` (Task 3) and stays buildable on its own in this task.

use std::collections::HashMap;

use crate::config::ModelWeights;

/// The configured weight whose prefix is the longest match of `model`, or `None`.
// Consumed by usage::attribute (Task 3).
#[allow(dead_code)]
pub fn matching<'a>(
    model: &str,
    weights: &'a HashMap<String, ModelWeights>,
) -> Option<&'a ModelWeights> {
    weights
        .iter()
        .filter(|(prefix, _)| model.starts_with(prefix.as_str()))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, w)| w)
}

/// `None` when no weight matches `model` — callers must never print a cost in that case
/// (D-usage-weights: "without a matching weight, tokens only, never invent a price").
// Consumed by usage::attribute (Task 3).
#[allow(dead_code)]
pub fn cost(
    model: &str,
    weights: &HashMap<String, ModelWeights>,
    input: u64,
    cache_write: u64,
    cache_read: u64,
    output: u64,
) -> Option<f64> {
    let w = matching(model, weights)?;
    let per = |n: u64, rate: f64| (n as f64) * rate / 1_000_000.0;
    Some(
        per(input, w.input)
            + per(cache_write, w.cache_write)
            + per(cache_read, w.cache_read)
            + per(output, w.output),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weights(pairs: &[(&str, f64, f64, f64, f64)]) -> HashMap<String, ModelWeights> {
        pairs
            .iter()
            .map(|(prefix, input, cache_write, cache_read, output)| {
                (
                    prefix.to_string(),
                    ModelWeights {
                        input: *input,
                        cache_write: *cache_write,
                        cache_read: *cache_read,
                        output: *output,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn no_prefix_matches_gives_none() {
        let w = weights(&[("claude-sonnet", 1.0, 1.0, 1.0, 1.0)]);
        assert!(matching("claude-opus-5", &w).is_none());
        assert!(cost("claude-opus-5", &w, 1000, 0, 0, 0).is_none());
    }

    #[test]
    fn the_longest_matching_prefix_wins() {
        let w = weights(&[("claude", 1.0, 0.0, 0.0, 0.0), ("claude-opus", 100.0, 0.0, 0.0, 0.0)]);
        let matched = matching("claude-opus-5", &w).unwrap();
        assert_eq!(matched.input, 100.0);
    }

    #[test]
    fn cost_sums_all_four_classes_per_million() {
        let w = weights(&[("claude-sonnet", 3.0, 3.75, 0.3, 15.0)]);
        let c = cost("claude-sonnet-5", &w, 1_000_000, 1_000_000, 1_000_000, 1_000_000).unwrap();
        assert!((c - (3.0 + 3.75 + 0.3 + 15.0)).abs() < 1e-9, "{c}");
    }

    #[test]
    fn zero_tokens_cost_zero_even_with_weights() {
        let w = weights(&[("claude-sonnet", 3.0, 3.75, 0.3, 15.0)]);
        assert_eq!(cost("claude-sonnet-5", &w, 0, 0, 0, 0), Some(0.0));
    }
}
```

- [ ] **Step 7: Register the module in `main.rs`**

Edit `crates/ratchet/src/main.rs`'s module list (alphabetical, as the rest already are):

```rust
mod cli;
mod clock;
mod config;
mod db;
mod guardrails;
mod hooks;
mod log;
mod model;
mod output;
mod pdf;
mod repo;
mod services;
mod usage;
```

- [ ] **Step 8: Run the new unit tests and confirm nothing else moved**

```bash
cargo test -p ratchet --bin ratchet usage::
cargo test -p ratchet --bin ratchet config::
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Expected: all PASS/clean. Then confirm the scenario tests are exactly as red as they were at the
end of Task 1 (same failure mode, not a new one):

```bash
cargo test -p ratchet --test spec usage::
```

Expected: all 27 still FAIL with clap's "unrecognized subcommand" — nothing in this task touched
`main.rs`'s `Cmd` enum or `cli/mod.rs`.

- [ ] **Step 9: Commit**

```bash
git add crates/ratchet/src/config.rs crates/ratchet/src/main.rs crates/ratchet/src/usage/mod.rs crates/ratchet/src/usage/transcript.rs crates/ratchet/src/usage/weights.rs
git commit -m "feat(usage): transcript parsing and weight matching, pure modules"
```

---

### Task 3: `usage::attribute` and the derived metrics (implementer)

**Files:**
- Create: `crates/ratchet/src/usage/attribute.rs`
- Modify: `crates/ratchet/src/usage/mod.rs` (one line, `pub mod attribute;`)

**Interfaces:**
- Consumes: `usage::transcript::{Call, Meta, Usage}` (Task 2), `usage::weights::cost` (Task 2),
  `config::ModelWeights` (Task 2), `crate::clock::parse` (already public).
- Produces (frozen for Task 4): `usage::attribute::{Totals, AttributedCall, Bucket, Hold,
  SessionEvent, SessionEventKind, SubagentEvent, holds, task_at, orientation, group_totals,
  buckets_of, totals_by_role_model, rounds_tokens, average_totals, orchestrator_share,
  cache_efficiency, abbreviate, since_cutoff, display_role, subagent_task_and_role, task_cost}`.

This is the module every number in Requirements 3–8 of `openspec/specs/usage/spec.md` comes from.
Read the design's D-usage-join, D-usage-subagents, D-usage-orientation, D-usage-rounds and
D-usage-weights (§3) before writing any code here — every public function below implements one
sentence of one of those decisions, named in its own doc comment.

**Green criterion for this task** (scenario tests still do not move — `cli/usage_cmd.rs` does not
exist until Task 4):

```bash
cargo test -p ratchet --bin ratchet usage::attribute::
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 1: Register the module**

Edit `crates/ratchet/src/usage/mod.rs`:

```rust
pub mod attribute;
pub mod transcript;
pub mod weights;
```

- [ ] **Step 2: Create `crates/ratchet/src/usage/attribute.rs`**

```rust
//! Joins parsed transcript calls with the sessions/events ratchet already records, and derives
//! every number Requirement 3 through 8 of `openspec/specs/usage/spec.md` define: which task (if
//! any) a call belongs to, a subagent's task and role, orientation, review rounds, and the
//! abbreviated/weighted numbers a report prints. Pure: no filesystem, no clock, no database —
//! `cli::usage_cmd` (Task 4) reads everything this module needs and hands it over as plain data.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::config::ModelWeights;
use crate::usage::transcript::{self, Meta, Usage};
use crate::usage::weights;

/// Sum of four token classes plus thinking. `Serialize`s with exactly the field names
/// Requirement 9 (`--json`) specifies. `thinking` is already counted inside `output` on the wire
/// (design §4: "out, with thinking inside") — tracked separately here only for display, and
/// `all()` does not double-add it.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Totals {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub thinking: u64,
}

impl Totals {
    pub fn add(&mut self, u: &Usage) {
        self.input += u.input;
        self.cache_write += u.cache_write;
        self.cache_read += u.cache_read;
        self.output += u.output;
        self.thinking += u.thinking;
    }

    pub fn merge(&mut self, other: &Totals) {
        self.input += other.input;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
        self.output += other.output;
        self.thinking += other.thinking;
    }

    /// Every class summed; `thinking` excluded on purpose (already inside `output`).
    pub fn all(&self) -> u64 {
        self.input + self.cache_write + self.cache_read + self.output
    }
}

/// `k`/`M` with one decimal, per design §4's column rule. Below 1000 prints as-is.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn abbreviate(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// "7d" / "30d" / an RFC 3339 date (`2026-09-01`, midnight UTC). The caller supplies the default
/// (`"7d"`) when `--since` is absent — this function does not know about defaults.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn since_cutoff(spec: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
    let spec = spec.trim();
    if let Some(days) = spec.strip_suffix('d') {
        let n: i64 = days.parse().map_err(|_| format!("invalid --since: {spec:?}"))?;
        if n <= 0 {
            return Err(format!("invalid --since: {spec:?}"));
        }
        return Ok(now - Duration::days(n));
    }
    let with_time = format!("{spec}T00:00:00Z");
    crate::clock::parse(&with_time).ok_or_else(|| format!("invalid --since: {spec:?}"))
}

/// `general-purpose` shows with the first 40 characters of its description and nothing more of
/// it; every other role prints as-is (D-usage-subagents).
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn display_role(role: &str, description: Option<&str>) -> String {
    if role == "general-purpose" {
        let clipped: String = description.unwrap_or_default().chars().take(40).collect();
        format!("general-purpose: {clipped}")
    } else {
        role.to_string()
    }
}

/// The two event kinds `holds` needs, already extracted from `events.kind`/`events.payload` by
/// the caller (`cli::usage_cmd`, Task 4, via `services::events::for_session`): a `task.claimed`
/// row becomes `Claimed`, a `task.status` row becomes `Status` with `to` read from
/// `payload.to`.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEventKind {
    Claimed { task: String },
    Status { task: String, to: String },
}

// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct SessionEvent {
    pub ts: DateTime<Utc>,
    pub kind: SessionEventKind,
}

/// One interval during which a session held one task, half-open `[start, end)`. `end: None`
/// means "still held" — `holds()` always closes these against `session_end` before returning.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct Hold {
    pub task: String,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
}

/// R3: a hold opens on `task.claimed` and closes on the first `task.status` event for that same
/// task whose `to` is `done`, `blocked` or `ready`, or at `session_end`, whichever comes first.
/// `review` does NOT close a hold (Assumption 3, settled): fix rounds after a verdict happen
/// with the task still in `review` and belong to it. A `task.status` event to `in_progress`
/// (or `review`) is a no-op here. A fresh `task.claimed` on a task already held opens a second
/// hold on it, harmless under "most recently claimed wins". Several holds
/// may be open at once (a session that claims a second task before releasing the first); no
/// hold's `end` is ever truncated by a later claim — `task_at` below is what implements "the
/// most recently claimed wins" for an instant covered by more than one open hold, by
/// construction, without this function needing a stack-pop/reactivate step.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn holds(events: &[SessionEvent], session_end: Option<DateTime<Utc>>) -> Vec<Hold> {
    let mut open: Vec<Hold> = Vec::new();
    let mut closed: Vec<Hold> = Vec::new();
    let mut ordered: Vec<&SessionEvent> = events.iter().collect();
    ordered.sort_by_key(|e| e.ts);
    for ev in ordered {
        match &ev.kind {
            SessionEventKind::Claimed { task } => {
                open.push(Hold { task: task.clone(), start: ev.ts, end: None });
            }
            SessionEventKind::Status { task, to }
                if matches!(to.as_str(), "done" | "blocked" | "ready") =>
            {
                if let Some(pos) = open.iter().position(|h| &h.task == task) {
                    let mut h = open.remove(pos);
                    h.end = Some(ev.ts);
                    closed.push(h);
                }
            }
            SessionEventKind::Status { .. } => {}
        }
    }
    for mut h in open {
        h.end = session_end;
        closed.push(h);
    }
    closed.sort_by_key(|h| h.start);
    closed
}

/// The task `at` belongs to: the hold with the latest `start` whose window contains `at` (start
/// inclusive, end exclusive), or `None` for "unassigned". "Most recently claimed wins" falls out
/// of `max_by_key(start)` directly when more than one hold's window contains `at`.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn task_at(holds: &[Hold], at: DateTime<Utc>) -> Option<&str> {
    holds
        .iter()
        .filter(|h| h.start <= at && h.end.map(|e| at < e).unwrap_or(true))
        .max_by_key(|h| h.start)
        .map(|h| h.task.as_str())
}

/// D-usage-orientation: the orchestrator's tokens from the session's first record until the
/// earliest of its first claim, or the call that first dispatches an `Agent`/`Edit`/`Write`/
/// `NotebookEdit` tool — that dispatching call itself is included (it is what ends orientation,
/// not what follows it). `calls_sorted` must be sorted by `ts` ascending and must be the
/// orchestrator's own calls only (role `orchestrator`) — a subagent's calls are never part of
/// orientation.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn orientation(calls_sorted: &[&transcript::Call], first_claim: Option<DateTime<Utc>>) -> Totals {
    let mut totals = Totals::default();
    for c in calls_sorted {
        if let Some(claim_ts) = first_claim {
            if c.ts >= claim_ts {
                break;
            }
        }
        let dispatch = c
            .tools
            .iter()
            .any(|t| matches!(t.as_str(), "Agent" | "Edit" | "Write" | "NotebookEdit"));
        totals.add(&c.usage);
        if dispatch {
            break;
        }
    }
    totals
}

/// Component-wise average, rounded to the nearest token. Empty input averages to zero. Used for
/// "orientation shown per task as the average over the task's sessions" (design §4).
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn average_totals(items: &[Totals]) -> Totals {
    if items.is_empty() {
        return Totals::default();
    }
    let n = items.len() as f64;
    let avg = |sum: u64| (sum as f64 / n).round() as u64;
    Totals {
        input: avg(items.iter().map(|t| t.input).sum()),
        cache_write: avg(items.iter().map(|t| t.cache_write).sum()),
        cache_read: avg(items.iter().map(|t| t.cache_read).sum()),
        output: avg(items.iter().map(|t| t.output).sum()),
        thinking: avg(items.iter().map(|t| t.thinking).sum()),
    }
}

/// One `subagent.start`/`subagent.stop` event, already extracted by the caller (Assumption 1 at
/// the top of this plan): `agent_id`/`agent_type`/`description` come from `events.payload`;
/// `task` comes from the event row's own `task_id` column, never from the payload (which has
/// no `task_id` field).
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentEvent {
    pub agent_id: String,
    pub agent_type: Option<String>,
    pub description: Option<String>,
    pub task: Option<String>,
}

/// D-usage-subagents' three-step resolution for one subagent transcript. `event` is the
/// `subagent.start`/`.stop` event matching this transcript's `agent_id` (`None` when no
/// start/stop event fired for this call — the chain falls through exactly the same way
/// either way). `parent_agent_calls` are the parent's own calls that carry
/// an `Agent` `tool_use`, as `(tool_use_id, ts)` pairs. `parent_holds` are the parent session's
/// holds (`holds()`, above). Returns `(task, role)`; `role` is not yet display-formatted — pass
/// it through `display_role` before printing.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn subagent_task_and_role(
    event: Option<&SubagentEvent>,
    meta: Option<&Meta>,
    parent_holds: &[Hold],
    parent_agent_calls: &[(String, DateTime<Utc>)],
    first_record_ts: Option<DateTime<Utc>>,
) -> (Option<String>, String) {
    if let Some(ev) = event {
        let role = ev.agent_type.clone().unwrap_or_else(|| "subagent".to_string());
        return (ev.task.clone(), role);
    }
    if let Some(tu_id) = meta.and_then(|m| m.tool_use_id.as_deref()) {
        if let Some((_, ts)) = parent_agent_calls.iter().find(|(id, _)| id == tu_id) {
            let task = task_at(parent_holds, *ts).map(str::to_string);
            let role = meta
                .and_then(|m| m.agent_type.clone())
                .unwrap_or_else(|| "subagent".to_string());
            return (task, role);
        }
    }
    let task = first_record_ts
        .and_then(|ts| task_at(parent_holds, ts))
        .map(str::to_string);
    let role = meta
        .and_then(|m| m.agent_type.clone())
        .unwrap_or_else(|| "subagent".to_string());
    (task, role)
}

/// One call, already resolved to its task (`None` = unassigned), session, role and model. The
/// flat shape `cli::usage_cmd` (Task 4) builds by walking sessions/transcripts and classifying
/// every call through `task_at`/`subagent_task_and_role`; every reducer below (`group_totals`,
/// `buckets_of`, `totals_by_role_model`, `rounds_tokens`) works from this one shape so the CLI
/// face only needs to build it once per report. `role` here is the *raw* role (e.g.
/// `general-purpose`, not yet display-formatted) — callers pass it through `display_role` for
/// anything printed.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AttributedCall {
    pub task: Option<String>,
    pub session: String,
    pub role: String,
    pub model: String,
    pub ts: DateTime<Utc>,
    pub usage: Usage,
}

/// One role×model×session row. `cost` is `None` unless `buckets_of` was given weights and a
/// prefix matched this bucket's `model`.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct Bucket {
    pub session: String,
    pub role: String,
    pub model: String,
    pub tokens: Totals,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

/// Sums `calls` grouped by an arbitrary string key — the shared engine behind `--by
/// task|role|model|session` (Task 4 supplies the key closure per flag). `BTreeMap` keeps output
/// order deterministic without a separate sort step.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn group_totals(
    calls: &[AttributedCall],
    key: impl Fn(&AttributedCall) -> String,
) -> BTreeMap<String, Totals> {
    let mut out: BTreeMap<String, Totals> = BTreeMap::new();
    for c in calls {
        out.entry(key(c)).or_default().add(&c.usage);
    }
    out
}

/// One task's calls, grouped into `(session, role, model)` rows — the granularity `--json`'s
/// `buckets` array uses (Requirement 9). `weights` is `None` when the machine config has no
/// `[usage.weights]` table at all; when `Some`, each bucket's `cost` is set from its own model,
/// independently — a bucket whose model matches no prefix simply has `cost: None`.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn buckets_of(calls: &[AttributedCall], weights: Option<&HashMap<String, ModelWeights>>) -> Vec<Bucket> {
    let mut totals: BTreeMap<(String, String, String), Totals> = BTreeMap::new();
    for c in calls {
        totals
            .entry((c.session.clone(), c.role.clone(), c.model.clone()))
            .or_default()
            .add(&c.usage);
    }
    totals
        .into_iter()
        .map(|((session, role, model), tokens)| {
            let cost = weights.and_then(|w| {
                weights::cost(&model, w, tokens.input, tokens.cache_write, tokens.cache_read, tokens.output)
            });
            Bucket { session, role, model, tokens, cost }
        })
        .collect()
}

/// The same calls collapsed to `(role, model)` — what the task-detail *text* view shows (design
/// §4: "a line per role × model"; which sessions were involved is listed separately, not as a
/// row dimension here).
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn totals_by_role_model(calls: &[AttributedCall]) -> BTreeMap<(String, String), Totals> {
    let mut out: BTreeMap<(String, String), Totals> = BTreeMap::new();
    for c in calls {
        out.entry((c.role.clone(), c.model.clone())).or_default().add(&c.usage);
    }
    out
}

/// R6: tokens between consecutive entries into `review`, the first round counting from
/// `first_claim`. `review_entries` are the task's `task.status → review` timestamps, in order;
/// `rounds` (the count Requirement 6 names) is simply `review_entries.len()`. `calls` must
/// already be filtered to one task (every `AttributedCall` with `task == Some(that_id)`).
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn rounds_tokens(
    calls: &[AttributedCall],
    first_claim: DateTime<Utc>,
    review_entries: &[DateTime<Utc>],
) -> Vec<Totals> {
    let mut bounds = vec![first_claim];
    bounds.extend(review_entries.iter().copied());
    bounds
        .windows(2)
        .map(|w| {
            let (start, end) = (w[0], w[1]);
            let mut t = Totals::default();
            for c in calls {
                if c.ts >= start && c.ts < end {
                    t.add(&c.usage);
                }
            }
            t
        })
        .collect()
}

/// `orchestrator` tokens ÷ all tokens, `0.0` when `buckets` is empty.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn orchestrator_share(buckets: &[Bucket]) -> f64 {
    let all: u64 = buckets.iter().map(|b| b.tokens.all()).sum();
    if all == 0 {
        return 0.0;
    }
    let orch: u64 = buckets
        .iter()
        .filter(|b| b.role == "orchestrator")
        .map(|b| b.tokens.all())
        .sum();
    orch as f64 / all as f64
}

/// `cache_read ÷ (input + cache_write + cache_read)`, `0.0` when that denominator is zero.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn cache_efficiency(t: &Totals) -> f64 {
    let denom = t.input + t.cache_write + t.cache_read;
    if denom == 0 {
        return 0.0;
    }
    t.cache_read as f64 / denom as f64
}

/// A task-level aggregate `cost`: the sum of every bucket's own cost, but only when *all* of them
/// matched a weight (Assumption 4 at the top of this plan) — a partial sum would be misleading
/// for a task that used more than one model.
// Consumed by cli::usage_cmd (Task 4).
#[allow(dead_code)]
pub fn task_cost(buckets: &[Bucket]) -> Option<f64> {
    if buckets.is_empty() {
        return None;
    }
    buckets
        .iter()
        .map(|b| b.cost)
        .collect::<Option<Vec<f64>>>()
        .map(|v| v.iter().sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        crate::clock::parse(s).unwrap()
    }

    fn usage(input: u64, output: u64) -> Usage {
        Usage { input, cache_write: 0, cache_read: 0, output, thinking: 0 }
    }

    fn call(ts: &str, model: &str, u: Usage, tools: &[&str]) -> transcript::Call {
        transcript::Call {
            ts: at(ts),
            model: model.to_string(),
            usage: u,
            tools: tools.iter().map(|s| s.to_string()).collect(),
            tool_use_ids: Vec::new(),
        }
    }

    // --- holds / task_at -------------------------------------------------------------------

    #[test]
    fn a_call_at_exactly_the_claim_timestamp_belongs_to_the_task() {
        let events = vec![SessionEvent {
            ts: at("2026-09-16T12:00:00Z"),
            kind: SessionEventKind::Claimed { task: "T-0001".into() },
        }];
        let h = holds(&events, None);
        assert_eq!(task_at(&h, at("2026-09-16T12:00:00Z")), Some("T-0001"));
        assert_eq!(task_at(&h, at("2026-09-16T11:59:59Z")), None);
    }

    #[test]
    fn a_call_at_exactly_the_release_timestamp_is_outside_the_window() {
        let events = vec![
            SessionEvent {
                ts: at("2026-09-16T12:00:00Z"),
                kind: SessionEventKind::Claimed { task: "T-0001".into() },
            },
            SessionEvent {
                ts: at("2026-09-16T12:10:00Z"),
                kind: SessionEventKind::Status { task: "T-0001".into(), to: "done".into() },
            },
        ];
        let h = holds(&events, None);
        assert_eq!(task_at(&h, at("2026-09-16T12:09:59Z")), Some("T-0001"));
        assert_eq!(
            task_at(&h, at("2026-09-16T12:10:00Z")),
            None,
            "the release instant is not inside the closed window"
        );
    }

    #[test]
    fn review_does_not_close_the_hold() {
        let events = vec![
            SessionEvent {
                ts: at("2026-09-16T12:00:00Z"),
                kind: SessionEventKind::Claimed { task: "T-0001".into() },
            },
            SessionEvent {
                ts: at("2026-09-16T12:10:00Z"),
                kind: SessionEventKind::Status { task: "T-0001".into(), to: "review".into() },
            },
            SessionEvent {
                ts: at("2026-09-16T12:20:00Z"),
                kind: SessionEventKind::Status { task: "T-0001".into(), to: "ready".into() },
            },
        ];
        let h = holds(&events, None);
        assert_eq!(
            task_at(&h, at("2026-09-16T12:15:00Z")),
            Some("T-0001"),
            "a fix round while the task sits in review still belongs to it"
        );
        assert_eq!(task_at(&h, at("2026-09-16T12:20:00Z")), None, "ready closes it");
    }

    #[test]
    fn most_recently_claimed_wins_while_two_tasks_are_held_at_once() {
        let events = vec![
            SessionEvent {
                ts: at("2026-09-16T12:00:00Z"),
                kind: SessionEventKind::Claimed { task: "T-0001".into() },
            },
            SessionEvent {
                ts: at("2026-09-16T12:05:00Z"),
                kind: SessionEventKind::Claimed { task: "T-0002".into() },
            },
            SessionEvent {
                ts: at("2026-09-16T12:10:00Z"),
                kind: SessionEventKind::Status { task: "T-0002".into(), to: "done".into() },
            },
        ];
        let h = holds(&events, None);
        assert_eq!(
            task_at(&h, at("2026-09-16T12:06:00Z")),
            Some("T-0002"),
            "the more recently claimed task wins while both are open"
        );
        assert_eq!(
            task_at(&h, at("2026-09-16T12:11:00Z")),
            Some("T-0001"),
            "T-0001 resumes once T-0002 is released"
        );
    }

    #[test]
    fn an_open_hold_closes_at_session_end() {
        let events = vec![SessionEvent {
            ts: at("2026-09-16T12:00:00Z"),
            kind: SessionEventKind::Claimed { task: "T-0001".into() },
        }];
        let h = holds(&events, Some(at("2026-09-16T13:00:00Z")));
        assert_eq!(task_at(&h, at("2026-09-16T12:59:59Z")), Some("T-0001"));
        assert_eq!(task_at(&h, at("2026-09-16T13:00:00Z")), None);
    }

    #[test]
    fn a_status_event_into_in_progress_neither_opens_nor_closes_a_hold() {
        let events = vec![
            SessionEvent {
                ts: at("2026-09-16T12:00:00Z"),
                kind: SessionEventKind::Claimed { task: "T-0001".into() },
            },
            SessionEvent {
                ts: at("2026-09-16T12:05:00Z"),
                kind: SessionEventKind::Status { task: "T-0001".into(), to: "in_progress".into() },
            },
        ];
        let h = holds(&events, None);
        assert_eq!(h.len(), 1, "the to=in_progress event produced no second hold");
        assert_eq!(task_at(&h, at("2026-09-16T12:05:00Z")), Some("T-0001"));
    }

    // --- orientation -------------------------------------------------------------------------

    #[test]
    fn orientation_stops_before_the_first_claim() {
        let c1 = call("2026-09-16T12:00:00Z", "m", usage(30, 4), &[]);
        let c2 = call("2026-09-16T12:01:00Z", "m", usage(30, 4), &[]);
        let c3 = call("2026-09-16T12:02:00Z", "m", usage(90, 10), &[]);
        let sorted = [&c1, &c2, &c3];
        let t = orientation(&sorted, Some(at("2026-09-16T12:01:30Z")));
        assert_eq!(t.input, 60);
    }

    #[test]
    fn orientation_includes_the_dispatching_call_itself() {
        let c1 = call("2026-09-16T12:00:00Z", "m", usage(25, 3), &[]);
        let c2 = call("2026-09-16T12:01:00Z", "m", usage(25, 3), &["Agent"]);
        let c3 = call("2026-09-16T12:02:00Z", "m", usage(80, 9), &[]);
        let sorted = [&c1, &c2, &c3];
        let t = orientation(&sorted, None);
        assert_eq!(t.input, 50, "both calls up to and including the dispatch count");
    }

    #[test]
    fn orientation_with_neither_a_claim_nor_a_dispatch_counts_everything() {
        let c1 = call("2026-09-16T12:00:00Z", "m", usage(10, 1), &[]);
        let c2 = call("2026-09-16T12:01:00Z", "m", usage(10, 1), &[]);
        let sorted = [&c1, &c2];
        let t = orientation(&sorted, None);
        assert_eq!(t.input, 20);
    }

    // --- rounds_tokens -----------------------------------------------------------------------

    #[test]
    fn rounds_tokens_splits_on_consecutive_review_entries() {
        let mk = |ts: &str, input: u64| AttributedCall {
            task: Some("T-0001".into()),
            session: "s-1".into(),
            role: "orchestrator".into(),
            model: "m".into(),
            ts: at(ts),
            usage: usage(input, input / 10),
        };
        let calls = vec![
            mk("2026-09-16T12:01:00Z", 40),
            mk("2026-09-16T12:06:00Z", 70),
            mk("2026-09-16T12:07:00Z", 70),
        ];
        let review_entries = [at("2026-09-16T12:04:00Z"), at("2026-09-16T12:08:00Z")];
        let rounds = rounds_tokens(&calls, at("2026-09-16T12:00:00Z"), &review_entries);
        assert_eq!(rounds.len(), 2);
        assert_eq!(rounds[0].input, 40);
        assert_eq!(rounds[1].input, 140, "the two calls between the two review entries");
    }

    // --- abbreviate / since_cutoff / display_role / averages ---------------------------------

    #[test]
    fn abbreviate_uses_k_and_m_with_one_decimal() {
        assert_eq!(abbreviate(999), "999");
        assert_eq!(abbreviate(1234), "1.2k");
        assert_eq!(abbreviate(2_500_000), "2.5M");
    }

    #[test]
    fn since_cutoff_understands_days_and_dates_and_refuses_junk() {
        let now = at("2026-09-16T12:00:00Z");
        assert_eq!(since_cutoff("7d", now).unwrap(), now - Duration::days(7));
        assert_eq!(since_cutoff("30d", now).unwrap(), now - Duration::days(30));
        assert_eq!(since_cutoff("2026-09-01", now).unwrap(), at("2026-09-01T00:00:00Z"));
        assert!(since_cutoff("0d", now).is_err());
        assert!(since_cutoff("nonsense", now).is_err());
    }

    #[test]
    fn display_role_clips_general_purpose_and_passes_others_through() {
        let desc = "search the whole repository for every remaining caller of the old helper";
        let clipped: String = desc.chars().take(40).collect();
        assert_eq!(
            display_role("general-purpose", Some(desc)),
            format!("general-purpose: {clipped}")
        );
        assert_eq!(display_role("ratchet:reviewer", None), "ratchet:reviewer");
    }

    #[test]
    fn average_totals_is_component_wise_and_rounds() {
        let a = Totals { input: 10, cache_write: 0, cache_read: 0, output: 1, thinking: 0 };
        let b = Totals { input: 5, cache_write: 0, cache_read: 0, output: 2, thinking: 0 };
        let avg = average_totals(&[a, b]);
        assert_eq!(avg.input, 8, "(10+5)/2 = 7.5, rounds to 8");
        assert_eq!(avg.output, 2, "(1+2)/2 = 1.5, rounds to 2");
        assert_eq!(average_totals(&[]).input, 0);
    }

    // --- buckets_of / task_cost ----------------------------------------------------------------

    #[test]
    fn buckets_of_groups_by_session_role_and_model_and_costs_independently() {
        let calls = vec![
            AttributedCall {
                task: Some("T-0001".into()), session: "s-1".into(), role: "orchestrator".into(),
                model: "claude-sonnet-5".into(), ts: at("2026-09-16T12:00:00Z"), usage: usage(1_000_000, 0),
            },
            AttributedCall {
                task: Some("T-0001".into()), session: "s-1".into(), role: "orchestrator".into(),
                model: "claude-haiku-5".into(), ts: at("2026-09-16T12:01:00Z"), usage: usage(1_000_000, 0),
            },
        ];
        let mut w = HashMap::new();
        w.insert(
            "claude-sonnet".to_string(),
            ModelWeights { input: 3.0, cache_write: 0.0, cache_read: 0.0, output: 0.0 },
        );
        let buckets = buckets_of(&calls, Some(&w));
        assert_eq!(buckets.len(), 2);
        let sonnet = buckets.iter().find(|b| b.model == "claude-sonnet-5").unwrap();
        assert_eq!(sonnet.cost, Some(3.0));
        let haiku = buckets.iter().find(|b| b.model == "claude-haiku-5").unwrap();
        assert_eq!(haiku.cost, None);
        assert_eq!(task_cost(&buckets), None, "one unweighted bucket means no aggregate cost");
    }

    #[test]
    fn task_cost_sums_when_every_bucket_matched() {
        let calls = vec![AttributedCall {
            task: Some("T-0001".into()), session: "s-1".into(), role: "orchestrator".into(),
            model: "claude-sonnet-5".into(), ts: at("2026-09-16T12:00:00Z"), usage: usage(1_000_000, 0),
        }];
        let mut w = HashMap::new();
        w.insert(
            "claude-sonnet".to_string(),
            ModelWeights { input: 3.0, cache_write: 0.0, cache_read: 0.0, output: 0.0 },
        );
        let buckets = buckets_of(&calls, Some(&w));
        assert_eq!(task_cost(&buckets), Some(3.0));
    }

    // --- subagent_task_and_role ------------------------------------------------------------------

    #[test]
    fn subagent_resolution_prefers_the_event_then_tool_use_id_then_first_timestamp() {
        let holds = vec![Hold { task: "T-0001".into(), start: at("2026-09-16T12:00:00Z"), end: None }];

        let via_event = subagent_task_and_role(
            Some(&SubagentEvent {
                agent_id: "a1".into(),
                agent_type: Some("ratchet:reviewer".into()),
                description: None,
                task: Some("T-0002".into()),
            }),
            None,
            &holds,
            &[],
            None,
        );
        assert_eq!(via_event, (Some("T-0002".to_string()), "ratchet:reviewer".to_string()));

        let meta = Meta {
            agent_type: Some("ratchet:implementer".into()),
            description: None,
            model: None,
            tool_use_id: Some("tu-9".into()),
        };
        let via_tool = subagent_task_and_role(
            None,
            Some(&meta),
            &holds,
            &[("tu-9".to_string(), at("2026-09-16T12:05:00Z"))],
            None,
        );
        assert_eq!(via_tool, (Some("T-0001".to_string()), "ratchet:implementer".to_string()));

        let via_ts = subagent_task_and_role(None, None, &holds, &[], Some(at("2026-09-16T12:05:00Z")));
        assert_eq!(via_ts, (Some("T-0001".to_string()), "subagent".to_string()));
    }

    // --- orchestrator_share / cache_efficiency ---------------------------------------------------

    #[test]
    fn orchestrator_share_and_cache_efficiency_are_ratios_of_all_tokens() {
        let buckets = vec![
            Bucket {
                session: "s-1".into(), role: "orchestrator".into(), model: "m".into(),
                tokens: Totals { input: 100, cache_write: 0, cache_read: 0, output: 0, thinking: 0 },
                cost: None,
            },
            Bucket {
                session: "s-1".into(), role: "ratchet:reviewer".into(), model: "m".into(),
                tokens: Totals { input: 300, cache_write: 0, cache_read: 0, output: 0, thinking: 0 },
                cost: None,
            },
        ];
        assert_eq!(orchestrator_share(&buckets), 0.25);
        assert_eq!(orchestrator_share(&[]), 0.0);
        let t = Totals { input: 70, cache_write: 0, cache_read: 30, output: 0, thinking: 0 };
        assert_eq!(cache_efficiency(&t), 0.3);
        assert_eq!(cache_efficiency(&Totals::default()), 0.0);
    }
}
```

- [ ] **Step 3: Run the new unit tests and confirm nothing else moved**

```bash
cargo test -p ratchet --bin ratchet usage::attribute::
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p ratchet --test spec usage::
```

Expected: the first three PASS/clean; the last still shows all 27 `usage__*` scenarios failing
with clap's "unrecognized subcommand" — same as after Task 2, nothing here touched `main.rs`'s
`Cmd` enum.

- [ ] **Step 4: Commit**

```bash
git add crates/ratchet/src/usage/mod.rs crates/ratchet/src/usage/attribute.rs
git commit -m "feat(usage): attribute calls to tasks, roles and models"
```

---

### Task 4: `cli::usage_cmd` and the `Cmd::Usage` wiring (implementer)

**Files:**
- Create: `crates/ratchet/src/cli/usage_cmd.rs`
- Modify: `crates/ratchet/src/cli/mod.rs` (one `pub mod usage_cmd;` line, alphabetical),
  `crates/ratchet/src/main.rs` (the `Cmd::Usage` variant and its dispatch arm — mirror the
  `Cmd::Map` arm exactly, including the `--session` plumbing)

**Interfaces:**
- Consumes (all frozen by Tasks 2–3, do not extend them here):
  `usage::transcript::{Usage, Call, ParseResult, Meta, parse, read_meta, slug_for,
  transcript_path, subagents_dir}`; `usage::weights::{matching, cost}`;
  `usage::attribute::{Totals, AttributedCall, Bucket, SubagentEvent, Hold, holds, task_at,
  orientation, subagent_task_and_role, group_totals, buckets_of, totals_by_role_model,
  rounds_tokens, orchestrator_share, cache_efficiency, task_cost, abbreviate, since_cutoff,
  display_role}`; `services::{tasks, events, sessions}` (reads only);
  `config::load_machine_config`; `output::{emit, emit_json}`; `clock::now`.
- Produces (frozen for Task 5): `pub fn run(...) -> i32` plus the `pub(crate)` seam Task 5
  builds on: `struct Collected { tasks, buckets, skipped, partial }` and
  `pub(crate) fn collect(...) -> Result<Collected, String>` and
  `pub(crate) fn one_task_summary(&Collected, task_id) -> String`.

**CLI surface this task owns** (clap structs in `usage_cmd.rs`, `--help` text included):
`usage` (default listing), `usage <id>` (one task), `--by task|role|model|session`,
`--since <7d|30d|date>`, `--all-repos`, `--json`. `--note` is EXPLICITLY NOT this task's:
adding the flag now would turn scenario 24 green early and steal Task 5's green criterion.
If you are tempted, re-read the allocation table instead.

**Green criterion for this task:** scenarios 1–23 and 27 green; scenarios 24–26 still fail (with
clap's "unexpected argument '--note'" — the mirror image of Task 2/3's "unrecognized
subcommand", expected and correct here):

```bash
cargo test -p ratchet --test spec usage::
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 1: Wire `Cmd::Usage` through `main.rs` and `cli/mod.rs`**

Add the variant and arm following `Cmd::Map` line for line (same `Face`-less thin dispatch:
parse args, call `usage_cmd::run`, return its code). Unlike `Map`, the arm passes the global
`Cli.session` (`session.as_deref()`) into `usage_cmd::run`, exactly as the `Cmd::Task` arm does,
because Task 5's `--note` attributes its write through `sessions::resolve` (explicit `--session`
> `RATCHET_SESSION_ID` > directory) and Task 5 may not touch `main.rs`. No other logic in
`main.rs` beyond the arm — a reviewer diffs this step in under a minute.

- [ ] **Step 2: Write the collection pass (`collect`)**

```rust
pub(crate) struct Collected {
    pub tasks: Vec<TaskRow>,        // id, title, status, touched_at
    pub calls: Vec<AttributedCall>, // every parsed call, task resolved or None
    pub buckets: Vec<Bucket>,       // via attribute::buckets_of (weights applied when configured)
    pub skipped: Vec<String>,       // "session <id>: no transcript" / "nothing understood", counted once each
    pub partial: u64,               // garbage lines + records without usage, summed
}
```

Resolution order inside `collect`, with no exceptions:
1. Projects dir: `RATCHET_CLAUDE_PROJECTS` when set and non-empty, else
   `~/.claude/projects`. A missing directory is scenario 3's error and names the path —
   fail before touching the database.
2. Open the database with `db::open_ready(home)` — the read-only opener every sibling face uses
   (`task_cmd.rs`, `session_cmd.rs`); NEVER `db::open`, which creates a schemaless file when
   none exists, NEVER `db::connect`, which migrates, and NEVER `events::emit`: scenario 26
   asserts zero new rows and zero files changed.
3. Sessions: `sessions::list(conn, repo_name)` for this repo, unless `--all-repos`. Note it
   filters by the repo's display name (the `repo` column), not by `repo_root` — pass the name
   `find_repo` resolves, as `session_cmd.rs` does.
4. Per session: `transcript::transcript_path` → missing file counts one `skipped` entry with
   the `no transcript` substring (scenario 2) and moves on; present file → `parse`, folding
   `ParseResult` counts into `partial` and skipping empty results with one `skipped` entry
   (scenarios 4–5 — the substrings `skipped ` and `partial ` come from here).
5. Holds: session events (`task.claimed`, `task.status`, `session.end`) → `attribute::holds`
   (Assumption 3: windows open on `task.claimed`, close on `done`/`blocked`/`ready` or session
   end; `review` leaves them open).
6. Orchestrator calls: `attribute::task_at(&holds, ts)` → `Some(task)` or `unassigned`.
7. Subagent transcripts under `subagents_dir`: `read_meta` for `tool_use_id`/`agent_type`;
   the `SubagentEvent` for `subagent_task_and_role` is built from the row's `task_id` column
   (Assumption 1, settled — query `subagent.start`/`subagent.stop` rows for this session and
   take `task`, NEVER the payload); fall through toolUseId → first timestamp per the
   frozen helper.
8. Window: `since_cutoff` (`7d` default, `--since` override) against `touched_at`
   (Assumption 2: `events::for_task(conn, id, 1)`).

- [ ] **Step 3: Write the four renders plus `--json`**

  - Default listing: one line per task in the window — id, title, status, `rounds ` (via
    `rounds_tokens`), and the four token classes abbreviated (`abbreviate` — the `k`/`M`
    substrings of scenario 22 come from here, never from ad-hoc formatting).
  - `usage <id>`: one line per role × model (`totals_by_role_model`, role via
    `display_role`), then `orientation` — computed per session with `orientation` and shown as
    `average_totals` over the task's sessions (Requirement 4; no scenario gives a task two
    sessions, so add a unit test in `usage_cmd.rs` with two sessions whose orientation differs
    and assert the average, never the sum) — then `rounds` (`rounds_tokens`), orchestrator
    share (`orchestrator_share`), cache efficiency (`cache_efficiency`).
  - `--by task|role|model|session`: `group_totals` across the window; the role shape appends
    `review rounds per task` and `orientation per session` (scenario 21's stable substrings),
    the latter again `average_totals` across the sessions in the window.
  - Cost column (listing and buckets): `task_cost` / `weights::cost` — present only when
    weights matched (scenarios 16–18); a non-matching model hides the whole column, never a
    partial sum (Assumption 4).
  - `--json`: one object — `tasks` (each with `id`, `title`, `status`, `rounds`,
    `orientation`, `buckets` with `session`/`role`/`model`/`tokens{input,cache_write,
    cache_read,output,thinking}` and `cost` only when matched) plus top-level `skipped`,
    `partial`, `version` (scenario 23 asserts shape, never exact numbers).
  - Anything over the terminal budget goes through `output::emit` / `output::emit_json`
    (the long-output rule — reuse, do not reimplement).

- [ ] **Step 4: Run the green criterion.** Expected: `usage__*` 1–23 PASS; 24–26 fail on the
  missing `--note` flag; clippy/fmt clean.

- [ ] **Step 5: Commit**

```bash
git add crates/ratchet/src/cli/usage_cmd.rs crates/ratchet/src/cli/mod.rs crates/ratchet/src/main.rs
git commit -m "feat(usage): report token cost per task, role and model"
```

---

### Task 5: `--note` plus README and doctrine (implementer)

**Files:**
- Modify: `crates/ratchet/src/cli/usage_cmd.rs` (add the `--note` flag and its write path —
  nothing else in this file changes), `README.md` (new `## Usage` section),
  `docs/agent-doctrine.md` (one new line).

**Interfaces:**
- Consumes: Task 4's frozen `collect` + `one_task_summary` (do not reshape them — if the
  summary paragraph needs a field `collect` does not return, that is a Task 4 defect: stop
  and report it, do not reach around into Task 3 internals from the CLI layer).
- Produces: nothing frozen — this is the last production task.

**Green criterion for this task:** scenarios 24–26 green (all 27 green), gate clean:

```bash
cargo test -p ratchet --test spec usage::
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

- [ ] **Step 1: Add `--note` (only valid with `usage <id>`)**

`ratchet usage <id> --note` appends a task note whose text starts with `usage: ` followed
by the one-task summary paragraph (total tokens abbreviated, rounds, orchestrator share —
`one_task_summary`, shared with the `<id>` render so the two can never drift). The write
goes through `services::tasks::note` — the SAME function `ratchet task note` calls, so a
missing task is refused with the identical message and no note lands anywhere (scenario 25
asserts message equality, not a paraphrase). `--note` without `<id>` is a clap error. The
flag changes no render.

- [ ] **Step 2: Prove the no-write default.** Scenario 26 (`without_note_nothing_is_written`)
  runs plain `usage <id>` and asserts the events table gained no row and the state directory
  gained no file. If it fails, the bug is a write smuggled into Task 4's collection (an
  `emit`, a `connect`/migrate, or a `map note`-style side file) — fix it there, not by
  weakening the test.

- [ ] **Step 3: Document (no test).** README gains `## Usage` after the `## Map` section:
  what `ratchet usage` reports, the four `--by` shapes, `--since`/`--all-repos`/`--json`,
  weights config (`[usage]` machine config, longest-prefix match), and the `--note` write
  (the command's ONLY write). `docs/agent-doctrine.md` gains one line under the task flow:
  cost questions are answered with `ratchet usage`, never by reading transcripts by hand.

- [ ] **Step 4: Run the green criterion.** Expected: all 27 `usage__*` green.

- [ ] **Step 5: Commit**

```bash
git add crates/ratchet/src/cli/usage_cmd.rs README.md docs/agent-doctrine.md
git commit -m "feat(usage): --note writes the one-task summary, plus docs"
```

---

### Task 6: Review (reviewer)

**Files:** none — this task writes no code and fixes nothing. A blocking finding is described
(back to the owning task's implementer); a deviation is judged (correct-as-implemented, with
the reasoning, or spec-text-must-change).

**Green criterion for this task:** the full gate below is green AND every check below it is
answered in the review note:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test -p ratchet
```

- [ ] **Step 1: Spec ↔ tests ↔ code, 1:1.** Every `#### Scenario` of
  `openspec/specs/usage/spec.md` names its `usage__*` test (27/27, no orphans either way);
  every stable substring in Task 1's list is produced by the module Task 1's table says
  produces it. `scenarios::every_scenario_has_a_test` is green.
- [ ] **Step 2: Adversarial probes with doubles** (throwaway temp dirs, `RATCHET_HOME` and
  `RATCHET_CLAUDE_PROJECTS` pointed at fixtures — never the real database or transcripts):
  a transcript that is all garbage; a session id with path separators (slug must not escape
  the projects dir); weights whose prefixes overlap (`anthropic/claude` vs `anthropic` —
  longest wins); `--note` on a missing task (byte-identical refusal to `task note`);
  `--all-repos` spanning two repos; `--since` with an invalid spec (error, no panic).
- [ ] **Step 3: No-write audit.** `usage` without `--note` opens the database read-only in
  effect: no new event rows, no files under the state dir, no migration (`db::connect`
  appears nowhere on this command's path — grep it).
- [ ] **Step 4: Side-effect hunt.** `git diff --stat` against the merge base shows ONLY the
  files the File structure section assigns to Tasks 1–5; no `Cargo.toml`/`Cargo.lock`
  changes; no edits to `task_cmd.rs`, `session_cmd.rs`, `map_cmd.rs`, `pdf_cmd.rs`,
  `guardrails/`, `hooks/`; the four Assumptions at the top hold as written (Assumption 1 is
  now a settled fact — verify the loader really reads the row's `task_id`).
- [ ] **Step 5: Verdict.** One of APPROVE / APPROVE WITH NITS / CHANGES NEEDED, recorded as
  a board note on the implementing task with the gate output, the probes run, and every
  deviation judged. The group merges only on APPROVE (nits filed, not fixed in this task).

---
