# ratchet — Group 6: `ratchet map` (repo orientation without re-reading the repo)

> **Owner rulings (2026-09-21, before execution):**
> 1. Tasks 1–5 are implemented in the worktree `.worktrees/t-0004` (branch `map-spec`, which
>    already holds `openspec/specs/map/spec.md` and its scenario tests), never in the main
>    checkout: the `main-tree` guardrail is live for this repo (`ratchet.toml` is present even
>    though untracked) and blocks tracked-file edits there. Task 6 (wiring this repo's own
>    `CLAUDE.md` and `.gitignore`) runs from the main checkout by the orchestrator after the
>    branch is merged; it is the only step that needs the main tree.
> 2. The `agent_frontmatter` test accepts `model` in {`sonnet`, `opus`, `haiku`, `fable`,
>    `inherit`} or a full model id, and `effort` in {`low`, `medium`, `high`, `xhigh`, `max`}.
> 3. The map header date comes from `clock::now(env)` as `%Y-%m-%d`; the footer tree hash is
>    `git rev-parse HEAD^{tree}`; `[map] exclude` uses the simplified `*` glob. All confirmed.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `ratchet map`: a deterministic, model-free scan of the repo (`git ls-files`, file
headers, manifests, and a small local notes file) that writes `.ratchet/map.md`, capped at 150
lines, plus `map note`, `map --missing`, `map status`, `map --wire` (CLAUDE.md import +
`.gitignore`), a session-start briefing line reporting the map's freshness, the `/ratchet:map`
command and `mapper` agent for the deep (per-file description) pass, and this repo's own
`--wire` run with a hand-written 15-line `CLAUDE.md` block.

**Architecture:** One new module, `crates/ratchet/src/map.rs` (pure: takes a repo root, the
`[map]` config and a clock, returns map text and small result structs — no model, no network),
and a thin `crates/ratchet/src/cli/map_cmd.rs` face, following the same split as `pdf.rs` /
`cli/pdf_cmd.rs`. Git is reached through ad hoc `git -C <root> …` shell-outs, the same pattern as
`repo::git_branch`. `hooks/briefing.rs` gains one optional line, computed from `map.rs`, appended
to (and dropped first from) the existing 40-line-capped briefing. `commands/map.md` and
`agents/mapper.md` are markdown-only; the `mapper` agent is model `haiku`, effort `low`, and is
never exercised by an automated test — its ceiling is `ratchet map note`'s own validation, which
every test in this plan drives directly.

**Tech Stack:** Rust 2021 (rust-version 1.79), existing deps only: `clap`, `serde`/`serde_json`,
`toml`, `chrono`, `fancy-regex` (used to translate a `[map] exclude` glob into a regex — no new
glob crate). No new dependency anywhere in this group.

**Spec:** `docs/superpowers/specs/2026-09-21-ratchet-map-design.md` (the whole document; most
directly §2 decisions D-map-deterministic through D-map-config, §3 the map's own format, §4
components, §5 data flow, §6 error handling, §7 testing, §8 group 6 delivery plan). Also extends
`docs/superpowers/specs/2026-09-16-ratchet-plugin-design.md` (referenced only for the pre-existing
conventions this plan reuses: D-english, D-specs-first, D-roles, the `main_tree` guardrail).
Ported requirements land in `openspec/specs/map/spec.md` (Task 1).

## Global Constraints

- **Git is allowed and expected in this repo**, per the precedent set by group 5
  (`docs/superpowers/plans/2026-09-17-ratchet-group-5-release.md`). Work on the branch `g6-map`,
  directly in the main checkout at `/Users/eduardoillanes/Documents/ratchet` (**not** a
  worktree — see the note below on why). Commit per task, on top of the current branch
  (`decouple-manifest` at the time this plan was written; branch `g6-map` from whatever `HEAD`
  is when work starts). Never force-push. Never commit to `main` directly.
- **Why not a worktree, even though `agents/implementer.md` says "always work in a worktree":**
  `ratchet map`'s own root resolution (`repo::find_repo` → `main_root_of`) deliberately collapses
  a linked worktree's cwd back to the *main* checkout root (`repo.rs:63-86`) — the map always
  describes the main tree, by design. Task 6 of this plan runs `ratchet map --wire` against this
  very repo and needs its writes (`CLAUDE.md`, `.gitignore`) to land on the branch actually
  checked out in the main tree, not silently on whatever branch happens to be checked out there
  while a worktree does the real work. Working directly on `g6-map` in the main checkout keeps
  one branch, one set of files, no cross-checkout confusion.
- **The `main_tree` guardrail is not a blocker here.** It intercepts only the `Edit`, `Write`,
  `NotebookEdit` and `MultiEdit` *tool calls* against an already-tracked file in the main
  checkout outside `worktrees_dir` (`crates/ratchet/src/guardrails/main_tree.rs:11,40`) — it
  does not intercept `Bash`, and it does not intercept anything the `ratchet` binary itself does
  internally (`ratchet map --wire`'s `fs::write` calls are not a Claude Code tool call at all).
  This repo's `ratchet.toml` currently exists but is **untracked** (see repo memory
  `project-ratchet-plugin.md`), so whether guardrails are actually live for the session that
  executes this plan is itself uncertain. If an Edit/Write to a tracked file (e.g. `main.rs`,
  `README.md`) is ever blocked by `main_tree` while working this plan: **stop and tell the
  owner** — do not add the file to `[guardrails] off` and do not route around it by any other
  means.
- Every shell that runs cargo starts with:
  ```bash
  export PATH="$HOME/.cargo/bin:$PATH"
  cd /Users/eduardoillanes/Documents/ratchet
  ```
- The crate is **bin-only**: unit tests run with `cargo test -p ratchet --bin ratchet`. Never
  `--lib`; this plan adds no `[lib]` target.
- Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p ratchet`.
  **The full gate is expected to be RED from the end of Task 1 until the end of Task 5** — Task 1
  writes one test function per scenario of `openspec/specs/map/spec.md` before any of the five
  §8 items are implemented, so most `map__*` tests fail on their assertions (never on a compile
  error — every test spawns the real `ratchet` binary as a subprocess, so a CLI surface that
  doesn't exist yet just returns clap's "unrecognized subcommand/argument", exit code 2, which
  the assertions read as a normal failure). This is by design, exactly as group 3's pdf plan did
  it with 9 tests instead of 31. Each task below instead has its own green criterion: a specific
  `cargo test -p ratchet --test spec map::` invocation naming the subset of tests that must pass
  at the end of *that* task, plus "no previously-green test regresses." Nobody weakens a test to
  turn the full gate green early. By the end of Task 5 all 31 scenarios are green and the full
  gate is clean; Task 6 (this repo's own wiring) and Task 7 (review) run it to confirm, not to
  finish it.
- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are **not** expected to go
  red at any point — they check syntax and lints, not test outcomes. If clippy complains about
  unused `pub` items before the CLI wires them up, add `#[allow(dead_code)]` item-by-item (the
  same pattern used throughout groups 0-3), removed once the item is reachable.
- All content, identifiers, messages and docs in English (spec D-english, inherited from the
  plugin design).
- **The header date and the determinism contract.** The map's header line embeds a date. That
  date comes from `clock::now(env)` (`crates/ratchet/src/clock.rs`), the same seam every other
  face already uses — **never `Utc::now()` directly**. Every scenario test that asserts
  byte-identical output across two runs sets `RATCHET_NOW` to a fixed value for both runs (the
  existing `support::hook_env`/`support::cli` helpers already accept `extra: &[(&str, &str)]` for
  this). The commit sha and tree hash in the footer trailer come from `git`, not from the clock,
  so determinism across two runs also requires `HEAD` and `.ratchet/map.notes` to be unchanged
  between them — state this in the test, don't rely on it being obvious.
- **`session.repo_root` is lower-cased; the map's git operations must not use it.**
  `support.rs`'s own comment on `repo_root_key` says so explicitly: "the main root, absolute and
  lower-cased." `git -C <path> …` is case-sensitive on Linux and macOS (the same problem
  `repo::rel_for_git` already solves for a different caller, `repo.rs:205-220`). Task 4 threads
  a case-preserving `main_root: &Path` (from `repo::Repo::main_root`, resolved at the
  `session-start` hook, never from `Session.repo_root`) into `hooks::briefing::build` for exactly
  this reason.
- **`ratchet map` and `ratchet map status` do not share an exit-code contract.** `ratchet map`
  (bare, `--missing`, `--wire`) exits 1 outside a marker repo, matching every other CLI face that
  needs a repo. `ratchet map status` exits 0 always and prints nothing outside a marker repo
  (design §4.1) — it is meant to be safe to run unconditionally from a script or a hook. Don't
  conflate the two when writing tests or the CLI dispatch.
- `.ratchet/map.md` and `.ratchet/map.notes` live **inside the repo tree**
  (`<main_root>/.ratchet/…`), not under `RATCHET_HOME`/`~/.ratchet` — a different "state root"
  concept from `ratchet pdf`'s `<home>/out/pdf/`. They are local and untracked; `ratchet map`
  never writes a tracked file on its own (D-map-untracked). Wiring `.gitignore` to cover them is
  the explicit, separate `--wire` step.
- No new Cargo dependency. `[map] exclude` glob patterns are matched by translating `*` to `.*`
  and escaping everything else, then running the already-present `fancy-regex` crate — see
  Task 2, `map::is_excluded`.
- This plan explicitly does **not** touch `crates/ratchet/src/cli/config_cmd.rs`'s `TEMPLATE`
  constant (the `ratchet.toml` scaffold `ratchet config init` writes) to add a commented `[map]`
  section — `[pdf]` isn't in that template either (it lives in machine config, a different file),
  and `[map]` is fully optional with empty-vec defaults. Noted here so it isn't mistaken for an
  oversight.

---

## File structure

```
openspec/specs/map/spec.md              Task 1 — 11 requirements, 31 scenarios
crates/ratchet/tests/spec/
├── main.rs                             Task 1 — + `mod map;` (alphabetical, after `bootstrap`)
├── support.rs                          Task 1 — append only: map(), write_rust_module(),
│                                        write_python_module(), write_ts_module(), commit(),
│                                        head_sha(), read_map(), notes_path_of()
└── map.rs                              Task 1 — 31 scenario tests
crates/ratchet/src/
├── main.rs                             Task 2 (Cmd::Map, MapCmd::Status) — extended by
│                                        Task 3 (--missing/--all, MapCmd::Note) and Task 5 (--wire)
├── config.rs                           Task 2 — + MapSection, RepoConfig.map
├── map.rs                              Task 2 (generate/cap/gate/layout/modules/status) —
│                                        extended by Task 3 (note/missing) and Task 5 (wire)
└── cli/
    ├── mod.rs                          Task 2 — + `pub mod map_cmd;`
    └── map_cmd.rs                      Task 2 (generate, status) — extended by Task 3
                                         (note, missing) and Task 5 (the --wire branch)
crates/ratchet/src/hooks/
├── briefing.rs                         Task 4 — build() gains a `main_root: &Path` parameter
│                                        and the map freshness line, dropped first under the cap
└── dispatch.rs                         Task 4 — session_start's call to briefing::build
crates/ratchet/tests/
└── agent_frontmatter.rs                Task 5 — new, not scenario-covered (plain integration test)
commands/map.md                         Task 5
agents/mapper.md                        Task 5
README.md                               Task 5 (new "## Map" section, agent count correction)
docs/agent-doctrine.md                  Task 5 (one new paragraph for `mapper`)
CLAUDE.md, .gitignore                   Task 6 — created/extended by running `ratchet map --wire`
                                         for real, plus a hand-written ≤15-line block in CLAUDE.md
```

No parallelism: Tasks 2-6 each extend the same `map.rs` / `map_cmd.rs` / `main.rs` the prior task
created, strictly sequential, same branch.

---

## Scenario → task allocation

Every scenario below is written once, in Task 1, against `openspec/specs/map/spec.md`. This
table says which task is expected to turn each one green — use it to know what "no regression"
means at each step, and to know you're not responsible for a scenario outside your task.

| # | Scenario (test function name) | Turns green in |
|---|---|---|
| 1 | `two_runs_produce_byte_identical_output` | Task 2 |
| 2 | `generating_a_map_prints_the_wrote_line` | Task 2 |
| 3 | `map_generation_outside_a_marker_repo_fails` | Task 2 |
| 4 | `an_unwired_repo_gets_a_hint_naming_wire` | Task 2 |
| 5 | `a_rust_doc_comment_becomes_the_module_sentence` | Task 2 |
| 6 | `a_python_module_docstring_becomes_the_module_sentence` | Task 2 |
| 7 | `a_typescript_leading_comment_becomes_the_module_sentence` | Task 2 |
| 8 | `a_module_with_no_header_shows_a_placeholder` | Task 2 |
| 9 | `a_note_describes_a_header_less_file` | Task 3 |
| 10 | `a_header_always_wins_over_a_note` | Task 3 |
| 11 | `a_stale_note_is_dropped_at_the_next_generation` | Task 3 |
| 12 | `map_note_records_a_sentence_for_a_tracked_file` | Task 3 |
| 13 | `map_note_replaces_an_existing_note_for_the_same_path` | Task 3 |
| 14 | `map_note_refuses_a_path_that_is_not_tracked` | Task 3 |
| 15 | `map_note_refuses_an_invalid_sentence` | Task 3 |
| 16 | `map_note_refuses_an_empty_sentence` | Task 3 |
| 17 | `missing_lists_every_file_with_no_header_and_no_note` | Task 3 |
| 18 | `missing_narrows_to_files_changed_since_the_recorded_commit` | Task 3 |
| 19 | `missing_all_widens_back_to_every_undescribed_file` | Task 3 |
| 20 | `a_module_list_over_the_cap_collapses_into_directory_counts` | Task 2 |
| 21 | `map__wire_appends_the_claude_md_import_and_the_gitignore_entry` | Task 5 |
| 22 | `map__wire_run_twice_changes_nothing_the_second_time` | Task 5 |
| 23 | `map__wire_refuses_a_symlinked_claude_md` | Task 5 |
| 24 | `no_map_prints_the_map_none_line` | Task 4 |
| 25 | `a_map_behind_head_prints_the_commits_behind_line` | Task 4 |
| 26 | `a_map_from_another_branch_prints_the_from_another_branch_line` | Task 4 |
| 27 | `a_current_map_prints_no_line` | Task 4 |
| 28 | `map_status_prints_the_same_line_the_briefing_would_show` | Task 2 |
| 29 | `map_status_prints_nothing_outside_a_marker_repo` | Task 2 |
| 30 | `map_exclude_leaves_matching_files_out_of_the_map` | Task 2 |
| 31 | `map_gate_replaces_detected_gate_commands_entirely` | Task 2 |

Task 2 turns 13 green (1-8, 20, 28-31); Task 3 turns 11 green (9-19); Task 4 turns 4 green
(24-27); Task 5 turns 3 green (21-23) plus the non-scenario `agent_frontmatter` test. Task 6 adds
no new scenario; it re-runs the full suite and must find all 31 green already.

---

### Task 1: `openspec/specs/map/spec.md` and its scenario tests (spec-test-author)

**Files:**
- Create: `openspec/specs/map/spec.md`, `crates/ratchet/tests/spec/map.rs`
- Modify: `crates/ratchet/tests/spec/main.rs` (one `mod` line), `crates/ratchet/tests/spec/support.rs`
  (append only — every existing helper, including `Sandbox`, `sandbox()`, `git()`, `write()`,
  `ratchet_bin()`, `code`, `stdout`, `stderr`, is untouched and reused, not replaced)

**Interfaces:**
- Consumes: `crate::support::{Sandbox, sandbox, unmanaged_dir, git, ratchet_bin, code, stdout,
  stderr}` (all already in the repo — see `crates/ratchet/tests/spec/support.rs`).
- Produces: the 31 scenario titles and their exact `fn map__<slug>()` names (table above);
  `support::{map, write_rust_module, write_python_module, write_ts_module, write_plain_file,
  commit, head_sha, read_map, wired_claude_md, symlink_claude_md}` — the frozen contract every
  later task's CLI and `map.rs` code is judged against.

The author of this task reads only the spec (once Step 1 below is written) and this task — not
the design document beyond what is quoted here, and not Tasks 2-6's planned code.

**Stable message fragments** (the implementation guarantees these substrings, verbatim where
quoted in full, and nothing else about the wording): `not a ratchet-managed repo` ·
`wrote ` · `--wire` · `not a tracked file` · `at most 120 characters` · `must not be empty` ·
`not a regular file` · the four fixed briefing/status lines quoted in full below.

- [ ] **Step 1: Create the branch**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/eduardoillanes/Documents/ratchet
git switch -c g6-map
```

- [ ] **Step 2: Write `openspec/specs/map/spec.md`**

```markdown
# map

Repo orientation, derived deterministically from the repository itself — no model involved. See
`docs/superpowers/specs/2026-09-21-ratchet-map-design.md` for the full design.

## Purpose

Every session and every subagent starts by re-learning where things are. `ratchet map` derives a
capped, up-to-date map (`.ratchet/map.md`) from `git ls-files`, file headers and manifest files,
on demand, and reports its own freshness so nobody has to guess whether it's stale.

## Requirements

### Requirement: Map generation is deterministic
Two runs of `ratchet map` over the same tree, at the same commit, with the same recorded notes,
SHALL produce byte-identical `.ratchet/map.md` content.

#### Scenario: Two runs produce byte identical output
- **WHEN** `ratchet map` is run twice in a row over the same commit with `RATCHET_NOW` fixed
- **THEN** the two writes of `.ratchet/map.md` are byte-for-byte identical

### Requirement: `ratchet map` writes the map and reports on wiring
`ratchet map` SHALL write `.ratchet/map.md` and print one line naming the file, its line count
and how many modules have no description. Outside a repo with a `ratchet.toml` marker it SHALL
fail without writing anything. When `CLAUDE.md` does not import the map, or `.gitignore` does not
cover `.ratchet/`, it SHALL print one hint line per missing wire, naming `--wire`.

#### Scenario: Generating a map prints the wrote line
- **WHEN** `ratchet map` is run in a marker repo
- **THEN** it prints a line starting with `wrote ` naming `.ratchet/map.md`, its line count and
  the count of modules without a description, and the file exists afterward

#### Scenario: Map generation outside a marker repo fails
- **WHEN** `ratchet map` is run in a directory with no `ratchet.toml` above it
- **THEN** it fails, the message says the directory is not a ratchet-managed repo, and no
  `.ratchet/map.md` is written

#### Scenario: An unwired repo gets a hint naming wire
- **WHEN** `ratchet map` is run in a marker repo whose `CLAUDE.md` does not import the map and
  whose `.gitignore` does not cover `.ratchet/`
- **THEN** it prints two hint lines, each naming `--wire`

### Requirement: Header extraction per language
The first sentence of a file's leading header comment, cut at 120 characters, SHALL become its
module sentence: `//!` lines in Rust, a leading docstring in Python, a leading `/** */` or `//`
comment block in TypeScript.

#### Scenario: A Rust doc comment becomes the module sentence
- **WHEN** a tracked `.rs` file's first lines are a `//!` doc comment
- **THEN** the map's Modules section shows that file with the doc comment's first sentence

#### Scenario: A Python module docstring becomes the module sentence
- **WHEN** a tracked `.py` file opens with a triple-quoted module docstring
- **THEN** the map's Modules section shows that file with the docstring's first sentence

#### Scenario: A TypeScript leading comment becomes the module sentence
- **WHEN** a tracked `.ts` file opens with a `/** … */` block comment
- **THEN** the map's Modules section shows that file with the comment's first sentence

#### Scenario: A module with no header shows a placeholder
- **WHEN** a tracked source file has no recognised header comment and no note
- **THEN** the map's Modules section shows that file with a bare `—` in place of a sentence

### Requirement: Notes fill in where there is no header, and a header always wins
A note recorded through `ratchet map note` SHALL be shown for a file with no header comment. A
file that has both a header comment and a note SHALL show the header comment. A note for a path
that no longer exists in the tree SHALL be dropped, silently, the next time the map is generated.

#### Scenario: A note describes a header-less file
- **WHEN** a note is recorded for a tracked file with no header comment, and the map is
  regenerated
- **THEN** the map's Modules section shows that file with the note's sentence

#### Scenario: A header always wins over a note
- **WHEN** a note is recorded for a tracked file that also has a header comment, and the map is
  regenerated
- **THEN** the map's Modules section shows that file with the header comment's sentence, not the
  note

#### Scenario: A stale note is dropped at the next generation
- **WHEN** a note exists for a path that has since been deleted from the tree, and the map is
  regenerated
- **THEN** the generation succeeds and the note for the deleted path no longer appears in
  `.ratchet/map.notes`

### Requirement: `map note` records one description, or refuses
`ratchet map note <path> "<sentence>"` SHALL record one sentence for a tracked file, replacing
any existing note for the same path. It SHALL refuse, with exit code 1 and one stderr line and
without changing `.ratchet/map.notes`, a path that is not a tracked file, a sentence over 120
characters or containing a newline, or an empty sentence.

#### Scenario: map note records a sentence for a tracked file
- **WHEN** `ratchet map note` is run naming a tracked file and a short sentence
- **THEN** it exits 0 and `.ratchet/map.notes` records that sentence for that path

#### Scenario: map note replaces an existing note for the same path
- **WHEN** `ratchet map note` is run twice for the same tracked file with two different sentences
- **THEN** `.ratchet/map.notes` holds only the second sentence for that path

#### Scenario: map note refuses a path that is not tracked
- **WHEN** `ratchet map note` is run naming a path that is not a tracked file
- **THEN** it exits 1, the message says the path is not a tracked file, and
  `.ratchet/map.notes` is unchanged

#### Scenario: map note refuses an invalid sentence
- **WHEN** `ratchet map note` is run with a sentence over 120 characters, and separately with a
  sentence containing a newline
- **THEN** both calls exit 1, the message says the sentence must be a single line of at most 120
  characters, and `.ratchet/map.notes` is unchanged

#### Scenario: map note refuses an empty sentence
- **WHEN** `ratchet map note` is run with an empty sentence
- **THEN** it exits 1, the message says the sentence must not be empty, and
  `.ratchet/map.notes` is unchanged

### Requirement: `--missing` lists undescribed source files, full and incremental
`ratchet map --missing` SHALL list, one path per line, the tracked source files with neither a
header comment nor a note. With a map already present, it SHALL narrow that list to files changed
since the map's recorded commit; `--all` SHALL widen it back to every such file regardless of the
recorded commit.

#### Scenario: Missing lists every file with no header and no note
- **WHEN** `ratchet map --missing` is run with no map present yet, over a tree with some files
  that have headers and some that don't
- **THEN** it lists exactly the files with neither a header nor a note, one per line

#### Scenario: Missing narrows to files changed since the recorded commit
- **WHEN** a map exists, a new commit adds one more header-less file, and `ratchet map --missing`
  is run
- **THEN** it lists only the file added since the map's recorded commit, not the header-less files
  that already existed when the map was generated

#### Scenario: Missing --all widens back to every undescribed file
- **WHEN** the same repo as the previous scenario is queried with `ratchet map --missing --all`
- **THEN** it lists every header-less, note-less file in the tree, not just the one added since
  the recorded commit

### Requirement: A map over the line cap collapses directories, deepest first
When the generated map would exceed 150 lines, the Modules section SHALL collapse the deepest
directories' file lists into one summary line each (`<dir>/ — <n> files`), deepest first, until
the map fits the cap or nothing more can be collapsed.

#### Scenario: A module list over the cap collapses into directory counts
- **WHEN** a tree has enough tracked source files that an uncollapsed map would exceed 150 lines
- **THEN** the generated map is at most 150 lines, and its Modules section shows at least one
  `<dir>/ — <n> files` summary line instead of per-file lines for that directory

### Requirement: `--wire` writes the import and the gitignore entry, idempotently
`ratchet map --wire` SHALL append `.ratchet/` to `.gitignore` (creating it if absent) and append
an import block to `CLAUDE.md` (creating it if absent) that imports `.ratchet/map.md`, and SHALL
change neither file when run again with both already in place. It SHALL refuse, without writing
the map, when `CLAUDE.md` or `.gitignore` exists but is not a regular file.

#### Scenario: Wire appends the CLAUDE.md import and the gitignore entry
- **WHEN** `ratchet map --wire` is run in a marker repo with neither file wired yet
- **THEN** `CLAUDE.md` now imports `.ratchet/map.md` and `.gitignore` now covers `.ratchet/`

#### Scenario: Wire run twice changes nothing the second time
- **WHEN** `ratchet map --wire` is run a second time immediately after the first
- **THEN** neither `CLAUDE.md` nor `.gitignore` changes, and the command says both are already
  wired

#### Scenario: Wire refuses a symlinked CLAUDE.md
- **WHEN** `CLAUDE.md` is a symlink instead of a regular file
- **THEN** `ratchet map --wire` fails, the message says it is not a regular file, and neither
  `CLAUDE.md` nor `.gitignore` is modified

### Requirement: The briefing reports the map's freshness
Session start SHALL append one line to the briefing when a marker repo has no map, a map whose
recorded commit is behind `HEAD`, or a map recorded from a commit that is not an ancestor of
`HEAD`. A current map SHALL add no line.

#### Scenario: No map prints the map none line
- **WHEN** a session starts in a marker repo with no `.ratchet/map.md`
- **THEN** the briefing contains the line `map: none — run /ratchet:map for the repo layout`

#### Scenario: A map behind HEAD prints the commits behind line
- **WHEN** a map is generated, one more commit is made, and a session starts
- **THEN** the briefing contains a line of the form `map: 1 commits behind — run /ratchet:map`

#### Scenario: A map from another branch prints the from another branch line
- **WHEN** a map is generated, then the branch is reset so the map's recorded commit is no longer
  an ancestor of `HEAD`, and a session starts
- **THEN** the briefing contains the line `map: from another branch — run /ratchet:map`

#### Scenario: A current map prints no line
- **WHEN** a map is generated and a session starts immediately after, with no further commits
- **THEN** the briefing contains no line starting with `map:`

### Requirement: `ratchet map status` mirrors the briefing line
`ratchet map status` SHALL print, in a marker repo, exactly the line the briefing would show, or
`map: current` when the map is up to date. Outside a marker repo it SHALL exit 0 and print
nothing.

#### Scenario: Map status prints the same line the briefing would show
- **WHEN** a map is behind `HEAD` and `ratchet map status` is run
- **THEN** it prints exactly the same `map: … commits behind …` line the briefing would show

#### Scenario: Map status prints nothing outside a marker repo
- **WHEN** `ratchet map status` is run in a directory with no `ratchet.toml` above it
- **THEN** it exits 0 and prints nothing

### Requirement: `[map]` config overrides exclusion and gate detection
`[map] exclude` in `ratchet.toml` SHALL keep matching files out of the map entirely. `[map] gate`
SHALL replace detected gate commands with the configured list, verbatim.

#### Scenario: Map exclude leaves matching files out of the map
- **WHEN** `[map] exclude` names a glob matching a tracked file, and the map is generated
- **THEN** that file appears nowhere in the generated map

#### Scenario: Map gate replaces detected gate commands entirely
- **WHEN** `[map] gate` is set to a custom command list in a repo that would otherwise detect a
  Cargo-based gate
- **THEN** the map's Gate section shows exactly the configured commands and none of the detected
  ones
```

- [ ] **Step 3: Confirm the checker sees 31 uncovered scenarios**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/eduardoillanes/Documents/ratchet
cargo test -p ratchet --test scenarios
```
Expected: `every_scenario_has_a_test` FAILS listing exactly 31 `map:` scenarios (and no other
spec regresses). Record the 31 names — they must match the allocation table exactly.

- [ ] **Step 4: Append map helpers to `crates/ratchet/tests/spec/support.rs`**

Reuses `Sandbox`, `sandbox()` and `git()` as they already exist — no new struct. Append at the
end of the file, under a new section comment:

```rust
// --- group 6: map ----------------------------------------------------------------------------

/// Run `ratchet map <args>` against the sandbox, from its root.
pub fn map(sb: &Sandbox, args: &[&str]) -> Output {
    Command::new(ratchet_bin())
        .arg("map")
        .args(args)
        .current_dir(sb.root())
        .env("RATCHET_HOME", sb.home.path())
        .env_remove("RATCHET_SESSION_ID")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap()
}

/// Like `map`, but with extra environment (`RATCHET_NOW`, mainly).
pub fn map_env(sb: &Sandbox, args: &[&str], extra: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(ratchet_bin());
    cmd.arg("map")
        .args(args)
        .current_dir(sb.root())
        .env("RATCHET_HOME", sb.home.path())
        .env_remove("RATCHET_SESSION_ID")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

/// Writes a `.rs` file with a `//!` doc-comment header (or none, when `header` is `None`) and a
/// short body — enough to be a plausible Rust module without needing real code.
pub fn write_rust_module(sb: &Sandbox, rel: &str, header: Option<&str>, body: &str) -> PathBuf {
    let text = match header {
        Some(h) => format!("//! {h}\n\n{body}\n"),
        None => format!("{body}\n"),
    };
    sb.write(rel, &text)
}

/// Writes a `.py` file with a triple-quoted module docstring (or none).
pub fn write_python_module(sb: &Sandbox, rel: &str, header: Option<&str>, body: &str) -> PathBuf {
    let text = match header {
        Some(h) => format!("\"\"\"{h}\"\"\"\n\n{body}\n"),
        None => format!("{body}\n"),
    };
    sb.write(rel, &text)
}

/// Writes a `.ts` file with a `/** … */` leading block comment (or none).
pub fn write_ts_module(sb: &Sandbox, rel: &str, header: Option<&str>, body: &str) -> PathBuf {
    let text = match header {
        Some(h) => format!("/**\n * {h}\n */\n\n{body}\n"),
        None => format!("{body}\n"),
    };
    sb.write(rel, &text)
}

/// Writes a file with no recognised header at all — any extension, any content.
pub fn write_plain_file(sb: &Sandbox, rel: &str, body: &str) -> PathBuf {
    sb.write(rel, body)
}

/// `git add <files> && git commit -m <msg>` in the sandbox's repo — the only way a fixture file
/// becomes visible to `git ls-files`, which is what `ratchet map` actually reads.
pub fn commit(sb: &Sandbox, files: &[&str], msg: &str) {
    let root = sb.repo.path();
    let mut args = vec!["add"];
    args.extend_from_slice(files);
    git(root, &args);
    git(root, &["commit", "-q", "-m", msg]);
}

/// `HEAD`'s full sha, from the sandbox's repo.
pub fn head_sha(sb: &Sandbox) -> String {
    let out = Command::new("git")
        .args(["-C", &sb.repo.path().to_string_lossy(), "rev-parse", "HEAD"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The written map's text, or empty when absent.
pub fn read_map(sb: &Sandbox) -> String {
    fs::read_to_string(sb.root().join(".ratchet/map.md")).unwrap_or_default()
}

/// `.ratchet/map.notes`'s text, or empty when absent.
pub fn read_notes(sb: &Sandbox) -> String {
    fs::read_to_string(sb.root().join(".ratchet/map.notes")).unwrap_or_default()
}

/// Replaces `CLAUDE.md` at the sandbox root with a symlink to `target` — used only by the
/// `--wire` refusal scenario. Unix-only (`cfg(unix)`), matching every other symlink-dependent
/// test convention in this repo (there are none yet outside this one; keep it gated the same way
/// the rest of the suite gates anything OS-specific).
#[cfg(unix)]
pub fn symlink_claude_md(sb: &Sandbox, target: &Path) {
    use std::os::unix::fs::symlink;
    let link = sb.root().join("CLAUDE.md");
    let _ = fs::remove_file(&link);
    symlink(target, &link).unwrap();
}

/// Run `ratchet map <args>` with no sandbox at all — for the "outside a marker repo" scenarios,
/// where `dir` deliberately has no `ratchet.toml` above it (`unmanaged_dir()`).
pub fn map_at(dir: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(ratchet_bin())
        .arg("map")
        .args(args)
        .current_dir(dir)
        .env("RATCHET_HOME", home)
        .env_remove("RATCHET_SESSION_ID")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap()
}
```

- [ ] **Step 5: Register the module**

`crates/ratchet/tests/spec/main.rs`, one line, alphabetical (after `bootstrap`, before `pdf`):

```rust
#![allow(non_snake_case)]

mod agent_protocol;
mod board;
mod bootstrap;
mod map;
mod pdf;
mod sessions;
mod support;
mod tasks;
```

- [ ] **Step 6: Write `crates/ratchet/tests/spec/map.rs` — part 1 of 2 (scenarios 1-20)**

```rust
//! Scenario tests for `openspec/specs/map/spec.md`. Every `#### Scenario` there has exactly one
//! test here, named by slug. No test involves a model — `mapper` (haiku) is never exercised;
//! everything a model would otherwise do (recording a note) is driven directly through
//! `ratchet map note`.

use crate::support::{
    code, commit, hook_env, map, map_at, map_env, read_map, read_notes, sandbox, session_payload,
    stderr, stdout, unmanaged_dir, write_plain_file, write_python_module, write_rust_module,
    write_ts_module, T0,
};

// --- determinism, generation, headers (Task 2) ---------------------------------------------

#[test]
fn map__two_runs_produce_byte_identical_output() {
    let sb = sandbox();
    write_rust_module(&sb, "src/a.rs", Some("Does a thing."), "pub fn a() {}");
    commit(&sb, &["src/a.rs"], "add a");
    let out1 = map_env(&sb, &[], &[("RATCHET_NOW", T0)]);
    assert_eq!(code(&out1), 0, "{}", stderr(&out1));
    let first = read_map(&sb);
    let out2 = map_env(&sb, &[], &[("RATCHET_NOW", T0)]);
    assert_eq!(code(&out2), 0, "{}", stderr(&out2));
    let second = read_map(&sb);
    assert_eq!(first, second, "two runs over the same commit must be byte-identical");
    assert!(!first.is_empty());
}

#[test]
fn map__generating_a_map_prints_the_wrote_line() {
    let sb = sandbox();
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let s = stdout(&out);
    let first_line = s.lines().next().unwrap();
    assert!(first_line.starts_with("wrote "), "{first_line}");
    assert!(first_line.contains(".ratchet/map.md"), "{first_line}");
    assert!(sb.root().join(".ratchet/map.md").is_file());
}

#[test]
fn map__map_generation_outside_a_marker_repo_fails() {
    let dir = unmanaged_dir();
    let home = tempfile::TempDir::new().unwrap();
    let out = map_at(dir.path(), home.path(), &[]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("not a ratchet-managed repo"), "{}", stderr(&out));
    assert!(!dir.path().join(".ratchet/map.md").exists());
}

#[test]
fn map__an_unwired_repo_gets_a_hint_naming_wire() {
    let sb = sandbox();
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let hints: Vec<&str> = stdout(&out).lines().filter(|l| l.contains("--wire")).collect();
    assert_eq!(hints.len(), 2, "{:?}", stdout(&out));
}

#[test]
fn map__a_rust_doc_comment_becomes_the_module_sentence() {
    let sb = sandbox();
    write_rust_module(&sb, "src/widget.rs", Some("Computes the widget checksum"), "pub fn f() {}");
    commit(&sb, &["src/widget.rs"], "add widget");
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    let line = text.lines().find(|l| l.contains("src/widget.rs")).expect(&text);
    assert!(line.contains("Computes the widget checksum"), "{line}");
}

#[test]
fn map__a_python_module_docstring_becomes_the_module_sentence() {
    let sb = sandbox();
    write_python_module(&sb, "src/widget.py", Some("Computes the widget checksum"), "def f(): pass");
    commit(&sb, &["src/widget.py"], "add widget");
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    let line = text.lines().find(|l| l.contains("src/widget.py")).expect(&text);
    assert!(line.contains("Computes the widget checksum"), "{line}");
}

#[test]
fn map__a_typescript_leading_comment_becomes_the_module_sentence() {
    let sb = sandbox();
    write_ts_module(&sb, "src/widget.ts", Some("Computes the widget checksum"), "export function f() {}");
    commit(&sb, &["src/widget.ts"], "add widget");
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    let line = text.lines().find(|l| l.contains("src/widget.ts")).expect(&text);
    assert!(line.contains("Computes the widget checksum"), "{line}");
}

#[test]
fn map__a_module_with_no_header_shows_a_placeholder() {
    let sb = sandbox();
    write_rust_module(&sb, "src/bare.rs", None, "pub fn f() {}");
    commit(&sb, &["src/bare.rs"], "add bare");
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    let line = text.lines().find(|l| l.contains("src/bare.rs")).expect(&text);
    assert!(line.trim_end().ends_with('—'), "{line}");
}

// --- notes (Task 3) --------------------------------------------------------------------------

#[test]
fn map__a_note_describes_a_header_less_file() {
    let sb = sandbox();
    write_rust_module(&sb, "src/bare.rs", None, "pub fn f() {}");
    commit(&sb, &["src/bare.rs"], "add bare");
    let n = map(&sb, &["note", "src/bare.rs", "Handles the bare case."]);
    assert_eq!(code(&n), 0, "{}", stderr(&n));
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    let line = text.lines().find(|l| l.contains("src/bare.rs")).expect(&text);
    assert!(line.contains("Handles the bare case."), "{line}");
}

#[test]
fn map__a_header_always_wins_over_a_note() {
    let sb = sandbox();
    write_rust_module(&sb, "src/both.rs", Some("The real header sentence"), "pub fn f() {}");
    commit(&sb, &["src/both.rs"], "add both");
    let n = map(&sb, &["note", "src/both.rs", "A note that should never show."]);
    assert_eq!(code(&n), 0, "{}", stderr(&n));
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    let line = text.lines().find(|l| l.contains("src/both.rs")).expect(&text);
    assert!(line.contains("The real header sentence"), "{line}");
    assert!(!line.contains("A note that should never show"), "{line}");
}

#[test]
fn map__a_stale_note_is_dropped_at_the_next_generation() {
    let sb = sandbox();
    write_rust_module(&sb, "src/gone.rs", None, "pub fn f() {}");
    commit(&sb, &["src/gone.rs"], "add gone");
    let n = map(&sb, &["note", "src/gone.rs", "Will be deleted."]);
    assert_eq!(code(&n), 0, "{}", stderr(&n));
    std::fs::remove_file(sb.root().join("src/gone.rs")).unwrap();
    crate::support::git(sb.repo.path(), &["rm", "-q", "src/gone.rs"]);
    crate::support::git(sb.repo.path(), &["commit", "-q", "-m", "remove gone"]);
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!read_notes(&sb).contains("src/gone.rs"), "{}", read_notes(&sb));
}

// --- `map note` (Task 3) ----------------------------------------------------------------------

#[test]
fn map__map_note_records_a_sentence_for_a_tracked_file() {
    let sb = sandbox();
    write_rust_module(&sb, "src/x.rs", None, "pub fn f() {}");
    commit(&sb, &["src/x.rs"], "add x");
    let out = map(&sb, &["note", "src/x.rs", "Does the x thing."]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(read_notes(&sb).contains("src/x.rs"), "{}", read_notes(&sb));
    assert!(read_notes(&sb).contains("Does the x thing."), "{}", read_notes(&sb));
}

#[test]
fn map__map_note_replaces_an_existing_note_for_the_same_path() {
    let sb = sandbox();
    write_rust_module(&sb, "src/x.rs", None, "pub fn f() {}");
    commit(&sb, &["src/x.rs"], "add x");
    map(&sb, &["note", "src/x.rs", "First sentence."]);
    let out = map(&sb, &["note", "src/x.rs", "Second sentence."]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let notes = read_notes(&sb);
    assert!(!notes.contains("First sentence."), "{notes}");
    assert!(notes.contains("Second sentence."), "{notes}");
    assert_eq!(notes.lines().filter(|l| l.contains("src/x.rs")).count(), 1, "{notes}");
}

#[test]
fn map__map_note_refuses_a_path_that_is_not_tracked() {
    let sb = sandbox();
    let out = map(&sb, &["note", "src/never-added.rs", "Whatever."]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("not a tracked file"), "{}", stderr(&out));
    assert!(read_notes(&sb).is_empty());
}

#[test]
fn map__map_note_refuses_an_invalid_sentence() {
    let sb = sandbox();
    write_rust_module(&sb, "src/x.rs", None, "pub fn f() {}");
    commit(&sb, &["src/x.rs"], "add x");
    let long = "a".repeat(121);
    let out1 = map(&sb, &["note", "src/x.rs", &long]);
    assert_eq!(code(&out1), 1);
    assert!(stderr(&out1).contains("at most 120 characters"), "{}", stderr(&out1));
    let out2 = map(&sb, &["note", "src/x.rs", "line one\nline two"]);
    assert_eq!(code(&out2), 1);
    assert!(stderr(&out2).contains("at most 120 characters"), "{}", stderr(&out2));
    assert!(read_notes(&sb).is_empty());
}

#[test]
fn map__map_note_refuses_an_empty_sentence() {
    let sb = sandbox();
    write_rust_module(&sb, "src/x.rs", None, "pub fn f() {}");
    commit(&sb, &["src/x.rs"], "add x");
    let out = map(&sb, &["note", "src/x.rs", ""]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("must not be empty"), "{}", stderr(&out));
    assert!(read_notes(&sb).is_empty());
}

// --- `--missing` (Task 3) ----------------------------------------------------------------------

#[test]
fn map__missing_lists_every_file_with_no_header_and_no_note() {
    let sb = sandbox();
    write_rust_module(&sb, "src/a.rs", Some("Has a header."), "pub fn a() {}");
    write_rust_module(&sb, "src/b.rs", None, "pub fn b() {}");
    write_rust_module(&sb, "src/c.rs", None, "pub fn c() {}");
    commit(&sb, &["src/a.rs", "src/b.rs", "src/c.rs"], "add a b c");
    let out = map(&sb, &["--missing"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let lines: Vec<&str> = stdout(&out).lines().collect();
    assert!(lines.contains(&"src/b.rs"), "{lines:?}");
    assert!(lines.contains(&"src/c.rs"), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("src/a.rs")), "{lines:?}");
}

#[test]
fn map__missing_narrows_to_files_changed_since_the_recorded_commit() {
    let sb = sandbox();
    write_rust_module(&sb, "src/old.rs", None, "pub fn a() {}");
    commit(&sb, &["src/old.rs"], "add old");
    let gen = map(&sb, &[]);
    assert_eq!(code(&gen), 0, "{}", stderr(&gen));
    write_rust_module(&sb, "src/new.rs", None, "pub fn b() {}");
    commit(&sb, &["src/new.rs"], "add new");
    let out = map(&sb, &["--missing"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let lines: Vec<&str> = stdout(&out).lines().collect();
    assert_eq!(lines, vec!["src/new.rs"], "{lines:?}");
}

#[test]
fn map__missing_all_widens_back_to_every_undescribed_file() {
    let sb = sandbox();
    write_rust_module(&sb, "src/old.rs", None, "pub fn a() {}");
    commit(&sb, &["src/old.rs"], "add old");
    map(&sb, &[]);
    write_rust_module(&sb, "src/new.rs", None, "pub fn b() {}");
    commit(&sb, &["src/new.rs"], "add new");
    let out = map(&sb, &["--missing", "--all"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let lines: Vec<&str> = stdout(&out).lines().collect();
    assert!(lines.contains(&"src/old.rs"), "{lines:?}");
    assert!(lines.contains(&"src/new.rs"), "{lines:?}");
}

// --- 150-line cap (Task 2) ---------------------------------------------------------------------

#[test]
fn map__a_module_list_over_the_cap_collapses_into_directory_counts() {
    let sb = sandbox();
    let mut files = Vec::new();
    for n in 0..200 {
        let rel = format!("src/gen/f{n:04}.rs");
        write_rust_module(&sb, &rel, None, "pub fn f() {}");
        files.push(rel);
    }
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    commit(&sb, &refs, "add 200 generated files");
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    assert!(text.lines().count() <= 150, "{} lines", text.lines().count());
    assert!(
        text.lines().any(|l| l.contains("src/gen/") && l.contains("files")),
        "expected a collapsed directory summary line:\n{text}"
    );
}
```

- [ ] **Step 7: Write `crates/ratchet/tests/spec/map.rs` — part 2 of 2 (scenarios 21-31)**

Append to the same file, after the Step 6 content, before the final closing of the module (there
is no `mod` wrapper in this file — every `#[test]` fn sits at the top level, same as `pdf.rs`):

```rust
// --- `--wire` (Task 5) --------------------------------------------------------------------------

#[test]
fn map__wire_appends_the_claude_md_import_and_the_gitignore_entry() {
    let sb = sandbox();
    let out = map(&sb, &["--wire"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let claude = std::fs::read_to_string(sb.root().join("CLAUDE.md")).unwrap();
    assert!(claude.contains("@.ratchet/map.md"), "{claude}");
    let gitignore = std::fs::read_to_string(sb.root().join(".gitignore")).unwrap();
    assert!(gitignore.contains(".ratchet/"), "{gitignore}");
}

#[test]
fn map__wire_run_twice_changes_nothing_the_second_time() {
    let sb = sandbox();
    let first = map(&sb, &["--wire"]);
    assert_eq!(code(&first), 0, "{}", stderr(&first));
    let claude_after_first = std::fs::read_to_string(sb.root().join("CLAUDE.md")).unwrap();
    let gitignore_after_first = std::fs::read_to_string(sb.root().join(".gitignore")).unwrap();
    let second = map(&sb, &["--wire"]);
    assert_eq!(code(&second), 0, "{}", stderr(&second));
    assert_eq!(
        std::fs::read_to_string(sb.root().join("CLAUDE.md")).unwrap(),
        claude_after_first
    );
    assert_eq!(
        std::fs::read_to_string(sb.root().join(".gitignore")).unwrap(),
        gitignore_after_first
    );
    assert!(stdout(&second).contains("already wired"), "{}", stdout(&second));
}

#[cfg(unix)]
#[test]
fn map__wire_refuses_a_symlinked_claude_md() {
    let sb = sandbox();
    let target = sb.repo.path().join("elsewhere.md");
    std::fs::write(&target, "not really CLAUDE.md").unwrap();
    crate::support::symlink_claude_md(&sb, &target);
    let out = map(&sb, &["--wire"]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("not a regular file"), "{}", stderr(&out));
    assert!(
        !sb.root().join(".gitignore").exists(),
        "gitignore must not be touched when CLAUDE.md refuses"
    );
}

// --- briefing (Task 4) ---------------------------------------------------------------------------

#[test]
fn map__no_map_prints_the_map_none_line() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-none", &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("map: none — run /ratchet:map for the repo layout"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn map__a_map_behind_head_prints_the_commits_behind_line() {
    let sb = sandbox();
    write_rust_module(&sb, "src/a.rs", None, "pub fn a() {}");
    commit(&sb, &["src/a.rs"], "add a");
    map(&sb, &[]);
    write_rust_module(&sb, "src/b.rs", None, "pub fn b() {}");
    commit(&sb, &["src/b.rs"], "add b");
    let root = sb.root();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-behind", &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("map: 1 commits behind — run /ratchet:map"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn map__a_map_from_another_branch_prints_the_from_another_branch_line() {
    let sb = sandbox();
    write_rust_module(&sb, "src/a.rs", None, "pub fn a() {}");
    commit(&sb, &["src/a.rs"], "add a");
    map(&sb, &[]);
    // Amending the last commit orphans the recorded sha: it becomes a sibling, not an ancestor,
    // of the new HEAD — exactly what `git merge-base --is-ancestor` reports as `false`.
    crate::support::git(sb.repo.path(), &["commit", "--amend", "-q", "-m", "amended"]);
    let root = sb.root();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-other", &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("map: from another branch — run /ratchet:map"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn map__a_current_map_prints_no_line() {
    let sb = sandbox();
    write_rust_module(&sb, "src/a.rs", None, "pub fn a() {}");
    commit(&sb, &["src/a.rs"], "add a");
    map(&sb, &[]);
    let root = sb.root();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-current", &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!stdout(&out).lines().any(|l| l.contains("map:")), "{}", stdout(&out));
}

// --- `map status` (Task 2) -----------------------------------------------------------------------

#[test]
fn map__map_status_prints_the_same_line_the_briefing_would_show() {
    let sb = sandbox();
    write_rust_module(&sb, "src/a.rs", None, "pub fn a() {}");
    commit(&sb, &["src/a.rs"], "add a");
    map(&sb, &[]);
    write_rust_module(&sb, "src/b.rs", None, "pub fn b() {}");
    commit(&sb, &["src/b.rs"], "add b");
    let root = sb.root();
    let briefing = hook_env(
        &sb,
        "session-start",
        &session_payload("s-cmp", &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    let status = map(&sb, &["status"]);
    assert_eq!(code(&status), 0, "{}", stderr(&status));
    let briefing_line = stdout(&briefing)
        .lines()
        .find(|l| l.starts_with("map:"))
        .unwrap()
        .to_string();
    assert_eq!(stdout(&status).trim(), briefing_line);
}

#[test]
fn map__map_status_prints_nothing_outside_a_marker_repo() {
    let dir = unmanaged_dir();
    let home = tempfile::TempDir::new().unwrap();
    let out = map_at(dir.path(), home.path(), &["status"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).trim().is_empty(), "{}", stdout(&out));
    assert!(stderr(&out).trim().is_empty(), "{}", stderr(&out));
}

// --- `[map]` config (Task 2) ----------------------------------------------------------------------

#[test]
fn map__map_exclude_leaves_matching_files_out_of_the_map() {
    let sb = sandbox();
    write_rust_module(&sb, "vendor/blob.rs", Some("Vendored, should never appear."), "pub fn f() {}");
    write_rust_module(&sb, "src/a.rs", Some("Ours, should appear."), "pub fn a() {}");
    commit(&sb, &["vendor/blob.rs", "src/a.rs"], "add vendor and a");
    sb.write_marker(
        "[repo]\ndefault_branch = \"main\"\nworktrees_dir = \".worktrees\"\n\n[map]\nexclude = [\"vendor/*\"]\n",
    );
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    assert!(!text.contains("vendor/blob.rs"), "{text}");
    assert!(text.contains("src/a.rs"), "{text}");
}

#[test]
fn map__map_gate_replaces_detected_gate_commands_entirely() {
    let sb = sandbox();
    write_plain_file(&sb, "Cargo.toml", "[package]\nname = \"x\"\nversion = \"0.1.0\"\n");
    commit(&sb, &["Cargo.toml"], "add Cargo.toml");
    sb.write_marker(
        "[repo]\ndefault_branch = \"main\"\nworktrees_dir = \".worktrees\"\n\n[map]\ngate = [\"make check\"]\n",
    );
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    assert!(text.contains("make check"), "{text}");
    assert!(!text.contains("cargo fmt"), "{text}");
}
```

- [ ] **Step 8: Run and confirm red-clean**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/eduardoillanes/Documents/ratchet
cargo test -p ratchet --test spec map::        # compiles (no `ratchet map` yet), every test FAILs
cargo test -p ratchet --test scenarios         # every_scenario_has_a_test now passes for `map:`
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: all 31 `map__*` tests fail on assertions (the `map` subcommand doesn't exist yet, so
every call exits 2 with clap's "unrecognized subcommand" — read as a normal assertion failure,
not a compile error) — compiling and failing for the right reason. `every_scenario_has_a_test`
now passes for the `map:` spec. Gate (`fmt`, `clippy`) is clean; `cargo test -p ratchet` as a
whole is red (expected, per Global Constraints) because of these 31 failures.

- [ ] **Step 9: Commit and hand off**

```bash
git add openspec/specs/map/spec.md crates/ratchet/tests/spec/map.rs \
        crates/ratchet/tests/spec/main.rs crates/ratchet/tests/spec/support.rs
git commit -m "test: scenario tests for ratchet map (group 6)

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

List the four files. State: `support.rs`'s map helpers (`map`, `map_env`, `map_at`,
`write_rust_module`, `write_python_module`, `write_ts_module`, `write_plain_file`, `commit`,
`read_map`, `read_notes`, `symlink_claude_md`) and every exact stable message fragment quoted
above are the frozen contract — Task 2 through Task 5's CLI and `map.rs` code must match them
exactly; a needed change to any of them is a fix round on this task, not a silent edit by
whoever implements next.

---

### Task 2: `map` module core — generate, cap, `[map]` config, `status` (implementer)

**Files:**
- Create: `crates/ratchet/src/map.rs`, `crates/ratchet/src/cli/map_cmd.rs`
- Modify: `crates/ratchet/src/config.rs` (add `MapSection`, add one field to `RepoConfig`),
  `crates/ratchet/src/cli/mod.rs` (one line), `crates/ratchet/src/main.rs` (`Cmd::Map`,
  `MapCmd::Status`, alphabetical `mod map;`)

**Interfaces:**
- Produces: `map::{MapError, MapSettings, Generated, Freshness, generate, write_map, map_path,
  notes_path, recorded_commit, freshness, briefing_line, status_line, is_excluded}`;
  `config::MapSection`, `RepoConfig.map`. Task 3 adds `note`/`missing` to this same file. Task 4
  consumes `map::briefing_line` from `hooks::briefing`. Task 5 adds `wire`/`WireResult` to this
  same file and extends `cli::map_cmd`.
- Consumes: nothing from Task 1's test file — only the frozen contract (message fragments,
  `support.rs` helpers) that file already commits to. Reuses `repo::find_repo`,
  `repo::is_tracked`, `repo::rel_for_git` (all already `pub`/`pub(crate)` in `repo.rs`) and
  `clock::now`.

This task does not read `crates/ratchet/tests/spec/**`. It is verified against the 13 scenario
tests Task 1 already wrote and committed, by name (allocation table above), plus its own unit
tests.

- [ ] **Step 1: Add `MapSection` to `config.rs`**

Add this struct after `GuardrailsSection`, and add one field to `RepoConfig`:

```rust
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MapSection {
    /// Directory globs (`*` matches any run of characters, including `/`) left out of the map
    /// entirely.
    pub exclude: Vec<String>,
    /// When non-empty, replaces gate detection entirely — printed verbatim, one per line.
    pub gate: Vec<String>,
}
```

In `RepoConfig`:

```rust
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RepoConfig {
    pub repo: RepoSection,
    pub guardrails: GuardrailsSection,
    pub thresholds: Thresholds,
    pub map: MapSection,
}
```

Add one test to `config.rs`'s existing `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn map_section_default_and_parsed() {
        let c = parse_repo_config("", Path::new("ratchet.toml")).unwrap();
        assert!(c.map.exclude.is_empty());
        assert!(c.map.gate.is_empty());

        let text = "[map]\nexclude = [\"vendor/*\"]\ngate = [\"make check\"]\n";
        let c = parse_repo_config(text, Path::new("ratchet.toml")).unwrap();
        assert_eq!(c.map.exclude, vec!["vendor/*"]);
        assert_eq!(c.map.gate, vec!["make check"]);
    }
```

- [ ] **Step 2: Write `crates/ratchet/src/map.rs`, failing tests first**

```rust
//! Deterministic repo orientation: `git ls-files`, file headers, manifests and a small local
//! notes file, written to `<repo root>/.ratchet/map.md`. No model is involved anywhere in this
//! file — the `mapper` agent (Task 5) only ever calls `ratchet map note`, the same command a
//! person can run by hand.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::{DateTime, Utc};

use crate::config::MapSection;
use crate::repo::is_tracked;

/// A map is context in every prompt; it never grows with the repo (design D-map-cap).
pub const MAP_LINE_CAP: usize = 150;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapError(String);

impl MapError {
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MapError {}

fn err(msg: impl Into<String>) -> MapError {
    MapError(msg.into())
}

pub fn map_path(main_root: &Path) -> PathBuf {
    main_root.join(".ratchet").join("map.md")
}

pub fn notes_path(main_root: &Path) -> PathBuf {
    main_root.join(".ratchet").join("map.notes")
}

// --- git plumbing ---------------------------------------------------------------------------

fn git_output(main_root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(["-C", &main_root.to_string_lossy()])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn tracked_files(main_root: &Path) -> Result<Vec<String>, MapError> {
    let out = Command::new("git")
        .args(["-C", &main_root.to_string_lossy(), "ls-files"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stderr(Stdio::null())
        .output()
        .map_err(|e| err(format!("git ls-files failed: {e}")))?;
    if !out.status.success() {
        return Err(err("git ls-files failed"));
    }
    let mut files: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.replace('\\', "/"))
        .filter(|l| !l.is_empty())
        .collect();
    files.sort();
    Ok(files)
}

// --- `[map] exclude` -------------------------------------------------------------------------

/// Translates a glob (`*` matches any run of characters, including `/`) into an anchored regex
/// and checks `rel_path` against it. No new dependency: reuses `fancy-regex`, already a crate
/// dependency for the guardrail rules.
pub fn is_excluded(rel_path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| {
        let mut re = String::from("^");
        for ch in p.chars() {
            match ch {
                '*' => re.push_str(".*"),
                '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                    re.push('\\');
                    re.push(ch);
                }
                other => re.push(other),
            }
        }
        re.push('$');
        fancy_regex::Regex::new(&re)
            .ok()
            .and_then(|r| r.is_match(rel_path).ok())
            .unwrap_or(false)
    })
}

// --- header extraction ------------------------------------------------------------------------

enum Style {
    RustDoc,
    PyDocstring,
    BlockOrLineComment,
    ShellShebang,
}

/// Design §3.4's language table. TS/JS/Go/Java/Kotlin/Swift/C/C++ share one comment style.
const LANG_TABLE: &[(&[&str], Style)] = &[
    (&["rs"], Style::RustDoc),
    (&["py"], Style::PyDocstring),
    (
        &["ts", "tsx", "js", "jsx", "go", "java", "kt", "swift", "c", "h", "cpp", "hpp", "cc"],
        Style::BlockOrLineComment,
    ),
    (&["sh", "bash"], Style::ShellShebang),
];

fn style_for(ext: &str) -> Option<&'static Style> {
    LANG_TABLE.iter().find(|(exts, _)| exts.contains(&ext)).map(|(_, s)| s)
}

/// First sentence, cut at 120 characters (design §3.4).
fn cut_sentence(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    let end = s.find(". ").map(|i| i + 1).unwrap_or(s.len());
    let cut = s[..end].trim();
    let cut = cut.trim_end_matches('.');
    if cut.chars().count() > 119 {
        format!("{}…", cut.chars().take(119).collect::<String>())
    } else {
        format!("{cut}.")
    }
}

fn read_head(path: &Path, n: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = fs::File::open(path).ok()?;
    let mut buf = vec![0u8; n];
    let got = f.read(&mut buf).ok()?;
    buf.truncate(got);
    Some(buf)
}

/// The file's header sentence, or `None` when it has no header this table recognises, or the
/// head is not valid UTF-8. Reads at most 4 KB (design §4.1).
fn header_sentence(path: &Path) -> Option<String> {
    let bytes = read_head(path, 4096)?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let style = style_for(&ext)?;
    let raw = match style {
        Style::RustDoc => {
            let lines: Vec<&str> = text.lines().take_while(|l| l.starts_with("//!")).collect();
            if lines.is_empty() {
                return None;
            }
            lines.iter().map(|l| l.trim_start_matches("//!").trim()).collect::<Vec<_>>().join(" ")
        }
        Style::PyDocstring => {
            let t = text.trim_start();
            let quote = if t.starts_with("\"\"\"") {
                "\"\"\""
            } else if t.starts_with("'''") {
                "'''"
            } else {
                return None;
            };
            let rest = &t[quote.len()..];
            let end = rest.find(quote)?;
            rest[..end].split_whitespace().collect::<Vec<_>>().join(" ")
        }
        Style::BlockOrLineComment => {
            let t = text.trim_start();
            if let Some(rest) = t.strip_prefix("/**").or_else(|| t.strip_prefix("/*")) {
                let end = rest.find("*/")?;
                rest[..end]
                    .lines()
                    .map(|l| l.trim().trim_start_matches('*').trim())
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else if t.starts_with("//") {
                let lines: Vec<&str> =
                    t.lines().take_while(|l| l.trim_start().starts_with("//")).collect();
                lines
                    .iter()
                    .map(|l| l.trim_start().trim_start_matches("//").trim())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                return None;
            }
        }
        Style::ShellShebang => {
            let mut lines = text.lines();
            if !lines.next()?.starts_with("#!") {
                return None;
            }
            let rest: Vec<&str> = lines.take_while(|l| l.starts_with('#')).collect();
            if rest.is_empty() {
                return None;
            }
            rest.iter().map(|l| l.trim_start_matches('#').trim()).collect::<Vec<_>>().join(" ")
        }
    };
    if raw.trim().is_empty() {
        None
    } else {
        Some(cut_sentence(&raw))
    }
}

// --- notes (full command lands in Task 3; generate() only reads the file here) ---------------

/// `path: sentence` per line (design D-map-notes). Malformed lines are skipped, not an error —
/// a hand-edited or partially-written notes file must never break generation.
fn load_notes(main_root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(text) = fs::read_to_string(notes_path(main_root)) else {
        return out;
    };
    for line in text.lines() {
        if let Some((path, sentence)) = line.split_once(": ") {
            out.insert(path.to_string(), sentence.to_string());
        }
    }
    out
}

// --- gate detection (unit-tested directly; only the Cargo branch is scenario-tested) ---------

fn detect_gate(root: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    if root.join("Cargo.toml").is_file() {
        lines.push("cargo fmt --all -- --check".to_string());
        lines.push("cargo clippy --all-targets -- -D warnings".to_string());
        lines.push("cargo test".to_string());
    }
    if root.join("pyproject.toml").is_file() || root.join("pytest.ini").is_file() {
        let uses_uv = root.join("uv.lock").is_file();
        lines.push(if uses_uv { "uv run pytest".to_string() } else { "pytest".to_string() });
        let pyproject = fs::read_to_string(root.join("pyproject.toml")).unwrap_or_default();
        if pyproject.contains("[tool.ruff]") {
            lines.push("ruff check".to_string());
        }
        if pyproject.contains("[tool.mypy]") {
            lines.push("mypy".to_string());
        }
    }
    if let Ok(text) = fs::read_to_string(root.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let runner = if root.join("pnpm-lock.yaml").is_file() {
                "pnpm"
            } else if root.join("yarn.lock").is_file() {
                "yarn"
            } else {
                "npm"
            };
            if let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) {
                for script in ["test", "lint", "typecheck"] {
                    if scripts.contains_key(script) {
                        lines.push(format!("{runner} run {script}"));
                    }
                }
            }
        }
    }
    if let Ok(text) = fs::read_to_string(root.join("Makefile")) {
        for target in ["test", "lint"] {
            if text.lines().any(|l| l.starts_with(&format!("{target}:"))) {
                lines.push(format!("make {target}"));
            }
        }
    }
    if root.join("go.mod").is_file() {
        lines.push("go test ./...".to_string());
        lines.push("go vet ./...".to_string());
    }
    lines
}

fn gate_section(root: &Path, cfg: &MapSection) -> Vec<String> {
    if !cfg.gate.is_empty() {
        cfg.gate.clone()
    } else {
        detect_gate(root)
    }
}
```

Continued in Step 3 (the rest of `map.rs`: layout/modules/tests/docs sections, the 150-line
collapse, `generate`, `write_map`, freshness/briefing/status) and Step 4 (the CLI face). Do not
run the gate until Step 5 — the file does not compile yet (it is one `impl` block short of
`generate`, added next).

- [ ] **Step 3: Append the rest of `map.rs`**

```rust
// --- layout ------------------------------------------------------------------------------------

fn summarize(files: &[&String]) -> (usize, String) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for f in files {
        if let Some(ext) = Path::new(f.as_str()).extension().and_then(|e| e.to_str()) {
            *counts.entry(format!(".{ext}")).or_insert(0) += 1;
        }
    }
    let mut by_count: Vec<(String, usize)> = counts.into_iter().collect();
    by_count.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let top2: Vec<String> = by_count.into_iter().take(2).map(|(e, _)| e).collect();
    (files.len(), top2.join(" "))
}

const SOURCE_MARKERS: &[&str] = &["src", "crates", "lib", "app", "cmd", "pkg", "packages"];
const ENTRY_POINTS: &[&str] =
    &["main.rs", "lib.rs", "__main__.py", "main.py", "index.ts", "index.js", "main.go"];

/// Returns the Layout section's text and the list of directories the Modules section should
/// treat as "source" (top-level dirs the manifest marks as source, plus one level under
/// `crates/*`/`packages/*`) — design §3.3.
fn layout_section(files: &[String]) -> (String, Vec<String>) {
    let mut top: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for f in files {
        if let Some((dir, _)) = f.split_once('/') {
            top.entry(dir.to_string()).or_default().push(f);
        }
    }
    let mut lines = Vec::new();
    let mut source_dirs = Vec::new();
    for (dir, dir_files) in &top {
        let (n, exts) = summarize(dir_files);
        lines.push(format!("{dir}/  {n} files  {exts}"));
        if SOURCE_MARKERS.contains(&dir.as_str()) {
            source_dirs.push(dir.clone());
        }
    }
    for dir in ["crates", "packages"] {
        if !top.contains_key(dir) {
            continue;
        }
        let mut subs: BTreeMap<String, Vec<&String>> = BTreeMap::new();
        for f in &top[dir] {
            let mut parts = f.splitn(3, '/');
            parts.next();
            if let (Some(sub), Some(_rest)) = (parts.next(), parts.next()) {
                subs.entry(format!("{dir}/{sub}")).or_default().push(f);
            }
        }
        for (sub, sub_files) in &subs {
            let (n, exts) = summarize(sub_files);
            lines.push(format!("  {sub}/  {n} files  {exts}"));
            source_dirs.push(sub.clone());
        }
    }
    let entries: Vec<&str> = files
        .iter()
        .filter(|f| {
            let name = f.rsplit('/').next().unwrap_or(f);
            ENTRY_POINTS.contains(&name) || f.starts_with("bin/")
        })
        .map(String::as_str)
        .collect();
    if !entries.is_empty() {
        lines.push(format!("Entry points: {}", entries.join(", ")));
    }
    if lines.is_empty() {
        (String::new(), source_dirs)
    } else {
        (format!("## Layout\n\n{}\n", lines.join("\n")), source_dirs)
    }
}

// --- modules -------------------------------------------------------------------------------

fn modules_section(
    root: &Path,
    files: &[String],
    source_dirs: &[String],
    notes: &BTreeMap<String, String>,
) -> (Vec<(PathBuf, String)>, usize, usize, usize) {
    let mut rows = Vec::new();
    let (mut with_header, mut with_note, mut without) = (0, 0, 0);
    for f in files {
        let ext = Path::new(f).extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
        let recognised_lang = ext.as_deref().map(|e| style_for(e).is_some()).unwrap_or(false);
        let under_source = source_dirs.iter().any(|d| f.starts_with(&format!("{d}/")));
        if !recognised_lang && !under_source {
            continue;
        }
        let sentence = if let Some(h) = header_sentence(&root.join(f)) {
            with_header += 1;
            h
        } else if let Some(n) = notes.get(f) {
            with_note += 1;
            n.clone()
        } else {
            without += 1;
            "—".to_string()
        };
        rows.push((PathBuf::from(f), format!("{f} — {sentence}")));
    }
    (rows, with_header, with_note, without)
}

/// Collapses the deepest directories first (most path components; ties broken by directory path,
/// ascending, for determinism), replacing their per-file lines with one `<dir>/ — <n> files`
/// line, until `overshoot` lines have been saved or nothing more can be collapsed.
fn collapse_modules(rows: &[(PathBuf, String)], overshoot: usize) -> Vec<String> {
    let mut by_dir: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for (path, _) in rows {
        *by_dir.entry(path.parent().unwrap_or(Path::new("")).to_path_buf()).or_insert(0) += 1;
    }
    let mut dirs: Vec<PathBuf> = by_dir.keys().cloned().collect();
    dirs.sort_by(|a, b| b.components().count().cmp(&a.components().count()).then(a.cmp(b)));
    let mut collapsed: std::collections::BTreeSet<PathBuf> = Default::default();
    let mut saved = 0usize;
    for dir in &dirs {
        if saved >= overshoot {
            break;
        }
        let n = by_dir[dir];
        if n <= 1 {
            continue; // collapsing one file to one line saves nothing
        }
        collapsed.insert(dir.clone());
        saved += n - 1;
    }
    let mut out = Vec::new();
    let mut emitted: std::collections::BTreeSet<PathBuf> = Default::default();
    for (path, line) in rows {
        let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
        if collapsed.contains(&dir) {
            if emitted.insert(dir.clone()) {
                out.push(format!("{}/ — {} files", dir.display(), by_dir[&dir]));
            }
        } else {
            out.push(line.clone());
        }
    }
    out
}

// --- tests, docs -----------------------------------------------------------------------------

fn tests_section(files: &[String]) -> String {
    let mut by_test_dir: BTreeMap<String, usize> = BTreeMap::new();
    let mut beside: BTreeMap<String, usize> = BTreeMap::new();
    for f in files {
        if f.starts_with("tests/") || f.contains("/tests/") {
            let dir = f.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_else(|| "tests".into());
            *by_test_dir.entry(dir).or_insert(0) += 1;
            continue;
        }
        let name = f.rsplit('/').next().unwrap_or(f);
        let beside_style = (name.starts_with("test_") && name.ends_with(".py"))
            || name.ends_with("_test.go")
            || name.ends_with(".test.ts");
        if beside_style {
            let dir = f.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_else(|| ".".into());
            *beside.entry(dir).or_insert(0) += 1;
        }
    }
    if by_test_dir.is_empty() && beside.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    for (dir, n) in &by_test_dir {
        lines.push(format!("{dir}/  {n} files"));
    }
    for (dir, n) in &beside {
        lines.push(format!("{dir}/  {n} test files"));
    }
    format!("## Tests\n\n{}\n", lines.join("\n"))
}

fn docs_section(files: &[String]) -> String {
    let mut lines = Vec::new();
    for name in ["README.md", "CLAUDE.md", "AGENTS.md"] {
        if files.iter().any(|f| f == name) {
            lines.push(name.to_string());
        }
    }
    let docs_count = files.iter().filter(|f| f.starts_with("docs/")).count();
    if docs_count > 0 {
        lines.push(format!("docs/  {docs_count} files"));
    }
    let specs: std::collections::BTreeSet<String> = files
        .iter()
        .filter_map(|f| f.strip_prefix("openspec/specs/"))
        .filter_map(|rest| rest.split('/').next())
        .map(str::to_string)
        .collect();
    if !specs.is_empty() {
        lines.push(format!("openspec/specs: {}", specs.into_iter().collect::<Vec<_>>().join(", ")));
    } else if files.iter().any(|f| f.starts_with("openspec/")) {
        lines.push("openspec/  present".to_string());
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("## Docs\n\n{}\n", lines.join("\n"))
    }
}

// --- wiring hints (detection only — the `--wire` write path is Task 5) -----------------------

fn wiring_hints(main_root: &Path) -> Vec<String> {
    let mut hints = Vec::new();
    let imports = fs::read_to_string(main_root.join("CLAUDE.md"))
        .map(|t| t.contains("@.ratchet/map.md"))
        .unwrap_or(false);
    if !imports {
        hints.push("hint: CLAUDE.md does not import the map — run `ratchet map --wire`".to_string());
    }
    let covers = fs::read_to_string(main_root.join(".gitignore"))
        .map(|t| t.lines().any(|l| matches!(l.trim(), ".ratchet/" | ".ratchet")))
        .unwrap_or(false);
    if !covers {
        hints.push("hint: .gitignore does not cover .ratchet/ — run `ratchet map --wire`".to_string());
    }
    hints
}

// --- generation ----------------------------------------------------------------------------

pub struct Generated {
    pub text: String,
    pub with_header: usize,
    pub with_note: usize,
    pub without: usize,
    pub hints: Vec<String>,
}

pub fn generate(main_root: &Path, cfg: &MapSection, now: DateTime<Utc>) -> Result<Generated, MapError> {
    let all_files = tracked_files(main_root)?;
    let files: Vec<String> = all_files.into_iter().filter(|f| !is_excluded(f, &cfg.exclude)).collect();

    let repo_name = main_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    let short_sha =
        git_output(main_root, &["rev-parse", "--short", "HEAD"]).ok_or_else(|| err("git rev-parse failed"))?;
    let full_sha = git_output(main_root, &["rev-parse", "HEAD"]).ok_or_else(|| err("git rev-parse failed"))?;
    let tree_sha = git_output(main_root, &["rev-parse", "HEAD^{tree}"]).unwrap_or_default();

    let header = format!(
        "# {repo_name} — map\n\ngenerated by ratchet map at {short_sha} ({}); do not edit, run /ratchet:map\n",
        now.format("%Y-%m-%d")
    );
    let gate_lines = gate_section(main_root, cfg);
    let gate = if gate_lines.is_empty() { String::new() } else { format!("\n## Gate\n\n{}\n", gate_lines.join("\n")) };

    let (layout, source_dirs) = layout_section(&files);
    let notes = load_notes(main_root);
    let (rows, with_header, with_note, without) = modules_section(main_root, &files, &source_dirs, &notes);
    let tests = tests_section(&files);
    let docs = docs_section(&files);

    let render_modules = |lines: &[String]| -> String {
        if lines.is_empty() { String::new() } else { format!("\n## Modules\n\n{}\n", lines.join("\n")) }
    };
    let module_lines: Vec<String> = rows.iter().map(|(_, l)| l.clone()).collect();
    let mut body = format!("{header}{gate}\n{layout}{}{tests}\n{docs}\n", render_modules(&module_lines));

    const FOOTER_LINES: usize = 2; // sections 1, 2, 7 never collapse (design §3)
    if body.lines().count() + FOOTER_LINES > MAP_LINE_CAP {
        let overshoot = body.lines().count() + FOOTER_LINES - MAP_LINE_CAP;
        let collapsed = collapse_modules(&rows, overshoot);
        body = format!("{header}{gate}\n{layout}{}{tests}\n{docs}\n", render_modules(&collapsed));
    }

    let footer = format!(
        "{with_header} files with a header, {with_note} with a note, {without} without either — run /ratchet:map --deep for the rest.\n<!-- ratchet-map commit={full_sha} tree={tree_sha} -->\n"
    );
    let hints = wiring_hints(main_root);
    Ok(Generated { text: format!("{body}{footer}"), with_header, with_note, without, hints })
}

pub fn write_map(main_root: &Path, generated: &Generated) -> Result<PathBuf, MapError> {
    let path = map_path(main_root);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| err(format!("could not create .ratchet/: {e}")))?;
    }
    fs::write(&path, &generated.text).map_err(|e| err(format!("could not write {}: {e}", path.display())))?;
    Ok(path)
}

// --- freshness: shared by `map status` (Task 2) and the briefing line (Task 4) ----------------

pub fn recorded_commit(main_root: &Path) -> Option<String> {
    let text = fs::read_to_string(map_path(main_root)).ok()?;
    let last = text.lines().last()?;
    rest_after(last, "<!-- ratchet-map commit=")
}

fn rest_after(line: &str, prefix: &str) -> Option<String> {
    line.strip_prefix(prefix)?.split_whitespace().next().map(str::to_string)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    NoMap,
    Behind(u64),
    OtherBranch,
    Current,
    Unknown,
}

pub fn freshness(main_root: &Path) -> Freshness {
    let Some(recorded) = recorded_commit(main_root) else {
        return Freshness::NoMap;
    };
    let Some(head) = git_output(main_root, &["rev-parse", "HEAD"]) else {
        return Freshness::Unknown;
    };
    if recorded == head {
        return Freshness::Current;
    }
    let is_ancestor = Command::new("git")
        .args(["-C", &main_root.to_string_lossy(), "merge-base", "--is-ancestor", &recorded, "HEAD"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !is_ancestor {
        return Freshness::OtherBranch;
    }
    match git_output(main_root, &["rev-list", "--count", &format!("{recorded}..HEAD")]) {
        Some(n) => n.trim().parse().map(Freshness::Behind).unwrap_or(Freshness::Unknown),
        None => Freshness::Unknown,
    }
}

/// The line the briefing appends, or `None` when the map is current or freshness can't be
/// determined (design §6: a briefing never costs a session on a git failure).
pub fn briefing_line(main_root: &Path) -> Option<String> {
    match freshness(main_root) {
        Freshness::NoMap => Some("map: none — run /ratchet:map for the repo layout".to_string()),
        Freshness::Behind(n) => Some(format!("map: {n} commits behind — run /ratchet:map")),
        Freshness::OtherBranch => Some("map: from another branch — run /ratchet:map".to_string()),
        Freshness::Current | Freshness::Unknown => None,
    }
}

/// `ratchet map status`'s one line — always something, unlike `briefing_line`.
pub fn status_line(main_root: &Path) -> String {
    briefing_line(main_root).unwrap_or_else(|| "map: current".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git(dir: &Path, args: &[&str]) {
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

    fn repo() -> tempfile::TempDir {
        let d = tempfile::TempDir::new().unwrap();
        git(d.path(), &["init", "-q", "-b", "main"]);
        d
    }

    #[test]
    fn cargo_manifest_detects_the_rust_gate() {
        let d = repo();
        fs::write(d.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        assert_eq!(
            detect_gate(d.path()),
            vec![
                "cargo fmt --all -- --check".to_string(),
                "cargo clippy --all-targets -- -D warnings".to_string(),
                "cargo test".to_string(),
            ]
        );
    }

    #[test]
    fn pyproject_detects_pytest_and_optional_ruff_mypy() {
        let d = repo();
        fs::write(d.path().join("pyproject.toml"), "[tool.ruff]\n[tool.mypy]\n").unwrap();
        assert_eq!(
            detect_gate(d.path()),
            vec!["pytest".to_string(), "ruff check".to_string(), "mypy".to_string()]
        );
        fs::write(d.path().join("uv.lock"), "").unwrap();
        assert_eq!(detect_gate(d.path())[0], "uv run pytest");
    }

    #[test]
    fn package_json_detects_npm_scripts_via_lockfile() {
        let d = repo();
        fs::write(d.path().join("package.json"), r#"{"scripts":{"test":"x","lint":"y"}}"#).unwrap();
        assert_eq!(detect_gate(d.path()), vec!["npm run test".to_string(), "npm run lint".to_string()]);
        fs::write(d.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_gate(d.path()), vec!["pnpm run test".to_string(), "pnpm run lint".to_string()]);
    }

    #[test]
    fn makefile_detects_test_and_lint_targets() {
        let d = repo();
        fs::write(d.path().join("Makefile"), "test:\n\techo t\nlint:\n\techo l\n").unwrap();
        assert_eq!(detect_gate(d.path()), vec!["make test".to_string(), "make lint".to_string()]);
    }

    #[test]
    fn go_mod_detects_go_test_and_vet() {
        let d = repo();
        fs::write(d.path().join("go.mod"), "module x\n").unwrap();
        assert_eq!(detect_gate(d.path()), vec!["go test ./...".to_string(), "go vet ./...".to_string()]);
    }

    #[test]
    fn map_gate_override_replaces_detection() {
        let d = repo();
        fs::write(d.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let cfg = MapSection { exclude: vec![], gate: vec!["make check".to_string()] };
        assert_eq!(gate_section(d.path(), &cfg), vec!["make check".to_string()]);
    }

    #[test]
    fn collapse_prefers_the_deepest_directory_first() {
        let rows: Vec<(PathBuf, String)> = vec![
            (PathBuf::from("a/x.rs"), "a/x.rs — one.".to_string()),
            (PathBuf::from("a/y.rs"), "a/y.rs — two.".to_string()),
            (PathBuf::from("a/b/z.rs"), "a/b/z.rs — three.".to_string()),
            (PathBuf::from("a/b/w.rs"), "a/b/w.rs — four.".to_string()),
        ];
        // Overshoot of 1: only the deepest directory (a/b, 2 components) collapses, not a/
        // (1 component) — even though both dirs have more than one file.
        let out = collapse_modules(&rows, 1);
        assert!(out.iter().any(|l| l.starts_with("a/b/ — 2 files")), "{out:?}");
        assert!(out.contains(&"a/x.rs — one.".to_string()));
        assert!(out.contains(&"a/y.rs — two.".to_string()));
    }

    #[test]
    fn excluded_paths_match_a_star_glob() {
        assert!(is_excluded("vendor/blob.rs", &["vendor/*".to_string()]));
        assert!(!is_excluded("src/vendor.rs", &["vendor/*".to_string()]));
    }

    #[test]
    fn a_missing_notes_file_is_an_empty_map() {
        let d = tempfile::TempDir::new().unwrap();
        assert!(load_notes(d.path()).is_empty());
    }
}
```

- [ ] **Step 4: Write `crates/ratchet/src/cli/map_cmd.rs`**

```rust
//! `ratchet map` (bare) and `ratchet map status`. `note`/`--missing` (Task 3) and the `--wire`
//! branch's body (Task 5) extend this file; the shapes below already accept their parameters so
//! neither later task edits a call site in `main.rs`.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::clock;
use crate::repo::find_repo;

fn fail(e: impl std::fmt::Display) -> i32 {
    eprintln!("error: {e}");
    1
}

fn not_a_repo(here: &std::path::Path) -> i32 {
    eprintln!(
        "error: not a ratchet-managed repo (no ratchet.toml found above {})",
        here.display()
    );
    1
}

/// `ratchet map` / `ratchet map --wire`. The wiring itself is Task 5; this task only prints the
/// hints `map::generate` already computes.
pub fn generate(_wire: bool, env: &HashMap<String, String>, cwd: Option<PathBuf>) -> i32 {
    let here = cwd.unwrap_or_else(|| PathBuf::from("."));
    let repo = match find_repo(&here) {
        Ok(Some(r)) => r,
        Ok(None) => return not_a_repo(&here),
        Err(e) => return fail(e),
    };
    let now = clock::now(env);
    let generated = match crate::map::generate(&repo.main_root, &repo.config.map, now) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    let path = match crate::map::write_map(&repo.main_root, &generated) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    println!(
        "wrote {} ({} lines, {} modules without a description)",
        path.display(),
        generated.text.lines().count(),
        generated.without
    );
    for hint in &generated.hints {
        println!("{hint}");
    }
    0
}

/// Exit 0 always; prints nothing outside a marker repo (design §4.1 — safe to run unconditionally).
pub fn status(cwd: Option<PathBuf>) -> i32 {
    let here = cwd.unwrap_or_else(|| PathBuf::from("."));
    match find_repo(&here) {
        Ok(Some(repo)) => {
            println!("{}", crate::map::status_line(&repo.main_root));
            0
        }
        Ok(None) => 0,
        Err(_) => 0,
    }
}
```

- [ ] **Step 5: Wire `cli/mod.rs` and `main.rs`**

`crates/ratchet/src/cli/mod.rs`, one line, alphabetical (after `db_cmd`, before `pdf_cmd`):

```rust
pub mod config_cmd;
pub mod db_cmd;
pub mod map_cmd;
pub mod pdf_cmd;
pub mod session_cmd;
pub mod task_cmd;
```

`crates/ratchet/src/main.rs`: add `mod map;` to the top module list (alphabetical, after `log`,
before `model`):

```rust
mod cli;
mod clock;
mod config;
mod db;
mod guardrails;
mod hooks;
mod log;
mod map;
mod model;
mod output;
mod pdf;
mod repo;
mod services;
```

Add a `Map` variant to the `Cmd` enum (after `Task`, before `Pdf`, matching the file-structure
order this plan lists them in):

```rust
    /// Repo orientation: a deterministic map of the tree, written to `.ratchet/map.md`.
    Map {
        #[command(subcommand)]
        cmd: Option<MapCmd>,
    },
```

Add the `MapCmd` enum, after `TaskCmd`:

```rust
#[derive(Subcommand)]
enum MapCmd {
    /// Print the map's freshness line, or `map: current`.
    Status,
}
```

Add the dispatch arm, after the `Cmd::Task { .. }` arm and before `Cmd::Pdf { .. }`:

```rust
        Cmd::Map { cmd } => match cmd {
            Some(MapCmd::Status) => cli::map_cmd::status(cwd),
            None => cli::map_cmd::generate(false, &env, cwd),
        },
```

- [ ] **Step 6: Run the green criterion**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/eduardoillanes/Documents/ratchet
cargo test -p ratchet --bin ratchet config:: map::
cargo test -p ratchet --test spec map:: 2>&1 | tee /tmp/g6-task2.txt
grep -E "^test map::" /tmp/g6-task2.txt | sort
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected in the `grep` output: exactly these 13 tests `... ok` —
`map__two_runs_produce_byte_identical_output`,
`map__generating_a_map_prints_the_wrote_line`,
`map__map_generation_outside_a_marker_repo_fails`,
`map__an_unwired_repo_gets_a_hint_naming_wire`,
`map__a_rust_doc_comment_becomes_the_module_sentence`,
`map__a_python_module_docstring_becomes_the_module_sentence`,
`map__a_typescript_leading_comment_becomes_the_module_sentence`,
`map__a_module_with_no_header_shows_a_placeholder`,
`map__a_module_list_over_the_cap_collapses_into_directory_counts`,
`map__map_status_prints_the_same_line_the_briefing_would_show`,
`map__map_status_prints_nothing_outside_a_marker_repo`,
`map__map_exclude_leaves_matching_files_out_of_the_map`,
`map__map_gate_replaces_detected_gate_commands_entirely` —
and the other 18 `map__*` tests `... FAILED` (not implemented yet; expected per the allocation
table). `config::` and the new `map::` unit tests (inside `--bin ratchet`) all pass. Gate
(`fmt`, `clippy`) is clean.

- [ ] **Step 7: Commit**

```bash
git add crates/ratchet/src/map.rs crates/ratchet/src/config.rs crates/ratchet/src/cli/mod.rs \
        crates/ratchet/src/cli/map_cmd.rs crates/ratchet/src/main.rs
git commit -m "feat: ratchet map — generate, cap, [map] config, status

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: `map note`, notes merge, `--missing` with incremental mode (implementer)

**Files:**
- Modify: `crates/ratchet/src/map.rs` (+ `note`, `missing`, `write_notes`),
  `crates/ratchet/src/cli/map_cmd.rs` (+ `note`, `missing`), `crates/ratchet/src/main.rs`
  (`MapCmd::Note`, `--missing`/`--all` flags on the `Map` variant)

**Interfaces:**
- Consumes: `map::{MapError, MapSection, load_notes, tracked_files, is_excluded, layout_section,
  style_for, header_sentence, recorded_commit, git_output}` (all Task 2, same file);
  `repo::{is_tracked, rel_for_git}` (already in the repo).
- Produces: `map::{note, missing}`; the `ratchet map note <path> "<sentence>"` and
  `ratchet map --missing [--all]` CLI surface. Nothing later depends on new names beyond these
  two — Task 4/5 don't call into this task's code.

- [ ] **Step 1: Append to `crates/ratchet/src/map.rs`**

Add near `load_notes` (notes are a small, related group):

```rust
fn write_notes(main_root: &Path, notes: &BTreeMap<String, String>) -> Result<(), MapError> {
    let path = notes_path(main_root);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| err(format!("could not create .ratchet/: {e}")))?;
    }
    let mut text = String::new();
    for (p, s) in notes {
        text.push_str(&format!("{p}: {s}\n"));
    }
    fs::write(&path, text).map_err(|e| err(format!("could not write {}: {e}", path.display())))
}

/// Records one description for a tracked file, replacing any existing note for the same path.
/// `target` may be absolute or relative to the process cwd — the CLI face resolves it before
/// calling this; what lands in `.ratchet/map.notes` is always the repo-root-relative,
/// case-preserving form (`repo::rel_for_git`), so the notes file stays portable.
pub fn note(main_root: &Path, target: &Path, sentence: &str) -> Result<(), MapError> {
    if sentence.is_empty() {
        return Err(err("sentence must not be empty"));
    }
    if sentence.contains('\n') || sentence.chars().count() > 120 {
        return Err(err("sentence must be a single line of at most 120 characters"));
    }
    if !is_tracked(main_root, target) {
        return Err(err(format!("not a tracked file: {}", target.display())));
    }
    let rel = crate::repo::rel_for_git(main_root, target)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .ok_or_else(|| err(format!("not a tracked file: {}", target.display())))?;
    let mut notes = load_notes(main_root);
    notes.insert(rel, sentence.to_string());
    write_notes(main_root, &notes)
}

/// Source files with neither a header nor a note. Full list when `all` or no map exists yet;
/// otherwise narrowed to files that changed since the map's recorded commit
/// (`git diff --name-only <recorded>..HEAD`, design §4.1).
pub fn missing(main_root: &Path, cfg: &MapSection, all: bool) -> Result<Vec<String>, MapError> {
    let all_files = tracked_files(main_root)?;
    let files: Vec<String> = all_files.into_iter().filter(|f| !is_excluded(f, &cfg.exclude)).collect();
    let (_, source_dirs) = layout_section(&files);
    let notes = load_notes(main_root);
    let mut candidates = Vec::new();
    for f in &files {
        let ext = Path::new(f).extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
        let recognised_lang = ext.as_deref().map(|e| style_for(e).is_some()).unwrap_or(false);
        let under_source = source_dirs.iter().any(|d| f.starts_with(&format!("{d}/")));
        if !recognised_lang && !under_source {
            continue;
        }
        if notes.contains_key(f) {
            continue;
        }
        if header_sentence(&main_root.join(f)).is_some() {
            continue;
        }
        candidates.push(f.clone());
    }
    if all {
        return Ok(candidates);
    }
    let Some(recorded) = recorded_commit(main_root) else {
        return Ok(candidates); // no map yet: the full list either way
    };
    let changed = git_output(main_root, &["diff", "--name-only", &format!("{recorded}..HEAD")]).unwrap_or_default();
    let changed_set: std::collections::BTreeSet<&str> = changed.lines().collect();
    Ok(candidates.into_iter().filter(|f| changed_set.contains(f.as_str())).collect())
}
```

Add two unit tests to the existing `#[cfg(test)] mod tests` in the same file:

```rust
    #[test]
    fn note_stores_the_repo_relative_form_even_from_an_absolute_target() {
        let d = repo();
        fs::write(d.path().join("x.rs"), "fn f() {}").unwrap();
        git(d.path(), &["add", "x.rs"]);
        git(d.path(), &["commit", "-q", "-m", "add x"]);
        note(d.path(), &d.path().join("x.rs"), "Does x.").unwrap();
        let notes = load_notes(d.path());
        assert_eq!(notes.get("x.rs"), Some(&"Does x.".to_string()));
    }

    #[test]
    fn missing_full_excludes_headered_and_noted_files() {
        let d = repo();
        fs::write(d.path().join("a.rs"), "//! Has a header\nfn a() {}").unwrap();
        fs::write(d.path().join("b.rs"), "fn b() {}").unwrap();
        fs::write(d.path().join("c.rs"), "fn c() {}").unwrap();
        git(d.path(), &["add", "a.rs", "b.rs", "c.rs"]);
        git(d.path(), &["commit", "-q", "-m", "add"]);
        note(d.path(), &d.path().join("c.rs"), "Has a note.").unwrap();
        let cfg = MapSection::default();
        let out = missing(d.path(), &cfg, true).unwrap();
        assert_eq!(out, vec!["b.rs".to_string()]);
    }
```

- [ ] **Step 2: Append to `crates/ratchet/src/cli/map_cmd.rs`**

```rust
pub fn note(path: &str, sentence: &str, cwd: Option<PathBuf>) -> i32 {
    let here = cwd.unwrap_or_else(|| PathBuf::from("."));
    let repo = match find_repo(&here) {
        Ok(Some(r)) => r,
        Ok(None) => return not_a_repo(&here),
        Err(e) => return fail(e),
    };
    let mut target = PathBuf::from(path);
    if !target.is_absolute() {
        target = here.join(target);
    }
    match crate::map::note(&repo.main_root, &target, sentence) {
        Ok(()) => 0,
        Err(e) => fail(e),
    }
}

pub fn missing(all: bool, cwd: Option<PathBuf>) -> i32 {
    let here = cwd.unwrap_or_else(|| PathBuf::from("."));
    let repo = match find_repo(&here) {
        Ok(Some(r)) => r,
        Ok(None) => return not_a_repo(&here),
        Err(e) => return fail(e),
    };
    match crate::map::missing(&repo.main_root, &repo.config.map, all) {
        Ok(files) => {
            for f in files {
                println!("{f}");
            }
            0
        }
        Err(e) => fail(e),
    }
}
```

- [ ] **Step 3: Extend the CLI surface in `main.rs`**

Change the `Map` variant to add the two flags:

```rust
    /// Repo orientation: a deterministic map of the tree, written to `.ratchet/map.md`.
    Map {
        #[command(subcommand)]
        cmd: Option<MapCmd>,
        /// List source files with no header and no note instead of generating.
        #[arg(long)]
        missing: bool,
        /// With --missing, widen to every undescribed file, not just those changed since the
        /// map's recorded commit.
        #[arg(long)]
        all: bool,
    },
```

Add the `Note` variant to `MapCmd`:

```rust
#[derive(Subcommand)]
enum MapCmd {
    /// Record one description for a header-less file.
    Note { path: String, sentence: String },
    /// Print the map's freshness line, or `map: current`.
    Status,
}
```

Replace the dispatch arm:

```rust
        Cmd::Map { cmd, missing, all } => match cmd {
            Some(MapCmd::Note { path, sentence }) => cli::map_cmd::note(&path, &sentence, cwd),
            Some(MapCmd::Status) => cli::map_cmd::status(cwd),
            None if missing => cli::map_cmd::missing(all, cwd),
            None => cli::map_cmd::generate(false, &env, cwd),
        },
```

- [ ] **Step 4: Run the green criterion**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/eduardoillanes/Documents/ratchet
cargo test -p ratchet --bin ratchet map::
cargo test -p ratchet --test spec map:: 2>&1 | tee /tmp/g6-task3.txt
grep -E "^test map::" /tmp/g6-task3.txt | sort
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: the 13 tests from Task 2 still `... ok`, plus these 11 now also `... ok`:
`map__a_note_describes_a_header_less_file`, `map__a_header_always_wins_over_a_note`,
`map__a_stale_note_is_dropped_at_the_next_generation`,
`map__map_note_records_a_sentence_for_a_tracked_file`,
`map__map_note_replaces_an_existing_note_for_the_same_path`,
`map__map_note_refuses_a_path_that_is_not_tracked`,
`map__map_note_refuses_an_invalid_sentence`, `map__map_note_refuses_an_empty_sentence`,
`map__missing_lists_every_file_with_no_header_and_no_note`,
`map__missing_narrows_to_files_changed_since_the_recorded_commit`,
`map__missing_all_widens_back_to_every_undescribed_file` — 24 of 31 green in total. The
remaining 7 (`wire_*` ×3, the four briefing scenarios) still fail — expected, Tasks 4-5. New
`map::` unit tests pass. Gate clean.

- [ ] **Step 5: Commit**

```bash
git add crates/ratchet/src/map.rs crates/ratchet/src/cli/map_cmd.rs crates/ratchet/src/main.rs
git commit -m "feat: ratchet map note, notes merge, --missing with incremental mode

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Briefing line (implementer)

**Files:**
- Modify: `crates/ratchet/src/hooks/briefing.rs` (`build()` gains a `main_root: &Path`
  parameter and the map line, dropped first under the 40-line cap; five existing unit tests
  touched), `crates/ratchet/src/hooks/dispatch.rs` (one call site)

**Interfaces:**
- Consumes: `map::briefing_line` (Task 2, unchanged by this task).
- Produces: nothing later tasks call — this is the last consumer of `map::briefing_line` in this
  plan.

**Why this is not a one-line change:** `session.repo_root` is lower-cased (see Global
Constraints); `map::briefing_line` shells out to `git -C <path> …`, which is case-sensitive on
Linux/macOS. `build()` must receive a case-preserving root — `repo::Repo::main_root`, resolved
once at `session-start`, never `Session.repo_root`. And the map line must be *dropped first* when
the 40-line cap is hit (design §4.3), which the existing truncation does not do on its own —
inserting the line only when there is room, before deciding whether to truncate, is the whole
fix.

- [ ] **Step 1: Change `build()`'s signature and body**

`crates/ratchet/src/hooks/briefing.rs`: add `use std::path::Path;` to the imports, add one
parameter, and change the body as follows (the early-return "quiet" branch and the tail assembly
both change; everything else — `orphans`, `mine`, `ready` computation, `task_line`,
`handoff_text`, `quote`, `short`, `prompt_line` — is untouched):

```rust
pub fn build(
    conn: &Connection,
    session: &Session,
    main_root: &Path,
    th: &Thresholds,
    now: DateTime<Utc>,
) -> String {
    let header = format!(
        "[ratchet] repo {} · session {} · branch {}",
        session.repo,
        short(&session.id),
        session.branch.as_deref().unwrap_or("?")
    );
    let map_line = crate::map::briefing_line(main_root);
    let orphans = tasks::orphaned(conn, &session.repo_root, th, now).unwrap_or_default();
    let mine = tasks::list(
        conn,
        &tasks::Filter {
            repo_root: Some(&session.repo_root),
            statuses: &[TaskStatus::InProgress],
            claimed_by: Some(&session.id),
            ..Default::default()
        },
    )
    .unwrap_or_default();
    let ready: Vec<Task> = tasks::list(
        conn,
        &tasks::Filter {
            repo_root: Some(&session.repo_root),
            statuses: &[TaskStatus::Ready],
            ..Default::default()
        },
    )
    .unwrap_or_default()
    .into_iter()
    .take(MAX_READY)
    .collect();
    if orphans.is_empty() && mine.is_empty() && ready.is_empty() {
        return match &map_line {
            Some(l) => format!("{header}\n{l}"),
            None => header,
        };
    }
    let mut lines = vec![header];
    if !mine.is_empty() {
        lines.push("Your tasks in progress:".to_string());
        lines.extend(mine.iter().map(|t| task_line(conn, t, true)));
    }
    if !orphans.is_empty() {
        lines.push("In progress, held by dead sessions (take them or leave them):".to_string());
        for orphan in &orphans {
            match tasks::get(conn, &orphan.task_id) {
                Ok(task) => lines.push(task_line(conn, &task, true)),
                Err(_) => lines.push(format!("  {}  {}", orphan.task_id, orphan.title)),
            }
        }
    }
    if !ready.is_empty() {
        lines.push("Ready to take:".to_string());
        lines.extend(ready.iter().map(|t| task_line(conn, t, false)));
    }
    lines.push(COMMANDS_LINE.to_string());
    // The map line is dropped first (design §4.3): only inserted when the rest already fits.
    if let Some(l) = &map_line {
        if lines.len() < MAX_LINES {
            lines.insert(1, l.clone());
        }
    }
    if lines.len() > MAX_LINES {
        lines.truncate(MAX_LINES - 2);
        lines.push("  … (more in `ratchet task list`)".to_string());
        lines.push(COMMANDS_LINE.to_string());
    }
    lines.join("\n")
}
```

- [ ] **Step 2: Fix the four call sites in `build()`'s own tests that don't need new assertions**

In `the_briefing_groups_mine_orphans_and_ready`, `at_most_five_ready_tasks_are_listed` and
`a_crowded_board_is_cut_to_forty_lines`, change the `build(...)` call from:

```rust
        let text = build(&conn, &session, &Thresholds::default(), now);
```
to:
```rust
        let text = build(&conn, &session, Path::new("root"), &Thresholds::default(), now);
```
(`"root"` matches `setup()`'s `repo_root: "root"` — a nonexistent path, so `map::briefing_line`
deterministically returns `Some("map: none…")` without any git call; none of these three tests'
assertions depend on line position beyond `position()`/`contains()`/`last()`, so no other change
is needed — verify this by reading each assertion, not by assuming it.)

- [ ] **Step 3: Replace `a_quiet_repo_gets_exactly_one_line` with two tests**

The old test's premise ("a quiet repo prints exactly one line") is no longer universally true —
a quiet repo with no map now prints two. Replace it with both halves of the real contract:

```rust
    #[test]
    fn a_quiet_repo_with_no_map_adds_the_map_none_line() {
        let (conn, session) = setup("session-abcdef0123");
        let text = build(
            &conn,
            &session,
            Path::new("root"),
            &Thresholds::default(),
            at("2026-09-16T12:01:00Z"),
        );
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(
            text.starts_with("[ratchet] repo demo · session session-…"),
            "{text}"
        );
        assert!(text.contains("branch main"), "{text}");
        assert!(
            text.contains("map: none — run /ratchet:map for the repo layout"),
            "{text}"
        );
    }

    #[test]
    fn a_quiet_repo_with_a_current_map_prints_exactly_one_line() {
        use std::process::{Command, Stdio};
        let (conn, session) = setup("session-abcdef0123");
        let d = tempfile::TempDir::new().unwrap();
        let git = |args: &[&str]| {
            let st = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(d.path())
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(st.success());
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["commit", "--allow-empty", "-q", "-m", "init"]);
        let generated =
            crate::map::generate(d.path(), &crate::config::MapSection::default(), at("2026-09-16T12:00:00Z"))
                .unwrap();
        crate::map::write_map(d.path(), &generated).unwrap();

        let text = build(
            &conn,
            &session,
            d.path(),
            &Thresholds::default(),
            at("2026-09-16T12:01:00Z"),
        );
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(
            text.starts_with("[ratchet] repo demo · session session-…"),
            "{text}"
        );
    }
```

- [ ] **Step 4: Update the call site in `dispatch.rs`**

`crates/ratchet/src/hooks/dispatch.rs`, inside `session_start`, change:

```rust
    println!(
        "{}",
        briefing::build(&conn, &session, &repo.config.thresholds, now)
    );
```
to:
```rust
    println!(
        "{}",
        briefing::build(&conn, &session, &repo.main_root, &repo.config.thresholds, now)
    );
```
(`repo: Repo` is already in scope in this function, from `find_repo(cwd)?` two lines above —
`&repo.main_root` deref-coerces from `&PathBuf` to `&Path`, no `.as_path()` needed.)

- [ ] **Step 5: Run the green criterion**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/eduardoillanes/Documents/ratchet
cargo test -p ratchet --bin ratchet briefing::
cargo test -p ratchet --test spec map:: 2>&1 | tee /tmp/g6-task4.txt
grep -E "^test map::" /tmp/g6-task4.txt | sort
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: `briefing::` unit tests all pass, including the two new ones and the three
signature-only-changed ones. In the `map::` scenario run, the 24 tests from Tasks 2-3 still `ok`,
plus these 4 now also `ok`: `map__no_map_prints_the_map_none_line`,
`map__a_map_behind_head_prints_the_commits_behind_line`,
`map__a_map_from_another_branch_prints_the_from_another_branch_line`,
`map__a_current_map_prints_no_line` — 28 of 31 green. The remaining 3 `wire_*` scenarios still
fail — Task 5. Gate clean.

- [ ] **Step 6: Commit**

```bash
git add crates/ratchet/src/hooks/briefing.rs crates/ratchet/src/hooks/dispatch.rs
git commit -m "feat: briefing reports the map's freshness, dropped first under the cap

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: `--wire`, `commands/map.md`, `agents/mapper.md`, README, agent doctrine (implementer)

**Files:**
- Modify: `crates/ratchet/src/map.rs` (+ `wire`, `WireResult`, `is_regular_file`),
  `crates/ratchet/src/cli/map_cmd.rs` (`generate` now uses its `wire` parameter),
  `crates/ratchet/src/main.rs` (`--wire` flag on `Map`), `README.md`, `docs/agent-doctrine.md`
- Create: `commands/map.md`, `agents/mapper.md`, `crates/ratchet/tests/agent_frontmatter.rs`

**Interfaces:**
- Produces: `map::{wire, WireResult}`; the full `ratchet map --wire` CLI surface — this turns
  Task 1's last 3 scenario tests green. Nothing later depends on new names.

- [ ] **Step 1: Append `wire` to `crates/ratchet/src/map.rs`**

```rust
pub struct WireResult {
    pub claude_md_changed: bool,
    pub gitignore_changed: bool,
}

const CLAUDE_IMPORT_BLOCK: &str = "<!-- ratchet map: this repo's layout, generated on demand, never by hand. -->\n<!-- refresh with `/ratchet:map`; do not edit `.ratchet/map.md` directly. -->\n@.ratchet/map.md\n";

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).map(|m| m.file_type().is_file()).unwrap_or(false)
}

/// Appends the `CLAUDE.md` import block and the `.gitignore` entry, creating either file if
/// absent. Idempotent: a field is `false` in the result when that file already had what it
/// needed, and nothing is written to it. Refuses, before touching either file, when `CLAUDE.md`
/// or `.gitignore` exists but is not a regular file (a symlink or a directory) — this is why
/// the CLI calls `wire` *before* `generate`/`write_map` when `--wire` is given: a refusal here
/// must leave the map itself unwritten too.
pub fn wire(main_root: &Path) -> Result<WireResult, MapError> {
    let claude_path = main_root.join("CLAUDE.md");
    if claude_path.exists() && !is_regular_file(&claude_path) {
        return Err(err("CLAUDE.md is not a regular file; refusing to touch it"));
    }
    let gitignore_path = main_root.join(".gitignore");
    if gitignore_path.exists() && !is_regular_file(&gitignore_path) {
        return Err(err(".gitignore is not a regular file; refusing to touch it"));
    }

    let claude_text = fs::read_to_string(&claude_path).unwrap_or_default();
    let claude_md_changed = !claude_text.contains("@.ratchet/map.md");
    if claude_md_changed {
        let mut new_text = claude_text;
        if !new_text.is_empty() && !new_text.ends_with('\n') {
            new_text.push('\n');
        }
        if !new_text.is_empty() {
            new_text.push('\n');
        }
        new_text.push_str(CLAUDE_IMPORT_BLOCK);
        fs::write(&claude_path, new_text).map_err(|e| err(format!("could not write CLAUDE.md: {e}")))?;
    }

    let gitignore_text = fs::read_to_string(&gitignore_path).unwrap_or_default();
    let gitignore_changed =
        !gitignore_text.lines().any(|l| matches!(l.trim(), ".ratchet/" | ".ratchet"));
    if gitignore_changed {
        let mut new_text = gitignore_text;
        if !new_text.is_empty() && !new_text.ends_with('\n') {
            new_text.push('\n');
        }
        new_text.push_str(".ratchet/\n");
        fs::write(&gitignore_path, new_text)
            .map_err(|e| err(format!("could not write .gitignore: {e}")))?;
    }

    Ok(WireResult { claude_md_changed, gitignore_changed })
}
```

Add two unit tests to the existing `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn wire_creates_both_files_when_absent() {
        let d = repo();
        let r = wire(d.path()).unwrap();
        assert!(r.claude_md_changed && r.gitignore_changed);
        assert!(fs::read_to_string(d.path().join("CLAUDE.md")).unwrap().contains("@.ratchet/map.md"));
        assert!(fs::read_to_string(d.path().join(".gitignore")).unwrap().contains(".ratchet/"));
    }

    #[test]
    fn wire_appends_without_disturbing_existing_content() {
        let d = repo();
        fs::write(d.path().join("CLAUDE.md"), "# My rules\n\nDo the thing.\n").unwrap();
        fs::write(d.path().join(".gitignore"), "target/\n").unwrap();
        let r = wire(d.path()).unwrap();
        assert!(r.claude_md_changed && r.gitignore_changed);
        let claude = fs::read_to_string(d.path().join("CLAUDE.md")).unwrap();
        assert!(claude.starts_with("# My rules"), "{claude}");
        assert!(claude.contains("@.ratchet/map.md"), "{claude}");
        let gitignore = fs::read_to_string(d.path().join(".gitignore")).unwrap();
        assert!(gitignore.contains("target/"), "{gitignore}");
        assert!(gitignore.contains(".ratchet/"), "{gitignore}");
    }
```

- [ ] **Step 2: Use `wire` in `cli/map_cmd.rs`'s `generate`**

Replace the whole `generate` function (the `_wire` parameter becomes `wire`, and a new block
runs before generation when it is `true`):

```rust
/// `ratchet map` / `ratchet map --wire`. When `wire` is set, wiring runs first, so a refusal
/// (a symlinked `CLAUDE.md` or `.gitignore`) leaves the map itself unwritten too.
pub fn generate(wire: bool, env: &HashMap<String, String>, cwd: Option<PathBuf>) -> i32 {
    let here = cwd.unwrap_or_else(|| PathBuf::from("."));
    let repo = match find_repo(&here) {
        Ok(Some(r)) => r,
        Ok(None) => return not_a_repo(&here),
        Err(e) => return fail(e),
    };
    if wire {
        match crate::map::wire(&repo.main_root) {
            Ok(r) if !r.claude_md_changed && !r.gitignore_changed => {
                println!("wire: already wired (CLAUDE.md, .gitignore)");
            }
            Ok(r) => {
                if r.claude_md_changed {
                    println!("wired: CLAUDE.md now imports .ratchet/map.md");
                }
                if r.gitignore_changed {
                    println!("wired: .gitignore now covers .ratchet/");
                }
            }
            Err(e) => return fail(e),
        }
    }
    let now = clock::now(env);
    let generated = match crate::map::generate(&repo.main_root, &repo.config.map, now) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    let path = match crate::map::write_map(&repo.main_root, &generated) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    println!(
        "wrote {} ({} lines, {} modules without a description)",
        path.display(),
        generated.text.lines().count(),
        generated.without
    );
    for hint in &generated.hints {
        println!("{hint}");
    }
    0
}
```

- [ ] **Step 3: Add the `--wire` flag in `main.rs`**

Add one field to the `Map` variant:

```rust
    Map {
        #[command(subcommand)]
        cmd: Option<MapCmd>,
        /// List source files with no header and no note instead of generating.
        #[arg(long)]
        missing: bool,
        /// With --missing, widen to every undescribed file, not just those changed since the
        /// map's recorded commit.
        #[arg(long)]
        all: bool,
        /// Wire CLAUDE.md and .gitignore, then generate.
        #[arg(long)]
        wire: bool,
    },
```

Change the dispatch arm's last line:

```rust
        Cmd::Map { cmd, missing, all, wire } => match cmd {
            Some(MapCmd::Note { path, sentence }) => cli::map_cmd::note(&path, &sentence, cwd),
            Some(MapCmd::Status) => cli::map_cmd::status(cwd),
            None if missing => cli::map_cmd::missing(all, cwd),
            None => cli::map_cmd::generate(wire, &env, cwd),
        },
```

- [ ] **Step 4: Write `commands/map.md`**

```markdown
---
name: "ratchet: map"
description: "Regenerate the repo map that CLAUDE.md imports, and optionally describe files with no header"
allowed-tools: Bash, Agent
category: "Setup"
tags: ["setup", "ratchet", "map"]
---

Run `"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd" map` with Bash from the repo root.

- Report the `wrote ...` line verbatim.
- If it printed one or two `hint:` lines naming `--wire`, show them and ask the user whether to
  run `"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd" map --wire`. Only run it on a clear yes — it
  edits `CLAUDE.md` and `.gitignore` at the repo root.
- If the argument is `--deep`: run `"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd" map --missing`.
  If it prints nothing, say the map already describes every file and stop. Otherwise dispatch
  the `mapper` agent with the printed list, in batches of at most 40 paths, one dispatch at a
  time — never in parallel, since every batch writes through the same
  `.ratchet/map.notes`. After each batch completes, run
  `"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd" map` again and report the new footer counts
  (`<n> with a header, <m> with a note, <k> without either`).
- If it says `binary not found`: point the user to the README's Install section.
```

- [ ] **Step 5: Write `agents/mapper.md`**

```markdown
---
name: mapper
description: Describes header-less source files, one sentence each, through `ratchet map note` — never edits any other file. Dispatch with a batch of at most 40 paths from `ratchet map --missing`.
model: haiku
effort: low
tools: Read, Grep, Glob, Bash
---

You are the mapper. Your prompt gives you a list of file paths, at most 40, all missing a
description in the repo's map. For each path:

1. `Read` the first 40 lines.
2. `Grep` the file for its public signatures (`pub fn`, `def `, `export function`, `class `, or
   whatever the language uses) if 40 lines was not enough to tell what the file is for.
3. Write one sentence saying what the file is FOR, not what it contains — "computes the token
   budget for a prompt", not "defines a struct and three functions".
4. Record it: `ratchet map note <path> "<sentence>"`. The sentence must be a single line of at
   most 120 characters — shorten it if `map note` refuses it.

Do not read a whole file end to end. Do not open any file outside your list. Do not run any
command other than `Read`, `Grep`, `Glob` and `ratchet map note`. Never edit a file directly —
`ratchet map note` is the only way a description reaches the map. If you cannot tell what a file
is for from its head and its signatures, skip it and say so in your report; do not guess.

Report at the end: how many files you described, and the paths of any you could not.
```

- [ ] **Step 6: Update `README.md`**

In `## What it does`, the bullet that undercounts agent profiles (pre-existing, off by one even
before this group — fix it while touching this section):

```markdown
- **Agent roles, skills and commands.** Seven agent profiles, the `ratchet-tasks` and
  `ratchet-pdf` skills, the OpenSpec skills with their `/opsx:*` commands, `/ratchet:init` and
  `/ratchet:map`.
```

In `## Agents and skills`, update the count and the list, and add a "## Map" section right after
it (before `## Build from source`):

```markdown
## Agents and skills

Seven agent profiles in `agents/`: `analyst`, `spec-test-author`, `implementer`, `reviewer`,
`refactorer`, `researcher`, `mapper` — see `docs/agent-doctrine.md` for how they are meant to be
combined. Two skills: `ratchet-tasks` (working the board, writing handoffs) and `ratchet-pdf`
(extracting text from a local PDF). The OpenSpec skills (`openspec-propose`, `-apply-change`,
`-update-change`, `-sync-specs`, `-archive-change`, `-explore`) and the `/opsx:*` commands are
included as-is and need the `openspec` CLI installed separately.

## Map

`ratchet map` derives `.ratchet/map.md` — the repo's layout, gate commands, and one sentence per
source file — from `git ls-files`, file headers and manifests, deterministically, with no model
involved. `/ratchet:map` runs it and offers to wire it into `CLAUDE.md` (`--wire`, run once,
never automatic); `/ratchet:map --deep` describes header-less files by dispatching the `mapper`
agent (`haiku`), which only ever records a sentence through `ratchet map note` — it edits no
file directly. `ratchet map status` prints the map's freshness (`map: current`, `map: N commits
behind`, or `map: from another branch`); the session-start briefing shows the same line when the
map is not current, dropped first if the briefing's own 40-line cap is tight. A map never grows
past 150 lines — over the cap, the deepest directories collapse into one summary line each.
`.ratchet/map.md` and `.ratchet/map.notes` are local and untracked; `--wire` is what adds
`.ratchet/` to `.gitignore`, not `ratchet map` on its own. Configure `[map] exclude` and
`[map] gate` in `ratchet.toml` when the defaults don't fit a repo.
```

- [ ] **Step 7: Update `docs/agent-doctrine.md`**

Change the opening line:

```markdown
ratchet ships seven agent profiles and two working rules. The rules are what make the profiles
worth having.
```

In "Rule 2", add one bullet after `researcher`:

```markdown
- **mapper** describes header-less files for `ratchet map`, one sentence each, through
  `ratchet map note` — the only file it ever touches is `.ratchet/map.notes`, and only through
  that command, never by editing it directly.
```

- [ ] **Step 8: Write `crates/ratchet/tests/agent_frontmatter.rs`**

A plain integration test, not scenario-covered — `agents/*.md` frontmatter is not an OpenSpec
capability, and this file did not exist before group 6.

```rust
//! Every `agents/*.md` frontmatter names a `model` and, where present, an `effort`, from the
//! allowed set. Written and made green in the same task that adds `mapper.md` (group 6, Task 5)
//! — a version of this test written before `mapper.md` existed would have started green, not
//! red, so it does not belong with the scenario-test-author's red-clean contract.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

const ALLOWED_MODELS: &[&str] = &["opus", "sonnet", "haiku"];
const ALLOWED_EFFORTS: &[&str] = &["low", "medium", "high"];

fn frontmatter(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

fn field<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}:");
    block.lines().find_map(|l| l.strip_prefix(&prefix)).map(|v| v.trim().trim_matches('"'))
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
                ALLOWED_MODELS.contains(&model),
                "{path:?}: model {model:?} not in {ALLOWED_MODELS:?}"
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
```

- [ ] **Step 9: Run the green criterion — the full local gate**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/eduardoillanes/Documents/ratchet
cargo test -p ratchet --bin ratchet map::
cargo test -p ratchet --test spec map:: 2>&1 | tee /tmp/g6-task5.txt
grep -cE "^test map::.*ok$" /tmp/g6-task5.txt
cargo test -p ratchet --test agent_frontmatter
cargo test -p ratchet --test scenarios
cargo test -p ratchet
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: `grep -c` prints `31` — every scenario in the allocation table is now green.
`agent_frontmatter` passes. `cargo test -p ratchet` (the whole crate) is **fully green** for the
first time since Task 1 — the exception called out in Global Constraints ends here, not at
Task 6. Gate clean.

- [ ] **Step 10: Commit**

```bash
git add crates/ratchet/src/map.rs crates/ratchet/src/cli/map_cmd.rs crates/ratchet/src/main.rs \
        commands/map.md agents/mapper.md README.md docs/agent-doctrine.md \
        crates/ratchet/tests/agent_frontmatter.rs
git commit -m "feat: ratchet map --wire, /ratchet:map, mapper agent, docs

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: This repository's own wiring (implementer)

**Files:**
- Create (by running the real binary, not by hand): `CLAUDE.md`, `.gitignore` (modified)
- Modify: `CLAUDE.md` (append the hand-written block, by hand, after `--wire` creates the file)

**Interfaces:** none — this task calls the CLI surface Tasks 2-5 already built, against the real
`/Users/eduardoillanes/Documents/ratchet` checkout, on the `g6-map` branch.

- [ ] **Step 1: Confirm the starting state**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/eduardoillanes/Documents/ratchet
test -f ratchet.toml && echo "marker present"
test -f CLAUDE.md && echo "CLAUDE.md already exists (unexpected)" || echo "no CLAUDE.md yet"
grep -c '\.ratchet' .gitignore || echo "gitignore does not cover .ratchet yet"
```
Expected: `marker present`, `no CLAUDE.md yet`, and the grep prints `0` or the "does not cover"
line (`.ratchet` should not already be in `.gitignore` at this point).

- [ ] **Step 2: Build and run `ratchet map --wire` for real**

```bash
cargo build -p ratchet
B=target/debug/ratchet
$B map --wire
```
Expected output, two or three lines: `wired: CLAUDE.md now imports .ratchet/map.md`,
`wired: .gitignore now covers .ratchet/`, then `wrote .ratchet/map.md (<n> lines, <k> modules
without a description)`.

- [ ] **Step 3: Inspect what was written**

```bash
cat CLAUDE.md
tail -3 .gitignore
head -3 .ratchet/map.md
git status --short
```
Expected: `CLAUDE.md` contains exactly the three-line import block from Task 5 Step 1
(`CLAUDE_IMPORT_BLOCK`); `.gitignore`'s last line is `.ratchet/`; `.ratchet/map.md`'s first line
is `# ratchet — map`; `git status --short` shows `CLAUDE.md` and `.gitignore` as new/modified,
and does **not** show anything under `.ratchet/` (it is now covered by the `.gitignore` entry
that was just added, written before the map itself per Task 5's ordering).

- [ ] **Step 4: Append the hand-written block to `CLAUDE.md`**

Append, by hand (not through `ratchet map`), after the auto-generated import block. Under 15
lines, and — per design §4.4 — no file paths in it, so nothing here can rot as the tree changes:

```markdown

## Working rules

Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p ratchet`.

- Only the services layer writes the database.
- Every mutation is an appended event.
- CLI faces carry no SQL.
- The plugin manifest carries no version; the binary version is pinned in a separate file. A
  release is a tag plus a crate version bump, together.
```

- [ ] **Step 5: Confirm the full gate is still green**

```bash
cargo test -p ratchet --test scenarios
cargo test -p ratchet
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: everything green — Task 6 changes nothing under `crates/ratchet/`, so this is a
confirmation, not new work.

- [ ] **Step 6: Commit, push, open the PR**

```bash
git add CLAUDE.md .gitignore
git status --short   # only CLAUDE.md and .gitignore — .ratchet/ must not appear
git commit -m "chore: wire this repo's own CLAUDE.md to the generated map

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
git push -u origin g6-map
gh pr create --base main --head g6-map --title "Group 6: ratchet map" --body "$(cat <<'EOF'
Group 6 per the design spec (docs/superpowers/specs/2026-09-21-ratchet-map-design.md) and its
implementation plan (docs/superpowers/plans/2026-09-21-ratchet-group-6-map.md): a deterministic,
model-free repo map (`ratchet map`), notes for header-less files (`map note`, `--missing`), a
session-start freshness line, `--wire`/`/ratchet:map`/the `mapper` agent for the deep pass, and
this repo's own wiring.

31 scenarios in `openspec/specs/map/spec.md`, full gate green.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
gh pr checks --watch
```
Expected: the PR is created; the `gate` checks from `.github/workflows/ci.yml` (shipped in group
5) run on Ubuntu, macOS and Windows and pass. `main` is untouched until Task 7's review and the
owner's merge.

---

### Task 7: Group review (reviewer, read-only)

**Files:** none modified. One review, at the end of the whole group, per the design's delivery
model (§8: "one review by `reviewer` at the end of the group") and repo memory ("one review per
task, minors to a single final wave, terse status").

- [ ] **Step 1:** From `/Users/eduardoillanes/Documents/ratchet` on branch `g6-map`, run
  `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p ratchet`,
  `cargo test -p ratchet --test scenarios`. All green, numbers recorded (31 `map__*` tests, the
  `map::` unit tests, `agent_frontmatter`, and every pre-existing test still passing —
  `briefing::` in particular, since Task 4 touched five of its tests).
- [ ] **Step 2:** Contrast the diff (`git -C /Users/eduardoillanes/Documents/ratchet log
  --oneline main..g6-map` and the full diff) against `openspec/specs/map/spec.md` and this plan:
  - All 31 scenarios have a passing test, matched one-for-one against the allocation table.
  - No model is exercised by any test (`grep -rn "haiku\|Agent(" crates/ratchet/tests` — the
    `mapper` agent must appear only in markdown, never invoked from a test).
  - `.ratchet/map.md` and `.ratchet/map.notes` are written only under a repo root resolved by
    `repo::find_repo` — never under `RATCHET_HOME` (`grep -n "ratchet_home" crates/ratchet/src/map.rs`
    returns nothing).
  - `wire` runs, and can refuse, strictly before `generate`/`write_map` in `cli/map_cmd.rs`
    (read the function; a refusal after the map is already written would violate "refuses …
    without writing the map").
  - `hooks/dispatch.rs` and `hooks/briefing.rs` are the *only* hook-layer files touched (Task 4);
    `hooks/handoff_rule.rs`, `guardrails/**`, `hooks.json` are untouched — grep this plan's task
    list to confirm, or diff their modification times.
  - The `main_tree` guardrail's own files (`crates/ratchet/src/guardrails/**`) are untouched, and
    Task 6's `CLAUDE.md`/`.gitignore` writes went through the binary's own `fs::write`
    (`map::wire`), not through an `Edit`/`Write` tool call on a tracked file in the main tree.
- [ ] **Step 3:** Adversarial probes with the real binary, in a throwaway temp git repo (never
  against this repo's own tree): a `[map] exclude` pattern with no `*` at all (exact-path
  exclude — confirm it still matches); running `ratchet map` twice with a file renamed in
  between (old path must disappear from `.ratchet/map.notes` at the next generation — this is
  the stale-note scenario already covered, but re-run it by hand once); a note sentence of
  exactly 120 characters (must be accepted — off-by-one on the boundary); `ratchet map --missing`
  in a repo with zero source files (must print nothing and exit 0, not error); `ratchet map
  status` run twice in a row with no commits in between (idempotent, same line both times).
- [ ] **Step 4:** Verdict: `APPROVED`, or `BLOCKING` items with concrete detail (`file:line`, the
  case that fails). A blocking item is described, not fixed. Report it directly — this group has
  no board task to attach a note to; the verdict is the final message handed back.

---

## Self-review against the spec

- **§1 Purpose / §2 decisions:** D-map-deterministic → Task 2 `generate` + scenario 1.
  D-map-on-demand → nothing auto-regenerates; session start only reports freshness (Task 4), and
  the plan adds no cron/hook that calls `generate`. D-map-notes → Task 3 `note`/`load_notes`,
  format `path: sentence`, header always wins (scenario 10). D-map-haiku → `agents/mapper.md`
  (Task 5), `model: haiku`, `effort: low`, tools `Read, Grep, Glob, Bash`, never exercised by a
  test (Task 7 Step 2 checks this explicitly). D-map-incremental → Task 3 `missing`'s
  `git diff --name-only <recorded>..HEAD` intersection (scenario 18). D-map-untracked → `.ratchet/`
  never written as a tracked file by `generate` on its own; `--wire` is the only thing that
  touches `.gitignore`/`CLAUDE.md` (Task 5); this repo's own `--wire` run is Task 6, explicit and
  once. D-map-cap → `MAP_LINE_CAP = 150`, `collapse_modules`, deepest-first, deterministic tie
  break (Task 2, scenario 20 + unit test). D-map-config → `MapSection` (Task 2, scenarios 30-31).
- **§3 the map's own format:** header (commit sha + date via `clock::now`) → `generate`'s
  `header` string. Gate → `gate_section`/`detect_gate` (Task 2, unit-tested per manifest per
  §7's own instruction; only the Cargo branch is scenario-tested, since every fixture repo in
  this plan is a Rust repo). Layout → `layout_section` (top-level + one level under
  `crates/*`/`packages/*`, entry points). Modules → `modules_section` + `header_sentence`'s
  per-language table (scenarios 5-8). Tests/Docs → `tests_section`/`docs_section`. Footer →
  `generate`'s trailing `format!` with the exact `<!-- ratchet-map commit=… tree=… -->` line
  `recorded_commit` later parses back out.
- **§4.1 CLI:** `ratchet map` / `--wire` / `--missing [--all]` / `note <path> "<sentence>"` /
  `status` — all five map to a named function across Tasks 2, 3 and 5, with the exact exit-code
  and printed-line contracts from Global Constraints and the spec's stable message fragments.
- **§4.2 command + agent:** `commands/map.md` and `agents/mapper.md`, Task 5, both quoting the
  design's own batching/no-parallel-dispatch language.
- **§4.3 briefing:** Task 4, `main_root` (case-preserving) threaded from `dispatch.rs`, dropped
  first under the cap, all three non-current lines verbatim, `Current`/`Unknown` both print
  nothing.
- **§4.4 this repo:** Task 6, real `--wire` run plus the hand-written block, no paths in it
  (design's own constraint, honored literally — no backtick-quoted filenames in Step 4's text).
- **§5 data flow:** `/ratchet:map` → `ratchet map` → (git, headers, manifests, notes) →
  `.ratchet/map.md` (Task 2/5); `/ratchet:map --deep` → `--missing` → `mapper` → `map note` ×N →
  `map` again (Task 5's `commands/map.md`); `SessionStart` → `briefing` → map status line,
  read-only (Task 4) — every arrow in the design's diagram has a task.
- **§6 error handling:** git failure → `generate` returns `Err`, CLI exits 1 (design's "exit 1
  with one stderr line"); briefing swallows a git/freshness failure silently
  (`Freshness::Unknown → None`, Task 4). Non-UTF-8 head → `header_sentence`'s
  `std::str::from_utf8(&bytes).ok()?` treats it as no header (Task 2). `--wire` refuses a
  non-regular file and writes nothing, including the map (Task 5, ordering explicit in Step 2's
  own doc comment). `note` validation exactly as design §4.1 lists it (Task 3). A stale note
  dropped, not an error (Task 3, scenario 11). `mapper` hitting its turn budget is out of this
  plan's test scope by design — no test in this plan invokes a model.
- **§7 testing:** scenario tests in `crates/ratchet/tests/spec/map.rs`, one per `#### Scenario`,
  fixtures in Rust/Python/TypeScript over a temporary git repo with a marker — Task 1, all 31.
  Unit tests for gate detection per manifest and collapse order — Task 2's `#[cfg(test)] mod
  tests` (6 gate-detection tests + collapse + exclude + notes-missing). The new
  `agent_frontmatter` test — Task 5, deliberately not scenario-governed (design: "There is no
  agent frontmatter test today; group 6 adds one").
- **§8 delivery plan:** the five numbered items map 1:1 to Tasks 2-6; Task 1 is the
  spec-test-author step the design's own prose describes but doesn't number; Task 7 is "one
  review by `reviewer` at the end of the group."
- **Type consistency checked:** `MapError` (`map.rs`) is the only error type across `generate`,
  `write_map`, `note`, `missing`, `wire` — `.message()` is what every `cli::map_cmd::*` function
  prints via `fail()`. `Generated`'s four fields (`text`, `with_header`, `with_note`, `without`,
  `hints`) are read only by `cli::map_cmd::generate`, matching Task 2's own definition — no
  other task adds or renames a field. `Freshness`'s five variants are matched exhaustively by
  both `briefing_line` (Task 2) and, transitively, `status_line` — Task 4 never re-derives
  freshness itself, it only calls `map::briefing_line`. `WireResult`'s two booleans are read only
  by `cli::map_cmd::generate`'s wire branch (Task 5) — no test reads the struct directly, only
  its effects (file contents, stdout).
- **Deliberately out of scope, stated as such rather than left silent:** `ratchet map --print`
  (design §9, "left out until someone needs it"); header extraction for languages beyond §3.4's
  table (design §9, "grows on demand" — this plan implements exactly the table's languages,
  Rust through shell, and no others); a commented `[map]` block in `ratchet config init`'s
  template (Global Constraints, explicit non-goal); per-task reviewer gates (the design's own
  delivery model is one review at the end of the group, not one per task, and repo memory says
  the same).
