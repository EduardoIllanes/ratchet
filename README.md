# ratchet

A Claude Code plugin that turns working rules into things the agent cannot skip from the
prompt: guardrails before every tool call, a task board with checklists and mandatory
handoffs, and separated agent roles (spec-test author, implementer, reviewer).

Status: group 2 — the task board (CLI, briefing, prompt reminder, handoff rule, output discipline). See
`docs/superpowers/specs/2026-09-16-ratchet-plugin-design.md`.

## Build from source (until releases exist)

    cargo build --release
    # copy target/release/ratchet (or ratchet.exe) into bin/

## Opt a repo in

Create `ratchet.toml` at the repo root (there is no generator yet; the template is in this section):

    [repo]
    default_branch = "main"
    worktrees_dir = ".worktrees"

    [guardrails]
    off = []

Without that file, every hook is a no-op.

## Agents and skills

Five agent profiles in `agents/`: `analyst`, `spec-test-author`, `implementer`, `reviewer`,
`researcher` — see `docs/agent-doctrine.md` for how they are meant to be combined. Two skills:
`ratchet-tasks` (working the board, writing handoffs) and `ratchet-pdf` (extracting text from a
local PDF). The OpenSpec skills (`openspec-propose`, `-apply-change`, `-update-change`,
`-sync-specs`, `-archive-change`, `-explore`) and the `/opsx:*` commands are included as-is
and need the `openspec` CLI installed separately.

## Install (group 0, from source)

1. `cargo build --release`
2. Copy `target/release/ratchet` (Windows: `ratchet.exe`) into `bin/` of this plugin directory,
   or export `RATCHET_BIN=<path>`.
3. Install the plugin in Claude Code from this directory (marketplace entry or
   `claude plugin add <path>`), then restart the session.
4. In a repo you want governed: create `ratchet.toml` at its root (see above).
5. Check: `ratchet guardrails list` from that repo, and
   `ratchet guardrails test Bash '{"command":"python x.py"}'` should exit 2 when the repo has a `.venv`.

Measured `pre-tool` latency on Windows 11 (release build): about 11-13 ms of ratchet's own
work above process-launch cost (full `pre-tool` runs ~53-56 ms in a direct harness, ~66 ms
through `cargo test`, against a ~40 ms do-nothing-binary floor on this machine). End-to-end
wall clock is dominated by this machine's process-launch overhead, so the 60 ms ceiling in
`cargo test -p ratchet --release --test latency` currently fails here even though the code's
own work is well inside the design spec's 30 ms budget. The same check in the Python-based
harness this replaces cost ~830 ms; exact figures for a given machine are printed by
`cargo test -p ratchet --release --test latency -- --nocapture`.

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

## PDF

`ratchet pdf <file> [--pages "1-8,12"] [--ocr]` extracts text from a local PDF via the external
`liteparse` CLI (`npm i -g @llamaindex/liteparse`) — no network code, no approval list, nothing
web-related. The fast pass runs first; text under the configured minimum retries once with OCR
automatically; `--ocr` forces OCR from the start. The extracted text is written to
`~/.ratchet/out/pdf/<stem>[-p<pages>][-ocr].txt`; the terminal prints only a short header
(pages, whether OCR was used, characters extracted, the sink path) and never the body. Configure
the extractor and its limits in `~/.ratchet/config.toml`:

    [pdf]
    extractor = "liteparse"
    timeout_s = 60
    ocr_timeout_s = 600
    ocr_min_chars = 200
    ocr_language = "eng"
    max_file_bytes = 209715200

The full audited **web**-fetch flow (approval lists, robots.txt, cache, forms) is not here and
is not planned — it stays in `ops`, where it already runs daily (owner decision, 2026-09-16; see
`docs/superpowers/specs/2026-09-16-ratchet-plugin-design.md` §2 D-pdf, §9).

## State and sessions

Everything ratchet remembers lives in one SQLite file: `~/.ratchet/ratchet.db` (override the
directory with `RATCHET_HOME`). SQLite is compiled into the binary — nothing to install — and
the schema is applied by embedded migrations. The session-start hook migrates automatically;
everywhere else, an out-of-date database says so and asks for `ratchet db migrate`.

    ratchet db path        # where the database is
    ratchet db migrate     # apply pending migrations
    ratchet session list   # one line per session: id, state, repo, branch, tasks, last signal
    ratchet session list --live
    ratchet session show   # the session covering this directory (or name one)

A session is registered by the hooks themselves, with the identifier Claude Code gives them —
ratchet never invents one. It records the repo, the directory, the worktree and branch when
there are any, the mode (`interactive` or `headless`), who launched it (`user` or `platform`),
and its signals. State is derived, never stored: `live` while the last signal is under
`live_minutes`, `idle` until `idle_minutes`, `orphaned` after that, `ended` once the session
closed. Both thresholds come from `[thresholds]` in the repo's `ratchet.toml` (10 and 60 by
default).

When a session dies, its work goes back: a task `in_progress` held by an ended or orphaned
session returns to `ready` without a session, at the end of that session and, for sessions that
died without a hook, at the next session start in that repo. The task keeps its whole history.

Two environment variables let a launcher place a headless session in the registry:
`RATCHET_SESSION_ID` (the identity, also written to the session's shell through
`CLAUDE_ENV_FILE`), `RATCHET_SESSION_MODE` and `RATCHET_LAUNCHED_BY`. `RATCHET_NOW` (RFC 3339)
replaces the clock for one process and exists so the time-dependent behaviour above can be
tested without sleeping.

The `PreToolUse` guardrail hook still opens no database at all: it is the hot path. Its own
work is the ~11-13 ms measured above; the wall-clock ceiling it is tested against fails on
this machine for the process-launch reasons given there.

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

## Not here (yet)

Release binaries and bootstrap (group 5). Group 4 content (agent profiles, skills, the OpenSpec
commands, and `docs/agent-doctrine.md`) is present — see "Agents and skills" above. The audited
web-fetch flow (approval lists, robots.txt, cache) that the original group 3 plan would have
ported is deliberately not planned for `ratchet` — it stays in `ops`.
