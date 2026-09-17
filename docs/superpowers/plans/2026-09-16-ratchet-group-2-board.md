# ratchet — Group 2: the board, the briefing and the handoff rule

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give ratchet the task board the harness exists for — tasks with a checklist, notes, handoffs and an append-only event per fact — its CLI, and the three places where the harness pushes it into the session: the `[ratchet]` briefing at start, the one-line reminder on every prompt, and the handoff rule that blocks the close once when a claimed task has no record of this turn.

**Architecture:** `services/tasks.rs` grows from group 1's orphan-release slice into the whole board and stays the only writer; every mutation happens inside one `IMMEDIATE` transaction and appends exactly one event per fact. `cli/task_cmd.rs` and `hooks/{briefing,handoff_rule}.rs` are thin faces over it, and `output.rs` gives every face the same compact-output discipline. Progress is `done items / total items`, computed on read, never stored and never declared. `pre-tool` is untouched and still opens nothing.

**Tech Stack:** Rust 2021, `rusqlite` (bundled), `chrono`, `serde_json`, `clap` 4 — exactly the stack groups 0 and 1 already pulled in. **No new dependency:** `Cargo.toml` is not edited by this group.

**Spec:** `docs/superpowers/specs/2026-09-16-ratchet-plugin-design.md` (§2 D-p3, D-marker, D-english, D-specs-first, D-roles, D-runner-out; §3 layout and the layer rule; §4.1 CLI and output discipline; §4.2 the `SessionStart`, `UserPromptSubmit` and `Stop` rows; §4.5 task board; §5 data flow; §6 error handling; §7 testing; §8 group 2).

**Predecessors:**
- `docs/superpowers/plans/2026-09-16-ratchet-group-0-guardrails.md` (complete; rulings R1-R10 hold).
- `docs/superpowers/plans/2026-09-16-ratchet-group-1-state.md` (in flight; rulings G1-R1..R4 and G1-P1 hold). **Group 2 starts only when group 1 is complete**: every type, table and function below is group 1's.

## Global Constraints

These are group 0's constraints and group 1's, unchanged, plus the ones this group adds. Every task's requirements implicitly include this section. Nothing here may be weakened.

**Carried over from groups 0 and 1 (do not weaken):**

- **No git commands in `C:\repos\ratchet` on the owner's machine** (spec D-roles). Every task ends with a hand-off listing the files created or changed; the owner commits from another account. Tests may run `git` inside temporary directories only. No `git status`, no `git diff`, not even read-only, inside this repo.
- Work directly in `C:\repos\ratchet` (there is no worktree because there is no git flow here; the `main-tree` guardrail of `ops` does not apply to this directory).
- All content, identifiers, messages and docs in English (spec D-english). The stderr prefix of a guardrail block is exactly `[ratchet guardrail:<id>]`; every other line ratchet writes into a session starts with `[ratchet] `.
- Hook exit codes: 0 allow / internal error, 2 block. Nothing else, ever (spec D-p3).
- `pre-tool` never opens a database and never spawns a subprocess except `git ls-files` when a write targets the main tree (spec 4.2).
- Latency target: `pre-tool` median under 30 ms in release on Windows; the test ceiling in debug builds is 200 ms (spec 4.2, 7).
- State root: `RATCHET_HOME` or `~/.ratchet` (spec D-state).
- Repo marker file name: `ratchet.toml` at the repo root (spec 4.3). No global list of repos.
- Rust toolchain: `stable-x86_64-pc-windows-gnu` with WinLibs gcc on PATH (group-0 ruling R3, amended). Every shell that runs cargo starts with:
  ```
  export PATH="$HOME/.cargo/bin:/c/Users/eillanes/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
  ```
- The crate is **bin-only** (group-0 ruling R8): unit tests run with `cargo test -p ratchet --bin ratchet`, lint with `cargo clippy -p ratchet --bin ratchet -- -D warnings`. Never `--lib`.
- Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p ratchet`, plus `cargo test -p ratchet --test scenarios`.
- Example patterns in tests and docs use neutral method names (`purge_all`, `write_rows`), never the write methods of a real database driver: the owner's own harness scans written content and blocks those names.
- No `rm -rf` anywhere (group-0 ruling R5): leave temporary files or use the scratchpad.
- **SQLite is embedded**: `rusqlite` with the `bundled` feature. No system `libsqlite3`, no `sqlite3` CLI, no other database crate.
- **One state database**: `<home>/ratchet.db`. Opened with WAL, `busy_timeout = 5000`, `foreign_keys = ON`.
- **Only `session-start` and `ratchet db migrate` migrate** (spec §6). Every other face calls `db::open_ready` and fails with `run ratchet db migrate` on an older schema; in a hook that failure is exit 0 plus one log line.
- **Timestamps** are RFC 3339 UTC with seconds precision and a `Z` suffix, stored as TEXT. One formatter (`clock::iso`), one parser (`clock::parse`).
- **`events` is append-only**: `INSERT` only, never `UPDATE` or `DELETE`, in any module, ever.
- **`services/` is the only writer** (spec §3 layer rule): no `INSERT`/`UPDATE`/`DELETE` in `db/`, `cli/`, `hooks/`, `output.rs` or the binary's unit tests. Scenario tests may seed and read rows directly because they stand outside the binary, and they say so in a comment.
- CLI commands exit 1 (not 2) on a user-facing error, with `error: …` on stderr.
- `hooks/hooks.json` is **not** modified: group 0 already registers all seven events.
- `RATCHET_NOW` (RFC 3339) overrides "now" in every face (G1-R3). Services always take `now` as a parameter and never read the clock themselves.
- Shared one-line edits to `main.rs`, `services/mod.rs`, `cli/mod.rs`, `hooks/mod.rs` and `tests/spec/main.rs` are **append-only** (G1-P1): whoever edits second reads the file first and appends its own line; nothing is reordered.

**Added by group 2:**

- **Progress is derived, never declared.** `progress` is computed from `checklist_items` on every read; there is no stored percentage and no interface — CLI, hook or JSON — accepts one. A task with no checklist reports *no* progress (not zero).
- **One event per fact.** Every board mutation appends exactly one event per fact it changed: `task.created`, `task.claimed`, `task.status`, `checklist.done`, `checklist.undone`, `note`, `handoff`, `task.archived`, `task.unarchived`. Bumping `tasks.updated_at` alongside another fact is part of that fact, not a second one, and emits nothing of its own.
- **One transaction per operation.** Every public writer in `services/tasks.rs` opens one `IMMEDIATE` transaction and does all its writes and all its events inside it; a failure half-way leaves nothing behind. Private helpers take the open connection and never open their own.
- **Group 1's surface is preserved, not rewritten.** `hooks/dispatch.rs::{session_start, session_end, register, ensure_session, export_session_id}`, all of `services/sessions.rs`, and `tasks::{Orphan, orphaned, release, release_dead, claimed_ids}` keep their exact signatures and their exact event sequences. Group 2 appends; the single body change it makes is named in Task 5 and is additive to a payload.
- **Exactly one new exit-2 path in the binary**: the handoff rule on `Stop`, and only when the session is interactive and `stop_hook_active` is false. Every other group-2 code path in a hook exits 0, including a database that cannot be opened.
- **The briefing is at most 40 lines**, is printed on `session-start` **before** the orphan sweep, with the same `now` (group 1 Task 10's marked seam), and is one single line when the repo has nothing pending.
- **Output discipline** (spec §4.1): any command output longer than 60 lines is written to `<home>/out/<YYYYMMDDTHHMMSS>-<name>.txt` and the terminal gets the first 20 lines plus `… (<n> lines in <path>)`. Hooks are exempt: the briefing has its own 40-line cap and is injected as context, not printed as a listing.
- `Cargo.toml` is **not** edited by this group.

Scenario slug rule (unchanged, enforced by `crates/ratchet/tests/scenarios.rs`): lowercase the scenario title, replace every run of non-alphanumerics with `_`, trim `_`; the test function is `fn <spec>__<slug>()` where `<spec>` is the spec directory name with `-` → `_`. This group has two prefixes: `tasks` and `agent_protocol`. The checker ignores `#### Scenario:` lines inside fenced code blocks, so **the fenced spec text in this plan does not count** — only what lands in `openspec/specs/**/spec.md` does, and the same holds for the fenced test code: the checker reads the landed files, never this plan.

---

## File structure

```
crates/ratchet/
├── src/main.rs                       + `--session` global option, `Cmd::Task` + arm   [7, 8]
├── src/model.rs                      + Task, ChecklistItem, 7 event kinds, transitions [Task 3]
├── src/output.rs                     emit, emit_json, format_task_line                [Task 7]
├── src/services/tasks.rs             + reads [4], create/transition/archive [5],
│                                       claim/check/note/handoff [6]
├── src/cli/mod.rs                    + `pub mod task_cmd;`                            [Task 8]
├── src/cli/task_cmd.rs               `ratchet task …`                                 [Task 8]
├── src/hooks/mod.rs                  + `pub mod briefing;` [9], `pub mod handoff_rule;` [11]
├── src/hooks/briefing.rs             build (briefing) [9], prompt_line [10]
├── src/hooks/handoff_rule.rs         should_block_stop                                [Task 11]
├── src/hooks/dispatch.rs             briefing at the seam [9], `beat`/prompt [10], stop [11]
└── tests/spec/{main,support,tasks,board}.rs   32 scenario tests                       [Task 2]

openspec/specs/tasks/spec.md          the board capability (21 scenarios)              [Task 1]
openspec/specs/agent-protocol/spec.md + 4 requirements (11 scenarios)                  [Task 1]
README.md                             one appended section                             [Task 12]
skills/ratchet-tasks/SKILL.md         three named edits                                [Task 12]
```

## Parallelism

Tasks with disjoint files may run at the same time; the cap is 5 concurrent agents (group-0 ruling R9) and these waves never exceed 2.

| Wave | Tasks | Why they don't collide |
|---|---|---|
| A | **1** alone | The contract. Everything else quotes it. `openspec/**` only. |
| B | **2**, **3** | 2 = `tests/spec/**` only; 3 = `src/model.rs` only. |
| C | **4**, **7** | 4 = `src/services/tasks.rs`; 7 = `src/output.rs` + one `mod` line and one option in `main.rs`. Disjoint. |
| D | **5** alone | `src/services/tasks.rs`, which 4 just changed. |
| E | **6** alone | `src/services/tasks.rs`, which 5 just changed. |
| F | **8**, **9** | 8 = `src/cli/task_cmd.rs`, `src/cli/mod.rs`, `main.rs`; 9 = `src/hooks/{briefing.rs,mod.rs,dispatch.rs}`. Disjoint. |
| G | **10** alone | `src/hooks/{briefing.rs,dispatch.rs}`, which 9 just changed. |
| H | **11**, **12** | 11 = `src/hooks/{handoff_rule.rs,mod.rs,dispatch.rs}`; 12 = `README.md`, `skills/ratchet-tasks/SKILL.md`. Disjoint. |
| I | **13** alone | Read-only review of everything. |

`main.rs` is touched by Tasks 7 (one `mod` line, one global option), 8 (one `Cmd` variant, one match arm, one `pub mod` line in `cli/mod.rs`) and nothing else; `hooks/mod.rs` by Tasks 9 and 11 (one `pub mod` line each); `tests/spec/main.rs` by Task 2 (two `mod` lines). All of these are append-only under G1-P1 and land in different waves, so no merge is ever needed.

Reviews follow group 0's and group 1's pattern: a task's review may run in the wave after it, alongside the next implementation task, as long as a fix round would touch only that task's files.

---

### Task 1: Ported specs — the board capability and the board half of `agent-protocol`

The behaviour comes from `C:\repos\ops`: `openspec/specs/tasks/spec.md` (identifier, fields, states, claim, checklist, derived progress, handoffs and notes, events) and the four board-facing requirements of `openspec/specs/agent-protocol/spec.md` (briefing, per-prompt reminder, handoff rule on close, compact output). Requirements about the web UI, the data layer and the agent runner are **not** ported. The language is English and every mention of `ops` becomes `ratchet` (D-english). Two things of the reference disappear with D-marker: the central registry of repos (so `_global` and "repo not registered" go away — a task belongs to the repo whose marker was found) and the `skill` field of a task (there is no column for it in migration `0001`).

**Files:**
- Create: `openspec/specs/tasks/spec.md`
- Modify: `openspec/specs/agent-protocol/spec.md` (append four requirements; correct the two-line note under the title)

**Interfaces:**
- Produces: the 21 + 11 scenario titles below. They are the contract for Task 2's tests and for the checker in `crates/ratchet/tests/scenarios.rs`. No Rust file changes in this task.
- Consumes: nothing.

- [ ] **Step 1: Write `openspec/specs/tasks/spec.md`**

```markdown
# tasks

The board: what work exists, who holds it, how far it has got, and what the next session needs
to know before touching it.

## Purpose

A task is the unit of work and of accountability. Its acceptance criteria live in a checklist,
so progress is something the work produces, not something an agent claims. Everything that
happens to a task is appended to an event log that is never edited, so the history of a decision
survives the session that took it.

## Requirements

### Requirement: Readable, stable identifier
Every task SHALL have a short identifier of the form `T-NNNN`, drawn from an increasing sequence
(four digits with leading zeros up to 9999, free to grow after that). An identifier SHALL never be
reused and never change.

#### Scenario: Creating a task assigns the next identifier
- **WHEN** tasks exist up to `T-0041` and a new one is created
- **THEN** the new task is `T-0042`, even if an earlier task was archived

### Requirement: Fields of a task
A task SHALL have a title, a body (may be empty), the repo it belongs to together with the root of
that repo's main checkout, a status, a priority from 1 to 4 (3 by default), tags (may be empty),
an optional parent task with one level of nesting only, the session holding it (optional), and the
instants it was created, last changed and, if it was, archived. A task SHALL be created only from
inside a repo that opted in, and SHALL record that repo.

#### Scenario: One level of subtasks
- **WHEN** a task is created whose parent already has a parent
- **THEN** the operation is refused, saying only one level of nesting is allowed

#### Scenario: A task belongs to the repo it was created in
- **WHEN** a task is created from a directory with no marker above it
- **THEN** the operation is refused, naming the marker file that opts a repo in, and no task is created

#### Scenario: Priority outside the range
- **WHEN** a task is created with priority 7
- **THEN** the operation is refused, saying the priority goes from 1 to 4

### Requirement: States and transitions
The status SHALL be one of `backlog`, `ready`, `in_progress`, `blocked`, `review`, `done`. The
allowed transitions SHALL be exactly: `backlog` to `ready`; `ready` to `in_progress`;
`in_progress` to `blocked`, `review` or `done`; `blocked` to `in_progress`; `review` to
`in_progress` or `done`; any status to `backlog`; and `in_progress`, `blocked` or `review` back to
`ready`, which is what letting go of a task means. A refused transition SHALL list the ones allowed
from the current status. Every transition SHALL record an event with origin, destination and the
optional reason. Moving to `done` SHALL require every checklist item to be done; a task with no
checklist SHALL require an explicit reason, which stays in the event.

#### Scenario: Invalid transition
- **WHEN** a task in `ready` is moved to `done`
- **THEN** the operation is refused and the message lists the transitions allowed from `ready`

#### Scenario: Done needs a complete checklist
- **WHEN** a task with unchecked items is moved to `done`
- **THEN** the operation is refused, listing the items still pending

#### Scenario: Done without a checklist needs a reason
- **WHEN** a task with no checklist is moved to `done` with no reason
- **THEN** the operation is refused asking for one; given a reason it moves, and the reason stays in the event

#### Scenario: A valid transition leaves an event
- **WHEN** a task goes from `in_progress` to `blocked` with the reason "waiting for credentials"
- **THEN** the status changes and one status event records the origin, the destination and that reason

### Requirement: Claiming a task
Claiming SHALL put the task in `in_progress` and associate it with the claiming session, recording
a claim event. Claiming a task in `backlog` SHALL take it through `ready` (two status events). A
`done` task SHALL NOT be claimable. The claiming session SHALL be registered; if it is not, the
operation is refused and the task does not change. A task held by a live or idle session SHALL
refuse a claim from another session, naming the holder and since when; if the holder is orphaned or
ended, the claim SHALL transfer the task and leave a note.

#### Scenario: Claim from backlog
- **WHEN** a session claims a task that is in `backlog`
- **THEN** the task is `in_progress` in that session's name, and the history shows the two status changes followed by the claim

#### Scenario: Claim with an unregistered session
- **WHEN** a task is claimed naming a session identifier the registry does not know
- **THEN** the operation is refused saying the session is not registered, and the task does not change

#### Scenario: Claim over a live session
- **WHEN** a second session claims a task held by a session that is live
- **THEN** the operation is refused, naming the session that holds it

#### Scenario: Claim over an orphaned session
- **WHEN** a second session claims a task held by a session that is orphaned
- **THEN** the task is now held by the second session and a note records the transfer

### Requirement: The checklist is the acceptance criteria
A task SHALL be able to carry ordered checklist items; each item SHALL be markable as done,
recording which session did it and when, and unmarkable. Marking and unmarking SHALL each record
their own event. Marking a position that does not exist SHALL be refused, listing the positions
that do.

#### Scenario: Check by position
- **WHEN** item 3 of a task is marked from a session
- **THEN** item 3 is done, recorded against that session, and an event says so

#### Scenario: A position that does not exist
- **WHEN** item 9 of a task with 5 items is marked
- **THEN** the operation is refused, listing the available items

### Requirement: Progress is derived
The progress of a task SHALL be computed as items done over total items. A task with no items SHALL
report no progress — not zero — and no interface SHALL accept a progress value from the outside.

#### Scenario: With a checklist
- **WHEN** a task has 5 items and 2 are done
- **THEN** every view of the task reports 2 of 5

#### Scenario: Without a checklist
- **WHEN** a task has no items
- **THEN** every view of the task reports its status and no progress at all

### Requirement: Notes and handoffs
A task SHALL accept notes and handoffs: free text recorded against the task and the session that
wrote it. A handoff SHALL describe what is left and how to resume, and SHALL NOT be empty. The last
handoff of a task SHALL be the most recent one and SHALL be available in every view of the task.

#### Scenario: The last handoff
- **WHEN** a task receives one handoff and then a second one
- **THEN** every view of the task shows the second one as its last handoff

#### Scenario: An empty handoff is refused
- **WHEN** a handoff is recorded with blank text
- **THEN** the operation is refused asking what is left and how to resume, and nothing is recorded

### Requirement: Archiving hides, it never deletes
A `done` task SHALL be archivable, which hides it from listings and from the briefing while keeping
its history; archiving anything not `done` SHALL be refused. An archived task SHALL be visible on
request and SHALL be restorable. Archiving and restoring SHALL each record their own event.

#### Scenario: Only a done task is archived
- **WHEN** a task that is `in_progress` is archived
- **THEN** the operation is refused, saying only a done task can be archived

#### Scenario: An archived task leaves the listing
- **WHEN** a `done` task is archived
- **THEN** the default listing no longer shows it, the listing that includes archived tasks does, and its history is intact

### Requirement: Every change leaves an event
Every creation or modification of a task or of its checklist SHALL append one event per fact, with
the source that caused it and the session that asked for it when there is one. Events SHALL never be
edited or deleted.

#### Scenario: Checking an item leaves an event
- **WHEN** an item of a task is marked and then unmarked from the same session
- **THEN** the task's history holds both facts, in that order, and no earlier event changed
```

- [ ] **Step 2: Append the four board requirements to `openspec/specs/agent-protocol/spec.md`**

First correct the note under the title. Replace:
```
How a Claude Code session interacts with ratchet through hooks. Group 0 covers repo opt-in,
guardrails and the never-break rule; briefing, reminders and the handoff rule arrive with
groups 1 and 2.
```
with:
```
How a Claude Code session interacts with ratchet through hooks: repo opt-in, guardrails, the
never-break rule, the briefing at start, the reminder on every prompt, the handoff rule when the
session closes, and the shape of what the CLI writes back.
```

Then append, at the end of the file, keeping every existing requirement exactly as it is:
```markdown
### Requirement: Briefing at session start
On session start in an opted-in repo, the hook SHALL write to standard output a plain-text briefing
of at most 40 lines containing: the repo, the abbreviated session identifier and the branch; the
session's own tasks in progress with their last handoff; the repo's tasks in progress held by
sessions that died, with their last handoff; up to five tasks ready to take, ordered by priority;
and one line naming the commands and the skill with the full guide. When the repo has none of those
tasks, the briefing SHALL be a single line. The briefing SHALL be built before the work of dead
sessions is returned to the queue, so an orphaned task is shown once with its handoff before it goes
back.

#### Scenario: Briefing with orphans and ready tasks
- **WHEN** a session starts in a repo with one task held by a dead session and three ready to take
- **THEN** the briefing shows the orphaned one with its last handoff under its own heading, the three ready ones under another, and ends with the line naming the commands and the skill

#### Scenario: No tasks, one line
- **WHEN** a session starts in a repo with no tasks in progress, none orphaned and none ready
- **THEN** the briefing is exactly one line, with the repo, the session and the branch

#### Scenario: The briefing never exceeds forty lines
- **WHEN** a session starts in a repo with far more tasks than fit
- **THEN** the briefing is at most 40 lines, the last two say where to see the rest and name the commands, and nothing is cut mid-line

### Requirement: Task reminder on every prompt
On every user prompt in an opted-in repo, if the session holds a task in progress, the hook SHALL
add to the context exactly one line with the identifier, the status, the progress and the abbreviated
last handoff, and say how many other tasks it holds when there are more. If the session holds no
task, the hook SHALL add nothing at all.

#### Scenario: With a claimed task
- **WHEN** a session holding a task at 2 of 5 receives a prompt
- **THEN** the context gains exactly one line naming the task, its status and its progress, and nothing else

#### Scenario: Without a claimed task
- **WHEN** a session holding no task receives a prompt
- **THEN** the hook adds no output at all

### Requirement: Handoff rule when the session closes
When a session tries to close while holding at least one task in progress that received no handoff,
no checklist change, no note and no status change from that session since its last prompt, the hook
SHALL block the close once, naming the task and the commands that record something. The hook SHALL
NOT block when the input says the close was already blocked once in this response, and SHALL NOT
block a session that is not interactive: nobody is there to record anything, and blocking would only
hold it open until its launcher kills it.

#### Scenario: Closing with nothing recorded
- **WHEN** a session holding a task in progress closes and nothing was recorded against that task since its last prompt
- **THEN** the close is blocked once, with a message naming the task and the commands that record a handoff, a check, a note or a status change

#### Scenario: Closing with something recorded
- **WHEN** the session checked an item of that task after its last prompt and then closes
- **THEN** the close is not blocked

#### Scenario: Retrying the close
- **WHEN** the input says the close was already blocked once in this response
- **THEN** the close is not blocked, even though the task still has no record

#### Scenario: Closing a headless session with nothing recorded
- **WHEN** a session registered as headless and launched by the platform closes holding a task in progress with no record since its last prompt
- **THEN** the close is not blocked, unlike an interactive session in the same situation

### Requirement: Compact output for agents
Listings SHALL use one line per task carrying identifier, status, priority, title and progress;
a detail view SHALL limit the history it prints to the last ten events. Every command SHALL offer
machine-readable output. Any output longer than 60 lines SHALL be written to a file under the state
directory, and the terminal SHALL receive the first 20 lines plus the path of that file.

#### Scenario: Long output goes to a file
- **WHEN** a command produces more than 60 lines
- **THEN** the terminal shows the first 20 lines and the path of a file that holds all of them

#### Scenario: A task listing is one line per task
- **WHEN** an agent lists the tasks ready to take
- **THEN** each task takes exactly one line with its identifier, status, priority, title and progress
```

- [ ] **Step 3: Check the scenario count and the checker's view**

```
export PATH="$HOME/.cargo/bin:/c/Users/eillanes/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cd /c/repos/ratchet && grep -c "^#### Scenario:" openspec/specs/tasks/spec.md openspec/specs/agent-protocol/spec.md
cd /c/repos/ratchet && cargo test -p ratchet --test scenarios 2>&1 | tail -40
```
Expected: `tasks/spec.md` = 21, `agent-protocol/spec.md` = 26 (group 0's 15 plus this group's 11). The checker **fails**, listing exactly 32 missing test names — that is the contract Task 2 fills. Copy that list into the hand-off.

- [ ] **Step 4: Hand off (no git)**

List the two files, the two scenario counts, and the 32 missing test names the checker printed.

---
### Task 2: Scenario tests, red-clean (spec-test-author)

**Files:**
- Create: `crates/ratchet/tests/spec/tasks.rs`, `crates/ratchet/tests/spec/board.rs`
- Modify: `crates/ratchet/tests/spec/main.rs` (two `mod` lines), `crates/ratchet/tests/spec/support.rs` (append helpers; do not change the existing ones)

**Interfaces:**
- Consumes — this is the whole contract the author gets, because the spec names no type, flag or variable:
  - `ratchet task list [--repo <name>] [--status <s>]… [--mine] [--tag <t>] [--all] [--json]`
  - `ratchet task show <id> [--json]`
  - `ratchet task new "<title>" [--body <text>] [--body-file <path>] [--check "<item>"]… [--priority <1-4>] [--tag <t>]… [--parent <id>] [--json]`
  - `ratchet task claim <id> [--json]`, `ratchet task status <id> <to> [--why "<reason>"] [--json]`
  - `ratchet task check <id> <position> [--undo] [--json]`
  - `ratchet task note <id> "<text>" [--json]`, `ratchet task handoff <id> "<text>" [--json]`
  - `ratchet task archive <id> [--json]`, `ratchet task unarchive <id> [--json]`
  - `--session <id>` is accepted anywhere on the command line (`ratchet --session X task claim T-0001` and `ratchet task claim T-0001 --session X` are the same thing).
  - Hooks, as in group 1: `ratchet hook session-start|prompt|stop|…` reading the harness JSON payload on stdin. Payload fields used here: `session_id`, `cwd`, `stop_hook_active`.
  - Environment: `RATCHET_HOME`, `RATCHET_SESSION_ID`, `RATCHET_SESSION_MODE` (`interactive`|`headless`), `RATCHET_LAUNCHED_BY`, `RATCHET_NOW` (RFC 3339 instant that replaces the clock, per process), `CLAUDE_ENV_FILE`, `CLAUDE_SCRATCHPAD`.
  - Exit codes: every hook 0, except a guardrail block and the handoff rule on `stop`, which are 2. CLI: 0 on success, 1 on a user-facing error with `error: …` on stderr.
  - Database (tests may read and seed it directly; production code never does outside `services/`): `<RATCHET_HOME>/ratchet.db`, tables `tasks(id, title, body, repo, repo_root, status, priority, parent_id, tags, claimed_by, created_at, updated_at, archived_at)`, `checklist_items(id, task_id, position, text, done, done_by_session, done_at)`, `sessions(…)`, `events(id, ts, session_id, task_id, kind, payload, source)`. Event kinds this group adds: `task.created`, `task.claimed`, `task.status`, `checklist.done`, `checklist.undone`, `note`, `handoff`, `task.archived`, `task.unarchived`.
  - Long output lands in `<RATCHET_HOME>/out/`.
- Produces: nothing for later tasks. These tests must be red for the right reason now and are **not edited by the implementer** of Tasks 3-12.

The author of this task reads only `openspec/specs/tasks/spec.md`, the four appended requirements of `openspec/specs/agent-protocol/spec.md`, this task and `README.md`; not the design document and not the other tasks.

- [ ] **Step 1: Extend the support module**

In `crates/ratchet/tests/spec/main.rs` add two lines, keeping the others:
```rust
mod board;
mod tasks;
```

Append to `crates/ratchet/tests/spec/support.rs` (every existing helper stays exactly as it is):
```rust
// --- group 2: the board ---------------------------------------------------------------------

/// A sandbox whose database exists and holds one session registered at `T0`. The session-start
/// hook is the only face allowed to create the database, so this is also how a test gets one.
pub fn board(session_id: &str) -> Sandbox {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload(session_id, &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "session-start failed: {}", stderr(&out));
    sb
}

/// Register one more session in the same sandbox, at `T0 + minutes`.
pub fn join(sb: &Sandbox, session_id: &str, minutes: i64) -> Output {
    let when = at(minutes);
    let root = sb.root();
    hook_env(
        sb,
        "session-start",
        &session_payload(session_id, &root),
        &root,
        &[("RATCHET_NOW", &when)],
    )
}

/// `ratchet task …` as `session_id`, at `T0 + minutes`, from the repo root.
pub fn task(sb: &Sandbox, args: &[&str], session_id: &str, minutes: i64) -> Output {
    let when = at(minutes);
    let mut argv = vec!["task"];
    argv.extend_from_slice(args);
    cli(
        sb,
        &argv,
        &sb.root(),
        &[("RATCHET_SESSION_ID", session_id), ("RATCHET_NOW", &when)],
    )
}

/// Create a task and return its identifier (the first token the command prints).
pub fn new_task(
    sb: &Sandbox,
    title: &str,
    checks: &[&str],
    session_id: &str,
    minutes: i64,
) -> String {
    let mut args = vec!["new", title];
    for c in checks {
        args.push("--check");
        args.push(c);
    }
    let out = task(sb, &args, session_id, minutes);
    assert_eq!(code(&out), 0, "task new failed: {}", stderr(&out));
    stdout(&out)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Kinds of a task's events, oldest first.
pub fn kinds_of(sb: &Sandbox, task_id: &str) -> Vec<String> {
    let conn = db(sb);
    let mut stmt = conn
        .prepare("SELECT kind FROM events WHERE task_id = ?1 ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map(rusqlite::params![task_id], |r| r.get::<_, String>(0))
        .unwrap();
    rows.map(|r| r.unwrap()).collect()
}

/// Raw payloads of a task's events of one kind, oldest first.
pub fn payloads_of(sb: &Sandbox, task_id: &str, kind: &str) -> Vec<String> {
    let conn = db(sb);
    let mut stmt = conn
        .prepare("SELECT payload FROM events WHERE task_id = ?1 AND kind = ?2 ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map(rusqlite::params![task_id, kind], |r| r.get::<_, String>(0))
        .unwrap();
    rows.map(|r| r.unwrap()).collect()
}

/// `(status, claimed_by, archived_at)` straight from the row.
pub fn task_state(sb: &Sandbox, task_id: &str) -> (String, Option<String>, Option<String>) {
    db(sb)
        .query_row(
            "SELECT status, claimed_by, archived_at FROM tasks WHERE id = ?1",
            rusqlite::params![task_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

/// Files the output discipline wrote under the state directory.
pub fn out_files(sb: &Sandbox) -> Vec<PathBuf> {
    let dir = sb.home.path().join("out");
    match fs::read_dir(&dir) {
        Err(_) => Vec::new(),
        Ok(entries) => entries.map(|e| e.unwrap().path()).collect(),
    }
}

/// Non-empty lines of a command's standard output.
pub fn lines(out: &Output) -> Vec<String> {
    stdout(out)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}
```

- [ ] **Step 2: Write `crates/ratchet/tests/spec/tasks.rs`**

```rust
//! One test per `#### Scenario` of openspec/specs/tasks/spec.md, named by slug. These tests drive
//! the real binary as a subprocess; they read and seed the database directly because they stand
//! outside the binary, which production code never does outside `services/`.

use crate::support::*;

// --- Requirement: Readable, stable identifier -----------------------------------------------

#[test]
fn tasks__creating_a_task_assigns_the_next_identifier() {
    let sb = board("s-1");
    let first = new_task(&sb, "first", &[], "s-1", 1);
    assert_eq!(first, "T-0001");
    let second = new_task(&sb, "second", &[], "s-1", 2);
    assert_eq!(second, "T-0002");
    // Closing and archiving the first one must not free its number.
    assert_eq!(code(&task(&sb, &["status", &first, "done", "--why", "no criteria"], "s-1", 3)), 0);
    assert_eq!(code(&task(&sb, &["archive", &first], "s-1", 4)), 0);
    let third = new_task(&sb, "third", &[], "s-1", 5);
    assert_eq!(third, "T-0003");
}

// --- Requirement: Fields of a task ----------------------------------------------------------

#[test]
fn tasks__one_level_of_subtasks() {
    let sb = board("s-2");
    let parent = new_task(&sb, "parent", &[], "s-2", 1);
    let child = task(&sb, &["new", "child", "--parent", &parent], "s-2", 2);
    assert_eq!(code(&child), 0, "{}", stderr(&child));
    let child_id = stdout(&child).split_whitespace().next().unwrap().to_string();
    let grandchild = task(&sb, &["new", "grandchild", "--parent", &child_id], "s-2", 3);
    assert_eq!(code(&grandchild), 1);
    assert!(
        stderr(&grandchild).to_lowercase().contains("one level"),
        "{}",
        stderr(&grandchild)
    );
    assert_eq!(count(&sb, "SELECT COUNT(*) FROM tasks", &[]), 2);
}

#[test]
fn tasks__a_task_belongs_to_the_repo_it_was_created_in() {
    let sb = board("s-3");
    let inside = new_task(&sb, "inside", &[], "s-3", 1);
    let repo_root: String = db(&sb)
        .query_row(
            "SELECT repo_root FROM tasks WHERE id = ?1",
            rusqlite::params![inside],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(repo_root, repo_root_key(&sb));

    let outside = unmanaged_dir();
    let when = at(2);
    let out = cli(
        &sb,
        &["task", "new", "orphan of no repo"],
        outside.path(),
        &[("RATCHET_SESSION_ID", "s-3"), ("RATCHET_NOW", &when)],
    );
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("ratchet.toml"), "{}", stderr(&out));
    assert_eq!(count(&sb, "SELECT COUNT(*) FROM tasks", &[]), 1);
}

#[test]
fn tasks__priority_outside_the_range() {
    let sb = board("s-4");
    let out = task(&sb, &["new", "too urgent", "--priority", "7"], "s-4", 1);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("1"), "{}", stderr(&out));
    assert!(stderr(&out).contains("4"), "{}", stderr(&out));
    assert_eq!(count(&sb, "SELECT COUNT(*) FROM tasks", &[]), 0);
}

// --- Requirement: States and transitions ----------------------------------------------------

#[test]
fn tasks__invalid_transition() {
    let sb = board("s-5");
    let id = new_task(&sb, "straight to done", &[], "s-5", 1);
    assert_eq!(code(&task(&sb, &["status", &id, "ready"], "s-5", 2)), 0);
    let out = task(&sb, &["status", &id, "done"], "s-5", 3);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("in_progress"), "allowed list missing: {err}");
    assert!(err.contains("backlog"), "allowed list missing: {err}");
    assert_eq!(task_state(&sb, &id).0, "ready");
}

#[test]
fn tasks__done_needs_a_complete_checklist() {
    let sb = board("s-6");
    let id = new_task(&sb, "two criteria", &["write it", "test it"], "s-6", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-6", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-6", 3)), 0);
    let out = task(&sb, &["status", &id, "done"], "s-6", 4);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("test it"), "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

#[test]
fn tasks__done_without_a_checklist_needs_a_reason() {
    let sb = board("s-7");
    let id = new_task(&sb, "no criteria", &[], "s-7", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-7", 2)), 0);
    let refused = task(&sb, &["status", &id, "done"], "s-7", 3);
    assert_eq!(code(&refused), 1, "stdout: {}", stdout(&refused));
    assert!(
        stderr(&refused).to_lowercase().contains("why"),
        "{}",
        stderr(&refused)
    );
    let accepted = task(&sb, &["status", &id, "done", "--why", "obsolete"], "s-7", 4);
    assert_eq!(code(&accepted), 0, "{}", stderr(&accepted));
    assert_eq!(task_state(&sb, &id).0, "done");
    let payloads = payloads_of(&sb, &id, "task.status");
    assert!(
        payloads.last().unwrap().contains("obsolete"),
        "{payloads:?}"
    );
}

#[test]
fn tasks__a_valid_transition_leaves_an_event() {
    let sb = board("s-8");
    let id = new_task(&sb, "blocked work", &[], "s-8", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-8", 2)), 0);
    let out = task(
        &sb,
        &["status", &id, "blocked", "--why", "waiting for credentials"],
        "s-8",
        3,
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).0, "blocked");
    let last = payloads_of(&sb, &id, "task.status").pop().unwrap();
    assert!(last.contains("in_progress"), "origin missing: {last}");
    assert!(last.contains("blocked"), "destination missing: {last}");
    assert!(last.contains("waiting for credentials"), "reason missing: {last}");
}

// --- Requirement: Claiming a task -----------------------------------------------------------

#[test]
fn tasks__claim_from_backlog() {
    let sb = board("s-9");
    let id = new_task(&sb, "to claim", &[], "s-9", 1);
    assert_eq!(task_state(&sb, &id).0, "backlog");
    let out = task(&sb, &["claim", &id], "s-9", 2);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let (status, holder, _) = task_state(&sb, &id);
    assert_eq!(status, "in_progress");
    assert_eq!(holder.as_deref(), Some("s-9"));
    let kinds = kinds_of(&sb, &id);
    assert_eq!(
        kinds,
        vec![
            "task.created".to_string(),
            "task.status".to_string(),
            "task.status".to_string(),
            "task.claimed".to_string(),
        ],
        "history was {kinds:?}"
    );
}

#[test]
fn tasks__claim_with_an_unregistered_session() {
    let sb = board("s-10");
    let id = new_task(&sb, "to claim", &[], "s-10", 1);
    let out = task(&sb, &["claim", &id, "--session", "s-ghost"], "s-10", 2);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(
        stderr(&out).to_lowercase().contains("not registered"),
        "{}",
        stderr(&out)
    );
    let (status, holder, _) = task_state(&sb, &id);
    assert_eq!(status, "backlog");
    assert_eq!(holder, None);
}

#[test]
fn tasks__claim_over_a_live_session() {
    let sb = board("s-11a");
    assert_eq!(code(&join(&sb, "s-11b", 1)), 0);
    let id = new_task(&sb, "contested", &[], "s-11a", 2);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-11a", 3)), 0);
    let out = task(&sb, &["claim", &id], "s-11b", 4);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("s-11a"), "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).1.as_deref(), Some("s-11a"));
}

#[test]
fn tasks__claim_over_an_orphaned_session() {
    let sb = board("s-12a");
    assert_eq!(code(&join(&sb, "s-12b", 1)), 0);
    let id = new_task(&sb, "abandoned", &[], "s-12a", 2);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-12a", 3)), 0);
    // Ninety minutes later, with no signal from either session, both are orphaned; a CLI call
    // does not sweep, so the task is still held when the second session asks for it.
    let out = task(&sb, &["claim", &id], "s-12b", 90);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).1.as_deref(), Some("s-12b"));
    let notes = payloads_of(&sb, &id, "note");
    assert!(
        notes.iter().any(|n| n.contains("s-12a") && n.contains("s-12b")),
        "transfer note missing: {notes:?}"
    );
}

// --- Requirement: The checklist is the acceptance criteria ----------------------------------

#[test]
fn tasks__check_by_position() {
    let sb = board("s-13");
    let id = new_task(&sb, "three criteria", &["one", "two", "three"], "s-13", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-13", 2)), 0);
    let out = task(&sb, &["check", &id, "3"], "s-13", 3);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let (done, by): (i64, Option<String>) = db(&sb)
        .query_row(
            "SELECT done, done_by_session FROM checklist_items WHERE task_id = ?1 AND position = 3",
            rusqlite::params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(done, 1);
    assert_eq!(by.as_deref(), Some("s-13"));
    assert!(
        payloads_of(&sb, &id, "checklist.done")
            .last()
            .unwrap()
            .contains("three"),
        "event does not name the item"
    );
}

#[test]
fn tasks__a_position_that_does_not_exist() {
    let sb = board("s-14");
    let id = new_task(&sb, "five criteria", &["a", "b", "c", "d", "e"], "s-14", 1);
    let out = task(&sb, &["check", &id, "9"], "s-14", 2);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("1"), "{err}");
    assert!(err.contains("5"), "{err}");
    assert_eq!(
        count(&sb, "SELECT COUNT(*) FROM checklist_items WHERE done = 1", &[]),
        0
    );
}

// --- Requirement: Progress is derived -------------------------------------------------------

#[test]
fn tasks__with_a_checklist() {
    let sb = board("s-15");
    let id = new_task(&sb, "five items", &["a", "b", "c", "d", "e"], "s-15", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-15", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-15", 3)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "2"], "s-15", 4)), 0);
    let shown = stdout(&task(&sb, &["show", &id], "s-15", 5));
    assert!(shown.contains("2/5"), "{shown}");
    let listed = stdout(&task(&sb, &["list"], "s-15", 6));
    assert!(listed.contains("2/5"), "{listed}");
}

#[test]
fn tasks__without_a_checklist() {
    let sb = board("s-16");
    let id = new_task(&sb, "no items", &[], "s-16", 1);
    let shown = stdout(&task(&sb, &["show", &id], "s-16", 2));
    assert!(shown.contains("backlog"), "{shown}");
    assert!(!shown.contains("0/0"), "reported a zero progress: {shown}");
    let listed = stdout(&task(&sb, &["list"], "s-16", 3));
    assert!(!listed.contains("0/0"), "reported a zero progress: {listed}");
}

// --- Requirement: Notes and handoffs --------------------------------------------------------

#[test]
fn tasks__the_last_handoff() {
    let sb = board("s-17");
    let id = new_task(&sb, "two handoffs", &[], "s-17", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-17", 2)), 0);
    assert_eq!(
        code(&task(&sb, &["handoff", &id, "the ten o'clock one"], "s-17", 3)),
        0
    );
    assert_eq!(
        code(&task(&sb, &["handoff", &id, "the twelve o'clock one"], "s-17", 4)),
        0
    );
    let shown = stdout(&task(&sb, &["show", &id], "s-17", 5));
    assert!(shown.contains("the twelve o'clock one"), "{shown}");
    let last_line = shown
        .lines()
        .rev()
        .find(|l| l.contains("handoff"))
        .unwrap_or_default()
        .to_string();
    assert!(
        !last_line.contains("the ten o'clock one"),
        "the older handoff is shown as the last one: {last_line}"
    );
}

#[test]
fn tasks__an_empty_handoff_is_refused() {
    let sb = board("s-18");
    let id = new_task(&sb, "needs a handoff", &[], "s-18", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-18", 2)), 0);
    let out = task(&sb, &["handoff", &id, "   "], "s-18", 3);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(
        stderr(&out).to_lowercase().contains("resume"),
        "{}",
        stderr(&out)
    );
    assert!(payloads_of(&sb, &id, "handoff").is_empty());
}

// --- Requirement: Archiving hides, it never deletes -----------------------------------------

#[test]
fn tasks__only_a_done_task_is_archived() {
    let sb = board("s-19");
    let id = new_task(&sb, "still open", &[], "s-19", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-19", 2)), 0);
    let out = task(&sb, &["archive", &id], "s-19", 3);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("done"), "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).2, None);
}

#[test]
fn tasks__an_archived_task_leaves_the_listing() {
    let sb = board("s-20");
    let id = new_task(&sb, "finished work", &[], "s-20", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-20", 2)), 0);
    assert_eq!(
        code(&task(&sb, &["status", &id, "done", "--why", "shipped"], "s-20", 3)),
        0
    );
    assert_eq!(code(&task(&sb, &["archive", &id], "s-20", 4)), 0);
    assert!(task_state(&sb, &id).2.is_some());
    let default_list = stdout(&task(&sb, &["list"], "s-20", 5));
    assert!(!default_list.contains(&id), "{default_list}");
    let all = stdout(&task(&sb, &["list", "--all"], "s-20", 6));
    assert!(all.contains(&id), "{all}");
    let shown = stdout(&task(&sb, &["show", &id], "s-20", 7));
    assert!(shown.contains("shipped"), "history lost: {shown}");
}

// --- Requirement: Every change leaves an event ----------------------------------------------

#[test]
fn tasks__checking_an_item_leaves_an_event() {
    let sb = board("s-21");
    let id = new_task(&sb, "one item", &["only criterion"], "s-21", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-21", 2)), 0);
    let before = kinds_of(&sb, &id);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-21", 3)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1", "--undo"], "s-21", 4)), 0);
    let after = kinds_of(&sb, &id);
    assert_eq!(after[..before.len()], before[..], "history was rewritten");
    assert_eq!(
        after[before.len()..],
        ["checklist.done".to_string(), "checklist.undone".to_string()]
    );
}
```

- [ ] **Step 3: Write `crates/ratchet/tests/spec/board.rs`**

```rust
//! One test per `#### Scenario` of the four board requirements of
//! openspec/specs/agent-protocol/spec.md, named by slug.

use crate::support::*;

// --- Requirement: Briefing at session start -------------------------------------------------

#[test]
fn agent_protocol__briefing_with_orphans_and_ready_tasks() {
    let sb = board("s-old");
    let held = new_task(&sb, "the abandoned one", &["a", "b"], "s-old", 1);
    assert_eq!(code(&task(&sb, &["claim", &held], "s-old", 2)), 0);
    assert_eq!(
        code(&task(
            &sb,
            &["handoff", &held, "stopped at the parser; resume with the fixtures"],
            "s-old",
            3
        )),
        0
    );
    for title in ["first ready", "second ready", "third ready"] {
        let id = new_task(&sb, title, &[], "s-old", 4);
        assert_eq!(code(&task(&sb, &["status", &id, "ready"], "s-old", 5)), 0);
    }
    // Ninety minutes with no signal: s-old is orphaned when the next session starts.
    let out = join(&sb, "s-new", 90);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.starts_with("[ratchet] repo "), "{text}");
    let orphan_heading = text
        .lines()
        .position(|l| l.to_lowercase().contains("dead session"))
        .expect("no heading for tasks held by dead sessions");
    let ready_heading = text
        .lines()
        .position(|l| l.to_lowercase().contains("ready"))
        .expect("no heading for ready tasks");
    assert!(orphan_heading < ready_heading, "{text}");
    assert!(text.contains(&held), "{text}");
    assert!(text.contains("resume with the fixtures"), "{text}");
    assert!(text.contains("first ready"), "{text}");
    assert!(text.contains("third ready"), "{text}");
    assert!(
        text.lines().last().unwrap().contains("ratchet-tasks"),
        "last line: {:?}",
        text.lines().last()
    );
    // The sweep runs after the briefing, so the task is back in the queue now.
    assert_eq!(task_state(&sb, &held).0, "ready");
}

#[test]
fn agent_protocol__no_tasks_one_line() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-alone", &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    assert_eq!(printed.len(), 1, "{printed:?}");
    assert!(printed[0].starts_with("[ratchet] repo "), "{}", printed[0]);
    assert!(printed[0].contains("session"), "{}", printed[0]);
    assert!(printed[0].contains("branch"), "{}", printed[0]);
}

#[test]
fn agent_protocol__the_briefing_never_exceeds_forty_lines() {
    let sb = board("s-crowded");
    // Sixty tasks in progress held by a session that was never registered: every one of them is
    // orphaned, and that list has no cap of its own.
    for n in 1..=60 {
        seed_task(&sb, &format!("T-{n:04}"), "in_progress", Some("s-ghost"));
    }
    let out = join(&sb, "s-reader", 1);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    assert!(printed.len() <= 40, "{} lines", printed.len());
    assert!(printed.len() > 30, "suspiciously short: {printed:?}");
    assert!(
        printed[printed.len() - 2].contains("ratchet task list"),
        "{:?}",
        printed[printed.len() - 2]
    );
    assert!(
        printed[printed.len() - 1].contains("ratchet-tasks"),
        "{:?}",
        printed[printed.len() - 1]
    );
}

// --- Requirement: Task reminder on every prompt ---------------------------------------------

#[test]
fn agent_protocol__with_a_claimed_task() {
    let sb = board("s-22");
    let id = new_task(&sb, "the claimed one", &["a", "b", "c", "d", "e"], "s-22", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-22", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-22", 3)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "2"], "s-22", 4)), 0);
    let when = at(5);
    let root = sb.root();
    let out = hook_env(
        &sb,
        "prompt",
        &session_payload("s-22", &root),
        &root,
        &[("RATCHET_NOW", &when)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    assert_eq!(printed.len(), 1, "{printed:?}");
    assert!(printed[0].starts_with("[ratchet] "), "{}", printed[0]);
    assert!(printed[0].contains(&id), "{}", printed[0]);
    assert!(printed[0].contains("in_progress"), "{}", printed[0]);
    assert!(printed[0].contains("(2/5)"), "{}", printed[0]);
}

#[test]
fn agent_protocol__without_a_claimed_task() {
    let sb = board("s-23");
    let _ = new_task(&sb, "nobody claimed me", &[], "s-23", 1);
    let when = at(2);
    let root = sb.root();
    let out = hook_env(
        &sb,
        "prompt",
        &session_payload("s-23", &root),
        &root,
        &[("RATCHET_NOW", &when)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
}

// --- Requirement: Handoff rule when the session closes --------------------------------------

/// A prompt, then a stop, both at explicit instants: the rule's window is "since the last prompt".
fn prompt_then_stop(sb: &Sandbox, session_id: &str, prompt_at: i64, stop_at: i64, retry: bool) -> std::process::Output {
    let root = sb.root();
    let p = at(prompt_at);
    let out = hook_env(
        sb,
        "prompt",
        &session_payload(session_id, &root),
        &root,
        &[("RATCHET_NOW", &p)],
    );
    assert_eq!(code(&out), 0, "prompt failed: {}", stderr(&out));
    let s = at(stop_at);
    let payload = serde_json::json!({
        "session_id": session_id,
        "cwd": root.to_string_lossy(),
        "stop_hook_active": retry,
    });
    hook_env(sb, "stop", &payload, &root, &[("RATCHET_NOW", &s)])
}

#[test]
fn agent_protocol__closing_with_nothing_recorded() {
    let sb = board("s-24");
    let id = new_task(&sb, "claimed and abandoned", &["a"], "s-24", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-24", 2)), 0);
    let out = prompt_then_stop(&sb, "s-24", 3, 4, false);
    assert_eq!(code(&out), 2, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.starts_with("[ratchet] "), "{err}");
    assert!(err.contains(&id), "{err}");
    assert!(err.contains("handoff"), "{err}");
    assert!(err.contains("check"), "{err}");
    assert!(err.contains("note"), "{err}");
    assert!(err.contains("status"), "{err}");
}

#[test]
fn agent_protocol__closing_with_something_recorded() {
    let sb = board("s-25");
    let id = new_task(&sb, "claimed and worked", &["a"], "s-25", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-25", 2)), 0);
    let root = sb.root();
    let p = at(3);
    assert_eq!(
        code(&hook_env(
            &sb,
            "prompt",
            &session_payload("s-25", &root),
            &root,
            &[("RATCHET_NOW", &p)]
        )),
        0
    );
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-25", 4)), 0);
    let s = at(5);
    let payload = serde_json::json!({
        "session_id": "s-25",
        "cwd": root.to_string_lossy(),
        "stop_hook_active": false,
    });
    let out = hook_env(&sb, "stop", &payload, &root, &[("RATCHET_NOW", &s)]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(stderr(&out), "");
}

#[test]
fn agent_protocol__retrying_the_close() {
    let sb = board("s-26");
    let id = new_task(&sb, "claimed and abandoned", &["a"], "s-26", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-26", 2)), 0);
    let blocked = prompt_then_stop(&sb, "s-26", 3, 4, false);
    assert_eq!(code(&blocked), 2);
    let root = sb.root();
    let s = at(5);
    let payload = serde_json::json!({
        "session_id": "s-26",
        "cwd": root.to_string_lossy(),
        "stop_hook_active": true,
    });
    let retry = hook_env(&sb, "stop", &payload, &root, &[("RATCHET_NOW", &s)]);
    assert_eq!(code(&retry), 0, "stderr: {}", stderr(&retry));
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

#[test]
fn agent_protocol__closing_a_headless_session_with_nothing_recorded() {
    let sb = sandbox();
    let root = sb.root();
    let started = hook_env(
        &sb,
        "session-start",
        &session_payload("s-27", &root),
        &root,
        &[
            ("RATCHET_NOW", T0),
            ("RATCHET_SESSION_MODE", "headless"),
            ("RATCHET_LAUNCHED_BY", "platform"),
        ],
    );
    assert_eq!(code(&started), 0, "{}", stderr(&started));
    let id = new_task(&sb, "claimed by a robot", &["a"], "s-27", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-27", 2)), 0);
    let out = prompt_then_stop(&sb, "s-27", 3, 4, false);
    assert_eq!(code(&out), 0, "a headless close was blocked: {}", stderr(&out));
}

// --- Requirement: Compact output for agents -------------------------------------------------

#[test]
fn agent_protocol__long_output_goes_to_a_file() {
    let sb = board("s-28");
    for n in 1..=70 {
        seed_task(&sb, &format!("T-{n:04}"), "ready", None);
    }
    let out = task(&sb, &["list"], "s-28", 1);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    assert!(printed.len() <= 21, "{} lines on the terminal", printed.len());
    let files = out_files(&sb);
    assert_eq!(files.len(), 1, "{files:?}");
    let body = std::fs::read_to_string(&files[0]).unwrap();
    assert_eq!(body.lines().count(), 70, "the file lost lines");
    assert!(
        printed.last().unwrap().contains(&files[0].display().to_string()),
        "the path is not on the terminal: {:?}",
        printed.last()
    );
}

#[test]
fn agent_protocol__a_task_listing_is_one_line_per_task() {
    let sb = board("s-29");
    let first = new_task(&sb, "first ready", &["a", "b"], "s-29", 1);
    assert_eq!(code(&task(&sb, &["status", &first, "ready"], "s-29", 2)), 0);
    let second = new_task(&sb, "second ready", &[], "s-29", 3);
    assert_eq!(code(&task(&sb, &["status", &second, "ready"], "s-29", 4)), 0);
    let out = task(&sb, &["list", "--status", "ready"], "s-29", 5);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    assert_eq!(printed.len(), 2, "{printed:?}");
    let line = printed.iter().find(|l| l.contains(&first)).unwrap();
    assert!(line.contains("ready"), "{line}");
    assert!(line.contains("p3"), "{line}");
    assert!(line.contains("first ready"), "{line}");
    assert!(line.contains("(0/2)"), "{line}");
    assert!(out_files(&sb).is_empty(), "a short listing went to a file");
}
```

- [ ] **Step 4: Prove they are red for the right reason**

```
export PATH="$HOME/.cargo/bin:/c/Users/eillanes/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cd /c/repos/ratchet && cargo test -p ratchet --test spec 2>&1 | tail -40
cd /c/repos/ratchet && cargo test -p ratchet --test scenarios 2>&1 | tail -10
cd /c/repos/ratchet && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: the file **compiles** (helpers and the binary's CLI surface are the only contract it uses), the 32 new tests fail because `ratchet task` does not exist yet — the failures say `error: unrecognized subcommand 'task'` or an exit code 1/2 mismatch, never a compile error and never a panic in the helpers. The checker is green (every scenario now has a test). Group 0's 21 and group 1's 24 scenario tests stay green: neither of them asserts an empty stdout on a `session-start` inside a marked repo.

- [ ] **Step 5: Hand off (no git)**

List the four files, the 32 test names, and the exact failure message of three of them.

---
### Task 3: `model.rs` — the board types, the new event kinds and the transition table

Ported from `C:\repos\ops\ops\core\models.py` (`Task`, `ChecklistItem`, `EventKind`, `TRANSITIONS`). Two fields of the reference are not ported because migration `0001` has no column for them: `skill` (dropped with the desk's skill catalogue) and any stored progress (there never was one).

**Files:**
- Modify: `crates/ratchet/src/model.rs`

**Interfaces:**
- Consumes: group 1's `model.rs` — the `db_enum!` macro, `TaskStatus`, `Source`, `Session`, `Event`, and `clock::parse`.
- Produces:
  - `model::EventKind` gains seven variants: `TaskCreated` (`task.created`), `TaskClaimed` (`task.claimed`), `TaskArchived` (`task.archived`), `TaskUnarchived` (`task.unarchived`), `ChecklistDone` (`checklist.done`), `ChecklistUndone` (`checklist.undone`), `Handoff` (`handoff`). The existing six stay, in place, with the same strings.
  - `model::allowed_transitions(from: TaskStatus) -> &'static [TaskStatus]`
  - `model::Task { id, title, body, repo, repo_root, status, priority, parent_id, tags, claimed_by, created_at, updated_at, archived_at }` with `Task::from_row(&Row) -> rusqlite::Result<Task>`, `Serialize`.
  - `model::ChecklistItem { id, task_id, position, text, done, done_by_session, done_at }` with `ChecklistItem::from_row`, `Serialize`.

- [ ] **Step 1: Write the failing tests**

Append to the existing `#[cfg(test)] mod tests` at the bottom of `crates/ratchet/src/model.rs`:
```rust
    #[test]
    fn the_board_event_kinds_have_their_database_strings() {
        assert_eq!(EventKind::TaskCreated.as_str(), "task.created");
        assert_eq!(EventKind::TaskClaimed.as_str(), "task.claimed");
        assert_eq!(EventKind::TaskArchived.as_str(), "task.archived");
        assert_eq!(EventKind::TaskUnarchived.as_str(), "task.unarchived");
        assert_eq!(EventKind::ChecklistDone.as_str(), "checklist.done");
        assert_eq!(EventKind::ChecklistUndone.as_str(), "checklist.undone");
        assert_eq!(EventKind::Handoff.as_str(), "handoff");
        assert_eq!(EventKind::from_db("checklist.done"), EventKind::ChecklistDone);
        // The kinds group 1 wrote keep their strings.
        assert_eq!(EventKind::SessionStart.as_str(), "session.start");
        assert_eq!(EventKind::TaskStatus.as_str(), "task.status");
    }

    #[test]
    fn the_transition_table_is_the_one_the_spec_lists() {
        use TaskStatus::*;
        assert_eq!(allowed_transitions(Backlog), &[Ready]);
        assert_eq!(allowed_transitions(Ready), &[InProgress, Backlog]);
        assert_eq!(
            allowed_transitions(InProgress),
            &[Blocked, Review, Done, Ready, Backlog]
        );
        assert_eq!(allowed_transitions(Blocked), &[InProgress, Ready, Backlog]);
        assert_eq!(allowed_transitions(Review), &[InProgress, Done, Ready, Backlog]);
        assert_eq!(allowed_transitions(Done), &[Backlog]);
        // Every state can be put back in the queue except the two that are already there.
        for from in [InProgress, Blocked, Review] {
            assert!(allowed_transitions(from).contains(&Ready), "{from:?}");
        }
        assert!(!allowed_transitions(Ready).contains(&Done));
    }

    #[test]
    fn a_task_reads_back_from_its_row_with_its_tags() {
        let conn = crate::db::open_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (id TEXT, title TEXT, body TEXT, repo TEXT, repo_root TEXT, \
             status TEXT, priority INTEGER, parent_id TEXT, tags TEXT, claimed_by TEXT, \
             created_at TEXT, updated_at TEXT, archived_at TEXT);
             INSERT INTO tasks VALUES ('T-0001','a title','body','demo','c:\\repos\\demo',\
             'in_progress',2,NULL,'[\"one\",\"two\"]','s-1','2026-09-16T12:00:00Z',\
             '2026-09-16T12:05:00Z',NULL);",
        )
        .unwrap();
        let task: Task = conn
            .query_row("SELECT * FROM tasks", [], |r| Task::from_row(r))
            .unwrap();
        assert_eq!(task.id, "T-0001");
        assert_eq!(task.status, TaskStatus::InProgress);
        assert_eq!(task.priority, 2);
        assert_eq!(task.tags, vec!["one".to_string(), "two".to_string()]);
        assert_eq!(task.claimed_by.as_deref(), Some("s-1"));
        assert!(task.archived_at.is_none());
        let text = serde_json::to_string(&task).unwrap();
        assert!(text.contains("\"status\":\"in_progress\""), "{text}");
    }

    #[test]
    fn broken_tags_do_not_fail_a_read() {
        let conn = crate::db::open_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (id TEXT, title TEXT, body TEXT, repo TEXT, repo_root TEXT, \
             status TEXT, priority INTEGER, parent_id TEXT, tags TEXT, claimed_by TEXT, \
             created_at TEXT, updated_at TEXT, archived_at TEXT);
             INSERT INTO tasks VALUES ('T-0002','t','','demo','root','weird',3,NULL,'not json',\
             NULL,'nonsense','2026-09-16T12:00:00Z',NULL);",
        )
        .unwrap();
        let task: Task = conn
            .query_row("SELECT * FROM tasks", [], |r| Task::from_row(r))
            .unwrap();
        assert!(task.tags.is_empty());
        assert_eq!(task.status, TaskStatus::Backlog); // lenient, like every other enum
        assert_eq!(crate::clock::iso(task.created_at), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn a_checklist_item_reads_back_from_its_row() {
        let conn = crate::db::open_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE checklist_items (id INTEGER, task_id TEXT, position INTEGER, text TEXT, \
             done INTEGER, done_by_session TEXT, done_at TEXT);
             INSERT INTO checklist_items VALUES (7,'T-0001',3,'test it',1,'s-1',\
             '2026-09-16T12:00:00Z');",
        )
        .unwrap();
        let item: ChecklistItem = conn
            .query_row("SELECT * FROM checklist_items", [], |r| {
                ChecklistItem::from_row(r)
            })
            .unwrap();
        assert_eq!(item.id, 7);
        assert_eq!(item.position, 3);
        assert!(item.done);
        assert_eq!(item.done_by_session.as_deref(), Some("s-1"));
        assert!(item.done_at.is_some());
    }
```

- [ ] **Step 2: Run them and watch them fail**

```
export PATH="$HOME/.cargo/bin:/c/Users/eillanes/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet model::
```
Expected: compile errors naming `TaskCreated`, `allowed_transitions`, `Task`, `ChecklistItem`.

- [ ] **Step 3: Extend the event kinds**

Replace the `db_enum!(EventKind, …)` call with this one. The six existing variants keep their position and their string; seven are added:
```rust
db_enum!(EventKind, Note, {
    SessionStart => "session.start",
    SessionPrompt => "session.prompt",
    SessionStop => "session.stop",
    SessionEnd => "session.end",
    TaskCreated => "task.created",
    TaskClaimed => "task.claimed",
    TaskStatus => "task.status",
    TaskArchived => "task.archived",
    TaskUnarchived => "task.unarchived",
    ChecklistDone => "checklist.done",
    ChecklistUndone => "checklist.undone",
    Handoff => "handoff",
    Note => "note",
});
```

- [ ] **Step 4: Add the transition table and the two structs**

Below the enums in `crates/ratchet/src/model.rs`:
```rust
/// Where a task may go from each status, in the order the message lists them. Ported from the
/// `tasks` spec: any status may go back to `backlog`, and `in_progress`, `blocked` and `review`
/// may go back to `ready` — that is what letting go of a task means.
pub fn allowed_transitions(from: TaskStatus) -> &'static [TaskStatus] {
    use TaskStatus::*;
    match from {
        Backlog => &[Ready],
        Ready => &[InProgress, Backlog],
        InProgress => &[Blocked, Review, Done, Ready, Backlog],
        Blocked => &[InProgress, Ready, Backlog],
        Review => &[InProgress, Done, Ready, Backlog],
        Done => &[Backlog],
    }
}

/// A unit of work. `repo` is the display name of the repo it was created in; `repo_root` is the
/// key every lookup scopes by (G1-R1). Progress is never a field: it is computed from the
/// checklist on every read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub body: String,
    pub repo: String,
    pub repo_root: String,
    pub status: TaskStatus,
    pub priority: i64,
    pub parent_id: Option<String>,
    pub tags: Vec<String>,
    pub claimed_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

impl Task {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Task> {
        let created: String = row.get("created_at")?;
        let updated: String = row.get("updated_at")?;
        let archived: Option<String> = row.get("archived_at")?;
        let tags: String = row.get("tags")?;
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch");
        Ok(Task {
            id: row.get("id")?,
            title: row.get("title")?,
            body: row.get("body")?,
            repo: row.get("repo")?,
            repo_root: row.get("repo_root")?,
            status: TaskStatus::from_db(&row.get::<_, String>("status")?),
            priority: row.get("priority")?,
            parent_id: row.get("parent_id")?,
            // Lenient like every other read: a row a newer build wrote must not fail a hook.
            tags: serde_json::from_str(&tags).unwrap_or_default(),
            claimed_by: row.get("claimed_by")?,
            created_at: clock::parse(&created).unwrap_or(epoch),
            updated_at: clock::parse(&updated).unwrap_or(epoch),
            archived_at: archived.as_deref().and_then(clock::parse),
        })
    }
}

/// One acceptance criterion. `position` is 1-based and is what the CLI names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChecklistItem {
    pub id: i64,
    pub task_id: String,
    pub position: i64,
    pub text: String,
    pub done: bool,
    pub done_by_session: Option<String>,
    pub done_at: Option<DateTime<Utc>>,
}

impl ChecklistItem {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<ChecklistItem> {
        let done_at: Option<String> = row.get("done_at")?;
        Ok(ChecklistItem {
            id: row.get("id")?,
            task_id: row.get("task_id")?,
            position: row.get("position")?,
            text: row.get("text")?,
            done: row.get::<_, i64>("done")? != 0,
            done_by_session: row.get("done_by_session")?,
            done_at: done_at.as_deref().and_then(clock::parse),
        })
    }
}
```

Items no task consumes yet get `#[allow(dead_code)]` with a comment naming the task that consumes them (groups 0 and 1's convention): `Task` and `allowed_transitions` are consumed by Task 4 and Task 5, `ChecklistItem` by Task 4 and Task 6.

- [ ] **Step 5: Run the tests**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet model:: && cargo test -p ratchet && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: the 5 new unit tests green, group 1's 3 `model::` tests still green, the rest of the suite unchanged (the 32 scenario tests of Task 2 still red).

- [ ] **Step 6: Hand off (no git)**

List the one file. State the 13 event kinds and their strings for the reviewer.

---

### Task 4: `services/tasks.rs` — the reads

Ported from the read half of `C:\repos\ops\ops\core\services\tasks.py` (`get`, `list_tasks`, `checklist`, `progress`, `last_handoff`). Group 1's `orphaned`, `release`, `release_dead` and `claimed_ids` stay exactly as they are; this task only appends.

**Files:**
- Modify: `crates/ratchet/src/services/tasks.rs`

**Interfaces:**
- Consumes: `model::{Task, ChecklistItem, TaskStatus, Event, EventKind}` (Task 3, group 1), `services::{ServiceError, events}` (group 1).
- Produces:
  - `tasks::Filter<'a> { repo_root: Option<&'a str>, repo: Option<&'a str>, statuses: &'a [TaskStatus], claimed_by: Option<&'a str>, tag: Option<&'a str>, include_archived: bool }`, `Default`.
  - `tasks::get(conn: &Connection, task_id: &str) -> Result<Task, ServiceError>` — `NotFound` when it does not exist.
  - `tasks::list(conn: &Connection, filter: &Filter<'_>) -> Result<Vec<Task>, ServiceError>` — ordered by priority then id; archived rows excluded unless asked for.
  - `tasks::checklist(conn: &Connection, task_id: &str) -> Result<Vec<ChecklistItem>, ServiceError>` — by position.
  - `tasks::progress(conn: &Connection, task_id: &str) -> Result<Option<(i64, i64)>, ServiceError>` — `None` when there is no checklist. Never stored.
  - `tasks::last_handoff(conn: &Connection, task_id: &str) -> Result<Option<Event>, ServiceError>`.
- **Preserves:** every function group 1 put in this file keeps its signature and its body.

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` at the bottom of `crates/ratchet/src/services/tasks.rs` (group 1's helpers `at` and `setup` stay):
```rust
    /// Seeds one task directly. Task 5 gives this module a real `create`; until it exists the
    /// reads need rows from somewhere, and a test inside the binary may use its own SQL.
    fn seed(conn: &Connection, id: &str, status: TaskStatus, priority: i64, tags: &str) {
        conn.execute(
            "INSERT INTO tasks(id,title,body,repo,repo_root,status,priority,tags,claimed_by,\
             created_at,updated_at) VALUES (?1,?2,'','demo','root',?3,?4,?5,NULL,?6,?6)",
            params![id, format!("title of {id}"), status.as_str(), priority, tags, "2026-09-16T12:00:00Z"],
        )
        .unwrap();
    }

    #[test]
    fn get_finds_a_task_and_says_so_when_it_does_not_exist() {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        seed(&c, "T-0001", TaskStatus::Ready, 3, "[\"one\"]");
        let task = get(&c, "T-0001").unwrap();
        assert_eq!(task.title, "title of T-0001");
        assert_eq!(task.tags, vec!["one".to_string()]);
        let err = get(&c, "T-9999").unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)), "{err:?}");
        assert!(err.to_string().contains("T-9999"), "{err}");
    }

    #[test]
    fn list_orders_by_priority_then_id_and_filters() {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        seed(&c, "T-0001", TaskStatus::Ready, 3, "[]");
        seed(&c, "T-0002", TaskStatus::Ready, 1, "[\"urgent\"]");
        seed(&c, "T-0003", TaskStatus::Backlog, 2, "[]");
        let all = list(&c, &Filter::default()).unwrap();
        assert_eq!(
            all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["T-0002", "T-0003", "T-0001"]
        );
        let ready = list(
            &c,
            &Filter {
                statuses: &[TaskStatus::Ready],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ready.len(), 2);
        let tagged = list(
            &c,
            &Filter {
                tag: Some("urgent"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(tagged.len(), 1);
        let elsewhere = list(
            &c,
            &Filter {
                repo_root: Some("another root"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(elsewhere.is_empty());
    }

    #[test]
    fn list_hides_archived_tasks_unless_asked() {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        seed(&c, "T-0001", TaskStatus::Done, 3, "[]");
        c.execute(
            "UPDATE tasks SET archived_at = '2026-09-16T12:00:00Z' WHERE id = 'T-0001'",
            [],
        )
        .unwrap();
        assert!(list(&c, &Filter::default()).unwrap().is_empty());
        let with_archived = list(
            &c,
            &Filter {
                include_archived: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(with_archived.len(), 1);
    }

    #[test]
    fn progress_is_derived_and_absent_without_a_checklist() {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        seed(&c, "T-0001", TaskStatus::InProgress, 3, "[]");
        assert_eq!(progress(&c, "T-0001").unwrap(), None);
        for (pos, text) in [(1, "a"), (2, "b"), (3, "c")] {
            c.execute(
                "INSERT INTO checklist_items(task_id, position, text) VALUES ('T-0001', ?1, ?2)",
                params![pos, text],
            )
            .unwrap();
        }
        assert_eq!(progress(&c, "T-0001").unwrap(), Some((0, 3)));
        c.execute(
            "UPDATE checklist_items SET done = 1 WHERE task_id = 'T-0001' AND position = 2",
            [],
        )
        .unwrap();
        assert_eq!(progress(&c, "T-0001").unwrap(), Some((1, 3)));
        let items = checklist(&c, "T-0001").unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].position, 1);
        assert!(items[1].done);
    }

    #[test]
    fn last_handoff_is_the_most_recent_one() {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        seed(&c, "T-0001", TaskStatus::InProgress, 3, "[]");
        assert!(last_handoff(&c, "T-0001").unwrap().is_none());
        for (minute, text) in [("10", "the early one"), ("12", "the late one")] {
            events::emit(
                &c,
                EventKind::Handoff,
                &json!({ "text": text }),
                Source::Cli,
                Some("s-1"),
                Some("T-0001"),
                at(&format!("2026-09-16T{minute}:00:00Z")),
            )
            .unwrap();
        }
        let last = last_handoff(&c, "T-0001").unwrap().unwrap();
        assert_eq!(last.payload["text"], "the late one");
    }
```

- [ ] **Step 2: Run to see them fail**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet services::tasks
```
Expected: compile errors naming `get`, `list`, `Filter`, `checklist`, `progress`, `last_handoff`.

- [ ] **Step 3: Implement the reads**

Append to `crates/ratchet/src/services/tasks.rs`, above the test module. The `use` block at the top of the file grows to:
```rust
use chrono::{DateTime, Utc};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, TransactionBehavior};
use serde_json::json;

use super::{events, sessions, ServiceError};
use crate::clock;
use crate::config::Thresholds;
use crate::model::{
    allowed_transitions, ChecklistItem, Event, EventKind, SessionState, Source, Task, TaskStatus,
};
```
(`allowed_transitions` and the write-side imports are consumed by Tasks 5 and 6; add them now so the `use` block is edited once.)

```rust
/// What a listing asks for. `repo_root` is the scoping key (G1-R1); `repo` filters on the display
/// name and exists for "everything called `web` on this machine".
#[derive(Debug, Default)]
pub struct Filter<'a> {
    pub repo_root: Option<&'a str>,
    pub repo: Option<&'a str>,
    pub statuses: &'a [TaskStatus],
    pub claimed_by: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub include_archived: bool,
}

pub fn get(conn: &Connection, task_id: &str) -> Result<Task, ServiceError> {
    let mut stmt = conn.prepare("SELECT * FROM tasks WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![task_id], |r| Task::from_row(r))?;
    match rows.next() {
        Some(row) => Ok(row?),
        None => Err(ServiceError::NotFound(format!(
            "task {task_id} does not exist"
        ))),
    }
}

/// Priority first (1 is the most urgent), then identifier, so a listing reads like a queue.
/// `tag` is filtered in Rust: tags are a JSON array in one column and SQLite has no operator for
/// it that is worth a dependency.
pub fn list(conn: &Connection, filter: &Filter<'_>) -> Result<Vec<Task>, ServiceError> {
    let mut sql = String::from("SELECT * FROM tasks WHERE 1 = 1");
    let mut args: Vec<String> = Vec::new();
    if !filter.include_archived {
        sql.push_str(" AND archived_at IS NULL");
    }
    if let Some(root) = filter.repo_root {
        sql.push_str(" AND repo_root = ?");
        args.push(root.to_string());
    }
    if let Some(name) = filter.repo {
        sql.push_str(" AND repo = ?");
        args.push(name.to_string());
    }
    if !filter.statuses.is_empty() {
        let holes = vec!["?"; filter.statuses.len()].join(",");
        sql.push_str(&format!(" AND status IN ({holes})"));
        args.extend(filter.statuses.iter().map(|s| s.as_str().to_string()));
    }
    if let Some(session_id) = filter.claimed_by {
        sql.push_str(" AND claimed_by = ?");
        args.push(session_id.to_string());
    }
    sql.push_str(" ORDER BY priority ASC, id ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), |r| Task::from_row(r))?;
    let mut out = Vec::new();
    for row in rows {
        let task = row?;
        if let Some(tag) = filter.tag {
            if !task.tags.iter().any(|t| t == tag) {
                continue;
            }
        }
        out.push(task);
    }
    Ok(out)
}

pub fn checklist(conn: &Connection, task_id: &str) -> Result<Vec<ChecklistItem>, ServiceError> {
    let mut stmt =
        conn.prepare("SELECT * FROM checklist_items WHERE task_id = ?1 ORDER BY position")?;
    let rows = stmt.query_map(params![task_id], |r| ChecklistItem::from_row(r))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Items done over items total, computed here every time. `None` means the task has no checklist:
/// that is *no progress*, not zero, and no face may turn it into a number (spec §4.5).
pub fn progress(conn: &Connection, task_id: &str) -> Result<Option<(i64, i64)>, ServiceError> {
    let (done, total): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(done), 0), COUNT(*) FROM checklist_items WHERE task_id = ?1",
        params![task_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(if total == 0 { None } else { Some((done, total)) })
}

pub fn last_handoff(conn: &Connection, task_id: &str) -> Result<Option<Event>, ServiceError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM events WHERE task_id = ?1 AND kind = ?2 ORDER BY id DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map(params![task_id, EventKind::Handoff.as_str()], |r| {
        Event::from_row(r)
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}
```

- [ ] **Step 4: Run the tests**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet services:: && cargo test -p ratchet && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: 5 new unit tests green, group 1's 7 in this module still green, nothing else changed.

- [ ] **Step 5: Hand off (no git)**

List the one file. State for the reviewer that `progress` returns `Option` and that nothing stores it.

---

### Task 5: `services/tasks.rs` — create, transition, archive, unarchive

Ported from `create`, `transition`, `_set_status`, `_require_done_evidence`, `archive` and `unarchive` of `C:\repos\ops\ops\core\services\tasks.py`. The one thing this task changes in group 1's code is named below and is additive.

**Files:**
- Modify: `crates/ratchet/src/services/tasks.rs`

**Interfaces:**
- Consumes: Task 4's reads, `model::allowed_transitions`, `events::emit`, `clock::iso`.
- Produces:
  - `tasks::NewTask<'a> { title, body, repo, repo_root, priority, parent_id: Option<&'a str>, tags: &'a [String], checklist: &'a [String] }`
  - `tasks::create(conn: &mut Connection, new: NewTask<'_>, source: Source, session_id: Option<&str>, now: DateTime<Utc>) -> Result<Task, ServiceError>`
  - `tasks::transition(conn: &mut Connection, task_id: &str, to: TaskStatus, source: Source, session_id: Option<&str>, why: Option<&str>, now) -> Result<Task, ServiceError>`
  - `tasks::archive(conn: &mut Connection, task_id: &str, source, session_id, now) -> Result<Task, ServiceError>`
  - `tasks::unarchive(conn: &mut Connection, task_id: &str, source, session_id, now) -> Result<Task, ServiceError>`
  - private `set_status_in(conn: &Connection, task: &Task, to, source, session_id, why, now) -> Result<(), ServiceError>` — the one place that writes a status and its event. Task 6 calls it from inside its own transaction.
- **Preserves:** `release`, `release_dead`, `orphaned` and `claimed_ids` keep their exact signatures and their exact event sequence (`task.status` then `note`). `hooks/dispatch.rs` is **not** edited by this task. The only change to group 1's code is inside `release`: it now routes its status write through `set_status_in`, which adds `from` and `claim_released` to the `task.status` payload it already wrote. Group 1's contract pins `to` and `why` and its scenario tests assert kinds and order, so this is additive (Ruling G2-R4).

- [ ] **Step 1: Write the failing tests**

Append to the test module of `crates/ratchet/src/services/tasks.rs`:
```rust
    fn fresh() -> Connection {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        c
    }

    fn simple<'a>(title: &'a str, checklist: &'a [String]) -> NewTask<'a> {
        NewTask {
            title,
            body: "",
            repo: "demo",
            repo_root: "root",
            priority: 3,
            parent_id: None,
            tags: &[],
            checklist,
        }
    }

    #[test]
    fn create_numbers_tasks_in_sequence_and_never_reuses_a_number() {
        let mut c = fresh();
        let items = vec!["one".to_string(), "two".to_string()];
        let first = create(&mut c, simple("first", &items), Source::Cli, Some("s-1"), at("2026-09-16T12:00:00Z")).unwrap();
        assert_eq!(first.id, "T-0001");
        assert_eq!(first.status, TaskStatus::Backlog);
        assert_eq!(checklist(&c, &first.id).unwrap().len(), 2);
        c.execute("DELETE FROM checklist_items WHERE task_id = 'T-0001'", []).unwrap();
        c.execute("DELETE FROM tasks WHERE id = 'T-0001'", []).unwrap();
        let second = create(&mut c, simple("second", &[]), Source::Cli, None, at("2026-09-16T12:01:00Z")).unwrap();
        assert_eq!(second.id, "T-0002", "a number was reused");
        assert_eq!(
            events::for_task(&c, "T-0002", 10).unwrap()[0].kind,
            "task.created"
        );
    }

    #[test]
    fn create_refuses_an_empty_title_a_bad_priority_and_a_grandchild() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let blank = NewTask { title: "   ", ..simple("x", &[]) };
        assert!(matches!(create(&mut c, blank, Source::Cli, None, ts).unwrap_err(), ServiceError::Invalid(_)));
        let urgent = NewTask { priority: 7, ..simple("x", &[]) };
        let err = create(&mut c, urgent, Source::Cli, None, ts).unwrap_err();
        assert!(err.to_string().contains('4'), "{err}");
        let parent = create(&mut c, simple("parent", &[]), Source::Cli, None, ts).unwrap();
        let child = create(&mut c, NewTask { parent_id: Some(&parent.id), ..simple("child", &[]) }, Source::Cli, None, ts).unwrap();
        let err = create(&mut c, NewTask { parent_id: Some(&child.id), ..simple("grandchild", &[]) }, Source::Cli, None, ts).unwrap_err();
        assert!(err.to_string().contains("one level"), "{err}");
        assert_eq!(list(&c, &Filter::default()).unwrap().len(), 2);
    }

    #[test]
    fn transition_follows_the_table_and_records_origin_and_destination() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        let err = transition(&mut c, &task.id, TaskStatus::Done, Source::Cli, None, None, ts).unwrap_err();
        assert!(err.to_string().contains("ready"), "allowed list missing: {err}");
        transition(&mut c, &task.id, TaskStatus::Ready, Source::Cli, None, None, ts).unwrap();
        let moved = transition(&mut c, &task.id, TaskStatus::InProgress, Source::Cli, Some("s-1"), None, ts).unwrap();
        assert_eq!(moved.status, TaskStatus::InProgress);
        let blocked = transition(&mut c, &task.id, TaskStatus::Blocked, Source::Cli, Some("s-1"), Some("waiting"), ts).unwrap();
        assert_eq!(blocked.status, TaskStatus::Blocked);
        let last = events::for_task(&c, &task.id, 10).unwrap().pop().unwrap();
        assert_eq!(last.kind, "task.status");
        assert_eq!(last.payload["from"], "in_progress");
        assert_eq!(last.payload["to"], "blocked");
        assert_eq!(last.payload["why"], "waiting");
    }

    #[test]
    fn done_needs_the_checklist_complete_or_a_reason() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let items = vec!["one".to_string(), "two".to_string()];
        let with = create(&mut c, simple("with a checklist", &items), Source::Cli, None, ts).unwrap();
        transition(&mut c, &with.id, TaskStatus::Ready, Source::Cli, None, None, ts).unwrap();
        transition(&mut c, &with.id, TaskStatus::InProgress, Source::Cli, None, None, ts).unwrap();
        let err = transition(&mut c, &with.id, TaskStatus::Done, Source::Cli, None, None, ts).unwrap_err();
        assert!(err.to_string().contains("two"), "{err}");
        let without = create(&mut c, simple("no checklist", &[]), Source::Cli, None, ts).unwrap();
        transition(&mut c, &without.id, TaskStatus::Ready, Source::Cli, None, None, ts).unwrap();
        transition(&mut c, &without.id, TaskStatus::InProgress, Source::Cli, None, None, ts).unwrap();
        assert!(transition(&mut c, &without.id, TaskStatus::Done, Source::Cli, None, None, ts).is_err());
        let done = transition(&mut c, &without.id, TaskStatus::Done, Source::Cli, None, Some("obsolete"), ts).unwrap();
        assert_eq!(done.status, TaskStatus::Done);
    }

    #[test]
    fn entering_done_gives_back_the_claim_and_says_so() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let task = create(&mut c, simple("held", &[]), Source::Cli, None, ts).unwrap();
        transition(&mut c, &task.id, TaskStatus::Ready, Source::Cli, None, None, ts).unwrap();
        transition(&mut c, &task.id, TaskStatus::InProgress, Source::Cli, None, None, ts).unwrap();
        c.execute("UPDATE tasks SET claimed_by = 's-1' WHERE id = ?1", params![task.id]).unwrap();
        let done = transition(&mut c, &task.id, TaskStatus::Done, Source::Cli, Some("s-1"), Some("shipped"), ts).unwrap();
        assert_eq!(done.claimed_by, None);
        let last = events::for_task(&c, &task.id, 10).unwrap().pop().unwrap();
        assert_eq!(last.payload["claim_released"], "s-1");
    }

    #[test]
    fn archiving_hides_a_done_task_and_refuses_anything_else() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        let err = archive(&mut c, &task.id, Source::Cli, None, ts).unwrap_err();
        assert!(err.to_string().contains("done"), "{err}");
        transition(&mut c, &task.id, TaskStatus::Ready, Source::Cli, None, None, ts).unwrap();
        transition(&mut c, &task.id, TaskStatus::InProgress, Source::Cli, None, None, ts).unwrap();
        transition(&mut c, &task.id, TaskStatus::Done, Source::Cli, None, Some("shipped"), ts).unwrap();
        let archived = archive(&mut c, &task.id, Source::Cli, None, ts).unwrap();
        assert!(archived.archived_at.is_some());
        assert!(list(&c, &Filter::default()).unwrap().is_empty());
        assert!(archive(&mut c, &task.id, Source::Cli, None, ts).is_err());
        let back = unarchive(&mut c, &task.id, Source::Cli, None, ts).unwrap();
        assert!(back.archived_at.is_none());
        assert_eq!(list(&c, &Filter::default()).unwrap().len(), 1);
        let kinds: Vec<String> = events::for_task(&c, &task.id, 20).unwrap().into_iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&"task.archived".to_string()));
        assert!(kinds.contains(&"task.unarchived".to_string()));
    }

    #[test]
    fn an_archived_task_cannot_be_moved_until_it_comes_back() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        transition(&mut c, &task.id, TaskStatus::Ready, Source::Cli, None, None, ts).unwrap();
        transition(&mut c, &task.id, TaskStatus::InProgress, Source::Cli, None, None, ts).unwrap();
        transition(&mut c, &task.id, TaskStatus::Done, Source::Cli, None, Some("shipped"), ts).unwrap();
        archive(&mut c, &task.id, Source::Cli, None, ts).unwrap();
        let err = transition(&mut c, &task.id, TaskStatus::Backlog, Source::Cli, None, None, ts).unwrap_err();
        assert!(err.to_string().contains("unarchive"), "{err}");
    }
```

- [ ] **Step 2: Run to see them fail**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet services::tasks
```
Expected: compile errors naming `create`, `NewTask`, `transition`, `archive`, `unarchive`.

- [ ] **Step 3: Implement**

Append to `crates/ratchet/src/services/tasks.rs`:
```rust
/// What `create` needs. `repo`/`repo_root` come from the marker the face resolved: ratchet has no
/// registry of repos, so a task cannot be created outside one (G2-R1).
pub struct NewTask<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub repo: &'a str,
    pub repo_root: &'a str,
    pub priority: i64,
    pub parent_id: Option<&'a str>,
    pub tags: &'a [String],
    pub checklist: &'a [String],
}

/// Creates a task in `backlog` with its checklist, inside one transaction, and appends one
/// `task.created` event. The identifier comes from the `task_seq` table, so a deleted or archived
/// task never gives its number back.
pub fn create(
    conn: &mut Connection,
    new: NewTask<'_>,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Task, ServiceError> {
    if new.title.trim().is_empty() {
        return Err(ServiceError::Invalid("the title cannot be empty".into()));
    }
    if !(1..=4).contains(&new.priority) {
        return Err(ServiceError::Invalid(
            "the priority goes from 1 (most urgent) to 4".into(),
        ));
    }
    if let Some(parent_id) = new.parent_id {
        let parent = get(conn, parent_id)?;
        if parent.parent_id.is_some() {
            return Err(ServiceError::Invalid(format!(
                "{parent_id} is already a subtask; only one level of nesting is allowed"
            )));
        }
    }
    let ts = clock::iso(now);
    let tags = serde_json::to_string(new.tags).unwrap_or_else(|_| "[]".to_string());
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("INSERT INTO task_seq DEFAULT VALUES", [])?;
    // `last_insert_rowid()` is the connection's, not the statement's: this read must stay here,
    // immediately after the sequence insert and before any other write in this transaction.
    // Moving it below the `tasks` insert would silently number the task after the wrong row.
    let task_id = format!("T-{:04}", tx.last_insert_rowid());
    tx.execute(
        "INSERT INTO tasks(id,title,body,repo,repo_root,status,priority,parent_id,tags,claimed_by,\
         created_at,updated_at,archived_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,?10,?10,NULL)",
        params![
            task_id,
            new.title.trim(),
            new.body,
            new.repo,
            new.repo_root,
            TaskStatus::Backlog.as_str(),
            new.priority,
            new.parent_id,
            tags,
            ts
        ],
    )?;
    for (position, text) in new.checklist.iter().enumerate() {
        tx.execute(
            "INSERT INTO checklist_items(task_id, position, text) VALUES (?1, ?2, ?3)",
            params![task_id, (position + 1) as i64, text],
        )?;
    }
    events::emit(
        &tx,
        EventKind::TaskCreated,
        &json!({
            "title": new.title.trim(),
            "repo": new.repo,
            "priority": new.priority,
            "parent_id": new.parent_id,
            "tags": new.tags,
            "checklist": new.checklist.len(),
        }),
        source,
        session_id,
        Some(&task_id),
        now,
    )?;
    tx.commit()?;
    get(conn, &task_id)
}

/// Moves a task, checking the table and the evidence `done` needs. One transaction, one event.
pub fn transition(
    conn: &mut Connection,
    task_id: &str,
    to: TaskStatus,
    source: Source,
    session_id: Option<&str>,
    why: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Task, ServiceError> {
    let task = get(conn, task_id)?;
    if task.archived_at.is_some() {
        return Err(ServiceError::Invalid(format!(
            "{task_id} is archived; unarchive it (ratchet task unarchive {task_id}) before moving it"
        )));
    }
    let allowed = allowed_transitions(task.status);
    if !allowed.contains(&to) {
        let listed = allowed
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ServiceError::Invalid(format!(
            "{task_id} cannot go from {} to {}; allowed from {}: {listed}",
            task.status.as_str(),
            to.as_str(),
            task.status.as_str()
        )));
    }
    if to == TaskStatus::Done {
        require_done_evidence(conn, &task, why)?;
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    set_status_in(&tx, &task, to, source, session_id, why, now)?;
    tx.commit()?;
    get(conn, task_id)
}

pub fn archive(
    conn: &mut Connection,
    task_id: &str,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Task, ServiceError> {
    let task = get(conn, task_id)?;
    if task.status != TaskStatus::Done {
        return Err(ServiceError::Invalid(format!(
            "only a done task can be archived; {task_id} is {}",
            task.status.as_str()
        )));
    }
    if task.archived_at.is_some() {
        return Err(ServiceError::Invalid(format!("{task_id} is already archived")));
    }
    let ts = clock::iso(now);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE tasks SET archived_at = ?1, updated_at = ?1 WHERE id = ?2",
        params![ts, task_id],
    )?;
    events::emit(&tx, EventKind::TaskArchived, &json!({}), source, session_id, Some(task_id), now)?;
    tx.commit()?;
    get(conn, task_id)
}

pub fn unarchive(
    conn: &mut Connection,
    task_id: &str,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Task, ServiceError> {
    let task = get(conn, task_id)?;
    let Some(archived_at) = task.archived_at else {
        return Err(ServiceError::Invalid(format!("{task_id} is not archived")));
    };
    let ts = clock::iso(now);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE tasks SET archived_at = NULL, updated_at = ?1 WHERE id = ?2",
        params![ts, task_id],
    )?;
    events::emit(
        &tx,
        EventKind::TaskUnarchived,
        &json!({ "archived_at": clock::iso(archived_at) }),
        source,
        session_id,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    get(conn, task_id)
}

/// The only place a status is written. Takes the open connection (a `&Transaction` derefs into
/// one), so a caller that changes a status twice — `claim` from `backlog` — does it all inside one
/// transaction. Entering `done` gives the claim back: a finished task with a holder is dirty data,
/// and the event records which session it was.
fn set_status_in(
    conn: &Connection,
    task: &Task,
    to: TaskStatus,
    source: Source,
    session_id: Option<&str>,
    why: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), ServiceError> {
    let ts = clock::iso(now);
    let released = if to == TaskStatus::Done {
        task.claimed_by.clone()
    } else {
        None
    };
    if released.is_some() {
        conn.execute(
            "UPDATE tasks SET status = ?1, claimed_by = NULL, updated_at = ?2 WHERE id = ?3",
            params![to.as_str(), ts, task.id],
        )?;
    } else {
        conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![to.as_str(), ts, task.id],
        )?;
    }
    events::emit(
        conn,
        EventKind::TaskStatus,
        &json!({
            "from": task.status.as_str(),
            "to": to.as_str(),
            "why": why,
            "claim_released": released,
        }),
        source,
        session_id,
        Some(&task.id),
        now,
    )?;
    Ok(())
}

/// `done` is the one status that needs evidence: every item checked, or an explicit reason when
/// there is no checklist to check.
fn require_done_evidence(
    conn: &Connection,
    task: &Task,
    why: Option<&str>,
) -> Result<(), ServiceError> {
    let items = checklist(conn, &task.id)?;
    if items.is_empty() {
        if why.map(|w| w.trim().is_empty()).unwrap_or(true) {
            return Err(ServiceError::Invalid(format!(
                "{} has no checklist: give a reason (--why) to close it as done",
                task.id
            )));
        }
        return Ok(());
    }
    let pending: Vec<String> = items
        .iter()
        .filter(|i| !i.done)
        .map(|i| format!("{}. {}", i.position, i.text))
        .collect();
    if pending.is_empty() {
        return Ok(());
    }
    Err(ServiceError::Invalid(format!(
        "cannot close {}: checklist pending: {}",
        task.id,
        pending.join("; ")
    )))
}
```

- [ ] **Step 4: Route `release` through `set_status_in`**

In group 1's `release`, keep the signature, the holder lookup, the `why` string, the transaction and the `note` event exactly as they are. Replace only the `UPDATE` and the first `events::emit` with one call, so the status write lives in one place:
```rust
    let task = get(conn, task_id)?;
    // … the existing holder lookup and `why` stay here, unchanged …
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    set_status_in(&tx, &task, TaskStatus::Ready, source, None, Some(&why), now)?;
    tx.execute(
        "UPDATE tasks SET claimed_by = NULL WHERE id = ?1",
        params![task_id],
    )?;
    events::emit(
        &tx,
        EventKind::Note,
        &json!({"text": format!("{why}; it was held by session {holder}")}),
        source,
        None,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    Ok(())
```
The sequence of events is still `task.status` then `note`; the payload of the first now also carries `from` and `claim_released` (Ruling G2-R4). Group 1's scenario tests assert the kinds and the `to`/`why` keys and stay green.

- [ ] **Step 5: Run the tests, including group 1's**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet services:: && cargo test -p ratchet --test spec sessions__ && cargo test -p ratchet && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: the 7 new unit tests green, group 1's 7 in this module green, **all 24 `sessions__` scenario tests still green** — that is the check that the `release` edit was additive.

- [ ] **Step 6: Hand off (no git)**

List the one file. Say explicitly which lines of `release` changed and confirm the 24 `sessions__` scenarios are green.

---

### Task 6: `services/tasks.rs` — claim, checklist, notes, handoffs

Ported from `claim`, `check`, `uncheck`, `_set_item`, `note`, `handoff` and `_claimed_since` of `C:\repos\ops\ops\core\services\tasks.py`.

**Files:**
- Modify: `crates/ratchet/src/services/tasks.rs`

**Interfaces:**
- Consumes: Tasks 4 and 5 (`get`, `list`, `checklist`, `set_status_in`), `sessions::{get, state}` and `config::Thresholds` (group 1).
- Produces:
  - `tasks::claim(conn: &mut Connection, task_id: &str, session_id: &str, th: &Thresholds, source: Source, now) -> Result<Task, ServiceError>`
  - `tasks::check(conn: &mut Connection, task_id: &str, position: i64, source, session_id: Option<&str>, now) -> Result<ChecklistItem, ServiceError>`
  - `tasks::uncheck(…)` — same shape.
  - `tasks::note(conn: &mut Connection, task_id: &str, text: &str, source, session_id, now) -> Result<Event, ServiceError>`
  - `tasks::handoff(…)` — same shape; refuses blank text.
- Note for the reviewer: marking an item bumps `tasks.updated_at` in the same transaction and emits **one** event (`checklist.done` or `checklist.undone`). The timestamp bump is part of that fact, not a second one.

- [ ] **Step 1: Write the failing tests**

Append to the test module of `crates/ratchet/src/services/tasks.rs`:
```rust
    fn with_session(id: &str, at_iso: &str) -> Connection {
        let mut c = fresh();
        sessions::upsert_start(
            &mut c,
            StartInput {
                session_id: id,
                repo: "demo",
                repo_root: "root",
                cwd: "root",
                worktree: None,
                branch: None,
                mode: SessionMode::Interactive,
                launched_by: LaunchedBy::User,
            },
            at(at_iso),
        )
        .unwrap();
        c
    }

    #[test]
    fn claim_walks_a_backlog_task_all_the_way_to_in_progress() {
        let mut c = with_session("s-1", "2026-09-16T12:00:00Z");
        let ts = at("2026-09-16T12:01:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        let claimed = claim(&mut c, &task.id, "s-1", &Thresholds::default(), Source::Cli, ts).unwrap();
        assert_eq!(claimed.status, TaskStatus::InProgress);
        assert_eq!(claimed.claimed_by.as_deref(), Some("s-1"));
        let kinds: Vec<String> = events::for_task(&c, &task.id, 10).unwrap().into_iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec!["task.created", "task.status", "task.status", "task.claimed"]);
    }

    #[test]
    fn claim_refuses_an_unregistered_session_and_a_done_task() {
        let mut c = with_session("s-1", "2026-09-16T12:00:00Z");
        let ts = at("2026-09-16T12:01:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        let err = claim(&mut c, &task.id, "s-ghost", &Thresholds::default(), Source::Cli, ts).unwrap_err();
        assert!(err.to_string().contains("not registered"), "{err}");
        assert_eq!(get(&c, &task.id).unwrap().status, TaskStatus::Backlog);
        claim(&mut c, &task.id, "s-1", &Thresholds::default(), Source::Cli, ts).unwrap();
        transition(&mut c, &task.id, TaskStatus::Done, Source::Cli, Some("s-1"), Some("shipped"), ts).unwrap();
        let err = claim(&mut c, &task.id, "s-1", &Thresholds::default(), Source::Cli, ts).unwrap_err();
        assert!(err.to_string().contains("done"), "{err}");
    }

    #[test]
    fn a_live_holder_keeps_the_task_and_a_dead_one_hands_it_over() {
        let mut c = with_session("s-a", "2026-09-16T12:00:00Z");
        sessions::upsert_start(
            &mut c,
            StartInput {
                session_id: "s-b",
                repo: "demo",
                repo_root: "root",
                cwd: "root",
                worktree: None,
                branch: None,
                mode: SessionMode::Interactive,
                launched_by: LaunchedBy::User,
            },
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        let ts = at("2026-09-16T12:01:00Z");
        let th = Thresholds::default();
        let task = create(&mut c, simple("contested", &[]), Source::Cli, None, ts).unwrap();
        claim(&mut c, &task.id, "s-a", &th, Source::Cli, ts).unwrap();
        let err = claim(&mut c, &task.id, "s-b", &th, Source::Cli, ts).unwrap_err();
        assert!(err.to_string().contains("s-a"), "{err}");
        // Ninety minutes on, s-a is orphaned: the claim goes through and leaves a note.
        let later = at("2026-09-16T13:31:00Z");
        let moved = claim(&mut c, &task.id, "s-b", &th, Source::Cli, later).unwrap();
        assert_eq!(moved.claimed_by.as_deref(), Some("s-b"));
        let notes: Vec<String> = events::for_task(&c, &task.id, 20)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "note")
            .map(|e| e.payload["text"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(notes.iter().any(|n| n.contains("s-a") && n.contains("s-b")), "{notes:?}");
    }

    #[test]
    fn checking_an_item_records_who_and_when_and_emits_one_event() {
        let mut c = with_session("s-1", "2026-09-16T12:00:00Z");
        let ts = at("2026-09-16T12:01:00Z");
        let items = vec!["one".to_string(), "two".to_string()];
        let task = create(&mut c, simple("work", &items), Source::Cli, None, ts).unwrap();
        let before = events::for_task(&c, &task.id, 50).unwrap().len();
        let item = check(&mut c, &task.id, 2, Source::Cli, Some("s-1"), ts).unwrap();
        assert!(item.done);
        assert_eq!(item.done_by_session.as_deref(), Some("s-1"));
        assert_eq!(progress(&c, &task.id).unwrap(), Some((1, 2)));
        let after = events::for_task(&c, &task.id, 50).unwrap();
        assert_eq!(after.len(), before + 1, "more than one event for one fact");
        assert_eq!(after.last().unwrap().kind, "checklist.done");
        let undone = uncheck(&mut c, &task.id, 2, Source::Cli, Some("s-1"), ts).unwrap();
        assert!(!undone.done);
        assert_eq!(undone.done_by_session, None);
        assert_eq!(progress(&c, &task.id).unwrap(), Some((0, 2)));
    }

    #[test]
    fn a_position_that_does_not_exist_lists_the_ones_that_do() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let items = vec!["one".to_string()];
        let task = create(&mut c, simple("work", &items), Source::Cli, None, ts).unwrap();
        let err = check(&mut c, &task.id, 9, Source::Cli, None, ts).unwrap_err();
        assert!(err.to_string().contains("1. one"), "{err}");
        let bare = create(&mut c, simple("no items", &[]), Source::Cli, None, ts).unwrap();
        let err = check(&mut c, &bare.id, 1, Source::Cli, None, ts).unwrap_err();
        assert!(err.to_string().contains("no checklist"), "{err}");
    }

    #[test]
    fn notes_are_free_text_and_a_handoff_cannot_be_blank() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        let ev = note(&mut c, &task.id, "found X, decided Y", Source::Cli, Some("s-1"), ts).unwrap();
        assert_eq!(ev.payload["text"], "found X, decided Y");
        let err = handoff(&mut c, &task.id, "   ", Source::Cli, Some("s-1"), ts).unwrap_err();
        assert!(err.to_string().contains("resume"), "{err}");
        assert!(last_handoff(&c, &task.id).unwrap().is_none());
        handoff(&mut c, &task.id, "half done; resume at item 2", Source::Cli, Some("s-1"), ts).unwrap();
        assert_eq!(
            last_handoff(&c, &task.id).unwrap().unwrap().payload["text"],
            "half done; resume at item 2"
        );
        assert!(note(&mut c, "T-9999", "into the void", Source::Cli, None, ts).is_err());
    }
```

- [ ] **Step 2: Run to see them fail**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet services::tasks
```
Expected: compile errors naming `claim`, `check`, `uncheck`, `note`, `handoff`.

- [ ] **Step 3: Implement**

Append to `crates/ratchet/src/services/tasks.rs`:
```rust
/// Puts the task in `in_progress` under `session_id`. A task in `backlog` goes through `ready`
/// (two status events) so the history reads the same whichever door it came in by. The claiming
/// session must be registered; a task held by a session that is still live or idle is not taken
/// from it, and one held by a dead session is, with a note.
pub fn claim(
    conn: &mut Connection,
    task_id: &str,
    session_id: &str,
    th: &Thresholds,
    source: Source,
    now: DateTime<Utc>,
) -> Result<Task, ServiceError> {
    let task = get(conn, task_id)?;
    if task.archived_at.is_some() {
        return Err(ServiceError::Invalid(format!(
            "{task_id} is archived; unarchive it before claiming it"
        )));
    }
    if task.status == TaskStatus::Done {
        return Err(ServiceError::Invalid(format!(
            "{task_id} is done; a done task cannot be claimed"
        )));
    }
    if sessions::get(conn, session_id)?.is_none() {
        return Err(ServiceError::Invalid(format!(
            "session {session_id} is not registered; {task_id} was not claimed"
        )));
    }
    let previous = task.claimed_by.clone();
    let mut transferred_from: Option<(String, SessionState)> = None;
    if let Some(holder_id) = previous.as_deref() {
        if holder_id != session_id {
            let holder_state = match sessions::get(conn, holder_id)? {
                Some(holder) => sessions::state(&holder, th, now),
                None => SessionState::Orphaned,
            };
            if matches!(holder_state, SessionState::Live | SessionState::Idle) {
                let since = claimed_since(conn, task_id, holder_id)?
                    .unwrap_or_else(|| clock::iso(task.updated_at));
                return Err(ServiceError::Invalid(format!(
                    "{task_id} is held by session {holder_id} ({}) since {since}",
                    holder_state.as_str()
                )));
            }
            transferred_from = Some((holder_id.to_string(), holder_state));
        }
    }
    let ts = clock::iso(now);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut current = task.clone();
    if current.status == TaskStatus::Backlog {
        set_status_in(&tx, &current, TaskStatus::Ready, source, Some(session_id), None, now)?;
        current.status = TaskStatus::Ready;
    }
    if current.status != TaskStatus::InProgress {
        set_status_in(&tx, &current, TaskStatus::InProgress, source, Some(session_id), None, now)?;
        current.status = TaskStatus::InProgress;
    }
    tx.execute(
        "UPDATE tasks SET claimed_by = ?1, updated_at = ?2 WHERE id = ?3",
        params![session_id, ts, task_id],
    )?;
    events::emit(
        &tx,
        EventKind::TaskClaimed,
        &json!({ "previous": previous }),
        source,
        Some(session_id),
        Some(task_id),
        now,
    )?;
    if let Some((holder_id, holder_state)) = transferred_from {
        events::emit(
            &tx,
            EventKind::Note,
            &json!({
                "text": format!(
                    "transferred from session {holder_id} ({}) to {session_id}",
                    holder_state.as_str()
                )
            }),
            source,
            Some(session_id),
            Some(task_id),
            now,
        )?;
    }
    tx.commit()?;
    get(conn, task_id)
}

pub fn check(
    conn: &mut Connection,
    task_id: &str,
    position: i64,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ChecklistItem, ServiceError> {
    set_item(conn, task_id, position, true, source, session_id, now)
}

pub fn uncheck(
    conn: &mut Connection,
    task_id: &str,
    position: i64,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ChecklistItem, ServiceError> {
    set_item(conn, task_id, position, false, source, session_id, now)
}

/// Marks or unmarks one item. The `updated_at` bump of the task belongs to the same fact as the
/// item write, so the pair emits exactly one event.
fn set_item(
    conn: &mut Connection,
    task_id: &str,
    position: i64,
    done: bool,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ChecklistItem, ServiceError> {
    let items = checklist(conn, task_id)?;
    if items.is_empty() {
        get(conn, task_id)?; // NotFound if the task itself is the problem
        return Err(ServiceError::Invalid(format!("{task_id} has no checklist")));
    }
    let Some(item) = items.iter().find(|i| i.position == position) else {
        let listed = items
            .iter()
            .map(|i| format!("{}. {}", i.position, i.text))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ServiceError::Invalid(format!(
            "{task_id} has no item {position}; available: {listed}"
        )));
    };
    let ts = clock::iso(now);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE checklist_items SET done = ?1, done_by_session = ?2, done_at = ?3 WHERE id = ?4",
        params![
            i64::from(done),
            if done { session_id } else { None },
            if done { Some(ts.as_str()) } else { None },
            item.id
        ],
    )?;
    tx.execute(
        "UPDATE tasks SET updated_at = ?1 WHERE id = ?2",
        params![ts, task_id],
    )?;
    events::emit(
        &tx,
        if done {
            EventKind::ChecklistDone
        } else {
            EventKind::ChecklistUndone
        },
        &json!({ "item_id": item.id, "position": position, "text": item.text }),
        source,
        session_id,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    let mut stmt = conn.prepare("SELECT * FROM checklist_items WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![item.id], |r| ChecklistItem::from_row(r))?;
    match rows.next() {
        Some(row) => Ok(row?),
        None => Err(ServiceError::NotFound(format!(
            "item {position} of {task_id} disappeared while writing it"
        ))),
    }
}

pub fn note(
    conn: &mut Connection,
    task_id: &str,
    text: &str,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Event, ServiceError> {
    get(conn, task_id)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ev = events::emit(
        &tx,
        EventKind::Note,
        &json!({ "text": text }),
        source,
        session_id,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    Ok(ev)
}

/// The text the next session reads first, so it is never allowed to be empty.
pub fn handoff(
    conn: &mut Connection,
    task_id: &str,
    text: &str,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Event, ServiceError> {
    get(conn, task_id)?;
    if text.trim().is_empty() {
        return Err(ServiceError::Invalid(
            "a handoff cannot be empty: what is left, and how to resume".into(),
        ));
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ev = events::emit(
        &tx,
        EventKind::Handoff,
        &json!({ "text": text }),
        source,
        session_id,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    Ok(ev)
}

/// When this session took the task, for the message that refuses a second claim.
fn claimed_since(
    conn: &Connection,
    task_id: &str,
    session_id: &str,
) -> Result<Option<String>, ServiceError> {
    Ok(conn
        .query_row(
            "SELECT ts FROM events WHERE task_id = ?1 AND session_id = ?2 AND kind = ?3 \
             ORDER BY id DESC LIMIT 1",
            params![task_id, session_id, EventKind::TaskClaimed.as_str()],
            |r| r.get::<_, String>(0),
        )
        .optional()?)
}
```
The test module needs three more imports, added to its `use` block: `use crate::model::{LaunchedBy, SessionMode};` and `use crate::services::sessions::StartInput;` (group 1 already imports `sessions`).

- [ ] **Step 4: Run the tests**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet services:: && cargo test -p ratchet && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: the 6 new unit tests green; every earlier unit test green; the 24 `sessions__` scenarios green; the 32 new scenario tests still red (no CLI yet).

- [ ] **Step 5: Hand off (no git)**

List the one file. State the event sequence of a claim from `backlog` (`task.status`, `task.status`, `task.claimed`) for the reviewer.

---
### Task 7: `output.rs` — the 60-line discipline, the task line, and `--session` as a global option

Ported from `emit`, `emit_json` and `MAX_STDOUT_LINES` of `C:\repos\ops\ops\cli\_shared.py` and from `format_task_line` of `ops\cli\briefing.py`. `format_task_line` lives here, not in `cli/`, because the briefing hook needs the same line and a hook must not import a CLI module.

**Files:**
- Create: `crates/ratchet/src/output.rs`
- Modify: `crates/ratchet/src/main.rs` (one `mod` line, one global option, one changed call)

**Interfaces:**
- Consumes: `model::Task` (Task 3), `services::tasks::progress`'s return shape `Option<(i64, i64)>` (Task 4 — only the shape, not the function).
- Produces:
  - `output::MAX_STDOUT_LINES: usize = 60`, `output::HEAD_LINES: usize = 20`
  - `output::out_dir(home: &Path) -> PathBuf` — `<home>/out`
  - `output::emit(home: &Path, name: &str, lines: &[String], now: DateTime<Utc>)`
  - `output::emit_json(home: &Path, name: &str, value: &serde_json::Value, now: DateTime<Utc>)`
  - `output::format_task_line(task: &Task, progress: Option<(i64, i64)>) -> String` — `T-0042  in_progress  p2  Title  (2/5)`
  - In `main.rs`: `--session <id>` accepted anywhere on the command line, and `ratchet session show` prefers its positional argument over it.

- [ ] **Step 1: Write the failing tests**

At the bottom of `crates/ratchet/src/output.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TaskStatus;

    fn at(s: &str) -> DateTime<Utc> {
        crate::clock::parse(s).unwrap()
    }

    fn task(id: &str, title: &str) -> Task {
        Task {
            id: id.into(),
            title: title.into(),
            body: String::new(),
            repo: "demo".into(),
            repo_root: "root".into(),
            status: TaskStatus::InProgress,
            priority: 2,
            parent_id: None,
            tags: Vec::new(),
            claimed_by: None,
            created_at: at("2026-09-16T12:00:00Z"),
            updated_at: at("2026-09-16T12:00:00Z"),
            archived_at: None,
        }
    }

    #[test]
    fn a_task_line_carries_id_status_priority_title_and_progress() {
        let line = format_task_line(&task("T-0042", "Port the board"), Some((2, 5)));
        assert!(line.starts_with("T-0042"), "{line}");
        assert!(line.contains("in_progress"), "{line}");
        assert!(line.contains("p2"), "{line}");
        assert!(line.contains("Port the board"), "{line}");
        assert!(line.contains("(2/5)"), "{line}");
        assert_eq!(line.lines().count(), 1);
    }

    #[test]
    fn a_task_with_no_checklist_shows_no_progress_at_all() {
        let line = format_task_line(&task("T-0043", "No criteria"), None);
        assert!(!line.contains('/'), "{line}");
        // Deliberately narrow: `(0/` is the shape that would mean "nothing done"; a bare `0`
        // would also match a priority or an identifier and fail for an unrelated reason.
        assert!(!line.contains("(0/"), "{line}");
    }

    #[test]
    fn short_output_never_touches_the_disk() {
        let dir = tempfile::TempDir::new().unwrap();
        let lines: Vec<String> = (1..=60).map(|n| format!("line {n}")).collect();
        emit(dir.path(), "task-list", &lines, at("2026-09-16T12:00:00Z"));
        assert!(!out_dir(dir.path()).exists(), "a short output created the directory");
    }

    #[test]
    fn long_output_goes_to_a_named_file_under_the_state_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let lines: Vec<String> = (1..=300).map(|n| format!("line {n}")).collect();
        emit(dir.path(), "task-list", &lines, at("2026-09-16T12:00:00Z"));
        let written: Vec<_> = std::fs::read_dir(out_dir(dir.path()))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(written.len(), 1, "{written:?}");
        let name = written[0].file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(name, "20260916T120000-task-list.txt");
        let body = std::fs::read_to_string(&written[0]).unwrap();
        assert_eq!(body.lines().count(), 300);
        assert!(body.ends_with('\n'));
    }

    #[test]
    fn json_output_obeys_the_same_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let big: Vec<i64> = (1..=200).collect();
        emit_json(
            dir.path(),
            "task-list",
            &serde_json::json!(big),
            at("2026-09-16T12:00:00Z"),
        );
        assert!(out_dir(dir.path()).exists());
    }
}
```

- [ ] **Step 2: Run to see them fail**

```
export PATH="$HOME/.cargo/bin:/c/Users/eillanes/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet output::
```
Expected: compile errors (no such module).

- [ ] **Step 3: Implement `crates/ratchet/src/output.rs`**

```rust
//! What a face prints. One rule (spec §4.1): more than 60 lines is not something to read in a
//! terminal, so it goes to a file under the state directory and the terminal gets the first 20
//! plus the path. Hooks are exempt — the briefing has its own 40-line cap and is context, not a
//! listing.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::model::Task;

pub const MAX_STDOUT_LINES: usize = 60;
pub const HEAD_LINES: usize = 20;

pub fn out_dir(home: &Path) -> PathBuf {
    home.join("out")
}

/// Prints `lines`, or writes them to `<home>/out/<stamp>-<name>.txt` and prints a window. If the
/// file cannot be written the whole output is printed instead: the discipline may cost a long
/// terminal, never a lost result.
pub fn emit(home: &Path, name: &str, lines: &[String], now: DateTime<Utc>) {
    if lines.len() <= MAX_STDOUT_LINES {
        for line in lines {
            println!("{line}");
        }
        return;
    }
    let dir = out_dir(home);
    let path = dir.join(format!("{}-{name}.txt", now.format("%Y%m%dT%H%M%S")));
    let body = format!("{}\n", lines.join("\n"));
    match fs::create_dir_all(&dir).and_then(|()| fs::write(&path, body)) {
        Ok(()) => {
            for line in lines.iter().take(HEAD_LINES) {
                println!("{line}");
            }
            println!("… ({} lines in {})", lines.len(), path.display());
        }
        Err(e) => {
            for line in lines {
                println!("{line}");
            }
            eprintln!("warning: could not write {}: {e}", path.display());
        }
    }
}

/// Machine output, under the same limit.
pub fn emit_json(home: &Path, name: &str, value: &serde_json::Value, now: DateTime<Utc>) {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_string());
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    emit(home, name, &lines, now);
}

/// The one line a task takes in every listing and in the briefing. A task with no checklist shows
/// no progress at all — not `(0/0)`, which would read as "nothing done" instead of "nothing to
/// do" (spec §4.5).
pub fn format_task_line(task: &Task, progress: Option<(i64, i64)>) -> String {
    let progress = progress
        .map(|(done, total)| format!("  ({done}/{total})"))
        .unwrap_or_default();
    format!(
        "{}  {:<12} p{}  {}{}",
        task.id,
        task.status.as_str(),
        task.priority,
        task.title,
        progress
    )
}
```

Add `mod output;` to the `mod` block of `crates/ratchet/src/main.rs`, in alphabetical order (append-only under G1-P1: read the block first).

- [ ] **Step 4: Make `--session` a global option**

In `crates/ratchet/src/main.rs`, add the field to `Cli` (the `Cmd` enum is untouched here):
```rust
struct Cli {
    /// Session to attribute writes to. Overrides `RATCHET_SESSION_ID` and the resolution by
    /// directory, and is accepted before or after the subcommand.
    #[arg(long, global = true)]
    session: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}
```
In `main()`, take a copy before the match consumes `cli.cmd`, and let `session show` keep preferring its positional argument:
```rust
    let session = cli.session.clone();
```
```rust
            SessionCmd::Show { id, json } => cli::session_cmd::show(
                &env,
                cwd,
                id.as_deref().or(session.as_deref()),
                json,
            ),
```
No other subcommand reads it yet; Task 8 does. **Do not** add a `--json` global option: `ratchet session list` and `ratchet session show` define their own (group 1, Task 11 — see `main.rs`) and two definitions of one argument id make clap panic at startup.

- [ ] **Step 5: Run**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet output:: && cargo test -p ratchet && cargo fmt --check && cargo clippy --all-targets -- -D warnings
cd /c/repos/ratchet && cargo run -p ratchet -- --session s-1 session show 2>&1 | head -3
cd /c/repos/ratchet && cargo run -p ratchet -- session show --session s-1 2>&1 | head -3
```
Expected: the 5 new unit tests green; the whole suite unchanged; both spellings of `--session` parse (both then fail with `error: …` because there is no such session, which is the point — clap accepted the flag in both positions).

- [ ] **Step 6: Hand off (no git)**

List the two files. State the exact file-name shape the discipline writes.

---

### Task 8: `ratchet task list|show|new|claim|status|check|note|handoff|archive|unarchive`

Ported from the `task` sub-app of `C:\repos\ops\ops\cli\app.py` and the session helpers of `ops\cli\_shared.py`. Thin: parse, resolve the session, call a service, print through `output`. No SQL in this file.

**Files:**
- Create: `crates/ratchet/src/cli/task_cmd.rs`
- Modify: `crates/ratchet/src/cli/mod.rs` (one `pub mod` line), `crates/ratchet/src/main.rs` (one `Cmd` variant, one `TaskCmd` enum, one match arm)

**Interfaces:**
- Consumes: every public function of `services::tasks` (Tasks 4-6), `services::{sessions::resolve, events::for_task}`, `output::{emit, emit_json, format_task_line}` (Task 7), `db::open_ready`, `config::ratchet_home`, `repo::{find_repo, normalize}`, `clock::{now, iso}`, and `main.rs`'s global `--session` (Task 7).
- Produces: one function per subcommand, each returning the process exit code. No later task consumes them.
- Session policy (Ruling G2-R6): `claim` **fails** (exit 1) when no session resolves, because a claim with no holder is meaningless; `new`, `status`, `check`, `note` and `handoff` record with no session and print one warning on stderr; `list --mine` fails, since it has nothing to filter by. An *unresolved* session and an *unregistered* one are two different errors with two different messages — the second comes from the service.

- [ ] **Step 1: The failing tests are the scenario tests of Task 2**

```
cd /c/repos/ratchet && cargo test -p ratchet --test spec tasks__ 2>&1 | tail -20
```
Expected: 21 failures, all saying the `task` subcommand does not exist.

- [ ] **Step 2: Write `crates/ratchet/src/cli/task_cmd.rs`**

```rust
//! `ratchet task …`: parse, resolve the session, call a service, print. No SQL here, and no
//! decision that belongs to the board — the service owns every rule.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::clock;
use crate::config::{ratchet_home, Thresholds};
use crate::db;
use crate::model::{Source, TaskStatus};
use crate::output;
use crate::repo::{find_repo, normalize};
use crate::services::{events, sessions, tasks};

const VALID_STATUSES: &str = "backlog, ready, in_progress, blocked, review, done";

/// What every subcommand needs, opened once: the database, the repo of this directory when there
/// is one, its thresholds, and the instant this command runs at.
struct Face {
    env: HashMap<String, String>,
    home: PathBuf,
    cwd: PathBuf,
    conn: Connection,
    repo_name: Option<String>,
    repo_root: Option<String>,
    th: Thresholds,
    now: DateTime<Utc>,
}

fn face(env: &HashMap<String, String>, cwd: Option<PathBuf>) -> Result<Face, String> {
    let home = ratchet_home(env);
    let conn = db::open_ready(&home).map_err(|e| e.to_string())?;
    let cwd = cwd.unwrap_or_else(|| PathBuf::from("."));
    let repo = find_repo(&cwd).map_err(|e| e.to_string())?;
    let th = repo
        .as_ref()
        .map(|r| r.config.thresholds.clone())
        .unwrap_or_default();
    let repo_name = repo.as_ref().map(|r| r.name.clone());
    let repo_root = repo
        .as_ref()
        .map(|r| normalize(&r.main_root).to_string_lossy().to_string());
    Ok(Face {
        env: env.clone(),
        home,
        cwd,
        conn,
        repo_name,
        repo_root,
        th,
        now: clock::now(env),
    })
}

fn fail(e: impl std::fmt::Display) -> i32 {
    eprintln!("error: {e}");
    1
}

fn session_of(f: &Face, explicit: Option<&str>) -> Option<String> {
    sessions::resolve(&f.conn, explicit, &f.env, &f.cwd, &f.th, f.now)
        .ok()
        .flatten()
}

/// A write nobody can be attributed to still happens — the board would lose the note otherwise —
/// but it says so once on stderr.
fn attributed(f: &Face, explicit: Option<&str>) -> Option<String> {
    let session = session_of(f, explicit);
    if session.is_none() {
        eprintln!(
            "warning: no session resolved (use --session or RATCHET_SESSION_ID); recording with no session"
        );
    }
    session
}

/// `TaskStatus::from_db` is lenient by design, so a name it does not know comes back as `backlog`.
/// Here that would silently move a task: compare the round trip and refuse instead.
fn parse_status(text: &str) -> Result<TaskStatus, String> {
    let status = TaskStatus::from_db(text);
    if status.as_str() == text {
        Ok(status)
    } else {
        Err(format!("unknown status `{text}`; valid: {VALID_STATUSES}"))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn list(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    repo: Option<&str>,
    statuses: &[String],
    mine: bool,
    tag: Option<&str>,
    all: bool,
    json_out: bool,
) -> i32 {
    let f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let mut parsed = Vec::new();
    for text in statuses {
        match parse_status(text) {
            Ok(s) => parsed.push(s),
            Err(e) => return fail(e),
        }
    }
    let claimed = if mine {
        match session_of(&f, session) {
            Some(id) => Some(id),
            None => return fail("--mine needs a session (use --session or RATCHET_SESSION_ID)"),
        }
    } else {
        None
    };
    // Inside a repo the board is that repo's; from outside one, or with an explicit --repo, the
    // listing spans the machine.
    let scope = if repo.is_some() { None } else { f.repo_root.clone() };
    let filter = tasks::Filter {
        repo_root: scope.as_deref(),
        repo,
        statuses: &parsed,
        claimed_by: claimed.as_deref(),
        tag,
        include_archived: all,
    };
    let found = match tasks::list(&f.conn, &filter) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    if json_out {
        let payload: Vec<Value> = found
            .iter()
            .map(|t| {
                let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
                if let Some(map) = v.as_object_mut() {
                    map.insert(
                        "progress".into(),
                        json!(tasks::progress(&f.conn, &t.id).ok().flatten()),
                    );
                }
                v
            })
            .collect();
        output::emit_json(&f.home, "task-list", &json!(payload), f.now);
        return 0;
    }
    let lines: Vec<String> = found
        .iter()
        .map(|t| output::format_task_line(t, tasks::progress(&f.conn, &t.id).ok().flatten()))
        .collect();
    output::emit(&f.home, "task-list", &lines, f.now);
    0
}

pub fn show(env: &HashMap<String, String>, cwd: Option<PathBuf>, id: &str, json_out: bool) -> i32 {
    let f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let task = match tasks::get(&f.conn, id) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let items = match tasks::checklist(&f.conn, id) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let progress = tasks::progress(&f.conn, id).ok().flatten();
    let last = tasks::last_handoff(&f.conn, id).ok().flatten();
    // The detail view shows the last ten events and no more (spec §4.1).
    let history = events::for_task(&f.conn, id, 10).unwrap_or_default();
    if json_out {
        let payload = json!({
            "task": task,
            "checklist": items,
            "progress": progress,
            "last_handoff": last,
            "events": history,
        });
        output::emit_json(&f.home, &format!("task-{id}"), &payload, f.now);
        return 0;
    }
    let mut lines = vec![
        format!("{}  {}", task.id, task.title),
        format!(
            "status {} · repo {} · priority p{}{}{}{}",
            task.status.as_str(),
            task.repo,
            task.priority,
            if task.archived_at.is_some() {
                " · archived"
            } else {
                ""
            },
            task.claimed_by
                .as_ref()
                .map(|s| format!(" · session {s}"))
                .unwrap_or_default(),
            if task.tags.is_empty() {
                String::new()
            } else {
                format!(" · tags {}", task.tags.join(", "))
            }
        ),
    ];
    if let Some((done, total)) = progress {
        lines.push(format!("progress {done}/{total}"));
    }
    if !task.body.trim().is_empty() {
        lines.push(String::new());
        lines.extend(task.body.trim_end().lines().map(str::to_string));
    }
    if !items.is_empty() {
        lines.push(String::new());
        for item in &items {
            lines.push(format!(
                "  {}. [{}] {}",
                item.position,
                if item.done { "x" } else { " " },
                item.text
            ));
        }
    }
    if let Some(ev) = &last {
        lines.push(String::new());
        lines.push(format!(
            "last handoff ({}): {}",
            clock::iso(ev.ts),
            ev.payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        ));
    }
    if !history.is_empty() {
        lines.push(String::new());
        lines.push("events:".to_string());
        for ev in &history {
            lines.push(format!(
                "  {}  {:<16} {}",
                clock::iso(ev.ts),
                ev.kind,
                summary(&ev.payload)
            ));
        }
    }
    output::emit(&f.home, &format!("task-{id}"), &lines, f.now);
    0
}

/// One short line for an event payload: its text when it has one, else its non-null keys.
fn summary(payload: &Value) -> String {
    if let Some(text) = payload.get("text").and_then(|v| v.as_str()) {
        return text.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    match payload.as_object() {
        None => String::new(),
        Some(map) => map
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| format!("{k}={}", v.to_string().trim_matches('"')))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn new(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    title: &str,
    body: Option<&str>,
    body_file: Option<PathBuf>,
    checks: &[String],
    priority: i64,
    tags: &[String],
    parent: Option<&str>,
    json_out: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let (Some(repo_name), Some(repo_root)) = (f.repo_name.clone(), f.repo_root.clone()) else {
        // There is no registry of repos (spec D-marker): a task belongs to the marker above it.
        return fail(
            "a task belongs to a repo: run this from inside a repo with a ratchet.toml at its root",
        );
    };
    let body_text = match (body, body_file) {
        (_, Some(path)) => match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => return fail(format!("{}: {e}", path.display())),
        },
        (Some(t), None) => t.to_string(),
        (None, None) => String::new(),
    };
    let session_id = attributed(&f, session);
    let created = tasks::create(
        &mut f.conn,
        tasks::NewTask {
            title,
            body: &body_text,
            repo: &repo_name,
            repo_root: &repo_root,
            priority,
            parent_id: parent,
            tags,
            checklist: checks,
        },
        Source::Cli,
        session_id.as_deref(),
        f.now,
    );
    match created {
        Err(e) => fail(e),
        Ok(task) => {
            if json_out {
                output::emit_json(&f.home, "task-new", &json!(task), f.now);
            } else {
                println!("{}  {}  (backlog)", task.id, task.title);
            }
            0
        }
    }
}

pub fn claim(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    json_out: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let Some(session_id) = session_of(&f, session) else {
        return fail("claiming needs a session (use --session or RATCHET_SESSION_ID)");
    };
    match tasks::claim(&mut f.conn, id, &session_id, &f.th, Source::Cli, f.now) {
        Err(e) => fail(e),
        Ok(task) => {
            if json_out {
                output::emit_json(&f.home, "task-claim", &json!(task), f.now);
            } else {
                println!("{} claimed by {session_id} ({})", task.id, task.status.as_str());
            }
            0
        }
    }
}

pub fn status(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    to: &str,
    why: Option<&str>,
    json_out: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let to = match parse_status(to) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let session_id = attributed(&f, session);
    match tasks::transition(
        &mut f.conn,
        id,
        to,
        Source::Cli,
        session_id.as_deref(),
        why,
        f.now,
    ) {
        Err(e) => fail(e),
        Ok(task) => {
            if json_out {
                output::emit_json(&f.home, "task-status", &json!(task), f.now);
            } else {
                println!("{} → {}", task.id, task.status.as_str());
            }
            0
        }
    }
}

pub fn check(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    position: i64,
    undo: bool,
    json_out: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let session_id = attributed(&f, session);
    let result = if undo {
        tasks::uncheck(&mut f.conn, id, position, Source::Cli, session_id.as_deref(), f.now)
    } else {
        tasks::check(&mut f.conn, id, position, Source::Cli, session_id.as_deref(), f.now)
    };
    match result {
        Err(e) => fail(e),
        Ok(item) => {
            if json_out {
                output::emit_json(&f.home, "task-check", &json!(item), f.now);
            } else {
                let progress = tasks::progress(&f.conn, id)
                    .ok()
                    .flatten()
                    .map(|(d, t)| format!("  ({d}/{t})"))
                    .unwrap_or_default();
                println!(
                    "{id} [{}] {}. {}{progress}",
                    if item.done { "x" } else { " " },
                    item.position,
                    item.text
                );
            }
            0
        }
    }
}

pub fn note(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    text: &str,
    json_out: bool,
) -> i32 {
    record(env, cwd, session, id, text, json_out, false)
}

pub fn handoff(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    text: &str,
    json_out: bool,
) -> i32 {
    record(env, cwd, session, id, text, json_out, true)
}

fn record(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    text: &str,
    json_out: bool,
    is_handoff: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let session_id = attributed(&f, session);
    let result = if is_handoff {
        tasks::handoff(&mut f.conn, id, text, Source::Cli, session_id.as_deref(), f.now)
    } else {
        tasks::note(&mut f.conn, id, text, Source::Cli, session_id.as_deref(), f.now)
    };
    match result {
        Err(e) => fail(e),
        Ok(ev) => {
            if json_out {
                output::emit_json(&f.home, "task-record", &json!(ev), f.now);
            } else if is_handoff {
                println!("{id} handoff recorded");
            } else {
                println!("{id} note recorded");
            }
            0
        }
    }
}

pub fn archive(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    json_out: bool,
) -> i32 {
    shelve(env, cwd, session, id, json_out, true)
}

pub fn unarchive(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    json_out: bool,
) -> i32 {
    shelve(env, cwd, session, id, json_out, false)
}

fn shelve(
    env: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    session: Option<&str>,
    id: &str,
    json_out: bool,
    hide: bool,
) -> i32 {
    let mut f = match face(env, cwd) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let session_id = session_of(&f, session);
    let result = if hide {
        tasks::archive(&mut f.conn, id, Source::Cli, session_id.as_deref(), f.now)
    } else {
        tasks::unarchive(&mut f.conn, id, Source::Cli, session_id.as_deref(), f.now)
    };
    match result {
        Err(e) => fail(e),
        Ok(task) => {
            if json_out {
                output::emit_json(&f.home, "task-archive", &json!(task), f.now);
            } else if hide {
                println!("{} archived (back with: ratchet task unarchive {})", task.id, task.id);
            } else {
                println!("{} unarchived ({})", task.id, task.status.as_str());
            }
            0
        }
    }
}
```

Add `pub mod task_cmd;` to `crates/ratchet/src/cli/mod.rs`, after the lines that are already there.

- [ ] **Step 3: Wire the subcommand in `crates/ratchet/src/main.rs`**

Add the variant to `Cmd` (append, do not reorder):
```rust
    /// The task board: `list`, `show`, `new`, `claim`, `status`, `check`, `note`, `handoff`,
    /// `archive`, `unarchive`.
    Task {
        #[command(subcommand)]
        cmd: TaskCmd,
    },
```
Add the enum next to the other `Subcommand` enums:
```rust
#[derive(Subcommand)]
enum TaskCmd {
    /// One line per task: id, status, priority, title and progress.
    List {
        /// Tasks of the repo with this name, across the machine.
        #[arg(long)]
        repo: Option<String>,
        /// Only these statuses; repeatable.
        #[arg(long = "status", short = 's')]
        statuses: Vec<String>,
        /// Only the ones this session holds.
        #[arg(long)]
        mine: bool,
        /// Only the ones carrying this tag.
        #[arg(long)]
        tag: Option<String>,
        /// Include archived tasks.
        #[arg(long, short = 'a')]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Body, checklist, last handoff and the last ten events of one task.
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Create a task in this repo, in `backlog`.
    New {
        title: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long = "body-file")]
        body_file: Option<std::path::PathBuf>,
        /// One acceptance criterion; repeatable, in order.
        #[arg(long = "check", short = 'c')]
        checks: Vec<String>,
        #[arg(long, short = 'p', default_value_t = 3)]
        priority: i64,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Take a task for this session.
    Claim {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Move a task: backlog, ready, in_progress, blocked, review, done.
    Status {
        id: String,
        to: String,
        /// Reason; required to close a task that has no checklist.
        #[arg(long)]
        why: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Mark a checklist item as done, or undo it.
    Check {
        id: String,
        position: i64,
        #[arg(long)]
        undo: bool,
        #[arg(long)]
        json: bool,
    },
    /// Record a decision or a finding on a task.
    Note {
        id: String,
        text: String,
        #[arg(long)]
        json: bool,
    },
    /// Record what is left and how to resume.
    Handoff {
        id: String,
        text: String,
        #[arg(long)]
        json: bool,
    },
    /// Hide a done task from the board.
    Archive {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Bring an archived task back.
    Unarchive {
        id: String,
        #[arg(long)]
        json: bool,
    },
}
```
And the arm, next to `Cmd::Session` (`session` is the copy of the global option Task 7 took before the match):
```rust
        Cmd::Task { cmd } => match cmd {
            TaskCmd::List {
                repo,
                statuses,
                mine,
                tag,
                all,
                json,
            } => cli::task_cmd::list(
                &env,
                cwd,
                session.as_deref(),
                repo.as_deref(),
                &statuses,
                mine,
                tag.as_deref(),
                all,
                json,
            ),
            TaskCmd::Show { id, json } => cli::task_cmd::show(&env, cwd, &id, json),
            TaskCmd::New {
                title,
                body,
                body_file,
                checks,
                priority,
                tags,
                parent,
                json,
            } => cli::task_cmd::new(
                &env,
                cwd,
                session.as_deref(),
                &title,
                body.as_deref(),
                body_file,
                &checks,
                priority,
                &tags,
                parent.as_deref(),
                json,
            ),
            TaskCmd::Claim { id, json } => {
                cli::task_cmd::claim(&env, cwd, session.as_deref(), &id, json)
            }
            TaskCmd::Status { id, to, why, json } => cli::task_cmd::status(
                &env,
                cwd,
                session.as_deref(),
                &id,
                &to,
                why.as_deref(),
                json,
            ),
            TaskCmd::Check {
                id,
                position,
                undo,
                json,
            } => cli::task_cmd::check(&env, cwd, session.as_deref(), &id, position, undo, json),
            TaskCmd::Note { id, text, json } => {
                cli::task_cmd::note(&env, cwd, session.as_deref(), &id, &text, json)
            }
            TaskCmd::Handoff { id, text, json } => {
                cli::task_cmd::handoff(&env, cwd, session.as_deref(), &id, &text, json)
            }
            TaskCmd::Archive { id, json } => {
                cli::task_cmd::archive(&env, cwd, session.as_deref(), &id, json)
            }
            TaskCmd::Unarchive { id, json } => {
                cli::task_cmd::unarchive(&env, cwd, session.as_deref(), &id, json)
            }
        },
```

- [ ] **Step 4: Run the scenario tests of the board**

```
cd /c/repos/ratchet && cargo test -p ratchet --test spec tasks__ 2>&1 | tail -20
cd /c/repos/ratchet && cargo test -p ratchet --test spec agent_protocol__a_task_listing agent_protocol__long_output 2>&1 | tail -10
cd /c/repos/ratchet && cargo test -p ratchet && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: all 21 `tasks__` scenarios green, plus the two compact-output ones. Still red: the three briefing scenarios, the two reminder ones and the four handoff-rule ones (Tasks 9-11). Groups 0 and 1 stay green.

- [ ] **Step 5: Hand off (no git)**

List the three files and which scenario tests turned green.

---

### Task 9: The `[ratchet]` briefing on `SessionStart`

Ported from `build` and its helpers in `C:\repos\ops\ops\cli\briefing.py`. This task fills the marked seam group 1 left in `session_start`: the briefing is built and printed **before** `release_dead`, with the same `now`, so an orphaned task is shown once with its last handoff while it is still claimed (the bug ops fixed in T-0021).

**Files:**
- Create: `crates/ratchet/src/hooks/briefing.rs`
- Modify: `crates/ratchet/src/hooks/mod.rs` (one `pub mod` line), `crates/ratchet/src/hooks/dispatch.rs` (the seam, two lines)

**Interfaces:**
- Consumes: `tasks::{Filter, list, get, progress, last_handoff, orphaned, Orphan}`, `output::format_task_line`, `model::{Session, Task, TaskStatus}`, `config::Thresholds`.
- Produces:
  - `briefing::MAX_LINES: usize = 40`, `briefing::MAX_READY: usize = 5`, `briefing::COMMANDS_LINE: &str`
  - `briefing::build(conn: &Connection, session: &Session, th: &Thresholds, now: DateTime<Utc>) -> String`
  - private `task_line`, `handoff_text`, `quote`, `short` — Task 10 reuses the last three.
- Reads only: the briefing never writes, so it cannot fail a registration.

- [ ] **Step 1: Write the failing unit tests**

At the bottom of `crates/ratchet/src/hooks/briefing.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::model::{LaunchedBy, SessionMode};
    use crate::services::sessions::{self, StartInput};
    use crate::services::tasks::NewTask;

    fn at(s: &str) -> DateTime<Utc> {
        crate::clock::parse(s).unwrap()
    }

    /// A database with one registered session, and the session it registered.
    fn setup(session_id: &str) -> (Connection, Session) {
        let mut conn = db::open_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        let session = sessions::upsert_start(
            &mut conn,
            StartInput {
                session_id,
                repo: "demo",
                repo_root: "root",
                cwd: "root",
                worktree: None,
                branch: Some("main"),
                mode: SessionMode::Interactive,
                launched_by: LaunchedBy::User,
            },
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        (conn, session)
    }

    fn make(conn: &mut Connection, title: &str) -> String {
        tasks::create(
            conn,
            NewTask {
                title,
                body: "",
                repo: "demo",
                repo_root: "root",
                priority: 3,
                parent_id: None,
                tags: &[],
                checklist: &[],
            },
            crate::model::Source::Cli,
            None,
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap()
        .id
    }

    #[test]
    fn a_quiet_repo_gets_exactly_one_line() {
        let (conn, session) = setup("session-abcdef0123");
        let text = build(&conn, &session, &Thresholds::default(), at("2026-09-16T12:01:00Z"));
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.starts_with("[ratchet] repo demo · session session…"), "{text}");
        assert!(text.contains("branch main"), "{text}");
    }

    #[test]
    fn the_briefing_groups_mine_orphans_and_ready() {
        let (mut conn, session) = setup("s-live");
        let now = at("2026-09-16T13:40:00Z");
        let mine = make(&mut conn, "what I am doing");
        tasks::claim(&mut conn, &mine, "s-live", &Thresholds::default(), crate::model::Source::Cli, at("2026-09-16T12:01:00Z")).unwrap();
        let ready = make(&mut conn, "free to take");
        tasks::transition(&mut conn, &ready, TaskStatus::Ready, crate::model::Source::Cli, None, None, at("2026-09-16T12:02:00Z")).unwrap();
        // A task held by a session that was never registered counts as orphaned.
        let lost = make(&mut conn, "left behind");
        tasks::transition(&mut conn, &lost, TaskStatus::Ready, crate::model::Source::Cli, None, None, at("2026-09-16T12:03:00Z")).unwrap();
        tasks::transition(&mut conn, &lost, TaskStatus::InProgress, crate::model::Source::Cli, None, None, at("2026-09-16T12:04:00Z")).unwrap();
        conn.execute("UPDATE tasks SET claimed_by = 's-ghost' WHERE id = ?1", rusqlite::params![lost]).unwrap();
        tasks::handoff(&mut conn, &lost, "stopped at the parser", crate::model::Source::Cli, Some("s-ghost"), at("2026-09-16T12:05:00Z")).unwrap();

        let text = build(&conn, &session, &Thresholds::default(), now);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("[ratchet] repo demo"), "{text}");
        let mine_at = lines.iter().position(|l| l.contains("Your tasks in progress")).unwrap();
        let orphan_at = lines.iter().position(|l| l.contains("dead session")).unwrap();
        let ready_at = lines.iter().position(|l| l.contains("Ready to take")).unwrap();
        assert!(mine_at < orphan_at && orphan_at < ready_at, "{text}");
        assert!(text.contains(&mine), "{text}");
        assert!(text.contains("stopped at the parser"), "{text}");
        assert!(text.contains("free to take"), "{text}");
        assert_eq!(*lines.last().unwrap(), COMMANDS_LINE);
    }

    #[test]
    fn at_most_five_ready_tasks_are_listed() {
        let (mut conn, session) = setup("s-1");
        for n in 1..=9 {
            let id = make(&mut conn, &format!("ready {n}"));
            tasks::transition(&mut conn, &id, TaskStatus::Ready, crate::model::Source::Cli, None, None, at("2026-09-16T12:01:00Z")).unwrap();
        }
        let text = build(&conn, &session, &Thresholds::default(), at("2026-09-16T12:02:00Z"));
        assert_eq!(text.matches("ready ").count(), 5, "{text}");
    }

    #[test]
    fn a_crowded_board_is_cut_to_forty_lines() {
        let (mut conn, session) = setup("s-1");
        for n in 1..=60 {
            let id = make(&mut conn, &format!("held {n}"));
            tasks::transition(&mut conn, &id, TaskStatus::Ready, crate::model::Source::Cli, None, None, at("2026-09-16T12:01:00Z")).unwrap();
            tasks::transition(&mut conn, &id, TaskStatus::InProgress, crate::model::Source::Cli, None, None, at("2026-09-16T12:02:00Z")).unwrap();
            conn.execute("UPDATE tasks SET claimed_by = 's-ghost' WHERE id = ?1", rusqlite::params![id]).unwrap();
        }
        let text = build(&conn, &session, &Thresholds::default(), at("2026-09-16T12:03:00Z"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), MAX_LINES, "{}", lines.len());
        assert!(lines[MAX_LINES - 2].contains("ratchet task list"), "{:?}", lines[MAX_LINES - 2]);
        assert_eq!(lines[MAX_LINES - 1], COMMANDS_LINE);
    }

    #[test]
    fn a_long_handoff_is_quoted_and_cut() {
        let text = quote(&"word ".repeat(40), 60);
        assert!(text.starts_with('"') && text.ends_with("…\""), "{text}");
        assert_eq!(text.chars().count(), 62);
        assert_eq!(quote("  two   spaces  ", 60), "\"two spaces\"");
    }
}
```

- [ ] **Step 2: Run to see them fail**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet hooks::briefing
```
Expected: compile errors (no such module).

- [ ] **Step 3: Write `crates/ratchet/src/hooks/briefing.rs`**

```rust
//! The `[ratchet]` briefing printed on session start, and (Task 10) the one-line reminder printed
//! on every prompt. Reads only: a briefing that failed must never cost a registration, so every
//! query here falls back to "nothing" instead of propagating an error.

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use crate::config::Thresholds;
use crate::model::{Session, Task, TaskStatus};
use crate::output;
use crate::services::tasks;

/// The hard cap of spec §4.2: a briefing is context, not a report.
pub const MAX_LINES: usize = 40;
/// Enough to choose from without turning the briefing into the board.
pub const MAX_READY: usize = 5;
pub const COMMANDS_LINE: &str =
    "Commands: ratchet task show|claim|check|note|handoff  ·  full guide: skill ratchet-tasks";

pub fn build(
    conn: &Connection,
    session: &Session,
    th: &Thresholds,
    now: DateTime<Utc>,
) -> String {
    let header = format!(
        "[ratchet] repo {} · session {} · branch {}",
        session.repo,
        short(&session.id),
        session.branch.as_deref().unwrap_or("?")
    );
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
        return header;
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
    if lines.len() > MAX_LINES {
        lines.truncate(MAX_LINES - 2);
        lines.push("  … (more in `ratchet task list`)".to_string());
        lines.push(COMMANDS_LINE.to_string());
    }
    lines.join("\n")
}

fn task_line(conn: &Connection, task: &Task, with_handoff: bool) -> String {
    let mut line = format!(
        "  {}",
        output::format_task_line(task, tasks::progress(conn, &task.id).ok().flatten())
    );
    if with_handoff {
        if let Some(text) = handoff_text(conn, &task.id) {
            line.push_str(&format!("   last handoff: {}", quote(&text, 60)));
        }
    }
    line
}

/// The text of the task's most recent handoff, when there is one with something in it.
pub(crate) fn handoff_text(conn: &Connection, task_id: &str) -> Option<String> {
    let event = tasks::last_handoff(conn, task_id).ok().flatten()?;
    let text = event.payload.get("text")?.as_str()?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// One flattened, quoted line of at most `width` characters of text, so a handoff written as a
/// paragraph cannot blow the 40-line cap on its own.
pub(crate) fn quote(text: &str, width: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > width {
        let cut: String = flat.chars().take(width - 1).collect();
        format!("\"{cut}…\"")
    } else {
        format!("\"{flat}\"")
    }
}

/// Session identifiers are long; eight characters are enough to name one in a briefing.
pub(crate) fn short(session_id: &str) -> String {
    if session_id.chars().count() > 8 {
        format!("{}…", session_id.chars().take(8).collect::<String>())
    } else {
        session_id.to_string()
    }
}
```

Add `pub mod briefing;` to `crates/ratchet/src/hooks/mod.rs`, after `pub mod dispatch;`.

- [ ] **Step 4: Fill the seam in `crates/ratchet/src/hooks/dispatch.rs`**

Replace the marked comment block group 1 left inside `session_start` with the briefing, keeping the sweep where it is:
```rust
    // The briefing is built HERE, with this same `now`, BEFORE the sweep below, so an orphaned
    // task is shown once with its last handoff while it is still claimed. Do not move the sweep
    // above this line.
    println!(
        "{}",
        briefing::build(&conn, &session, &repo.config.thresholds, now)
    );
```
and add `use super::briefing;` to the file's imports.

- [ ] **Step 5: Run**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet hooks:: && cargo test -p ratchet --test spec 2>&1 | tail -20
cd /c/repos/ratchet && cargo test -p ratchet --release --test latency -- --nocapture
cd /c/repos/ratchet && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: the 5 new unit tests green; the three briefing scenarios green; the 24 `sessions__` scenarios still green (none of them asserts an empty stdout on a `session-start` inside a marked repo); the latency median unchanged — `pre_tool` does not reach this code.

- [ ] **Step 6: Hand off (no git)**

List the three files, the scenario counts and the release latency median.

---
### Task 10: The one-line reminder on `UserPromptSubmit`

Ported from `prompt_line` of `C:\repos\ops\ops\cli\briefing.py`. The hook that prints it needs the session it just touched, so this task also gives `dispatch.rs` the one small helper the `stop` hook of Task 11 reuses.

**Files:**
- Modify: `crates/ratchet/src/hooks/briefing.rs`, `crates/ratchet/src/hooks/dispatch.rs`

**Interfaces:**
- Consumes: `briefing::{handoff_text, quote}` (Task 9), `tasks::{Filter, list, progress}`, group 1's `dispatch::{Payload, session_identity, ensure_session, heartbeat}`.
- Produces:
  - `briefing::prompt_line(conn: &Connection, session: &Session) -> Option<String>` — `None` when the session holds nothing.
  - private `dispatch::Beat { conn: Connection, session: Session }` and `dispatch::beat(payload, env, cwd, home, kind) -> Result<Option<Beat>, String>` — the heartbeat plus the handles a hook needs afterwards. `heartbeat` becomes a one-line wrapper over it and keeps its own signature; `session_start` and `session_end` are not touched.
- Unlike the briefing, the reminder is **not** scoped to the repo: a session that holds a task of another checkout still has to be reminded of it (reference parity).

- [ ] **Step 1: Write the failing unit tests**

Append to the test module of `crates/ratchet/src/hooks/briefing.rs` (the helpers `setup`, `make` and `at` of Task 9 stay):
```rust
    #[test]
    fn no_claimed_task_means_no_line() {
        let (conn, session) = setup("s-1");
        assert_eq!(prompt_line(&conn, &session), None);
    }

    #[test]
    fn the_reminder_names_the_task_its_progress_and_its_last_handoff() {
        let (mut conn, session) = setup("s-1");
        let id = tasks::create(
            &mut conn,
            crate::services::tasks::NewTask {
                title: "the claimed one",
                body: "",
                repo: "demo",
                repo_root: "root",
                priority: 3,
                parent_id: None,
                tags: &[],
                checklist: &["a".to_string(), "b".to_string(), "c".to_string()],
            },
            crate::model::Source::Cli,
            None,
            at("2026-09-16T12:01:00Z"),
        )
        .unwrap()
        .id;
        tasks::claim(&mut conn, &id, "s-1", &Thresholds::default(), crate::model::Source::Cli, at("2026-09-16T12:02:00Z")).unwrap();
        tasks::check(&mut conn, &id, 1, crate::model::Source::Cli, Some("s-1"), at("2026-09-16T12:03:00Z")).unwrap();
        tasks::handoff(&mut conn, &id, "half way; resume at item 2", crate::model::Source::Cli, Some("s-1"), at("2026-09-16T12:04:00Z")).unwrap();
        let line = prompt_line(&conn, &session).unwrap();
        assert_eq!(line.lines().count(), 1, "{line}");
        assert!(line.starts_with("[ratchet] "), "{line}");
        assert!(line.contains(&id), "{line}");
        assert!(line.contains("in_progress"), "{line}");
        assert!(line.contains("(1/3)"), "{line}");
        assert!(line.contains("resume at item 2"), "{line}");
        assert!(!line.contains("more"), "{line}");
    }

    #[test]
    fn holding_more_than_one_task_says_how_many() {
        let (mut conn, session) = setup("s-1");
        for title in ["first", "second", "third"] {
            let id = make(&mut conn, title);
            tasks::claim(&mut conn, &id, "s-1", &Thresholds::default(), crate::model::Source::Cli, at("2026-09-16T12:01:00Z")).unwrap();
        }
        let line = prompt_line(&conn, &session).unwrap();
        assert!(line.contains("(+2 more)"), "{line}");
    }
```

- [ ] **Step 2: Run to see them fail**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet hooks::briefing
```
Expected: compile errors naming `prompt_line`.

- [ ] **Step 3: Add `prompt_line` to `crates/ratchet/src/hooks/briefing.rs`**

```rust
/// One line for the prompt hook, or nothing at all. Deliberately not scoped to the repo: a session
/// that holds a task of another checkout still has to be reminded of it before it starts the next
/// turn.
pub fn prompt_line(conn: &Connection, session: &Session) -> Option<String> {
    let mine = tasks::list(
        conn,
        &tasks::Filter {
            statuses: &[TaskStatus::InProgress],
            claimed_by: Some(&session.id),
            ..Default::default()
        },
    )
    .ok()?;
    let task = mine.first()?;
    let progress = tasks::progress(conn, &task.id)
        .ok()
        .flatten()
        .map(|(done, total)| format!(" ({done}/{total})"))
        .unwrap_or_default();
    let handoff = handoff_text(conn, &task.id)
        .map(|text| format!(" · last handoff: {}", quote(&text, 70)))
        .unwrap_or_default();
    let more = if mine.len() > 1 {
        format!(" (+{} more)", mine.len() - 1)
    } else {
        String::new()
    };
    Some(format!(
        "[ratchet] {} {}{progress}{handoff}{more}",
        task.id,
        task.status.as_str()
    ))
}
```

- [ ] **Step 4: Give `dispatch.rs` the shared heartbeat**

Replace group 1's `heartbeat` with these three items, keeping `heartbeat`'s signature so nothing else changes:
```rust
/// What a hook has after the heartbeat: the open database and the session it just refreshed.
struct Beat {
    conn: Connection,
    session: Session,
}

/// The heartbeat every hook that is not `session-start` performs. `Ok(None)` means there was
/// nothing to do — no marker above `cwd`, or no identity the harness fixed — and the caller exits
/// 0 without opening anything else.
fn beat(
    payload: &Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
    kind: Option<EventKind>,
) -> Result<Option<Beat>, String> {
    let Some(repo) = find_repo(cwd).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let Some(session_id) = session_identity(payload, env) else {
        return Ok(None);
    };
    let now = clock::now(env);
    let mut conn = db::open_ready(home).map_err(|e| e.to_string())?;
    ensure_session(&mut conn, &repo, &session_id, cwd, env, now).map_err(|e| e.to_string())?;
    let session = sessions::touch(&mut conn, &session_id, kind, now).map_err(|e| e.to_string())?;
    Ok(Some(Beat { conn, session }))
}

fn heartbeat(
    payload: &Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
    kind: Option<EventKind>,
) -> Result<i32, String> {
    beat(payload, env, cwd, home, kind)?;
    Ok(0)
}

/// Heartbeat, then the reminder. Printing nothing is the normal case.
fn prompt(
    payload: &Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
) -> Result<i32, String> {
    let Some(b) = beat(payload, env, cwd, home, Some(EventKind::SessionPrompt))? else {
        return Ok(0);
    };
    if let Some(line) = briefing::prompt_line(&b.conn, &b.session) {
        println!("{line}");
    }
    Ok(0)
}
```
and point the `prompt` arm of `dispatch` at it:
```rust
        "prompt" => prompt(&payload, env, &cwd, home),
```
The `stop`, `subagent-stop`, `pre-compact` and `session-end` arms are untouched by this task.

- [ ] **Step 5: Run**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet hooks:: && cargo test -p ratchet --test spec 2>&1 | tail -20
cd /c/repos/ratchet && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: the 3 new unit tests green; `agent_protocol__with_a_claimed_task` and `agent_protocol__without_a_claimed_task` green; the four handoff-rule scenarios still red; everything else green.

- [ ] **Step 6: Hand off (no git)**

List the two files. Quote one real reminder line from a manual run for the reviewer.

---

### Task 11: The handoff rule on `Stop`

Ported from `C:\repos\ops\ops\cli\handoff_rule.py`, comments included: every branch of that file is a bug somebody hit. This is the **only** exit-2 path group 2 adds to the binary.

**Files:**
- Create: `crates/ratchet/src/hooks/handoff_rule.rs`
- Modify: `crates/ratchet/src/hooks/mod.rs` (one `pub mod` line), `crates/ratchet/src/hooks/dispatch.rs` (one arm, one handler)

**Interfaces:**
- Consumes: `tasks::{Filter, list}`, `events::for_task`, `model::{SessionMode, TaskStatus}`, `dispatch::{beat, Beat}` (Task 10), `hooks::BLOCK` (group 0).
- Produces:
  - `handoff_rule::should_block_stop(conn: &Connection, session_id: &str, stop_hook_active: bool, mode: SessionMode) -> Option<String>` — the message, or nothing.
  - private `dispatch::stop(...)` — heartbeat, then the rule; `[ratchet] ` + message on stderr and exit 2, or 0.
- The rule reads only. It never writes, never changes a status, and never blocks twice.

- [ ] **Step 1: Write the failing unit tests**

At the bottom of `crates/ratchet/src/hooks/handoff_rule.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    use crate::config::Thresholds;
    use crate::db;
    use crate::model::{LaunchedBy, SessionMode, Source};
    use crate::services::sessions::{self, StartInput};
    use crate::services::tasks::{self, NewTask};
    use crate::services::events;
    use crate::model::EventKind;

    fn at(s: &str) -> DateTime<Utc> {
        crate::clock::parse(s).unwrap()
    }

    /// A session holding one task with a checklist, and no prompt yet.
    fn holding() -> (rusqlite::Connection, String) {
        let mut conn = db::open_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        sessions::upsert_start(
            &mut conn,
            StartInput {
                session_id: "s-1",
                repo: "demo",
                repo_root: "root",
                cwd: "root",
                worktree: None,
                branch: None,
                mode: SessionMode::Interactive,
                launched_by: LaunchedBy::User,
            },
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        let id = tasks::create(
            &mut conn,
            NewTask {
                title: "the work",
                body: "",
                repo: "demo",
                repo_root: "root",
                priority: 3,
                parent_id: None,
                tags: &[],
                checklist: &["one".to_string()],
            },
            Source::Cli,
            None,
            at("2026-09-16T12:01:00Z"),
        )
        .unwrap()
        .id;
        tasks::claim(&mut conn, &id, "s-1", &Thresholds::default(), Source::Cli, at("2026-09-16T12:02:00Z")).unwrap();
        (conn, id)
    }

    fn prompt(conn: &rusqlite::Connection, when: &str) {
        events::emit(
            conn,
            EventKind::SessionPrompt,
            &serde_json::json!({}),
            Source::Hook,
            Some("s-1"),
            None,
            at(when),
        )
        .unwrap();
    }

    #[test]
    fn a_session_holding_nothing_closes() {
        let mut conn = db::open_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        assert_eq!(
            should_block_stop(&conn, "s-1", false, SessionMode::Interactive),
            None
        );
    }

    #[test]
    fn a_claim_and_nothing_else_is_blocked_once() {
        let (conn, id) = holding();
        prompt(&conn, "2026-09-16T12:03:00Z");
        let message = should_block_stop(&conn, "s-1", false, SessionMode::Interactive).unwrap();
        assert!(message.contains(&id), "{message}");
        assert!(message.contains("ratchet task handoff"), "{message}");
        assert!(message.contains("check|note|status"), "{message}");
        // The retry never blocks.
        assert_eq!(
            should_block_stop(&conn, "s-1", true, SessionMode::Interactive),
            None
        );
        // Neither does a session nobody is watching.
        assert_eq!(
            should_block_stop(&conn, "s-1", false, SessionMode::Headless),
            None
        );
    }

    #[test]
    fn any_record_of_this_turn_lets_the_session_close() {
        for (label, record) in [
            ("check", 0),
            ("note", 1),
            ("status", 2),
            ("handoff", 3),
        ] {
            let (mut conn, id) = holding();
            prompt(&conn, "2026-09-16T12:03:00Z");
            match record {
                0 => {
                    tasks::check(&mut conn, &id, 1, Source::Cli, Some("s-1"), at("2026-09-16T12:04:00Z")).unwrap();
                }
                1 => {
                    tasks::note(&mut conn, &id, "found X", Source::Cli, Some("s-1"), at("2026-09-16T12:04:00Z")).unwrap();
                }
                2 => {
                    tasks::transition(&mut conn, &id, crate::model::TaskStatus::Blocked, Source::Cli, Some("s-1"), Some("waiting"), at("2026-09-16T12:04:00Z")).unwrap();
                }
                _ => {
                    tasks::handoff(&mut conn, &id, "half done", Source::Cli, Some("s-1"), at("2026-09-16T12:04:00Z")).unwrap();
                }
            }
            assert_eq!(
                should_block_stop(&conn, "s-1", false, SessionMode::Interactive),
                None,
                "a {label} did not count as a record"
            );
        }
    }

    #[test]
    fn a_record_from_another_session_does_not_count() {
        let (mut conn, id) = holding();
        prompt(&conn, "2026-09-16T12:03:00Z");
        tasks::note(&mut conn, &id, "passing by", Source::Cli, Some("s-other"), at("2026-09-16T12:04:00Z")).unwrap();
        assert!(should_block_stop(&conn, "s-1", false, SessionMode::Interactive).is_some());
    }

    #[test]
    fn a_handoff_survives_one_prompt_but_not_two() {
        let (mut conn, id) = holding();
        prompt(&conn, "2026-09-16T12:03:00Z");
        tasks::handoff(&mut conn, &id, "wrote it before the next prompt", Source::Cli, Some("s-1"), at("2026-09-16T12:04:00Z")).unwrap();
        // The user's next prompt arrives seconds later and moves the window: the handoff still
        // counts for this one turn of grace (the race of the 2026-08-28 demo).
        prompt(&conn, "2026-09-16T12:05:00Z");
        assert_eq!(
            should_block_stop(&conn, "s-1", false, SessionMode::Interactive),
            None
        );
        // One more turn with no work at all, and the exemption is over.
        prompt(&conn, "2026-09-16T12:09:00Z");
        assert!(should_block_stop(&conn, "s-1", false, SessionMode::Interactive).is_some());
    }

    #[test]
    fn without_any_prompt_the_window_starts_at_the_session_start() {
        let (conn, _id) = holding();
        // Hooks installed with the session already open: there is no prompt event, and the claim
        // alone must still block. A cutoff of 0 would accept any historical event.
        assert!(should_block_stop(&conn, "s-1", false, SessionMode::Interactive).is_some());
    }
}
```

- [ ] **Step 2: Run to see them fail**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet hooks::handoff_rule
```
Expected: compile errors (no such module).

- [ ] **Step 3: Write `crates/ratchet/src/hooks/handoff_rule.rs`**

```rust
//! The handoff rule on `Stop`: a session that holds a task in progress and left no record of this
//! turn is blocked once and asked for a handoff. Every branch here is a bug somebody hit; the
//! comments say which.

use rusqlite::{params, Connection};

use crate::model::{EventKind, SessionMode, TaskStatus};
use crate::services::{events, tasks};

/// What counts as "this session left a record on this task".
const ACTIVITY: [&str; 5] = [
    "handoff",
    "checklist.done",
    "checklist.undone",
    "note",
    "task.status",
];

pub fn should_block_stop(
    conn: &Connection,
    session_id: &str,
    stop_hook_active: bool,
    mode: SessionMode,
) -> Option<String> {
    // Nobody is on the other side of a headless run: blocking would only hold it open until its
    // launcher kills it, and the launcher records the outcome itself.
    if mode == SessionMode::Headless {
        return None;
    }
    // Never twice: the harness says the close was already blocked once in this response.
    if stop_hook_active {
        return None;
    }
    let mine = tasks::list(
        conn,
        &tasks::Filter {
            statuses: &[TaskStatus::InProgress],
            claimed_by: Some(session_id),
            ..Default::default()
        },
    )
    .ok()?;
    if mine.is_empty() {
        return None;
    }
    let cutoff = cutoff_id(conn, session_id);
    let stale: Vec<String> = mine
        .iter()
        .filter(|t| {
            !has_activity_since(conn, &t.id, session_id, cutoff)
                && !handoff_since_claim(conn, &t.id, session_id)
        })
        .map(|t| t.id.clone())
        .collect();
    let first = stale.first()?.clone();
    let verb = if stale.len() == 1 { "is" } else { "are" };
    Some(format!(
        "{} {verb} still in_progress with nothing recorded this turn. \
         Run `ratchet task handoff {first} \"what is left and how to resume\"` \
         (or `ratchet task check|note|status {first} …`) before you finish.",
        stale.join(", ")
    ))
}

/// The id of this session's last prompt; with no prompts at all (hooks installed while the session
/// was already open) its `session.start` — never 0, which would accept any historical event.
fn cutoff_id(conn: &Connection, session_id: &str) -> i64 {
    for kind in [EventKind::SessionPrompt, EventKind::SessionStart] {
        let found: Option<i64> = conn
            .query_row(
                "SELECT MAX(id) FROM events WHERE session_id = ?1 AND kind = ?2",
                params![session_id, kind.as_str()],
                |r| r.get(0),
            )
            .unwrap_or(None);
        if let Some(id) = found.filter(|id| *id > 0) {
            return id;
        }
    }
    0
}

/// Something this session did to this task after the cutoff. The `task.status` to `in_progress`
/// that `claim` itself emits does not count: claiming and doing nothing is exactly what the rule
/// is for.
fn has_activity_since(
    conn: &Connection,
    task_id: &str,
    session_id: &str,
    after_id: i64,
) -> bool {
    let Ok(history) = events::for_task(conn, task_id, 50) else {
        return false;
    };
    for event in history.iter().rev() {
        if event.id <= after_id {
            break;
        }
        if event.session_id.as_deref() != Some(session_id)
            || !ACTIVITY.contains(&event.kind.as_str())
        {
            continue;
        }
        if event.kind == EventKind::TaskStatus.as_str()
            && event.payload.get("to").and_then(|v| v.as_str()) == Some("in_progress")
        {
            continue;
        }
        return true;
    }
    false
}

/// A handoff of this session, written during this holding, counts even when a prompt has since
/// moved the window: the agent writes the handoff and the user's next prompt arrives seconds
/// later, and the Stop of that turn used to see nothing (the 2026-08-28 demo). The lower bound is
/// not only the claim — that would exempt a long holding with one early handoff for ever — but the
/// *second to last* prompt, so the grace lasts exactly one turn.
fn handoff_since_claim(conn: &Connection, task_id: &str, session_id: &str) -> bool {
    let claim_id: Option<i64> = conn
        .query_row(
            "SELECT MAX(id) FROM events WHERE task_id = ?1 AND session_id = ?2 AND kind = ?3",
            params![task_id, session_id, EventKind::TaskClaimed.as_str()],
            |r| r.get(0),
        )
        .unwrap_or(None);
    let Some(claim_id) = claim_id.filter(|id| *id > 0) else {
        return false;
    };
    let cutoff = claim_id.max(penultimate_prompt_id(conn, session_id));
    let handoff_id: Option<i64> = conn
        .query_row(
            "SELECT MAX(id) FROM events WHERE task_id = ?1 AND session_id = ?2 AND kind = ?3",
            params![task_id, session_id, EventKind::Handoff.as_str()],
            |r| r.get(0),
        )
        .unwrap_or(None);
    handoff_id.map(|id| id > cutoff).unwrap_or(false)
}

/// The id of this session's second-to-last prompt, or 0 when there are fewer than two: with a
/// single prompt there is no previous turn of grace to close.
fn penultimate_prompt_id(conn: &Connection, session_id: &str) -> i64 {
    let Ok(mut stmt) = conn.prepare(
        "SELECT id FROM events WHERE session_id = ?1 AND kind = ?2 ORDER BY id DESC LIMIT 2",
    ) else {
        return 0;
    };
    let Ok(rows) = stmt.query_map(
        params![session_id, EventKind::SessionPrompt.as_str()],
        |r| r.get::<_, i64>(0),
    ) else {
        return 0;
    };
    let ids: Vec<i64> = rows.filter_map(|r| r.ok()).collect();
    if ids.len() < 2 {
        0
    } else {
        ids[1]
    }
}
```

Add `pub mod handoff_rule;` to `crates/ratchet/src/hooks/mod.rs`.

- [ ] **Step 4: Wire the `stop` hook in `crates/ratchet/src/hooks/dispatch.rs`**

Replace the `stop` arm and add the handler:
```rust
        "stop" => stop(&payload, env, &cwd, home),
```
```rust
/// Heartbeat, then the handoff rule. The only place in this group that returns a non-zero code.
fn stop(
    payload: &Payload,
    env: &HashMap<String, String>,
    cwd: &Path,
    home: &Path,
) -> Result<i32, String> {
    let Some(b) = beat(payload, env, cwd, home, Some(EventKind::SessionStop))? else {
        return Ok(0);
    };
    match handoff_rule::should_block_stop(
        &b.conn,
        &b.session.id,
        payload.stop_hook_active,
        b.session.mode,
    ) {
        Some(message) => {
            eprintln!("[ratchet] {message}");
            Ok(super::BLOCK)
        }
        None => Ok(0),
    }
}
```
and add `use super::handoff_rule;` to the imports. Note the order: the heartbeat writes `session.stop` **before** the rule reads, and the rule's window is the last `session.prompt`, so the heartbeat cannot exempt the session from its own rule.

- [ ] **Step 5: Run the whole suite**

```
cd /c/repos/ratchet && cargo test -p ratchet --bin ratchet hooks:: && cargo test -p ratchet && cargo test -p ratchet --test scenarios
cd /c/repos/ratchet && cargo test -p ratchet --release --test latency -- --nocapture
cd /c/repos/ratchet && cargo fmt --check && cargo clippy --all-targets -- -D warnings
```
Expected: everything green — group 0's 21 scenarios, group 1's 24, this group's 32, every unit test, the checker, and the latency test unchanged.

- [ ] **Step 6: Prove the exit codes**

```
grep -rn "BLOCK\|exit(2)\|Ok(2)" crates/ratchet/src/
```
Expected: `BLOCK` is defined once in `hooks/mod.rs` and used in exactly two places — `dispatch::pre_tool` (group 0) and `dispatch::stop` (this task). State both in the hand-off.

- [ ] **Step 7: Hand off (no git)**

List the three files, the full test counts, and the two exit-2 sites.

---

### Task 12: README section and the `ratchet-tasks` skill

The skill was written in group 4 against a CLI that did not exist yet. Two of its statements are now decided differently by this plan, and one thing it never mentions is part of the shipped surface. Each edit below is named; nothing else in the file changes, and no mismatch is fixed silently.

**Files:**
- Modify: `README.md`, `skills/ratchet-tasks/SKILL.md`

**Interfaces:**
- Consumes: the CLI surface Task 8 shipped and the messages Tasks 9-11 print. Verify each claim against the real binary before writing it down.

- [ ] **Step 1: Check the three claims against the shipped binary**

```
export PATH="$HOME/.cargo/bin:/c/Users/eillanes/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cd /c/repos/ratchet && cargo run -p ratchet -- task --help
cd /c/repos/ratchet && cargo run -p ratchet -- task list --help
grep -n "session\|archive\|task list" skills/ratchet-tasks/SKILL.md
```
Record, for the hand-off, which of these the skill already says correctly: (a) `--session` valid on every writing subcommand — **true**, and now also before the subcommand; (b) `ratchet task claim` transfers an orphaned task and leaves a note — **true**; (c) `ratchet task list` and `archive`/`unarchive` — **absent** from the skill, though both ship.

- [ ] **Step 2: Three named edits to `skills/ratchet-tasks/SKILL.md`**

Edit 1 — step 1 of "Minimum cycle" gains the listing command:
```markdown
1. **Look before you claim**: `ratchet task list` is the repo's board (`--mine`, `--status ready`,
   `--tag <t>`); `ratchet task show T-0042` gives one task's body, checklist, last handoff and
   last ten events.
```

Edit 2 — a new paragraph at the end of the "Orphaned tasks" section:
```markdown
When the owner has reviewed a task that is `done`, `ratchet task archive T-0042` takes it off the
board without losing anything: `ratchet task list --all` still lists it, `ratchet task show` still
has its whole history, and `ratchet task unarchive T-0042` brings it back. Archiving is the owner's
call, not yours — ask before you tidy.
```

Edit 3 — the "Session identity" paragraph, whose last sentence changes because `--session` is now a global option:
```markdown
The CLI resolves your session on its own (`RATCHET_SESSION_ID`, or the live session whose
directory covers your cwd). If it says "no session resolved", pass `--session <id>` (the id is in
the briefing); it is accepted anywhere on the command line, `ratchet --session <id> task claim
T-0042` and `ratchet task claim T-0042 --session <id>` alike. A `claim` with no session fails
outright; a `note`, `check`, `status` or `handoff` is recorded with no session and warns once, so
never ignore that warning — an unattributed record is a record nobody can be asked about.
```

- [ ] **Step 3: Append the board section to `README.md`**

Read the file first (group 0's Task 11 and group 4 both appended to it; G4-R1). Two edits, nothing else. First, correct the line group 1 left promising a command this group does not ship: `(\`ratchet config init\` arrives with group 2)` becomes `(there is no generator yet; the template is in this section)`. Then insert, after the "State and sessions" section group 1 appended and before `## Not here (yet)`:

```markdown
## The board

Work lives in tasks, and a task moves only in ways you can check afterwards.

    ratchet task list                       # the board of this repo, one line per task
    ratchet task list --mine --status in_progress
    ratchet task show T-0042                # body, checklist, last handoff, last ten events
    ratchet task new "Port the parser" -c "tests green" -c "docs updated"
    ratchet task claim T-0042               # takes it for your session
    ratchet task check T-0042 1             # one acceptance criterion met
    ratchet task note T-0042 "found X, decided Y"
    ratchet task handoff T-0042 "what is left and how to resume"
    ratchet task status T-0042 review       # or done, when the checklist is complete
    ratchet task archive T-0042             # a reviewed done task leaves the board

Identifiers are `T-0001`, `T-0002`, … from a sequence that never reuses a number. A task carries a
title, a body, a priority from 1 to 4, tags, an optional parent, and the checklist that is its
acceptance criteria. **Progress is derived**: it is `items done / items total`, computed on every
read. Nothing anywhere accepts "about 60 % done", and a task with no checklist reports no progress
at all — which is also why closing one as `done` then needs an explicit `--why`.

Statuses are `backlog → ready → in_progress → blocked → review → done`, plus "back to the queue"
(`in_progress`, `blocked` or `review` → `ready`) and "back to the start" (anything → `backlog`). A
refused move lists the ones that were allowed. Every change appends one event per fact — created,
claimed, status, checklist, note, handoff, archived — and events are never edited or deleted.

Claiming ties a task to your session. A task held by a session that is still alive is not taken
from it; one held by a session that died is, with a note recording the transfer. Sessions that die
give their work back on their own (see *State and sessions*).

Three things the harness does with the board, without being asked:

- **At session start** it prints a briefing of at most 40 lines: the repo, the session and the
  branch; your tasks in progress with their last handoff; the repo's tasks held by dead sessions,
  with theirs; up to five ready to take; and where the full guide is. A repo with nothing pending
  gets one line.
- **On every prompt**, if you hold a task, one line: `[ratchet] T-0042 in_progress (2/5) · last
  handoff: "…"`.
- **When the session tries to close** holding a task you recorded nothing about this turn, it is
  blocked once and asked for `ratchet task handoff …`. It never blocks twice, and never blocks a
  headless run, where nobody could answer.

Any command that would print more than 60 lines writes them to `~/.ratchet/out/<timestamp>-<name>.txt`
instead and prints the first 20 plus that path. `--json` gives the machine-readable form of any
command; `--session <id>` attributes a write explicitly and is accepted anywhere on the line.
```

- [ ] **Step 4: Verify every command in the two files really runs**

```
cd /c/repos/ratchet && grep -ho "ratchet task [a-z-]*" README.md skills/ratchet-tasks/SKILL.md | sort -u
cd /c/repos/ratchet && cargo run -p ratchet -- task --help | head -20
```
Expected: every verb that appears in the docs is in `--help`, and nothing in the docs names a command that does not exist (in particular, neither file may promise `ratchet config init`).

- [ ] **Step 5: Hand off (no git)**

List the two files and the three skill edits, and state which claims of the skill were verified as already correct.

---

### Task 13: Group review (reviewer, read-only)

**Files:** none modified. A blocking item is described with `file:line`, never fixed here.

- [ ] **Step 1: Run the gate** from `C:\repos\ratchet`, with the PATH preamble: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p ratchet`, `cargo test -p ratchet --test scenarios`, `cargo test -p ratchet --release --test latency -- --nocapture`. All green, numbers recorded: the 77 scenarios (21 + 24 + 32), the unit-test count, the release `pre_tool` median against group 1's, and the size of the binary.
- [ ] **Step 2: Contrast against `openspec/specs/tasks/spec.md`, the four appended requirements of `openspec/specs/agent-protocol/spec.md`, and this plan.** Every one of the 32 scenarios must have a test that really bites: pick four (one per new requirement group — a transition rule, a claim rule, the briefing cap, the handoff rule) and break the production line each covers, confirm the test fails, restore.
- [ ] **Step 3: Check the layer rule and the group-1 surface by grep.**
  - `INSERT|UPDATE|DELETE` appears only under `src/services/` and `src/db/migrations/`; `events` is never the target of an `UPDATE` or a `DELETE` anywhere.
  - `db::connect` is called only from `hooks::dispatch::session_start` and `cli::db_cmd::migrate`; every other face uses `db::open_ready`.
  - `Command::new` only in `repo::is_tracked` and `repo::git_branch`; neither is reachable from `pre_tool`.
  - `BLOCK` is used in exactly two places: `dispatch::pre_tool` and `dispatch::stop`.
  - `services::sessions` is byte-for-byte what group 1 left, and `tasks::{orphaned, release_dead, claimed_ids}` keep their signatures; `release` differs only by routing its status write through `set_status_in` (G2-R4).
  - No `Cargo.toml` change in this group.
- [ ] **Step 4: Adversarial probes** with the real binary and a temporary `RATCHET_HOME`:
  - `ratchet task new "x"` from a directory with no marker (exit 1, message, nothing written) and from a linked worktree of an opted-in repo (the task must land under the **main** root's `repo_root`, so the same board is visible from both);
  - two `ratchet task claim` of the same task from two processes started at the same moment (exactly one wins; the loser's message names the holder; no half-written claim);
  - `ratchet task check T-0001 1` on a task whose `checklist_items` rows were deleted behind the CLI's back (a clean error, never a panic);
  - a `stop` hook for a session holding **two** tasks with no record (the message names both, blocks once, and the retry does not block);
  - a `stop` hook whose payload has `stop_hook_active: true` and a database that cannot be opened (exit 0, one log line, no block — an unusable database must never block a close);
  - `ratchet task list --json` with 300 tasks (the file under `out/` is valid JSON, the terminal shows 20 lines and the path);
  - a task whose title contains a newline and a double quote (the listing stays one line per task, and the briefing's 40-line cap holds);
  - `RATCHET_NOW` set to something unparseable while a `session-start` runs (the system clock is used, nothing panics).
- [ ] **Step 5: Verdict** as a written note: APPROVED, or BLOCKING items with `file:line`. Answer two questions explicitly: (a) can group 3 (`fetch`) add its execution record as an event without touching anything in `services/tasks.rs`? (b) is there any path by which a board write happens outside `services/`, including through a test helper that the binary also uses?

---

## Owner questions

The spec is silent on these. Each is decided here so the group does not stall; each is a ruling the owner can reverse with one edit.

1. **A task's repo, with no registry of repos.** The reference validates `repo` against a central `repos.yml` and has a `_global` bucket; D-marker deleted both. **Ruling G2-R1:** a task belongs to the marker above the directory it was created in — `repo` is the display name, `repo_root` the key (G1-R1). `ratchet task new` outside a marked repo fails with a message naming `ratchet.toml`; `--repo <name>` exists on `list` only, as a cross-machine filter, exactly as on `session list`. From inside a repo, `list` scopes to that `repo_root`; from outside one it lists across repos rather than nothing. The reference's two scenarios about the registry (`_global`, "repo not registered") are not ported. Cost if wrong: one filter branch in `task_cmd::list`.
2. **Fields the schema has no room for.** `0001` has no `skill` column, and the reference's `update` service (title, body, priority, tags) has no CLI in spec §4.1. **Ruling G2-R2:** no `skill` field and no `update` service in group 2, so no `task.updated` event either; the reference's "editing a title" scenario is replaced by one over the checklist. Cost if wrong: a migration `0002` and one CLI verb, additive.
3. **`--json` and `--session` as "global options" (spec §4.1).** Making both global in clap is impossible today: group 1 already defines a local `--json` on `session list` and `session show` (Task 11, landed — `main.rs`), and two definitions of one argument id make clap panic at startup. (This ruling originally cited group 3's `fetch --json` per G3-R15; that citation died when the owner cut group 3 to local PDFs only, but the ruling's conclusion is unchanged because group 1 landed a local `--json` of its own.) **Ruling G2-R3:** `--session` becomes a true global option (no subcommand defines one); `--json` stays a per-subcommand flag, present on every command that prints. `ratchet session show <id>` prefers its positional argument over the global one, so the two never disagree. Cost if wrong: one attribute per subcommand.
4. **Group 1's `release` and the richer status payload.** Group 2's `transition` records `from`, `to`, `why` and `claim_released`; group 1's `release` recorded only `to` and `why`. **Ruling G2-R4:** `release` keeps its signature, its two events and their order, and routes its status write through `set_status_in`, which adds the two keys. Group 1's contract pins `to` and `why` and its tests assert kinds and order, so this is additive. Cost if wrong: four lines and a re-run of the 24 `sessions__` scenarios, which Task 5 does anyway.
5. **Where the briefing goes.** **Ruling G2-R5:** exactly at group 1's marked seam — built and printed before `release_dead`, with the same `now`. Reversing the order makes an orphaned task appear as `ready` without its handoff ever being shown (the bug ops fixed in T-0021). The scenario `briefing_with_orphans_and_ready_tasks` asserts both halves.
6. **A write with no session.** **Ruling G2-R6:** `claim` fails (exit 1) when no session resolves — a claim with no holder is meaningless — and so does `list --mine`. `new`, `status`, `check`, `note` and `handoff` record with a null session and print one warning on stderr, because losing the note would be worse than not knowing who wrote it. An *unresolved* session (the CLI could not tell which one you are) and an *unregistered* one (you named one the registry does not know) are different errors with different messages; the second comes from the service and is what the spec's claim scenario pins.
7. **Archived tasks and the harness.** **Ruling G2-R7:** an archived task is invisible to `task list` (unless `--all`), to the briefing, to the prompt reminder and to the orphan sweep, and cannot be moved or claimed until it is unarchived. Only its own `show` and `--all` reach it. Cost if wrong: one `include_archived` flag flipped.
8. **How wide the orphan sweep is.** The reference releases `in_progress`, `blocked` and `review`; group 1 shipped `in_progress` only, and its spec says so. **Ruling G2-R8:** leave it at `in_progress`. Widening it is a change to group 1's `orphaned`, its spec text and its scenarios, for a case (a blocked task held by a dead session) the briefing already shows through `task list`. Cost if wrong: one SQL clause and one scenario.
9. **`ratchet config init`.** Spec §4.1 lists it; §8 puts it in no group, and group 1's README correction promised it "with group 2". §8 governs the split, and group 2 is "Board … briefing … output discipline". **Ruling G2-R9:** not shipped here. Task 12 corrects that README line to say the template is in the README itself, and no message in the binary names a command that does not exist. Cost if wrong: a 20-line task writing a commented template.

## Self-review against the spec

- **Spec coverage (group 2).** §8 group 2 lists eight things. Tasks service: Tasks 4-6. Tasks CLI: Task 8. Checklist: Tasks 3 (`ChecklistItem`), 4 (`checklist`, `progress`), 6 (`check`/`uncheck`), 8 (`task check`). Notes and handoffs: Task 6 and Task 8. Events: Task 3 (the seven new kinds) and every writer in Tasks 5-6, with the append-only constraint restated globally. Briefing: Task 9. Prompt reminder: Task 10. Handoff rule on `Stop`: Task 11. Output discipline: Task 7 (`output.rs`) and Task 8 (every face goes through it). §4.1's CLI line for `task` is covered verb by verb, `unarchive` included. §4.5's "ids from a monotonic sequence", "invalid transitions rejected with the allowed ones listed", "done needs a complete checklist or `--why`", "claim rules", "progress computed never stored", "note and handoff are events", "every mutation writes one row to `events` with its source" each have a named task and a scenario. §4.2's three rows (SessionStart, UserPromptSubmit, Stop) are Tasks 9, 10, 11; their exact message shapes are in the plan body, not paraphrased. §6: the briefing reads through `open_ready` on a connection `session_start` already migrated, and every failure path stays exit 0 except the rule. D-english: every string. D-specs-first: Tasks 1-2, checker green. D-roles: Task 2 is the `spec-test-author`'s, Tasks 3-12 the implementer's, Task 13 the reviewer's; no git anywhere. D-runner-out: the handoff rule reads `SessionMode::Headless` so a future runner's sessions close cleanly, and nothing else of the runner appears.
- **Deliberately deferred.** `ratchet config show|init` (G2-R9), a task `update` verb and the `skill` field (G2-R2), widening the orphan sweep (G2-R8), a global `--json` (G2-R3), and everything of groups 3 and 5. Group 4's content already exists and is only touched by Task 12's three named edits.
- **Placeholder scan.** No "TBD", no "add error handling", no "similar to Task N": every step carries its code or its exact command. The only judgement call left to an implementer is the wording of a hand-off, and Task 12's three edits are quoted in full rather than described.
- **Type consistency.** `tasks::Filter` has the same six fields in Tasks 4, 8, 9, 10 and 11, and is always built with `..Default::default()`. `tasks::progress` returns `Option<(i64, i64)>` everywhere, and `output::format_task_line` is the only place that turns it into text — used by `cli::task_cmd::list` and `hooks::briefing::task_line` alike. `set_status_in(&Connection, &Task, TaskStatus, Source, Option<&str>, Option<&str>, DateTime<Utc>)` has the same seven parameters in Tasks 5, 6 and the `release` edit. `events::emit` keeps group 1's seven parameters at every call site. `Thresholds` is group 0's, never redefined, and reaches `claim`, `orphaned`, `state` and the briefing as `&Thresholds`. `Source::Cli` is used by every CLI write and `Source::Hook` by every hook write, as in group 1. Event kind strings (`task.created`, `task.claimed`, `task.status`, `checklist.done`, `checklist.undone`, `note`, `handoff`, `task.archived`, `task.unarchived`) match between `model::EventKind`, `handoff_rule::ACTIVITY`, the scenario tests and Task 2's Interfaces block. `Beat { conn, session }` is defined in Task 10 and consumed by Task 11 only.

