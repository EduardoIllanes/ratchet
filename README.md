# ratchet

A Claude Code plugin that turns your working rules into things the agent cannot skip: a
guardrail runs before every tool call, work lives on a task board with checklists and
mandatory handoffs, and agent roles are kept separate (spec-test author, implementer,
reviewer). Everything is enforced by hooks and one small binary, not by prompt text.

## What it does

- **Guardrails before every tool call.** Four built-in rules (Python outside the venv,
  destructive git, `.env` files, writes to the main tree) plus your own, per machine or per
  repo. A blocked call gets a one-line reason and the alternative.
- **A task board that cannot be faked.** Tasks with acceptance checklists; progress is
  derived from the checklist, never declared. Every change is an append-only event.
- **Mandatory handoffs.** A session that tries to close while holding a task with nothing
  recorded is asked, once, to write what is left.
- **Session registry.** Every Claude Code session is registered by the hooks; when one dies,
  its tasks go back to the queue on their own.
- **A briefing at session start.** Repo, branch, your tasks in progress with their last
  handoff, orphaned tasks, and up to five ready to take. One line per prompt after that.
- **Local PDF extraction** (`ratchet pdf`) through the `liteparse` CLI, with automatic OCR
  retry and output kept out of the terminal.
- **Agent roles, skills and commands.** Five agent profiles, the `ratchet-tasks` and
  `ratchet-pdf` skills, the OpenSpec skills with their `/opsx:*` commands, and `/ratchet:init`.

Cost of the hot path: the guardrail hook does 11-13 ms of its own work per tool call (see
*Latency* at the end).

## Quick install

    claude plugin marketplace add EduardoIllanes/ratchet
    claude plugin install ratchet@ratchet

That is all. The first time a hook runs it downloads the prebuilt `ratchet` binary for your
platform (macOS arm64/x64, Linux x64, Windows x64) from the release pinned in
`.claude-plugin/binary-version`, verifies its SHA-256 against the release's `SHA256SUMS.txt`,
and puts it in the plugin's `bin/`. No Rust, no PATH changes, nothing installed anywhere else.

The plugin manifest carries no version on purpose: Claude Code tracks the marketplace commit,
so `claude plugin update ratchet@ratchet` picks up every change to agents, skills and hooks
without waiting for a binary release. Each update lands in a fresh plugin dir, and the first
hook there downloads the pinned binary again (a few MB). A new binary ships as a tagged release
that bumps `Cargo.toml` and `binary-version` together.

Then opt a repo in. Open a Claude Code session at its root and run:

    /ratchet:init

`ratchet` is not on your shell's PATH: the binary lives only in the plugin's `bin/`, and Claude
Code adds that directory to the PATH of its own sessions. So every `ratchet ...` command in this
README runs inside a session, either through the `!` prefix (`! ratchet task list`) or by
letting the agent run it. To use it from a terminal, put that `bin/` on your PATH or link the
binary, for example `ln -s "<plugin dir>/bin/ratchet" ~/.local/bin/ratchet`; the link breaks
on every plugin update, because the plugin dir is named after the marketplace commit.

If the download cannot happen (offline, unsupported platform, checksum mismatch), the hook
prints one line and exits 0; the session is not affected, and the hooks stay quiet for an hour
before retrying. To retry now: `bash <plugin dir>/hooks/bootstrap.sh`. To install by hand:
download the asset for your platform from https://github.com/EduardoIllanes/ratchet/releases,
verify it against `SHA256SUMS.txt`, and unpack the single file it contains into
`<plugin dir>/bin/`; or export `RATCHET_BIN=<path to a binary you built>`, which the hooks check
first. The plugin dir is the `installPath` for `ratchet@ratchet` in
`~/.claude/plugins/installed_plugins.json`.

## Quick tour

Real output from a throwaway repo, run from inside a Claude Code session (`!` prefix). Opt it in:

    $ ratchet config init
    wrote /tmp/demo/ratchet.toml

Ask the guardrails what they would do with a tool call (the repo has a `.venv`; exit code 2
means blocked, which is what the hook returns to Claude Code):

    $ ratchet guardrails test Bash '{"command":"python x.py"}'
    [ratchet guardrail:python-venv] Python must run through the repo's virtualenv, not the global interpreter. Prefix the command with `uv run` (e.g. `uv run python scripts/x.py`) or call the venv interpreter (`.venv/Scripts/python` or `.venv/bin/python`).

Create a task with its acceptance criteria and work it:

    $ ratchet task new "Port the parser" -c "tests green" -c "docs updated"
    T-0001  Port the parser  (backlog)

    $ ratchet task list
    T-0001  backlog      p3  Port the parser  (0/2)

    $ ratchet task check T-0001 1
    T-0001 [x] 1. tests green  (1/2)

    $ ratchet task handoff T-0001 "parser ported; docs still pending"
    T-0001 handoff recorded

    $ ratchet task show T-0001
    T-0001  Port the parser
    status backlog · repo demo · priority p3
    progress 1/2

      1. [x] tests green
      2. [ ] docs updated

    last handoff (2026-09-17T13:15:38Z): parser ported; docs still pending

    events:
      2026-09-17T13:15:38Z  task.created     checklist=2 priority=3 repo=demo tags=[] title=Port the parser
      2026-09-17T13:15:38Z  checklist.done   tests green
      2026-09-17T13:15:38Z  handoff          parser ported; docs still pending

Inside a Claude Code session the hooks do the rest: `ratchet task claim T-0001` ties the task
to that session, the next session start prints the briefing with the last handoff, and closing
the session without recording anything is refused once.

## Opt a repo in

Run `/ratchet:init` from a Claude Code session at the repo root (or `ratchet config init` where the binary is on your PATH, see *Quick install*), which writes this file with comments:

    [repo]
    default_branch = "main"
    worktrees_dir = ".worktrees"

    [guardrails]
    off = []

Without that file, every hook is a no-op.

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
board, session or db command; `--session <id>` attributes a write explicitly and is accepted anywhere on the line.

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
this machine for the process-launch reasons given under *Latency*.

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

## Agents and skills


Six agent profiles in `agents/`: `analyst`, `spec-test-author`, `implementer`, `reviewer`,
`refactorer`, `researcher` — see `docs/agent-doctrine.md` for how they are meant to be combined. Two skills:
`ratchet-tasks` (working the board, writing handoffs) and `ratchet-pdf` (extracting text from a
local PDF). The OpenSpec skills (`openspec-propose`, `-apply-change`, `-update-change`,
`-sync-specs`, `-archive-change`, `-explore`) and the `/opsx:*` commands are included as-is
and need the `openspec` CLI installed separately.

## Build from source


With a Rust toolchain (1.79 or newer; on macOS also the Xcode Command Line Tools, on Windows
the Visual Studio Build Tools, both for the bundled SQLite):

    cargo build --release
    cp target/release/ratchet <plugin dir>/bin/     # ratchet.exe on Windows

## Latency


Measured `pre-tool` cost on Windows 11 (release build): about 11-13 ms of ratchet's own work
above process-launch cost (full `pre-tool` runs ~53-56 ms in a direct harness against a ~40 ms
do-nothing-binary floor on that machine). The check this replaces, in a Python harness, cost
~830 ms. `cargo test -p ratchet --release --test latency -- --nocapture` prints the figures for
your machine; the 60 ms ceiling in that test is a local sanity check and fails on slow
launchers, which is why CI reports it without gating on it.

## Not here (yet)


Binaries are not code-signed or notarized; macOS Gatekeeper may ask once
(`xattr -d com.apple.quarantine bin/ratchet` clears it). No Linux arm64 build. The audited
web-fetch flow (approval lists, robots.txt, cache) that the original group 3 plan would have
ported is deliberately not planned for `ratchet` — it stays in `ops`.

## Status

v0.1.1 — groups 0-5 of the design spec are shipped (guardrails, state, task board,
`ratchet pdf`, content, release). v0.1.1 fixes the hook scripts' missing exec bit, which
made every hook fail with "Permission denied" on a fresh v0.1.0 install. Design: `docs/superpowers/specs/2026-09-16-ratchet-plugin-design.md`.
Agent doctrine: `docs/agent-doctrine.md`.
