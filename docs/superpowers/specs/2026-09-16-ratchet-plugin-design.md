# ratchet — a Claude Code plugin for spec-driven, guardrailed agent work

> Design document, 2026-09-16. Approved by the owner in conversation before being written.
> `ratchet` is the domain-agnostic extraction of the harness that runs in the private `ops`
> platform. `ops` itself is not modified by this work and keeps running as is.

## 1. Purpose

`ops` is two things: a local task platform for one investment desk, and a **harness for
Claude Code** that turns working rules into things the agent cannot skip from the prompt
(session briefing at start, a guardrail check before every tool call, a mandatory handoff
before the session closes, and separated agent roles for writing tests, implementing and
reviewing). The second half is not tied to the desk. `ratchet` packages that half as a
Claude Code plugin anyone can install, with nothing from the desk in it.

The name is the idea: work only moves forward in verifiable clicks (checklist items,
handoffs, events), and the harness stops it from slipping back.

**In scope**

- The seven Claude Code hooks and their behaviour: session registry, `[ratchet]` briefing,
  per-prompt task reminder, guardrails, handoff rule, session end.
- The task board (tasks, checklists, notes, handoffs, events) and the sessions registry,
  with a CLI to operate them.
- Declarative guardrails, with four built-in rules and user extension.
- `ratchet pdf`: local PDF text extraction via the external `liteparse` CLI, with an OCR
  retry policy, page ranges, a sink and header-only output. It is the tool of the
  `researcher` agent for documents already on disk.
- Five agent profiles, the `ratchet-tasks` skill, the six OpenSpec skills and the six
  `/opsx:*` commands, all in English.

**Out of scope (deliberately, and stated in the README)**

- The desk data layer: Mongo access, the `mongo-readonly` guardrail as a built-in (users
  can add it as a custom rule), datamart, recipes, widgets, collections catalogue, DuckDB.
- The agent runner (queue + detached worker running `claude -p`). Its spec has 69 scenarios
  and is the largest subsystem; it is a candidate for a later change once the plugin works.
- The FastAPI service and the React web UI.
- The git hooks and `check.ps1` gate of `ops` (they are ruff/mypy/pytest specific). The
  plugin documents that a local gate should exist; it does not ship one.
- Any credentials, `.env` files, or the owner's personal memories.

## 2. Decisions

Each decision names at least one alternative and why it lost.

**D-runtime — Rust binary, not Python.** Measured on the owner's Windows machine, the `ops`
`PreToolUse` hook costs ~830 ms per tool call (Python interpreter start ~300-350 ms, then
~400 ms of `pydantic` and model imports the guardrail does not even use); wrapped in `uv run`
it is ~1000 ms. A compiled binary does the same in 5-20 ms. A plugin also cannot require
`uv`, a venv and a hard-coded interpreter path on every machine. Rust over Go: consistent
with the tooling the owner already uses (`uv`, `ruff` are Rust), mature CLI ecosystem
(`clap`), embedded SQLite (`rusqlite` with `bundled`). Go would have been equally fast and
easier to read; it lost on ecosystem fit only. A stdlib-only Python rewrite would still
pay the ~350 ms interpreter floor on Windows and keep the `uv` requirement.

**D-regex — `fancy-regex` for guardrail patterns.** The built-in `git-destructive` rule uses
lookahead. Neither Rust `regex` nor Go `regexp` supports lookaround; `fancy-regex` does,
so the existing patterns port verbatim. Cost: no linear-time guarantee for user-supplied
patterns. Accepted because patterns come from the user's own config, and the hook applies
a per-rule size cap on the scanned text.

**D-binary-delivery — GitHub releases + bootstrap, not `cargo install`.** The plugin repo
publishes one binary per platform (Windows x64, macOS arm64/x64, Linux x64) tagged with the
plugin version. `hooks/run-hook.cmd` (a polyglot cmd/bash wrapper, same trick as the
`superpowers` plugin) looks for `${CLAUDE_PLUGIN_ROOT}/bin/ratchet[.exe]`; if missing, it
downloads the release matching the plugin's version, verifies the published SHA-256, and
runs it. `cargo install` lost because it needs a Rust toolchain on the user's machine.
Vendoring binaries inside the plugin repo lost because it bloats every install and every
update. The bootstrap runs once; if it cannot download (offline, blocked), the hook exits 0
and prints one line explaining how to place the binary manually. Users with a toolchain can
always build from source and drop the binary in `bin/`.

**D-marker — the repo opts in with a `ratchet.toml` at its root, not a global repo list.**
`ops` decides which repos it governs from a central `repos.yml`. That does not distribute:
every user would edit a central file, and the plugin's hooks are global (user scope). With a
marker, every hook first resolves the repo root of `cwd` (walking up to the nearest
`ratchet.toml`, or the git toplevel that contains one); no marker → exit 0 immediately,
nothing is opened. This also guarantees ratchet never fires inside `C:\repos\ops` on the
owner's machine, which has no marker and keeps its own hooks. A global
`~/.ratchet/config.toml` exists only for machine-level settings (home dir, log, pdf
limits, extra guardrails), never for the list of repos.

**D-state — everything under `~/.ratchet/`.** `ratchet.db` (SQLite, WAL), `ratchet.log`,
`out/` (long CLI output), `data/cache/`, `data/sink/`. Overridable with `RATCHET_HOME`.
One file per concern, deletable to reset.

**D-p3 — hooks never break a session.** Any internal error in a hook is exit 0 plus one
line in `ratchet.log`. The two deliberate exceptions, as in `ops`: a guardrail match (exit
2, blocks the tool, message + alternative on stderr) and the handoff rule on `Stop` (exit 2
once; never on the retry, detected via `stop_hook_active`).

**D-english — all plugin content and code in English.** The owner's goal is distribution
beyond the team. Agent profiles, skills, guardrail messages, briefing and docs are
translated, not transliterated: names change (`analista-ops` → `analyst`,
`autor-tests-spec` → `spec-test-author`, `implementador-ops` → `implementer`,
`revisor-ops` → `reviewer`, `investigador` → `researcher`; `ops-tasks` → `ratchet-tasks`),
and every mention of `ops` becomes `ratchet`.

**D-pdf — local PDF reading only, cut from the previously planned `ratchet fetch` on
2026-09-16 (owner).** The superseded D-fetch decision ported the whole audited flow of
`ops data fetch` (approval lists, redirect re-validation, robots, cache, forms) as
`ratchet fetch`, using liteparse (`@llamaindex/liteparse`, npm) for the PDF half. The owner
cut that scope on 2026-09-16: `ratchet` v1 ships only `ratchet pdf <file> [--pages]
[--ocr]`, a thin wrapper over the external `liteparse` CLI for a PDF already on disk,
because the owner only wants PDF reading in the plugin — the audited, approval-gated web
retrieval flow stays in `ops`, where it already runs daily and does not need to be
distributed yet. `liteparse` stays an external dependency
(`npm i -g @llamaindex/liteparse`); the plugin says so and `ratchet pdf` fails with a clear
message when it is missing. No network code, no allowlist, no `--explicit`/`--from`, no
HTML, no robots, no cache, no forms, no cookies, and no trace of any URL ships in v1
(see §9).

**D-runner-out — no agent runner in v1.** See scope. The hooks still read
`RATCHET_SESSION_MODE` (`interactive`/`headless`) and `RATCHET_LAUNCHED_BY`
(`user`/`platform`) so a headless `claude -p` session registers like an interactive one,
as the protocol spec requires; that is one line and keeps the door open.

**D-specs-first — the OpenSpec specs of `ops` are the contract.** `agent-protocol`,
`tasks`, `sessions` and `desk-research` are ported to English into `openspec/specs/` of
this repo, minus scenarios about the data layer, the web API and the runner. Scenario tests
in Rust reference each `#### Scenario` by slug, and a `check_scenarios` step in the gate
enforces the one-to-one mapping. This is how the rewrite stays honest: the language
changed, the behaviour did not.

**D-roles — the same separation of roles applies to building ratchet.** Spec-test author,
implementer and reviewer are different subagents with different context, per task group.
On the owner's machine the work happens in `C:\repos\ratchet` **without any git command**
(the owner publishes it from another GitHub account); agents deliver files and say what to
commit.

## 3. Repository layout

```
ratchet/
├── .claude-plugin/plugin.json       name "ratchet", version, description, author, license
├── hooks/
│   ├── hooks.json                   seven events → run-hook.cmd hook <event>
│   ├── run-hook.cmd                 polyglot wrapper: finds/bootstraps bin/ratchet, execs it
│   └── bootstrap.sh                 download release binary for this platform, verify sha256
├── bin/                             gitignored; binary lands here on first run
├── agents/                          analyst.md, spec-test-author.md, implementer.md,
│                                    reviewer.md, researcher.md
├── skills/
│   ├── ratchet-tasks/SKILL.md       operating the board: claim, check, note, handoff
│   ├── ratchet-pdf/SKILL.md         local PDF extraction, page-range budget, read the sink
│   └── openspec-*/                  the six OpenSpec skills (need the `openspec` CLI)
├── commands/opsx/                   apply, archive, explore, propose, sync, update
├── crates/ratchet/                  the Rust binary (single crate, workspace-ready)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                  clap: hook, task, session, pdf, db, guardrails, config
│       ├── config.rs                ratchet.toml (repo) + ~/.ratchet/config.toml (machine)
│       ├── repo.rs                  resolve repo root, worktrees, tracked-file check
│       ├── db/                      migrations (embedded SQL), connection, WAL
│       ├── model.rs                 Task, ChecklistItem, Session, Event, enums
│       ├── services/                tasks.rs, sessions.rs, events.rs — the ONLY writers
│       ├── hooks/                   dispatch.rs, briefing.rs, handoff_rule.rs
│       ├── guardrails/              rules.rs (built-ins), eval.rs, segment.rs
│       ├── pdf.rs                   PdfExtractor seam, liteparse invocation, OCR retry
│       │                            policy, page-range parsing, sink writer
│       └── output.rs                compact output, long output to ~/.ratchet/out
├── openspec/                        config.yaml + specs/{agent-protocol,tasks,sessions,pdf}
├── scripts/check_scenarios.*        every active scenario has a test that references it
├── docs/                            README (install, replicate), this spec, agent doctrine
└── README.md
```

Layer rule, carried over verbatim: `services/` is the only module that writes to SQLite;
every mutation appends an event; the CLI and hooks are thin faces over `services/`. Task
progress is derived from the checklist, never declared.

## 4. Components

### 4.1 The binary and its CLI

```
ratchet
├── hook   session-start | prompt | pre-tool | stop | subagent-stop | pre-compact | session-end
│          (reads the Claude Code JSON payload on stdin; exit codes per D-p3)
├── task   list show new claim status archive unarchive check note handoff
├── session list show
├── pdf    <file> [--pages "1-8,12"] [--ocr]
├── guardrails list | test <tool> <json-payload>      (dry-run a rule set)
├── db     migrate | path
├── config show | init                                (writes a ratchet.toml template)
└── version
```

Global options: `--json` for machine output, `--session <id>`. Output discipline: any output
longer than 60 lines goes to `~/.ratchet/out/<timestamp>-<cmd>.txt` and the terminal gets
the first 20 lines plus the path.

Session identity resolution, in order: `--session`, `RATCHET_SESSION_ID`, then the live
session whose directory (or worktree) covers `cwd`, most specific first.

### 4.2 Hooks

`hooks.json` registers the seven events. All of them call
`"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd" hook <event>` with `shell: bash`. The
`PreToolUse` matcher is `Bash|PowerShell|Edit|Write|NotebookEdit|MultiEdit`, which fixes
the `ops` discrepancy where the installer omitted `PowerShell`.

| Event | Behaviour |
|---|---|
| `SessionStart` | Resolve repo via marker; register or refresh the session (mode, launched-by, branch, worktree); write `export RATCHET_SESSION_ID=…` to `CLAUDE_ENV_FILE`; release tasks claimed by dead sessions (heartbeat expired); print the `[ratchet]` briefing (≤ 40 lines: repo · session · branch, your in-progress tasks with last handoff, orphaned tasks, up to 5 `ready`, one line pointing to the `ratchet-tasks` skill). |
| `UserPromptSubmit` | Heartbeat. If you hold an `in_progress` task, print `[ratchet] T-0042 in_progress (3/7) · last handoff: "…"`. |
| `PreToolUse` | Hot path, never opens the DB. Evaluate guardrails; on match write `[ratchet guardrail:<id>] <message> <alternative>` to stderr and exit 2. |
| `Stop` | Handoff rule: if you hold an `in_progress` task with no record from you since the last prompt (handoff, check, note, or a status change other than your own claim), block once asking for `ratchet task handoff …`. Never blocks when `stop_hook_active` is true. |
| `SubagentStop`, `PreCompact` | Heartbeat. |
| `SessionEnd` | Mark the session `ended`, return its `in_progress` tasks to `ready`. |

Target latency: `pre-tool` under 30 ms end to end on Windows including the wrapper; all
other hooks under 100 ms. Measured in the gate with a benchmark test, not assumed.

### 4.3 Repo marker `ratchet.toml`

```toml
[repo]
name = "myproject"            # optional; defaults to the directory name
default_branch = "main"
worktrees_dir = ".worktrees"  # relative to the repo root; writes here are never "main tree"

[guardrails]
off = []                       # ids of built-in rules to disable in this repo
extra = "ratchet/guardrails.toml"   # optional: repo-specific rules, same schema as built-ins

[thresholds]                   # optional, defaults shown
live_minutes = 10
idle_minutes = 60
```

`ratchet config init` writes this file with comments. Without it every hook is a no-op.

### 4.4 Guardrails

Same declarative model as `ops`: `id`, `tools`, `kind` (`command` | `file_path` |
`content` | `main_tree`), `pattern`, optional `exempt` (same segment), optional `requires`
(`venv`), `message`, `alternative`. `command` splits the command into segments on `;`,
`&&`, `||`, `|` and newlines outside quotes, and tests each segment. `PowerShell` is
covered by the same rules as `Bash`.

Built-ins, embedded in the binary:

| id | tools | blocks | alternative it suggests |
|---|---|---|---|
| `python-venv` | Bash, PowerShell | bare `python`/`pip`/`pytest`/`uvicorn`/`mypy`/`ruff` (with env prefixes); exempt if the segment has `uv run` or `.venv/…`; only when the repo has a `.venv` | `uv run …` or the venv interpreter |
| `git-destructive` | Bash, PowerShell | `git push --force`/`-f` (allows `--force-with-lease`), `git reset --hard`, `git checkout -- .`, `git clean -f…`, recursive+forced file deletion in any spelling | ask the owner, or a reversible alternative |
| `env-files` | Edit, Write, NotebookEdit, MultiEdit, PowerShell | any `file_path` ending in `.env` or `.env.<x>` | credentials live outside the repo; ask the owner for a missing variable |
| `main-tree` | Edit, Write, NotebookEdit, MultiEdit | writing a **tracked** file in the main tree, outside `worktrees_dir` | `git worktree add <worktrees_dir>/<name> -b <branch>` |

Extension points: `~/.ratchet/guardrails.toml` (machine-wide, appended) and the repo's
`extra` file (appended). A user rule with the id of a built-in replaces it. The README
shows `mongo-readonly` as the worked example of a custom `content` rule.

Known false positives are documented, not fixed: a quoted string containing `"; mypy"`
trips `python-venv` because segments split on `;`; `content` rules scan file contents,
so quoting a blocked pattern in documentation blocks the write.

### 4.5 Task board and sessions

Ported one to one from the `tasks` and `sessions` specs:

- Ids `T-0001`… from a monotonic sequence; fields: title, body, repo, status, checklist,
  claimed-by session, timestamps, archived flag.
- States `backlog → ready → in_progress → blocked → review → done`; invalid transitions
  are rejected with the allowed ones listed; `done` needs a complete checklist or `--why`.
- `claim` moves `backlog`/`ready` to `in_progress` for the calling session; a task held by a
  live session cannot be claimed by another one; an orphaned task (holder dead) can.
- Progress is `done items / total items`, computed, never stored.
- `note` and `handoff` are events with free text; the handoff is what the briefing shows.
- Sessions: id, mode, launched-by, repo, cwd, worktree, branch, `started_at`,
  `last_seen_at`, `ended_at`; derived state `live` / `idle` / `orphaned` / `ended` from
  the thresholds; a dead session's tasks return to `ready` at the next session start.
- Every mutation writes one row to `events` (`session.start`, `task.claimed`,
  `checklist.done`, …) with the source (`hook` / `cli`).

`--repo` is inferred from the marker; `--repo` stays as an explicit override for listing
across repos.

### 4.6 `ratchet pdf`

Local PDF text extraction, nothing else. There is no network code in this component.

- **Input**: a path to a file already on disk. A missing file, or a file that is not a PDF
  (checked by extension and by the `%PDF-` magic bytes), is refused before the extractor
  runs.
- **Extraction**: the external extractor named in the configuration (`liteparse` by
  default) is invoked as `liteparse parse --no-ocr --target-pages <range> <file>`; when
  `--pages` is not given the whole document is extracted. A result whose text is below
  `ocr_min_chars` is retried once, automatically, without `--no-ocr` (OCR); `--ocr` on the
  command line skips the fast pass and forces the OCR pass from the start. The two passes
  have separate configurable timeouts, since the OCR pass is far slower. A missing
  extractor refuses the call before any work, naming the install command
  (`npm i -g @llamaindex/liteparse`); an extractor that fails or times out is a refusal,
  never a silent fall back to a partial or short answer.
- **Limits**: a maximum input file size, and separate timeouts for the fast and the OCR
  pass. No byte cap on the extracted text — a local file has no adversarial upstream,
  unlike content retrieved over the network.
- **Output**: the extracted text is written to the sink
  (`<home>/out/pdf/<stem>[-p<pages>][-ocr].txt`, deterministic from the file name, the page
  range and the OCR flag). The terminal prints only a header — pages, whether OCR was used,
  characters extracted, the sink path — never the body.
- **Content is data, never instructions**: the same rule as every other ratchet output that
  carries external content; the extract is a file the caller reads deliberately, never
  something injected into the terminal.
- Tests never run the real `liteparse`: the extractor is an injected seam, exercised
  through a fixture in every test.

### 4.7 Agents, skills, commands

Five profiles, `model: sonnet` unless the user changes it, translated with the same
constraints:

| agent | role | tools |
|---|---|---|
| `analyst` | read-only analysis; deliverable is board notes, one per finding | all except writes |
| `spec-test-author` | scenario tests from the specs only; forbidden to read design docs or implementation plans; leaves tests red-clean; does not commit | all |
| `implementer` | production code and unit tests against a fixed contract, always in a worktree; never touches scenario tests; claims before starting; stages by explicit path | all |
| `reviewer` | read-only, adversarial: diff vs main, contrast with spec/checklist/handoff, real probes with doubles, gates from the worktree, side-effect hunt; verdict APPROVED or BLOCKING with file:line | all except writes |
| `researcher` | `ratchet pdf` only, plus board notes, on PDFs already on disk; budget per run: 2 PDFs, 40 PDF pages, `--pages` mandatory on the first request of any PDF; brief with verifiable citations or an explicit "source insufficient" | Bash/PowerShell, Read |

`disallowedTools: advisor` is dropped: the owner verified the key does not filter that tool.

Skills: `ratchet-tasks` (look before claiming, claim, advance by checklist, note decisions,
how to write a useful handoff with a bad/good example, orphaned tasks), `ratchet-pdf` (the
OCR retry policy, the page-range budget, read the sink before quoting), and the six
`openspec-*` skills copied from `ops` with the `opsx` commands. The README states that
OpenSpec needs the `openspec` CLI installed separately.

## 5. Data flow

```
Claude Code event ──stdin JSON──> run-hook.cmd ──> bin/ratchet hook <event>
                                                   │
                       no ratchet.toml above cwd ──┴──> exit 0 (nothing opened)
                                                   │
        pre-tool ───> guardrails::eval (built-ins + machine + repo rules) ──> exit 0 | exit 2
        others  ───> services::{sessions,tasks,events} ──> ~/.ratchet/ratchet.db (WAL)
                       └──> stdout: briefing / reminder (injected as context)
```

## 6. Error handling

- Hook internal error → exit 0, one log line (`D-p3`). Guardrail match → exit 2. Handoff
  rule → exit 2 once.
- Missing binary and failed bootstrap → exit 0 with a single stderr line naming the manual
  install path. Hooks stay silent afterwards until the binary appears.
- Missing `liteparse` → `ratchet pdf` fails before any extraction attempt with the install
  command in the message.
- DB schema out of date → the CLI says `run ratchet db migrate`; hooks migrate
  automatically on `session-start` (idempotent, embedded SQL migrations with a
  `schema_version` table) and never elsewhere.
- Invalid `ratchet.toml` → hooks exit 0 with one log line; the CLI reports the parse error
  with line and key.

## 7. Testing

- **Scenario tests** in `crates/ratchet/tests/spec/`, one per `#### Scenario` of the four
  ported specs, referenced by slug; `scripts/check_scenarios` fails the gate on any
  scenario without a test.
- **Unit tests** per module; guardrail rules get a table of blocked / allowed commands that
  is copied from the `ops` test suite so the port is provably equivalent.
- **Hook tests** run the binary as a subprocess with real JSON payloads and assert exit code,
  stdout and stderr; a temporary `RATCHET_HOME` and a temporary git repo with a marker per
  test.
- **Latency test**: `pre-tool` end to end under the target on the CI runner, reported, and
  failing above a hard ceiling.
- **PDF tests** use a fake extractor placed on `PATH`, never the real `liteparse`; there is
  no network code in this component to fake.
- Gate: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `check_scenarios`.

## 8. Delivery plan (task groups)

Each group: scenario tests first by `spec-test-author`, implementation by `implementer`,
review by `reviewer`. No git on the owner's machine (D-roles).

0. **Skeleton and hot path.** Plugin manifest, `hooks.json`, `run-hook.cmd`, crate
   skeleton, `ratchet.toml` resolution, `ratchet hook pre-tool` with the four built-in
   guardrails and user extension, `guardrails list|test`, latency benchmark. Ships a
   usable guardrail-only plugin.
1. **State.** Embedded migrations, `db migrate|path`, sessions service, the other six hooks
   (registry, heartbeat, orphan release, session end), `session list|show`.
2. **Board.** Tasks service and CLI, checklist, notes, handoffs, events, briefing and prompt
   reminder, handoff rule on `Stop`, output discipline.
3. **PDF.** `ratchet pdf` over the liteparse CLI, sink, header-only output.
4. **Content.** Five agents, `ratchet-tasks`, `ratchet-pdf`, OpenSpec skills and
   commands, README with install and "what is not here" sections.
5. **Release.** Cross-platform build workflow, release assets with checksums, bootstrap
   download path exercised end to end on a clean machine.

## 9. Open points

None blocking. Two the owner may revisit later: adding the agent runner as its own change,
and whether `ratchet` should ship a generic local gate script. Web fetch with
approval/robots/cache was removed from v1 on 2026-09-16 (owner); it stays in ops.

## 10. Group 5 amendments (2026-09-17, owner)

Decisions taken while planning group 5 (Release). Where they conflict with sections above,
these win.

**D-no-bootstrap — amends D-binary-delivery.** There is no `hooks/bootstrap.sh` and
`run-hook.cmd` does not download anything. It keeps its current behaviour: use `RATCHET_BIN`
if set, else `bin/ratchet[.exe]`, else print one stderr line and exit 0. Installation is
manual and documented in the README: download the release asset for the platform, verify the
checksum, unpack into `bin/`; or build from source. Rationale: a plugin that downloads and
runs binaries on first use, or installs a Rust toolchain, is more invasive than the problem
warrants; the release assets and a three-line README section cover the same need. Section 3
layout and section 6 error handling read accordingly (`bootstrap.sh` removed; "failed
bootstrap" is now just "missing binary").

**D-release-assets.** GitHub releases are cut by a workflow triggered on tags `v*`. The
workflow fails if the tag does not match `Cargo.toml`'s `version` and `plugin.json`'s
`version`. Four targets: `x86_64-pc-windows-msvc`, `aarch64-apple-darwin`,
`x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`. One archive per target named
`ratchet-<version>-<target>.tar.gz` (`.zip` on Windows) containing only the binary, plus one
`SHA256SUMS.txt` covering all archives. `Cargo.toml` is the version's source of truth; a test
asserts `plugin.json` matches it.

**D-ci-gate.** `ci.yml` runs on push and pull request to `main` on Ubuntu, macOS and
Windows: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
The `latency` test is excluded from that gate and run in a separate step in release mode
with `--nocapture` and `continue-on-error`: a report, not a gate. The 60 ms local ceiling
stays as is.

**D-check-scenarios-in-cargo — amends section 3 and section 7.** `scripts/check_scenarios.*`
is not created. The check already exists as the integration test
`crates/ratchet/tests/scenarios.rs` (`every_scenario_has_a_test`): it walks
`openspec/specs/*/spec.md`, extracts every `#### Scenario:` heading outside fenced blocks,
derives the slug, and fails listing every scenario that no file under
`crates/ratchet/tests/spec/` references as `fn <spec>__<slug>(`. It is part of the gate by
virtue of `cargo test`; group 5 adds nothing here beyond running it in CI.

**Group 5 scope.** The two workflows, the version-match test, the README install rewrite plus
the marketplace manifest, the deferred group-4 consistency pass of skills and agents against
the CLI shipped by groups 2 and 3, and the `v0.1.0` release as the end-to-end proof. Out of
scope: automatic bootstrap, `cargo install`, code signing or notarization, Linux arm64.
