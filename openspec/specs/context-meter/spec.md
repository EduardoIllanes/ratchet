# context-meter

How full the context windows of a Claude Code session are, shown while the session works: the
main window on the status line, a toast when it crosses a threshold, every subagent's window
beside it, and a pane that breaks the main window down by category.

## Purpose

The doctrine keeps the orchestrator's context useful by delegating; nothing showed whether it
was working. The meter is a function-hook module (Claude Code's early-access plugin API) that
ships inside the plugin beside the command hooks. It reads only what the engine reports to
hooks — the status line's figures, each model request's token counts, the subagent list — and
writes nothing to disk. It runs only in repos that opted in to ratchet.

## Requirements

### Requirement: The module ships inside the plugin and validates
`hooks/hooks.json` SHALL keep its command hooks and SHALL also name the module under
`"modules": ["./context-meter/register.tsx"]`; `.claude-plugin/plugin.json` SHALL name the
module's state contract under `"types"` and SHALL still carry no `version`. The module's state
SHALL live under `PluginState['ratchet']`. `claude plugin validate` on the repository root SHALL
pass. Every scenario of this spec but this requirement's SHALL be a test of the module under
`claude plugin test`, named exactly as the scenario, and its Rust spec function SHALL assert
that `claude plugin test` on the repository root printed `(pass) <scenario title>`. With no
`claude` on the path those Rust functions SHALL fail naming
`npm install -g @anthropic-ai/claude-code@2.1.288`, the version the module is tested against,
which CI SHALL install before `cargo test` on every platform.

#### Scenario: The plugin validates with the module declared
- **WHEN** `claude plugin validate` runs on the repository root
- **THEN** it exits 0, and its output names the module's hooks `session.start`,
  `session.measure`, `turn.step` and `ui.render` for the `Pane` component

### Requirement: Only a repo that opted in is metered
On `session.start` the module SHALL walk the session's `cwd` and each of its ancestors, nearest
first, splitting on both `/` and `\`, and SHALL treat the session as opted in when one of them
holds `ratchet.toml` as a regular file (`$.fs.stat` resolving with `kind: 'file'`; a stat that
rejects counts as absent) — the rule `repo::find_repo` applies. A session that did not opt in
SHALL get no status line, no toast, no `/ctx` command and no pane from the module, and every
hook SHALL pass its event on unchanged.

#### Scenario: Without ratchet.toml the meter stays silent
- **WHEN** the session starts in `/work/plain/src` with no `ratchet.toml` in it or any ancestor, and a measurement then reports 90% of a 200000-token window
- **THEN** the module pins no status line, shows no toast, and registers no command

#### Scenario: A ratchet.toml in an ancestor opts the session in
- **WHEN** the session starts in `/work/repo/crates/core` and only `/work/repo/ratchet.toml` exists, as a file
- **THEN** the module registers the `ctx` command and pins a status line

#### Scenario: A directory named ratchet.toml does not opt in
- **WHEN** the session starts in `/work/repo` and `/work/repo/ratchet.toml` is a directory
- **THEN** the module registers no command and pins no status line

### Requirement: The status line shows the main window's fill
In an opted-in session the module SHALL pin one status line,
`ctx <bar> <percent>% · <tokens>/<window>`, where `<bar>` is ten cells, `round(percent / 10)`
of them `█` and the rest `░`, clamped to ten; tokens are written as the integer below 1000,
`round(n / 1000)` followed by `k` below one million, and `n / 1000000` followed by `M` above,
with one decimal unless it is whole. Before any response of the live window (`tokens` absent)
the line SHALL be `ctx ░░░░░░░░░░ –/<window>`. It SHALL be refreshed on each `session.measure`
whose `changed` names `context`, from that measurement's `context`, and after each model request
of the main loop (a `turn.step` with no `agentId` whose result carries `usage`), with
`tokens = input_tokens + cache_read_input_tokens + cache_creation_input_tokens`, the window from
`$.session.usage()`, and `percent = round(tokens / window × 100)`.

#### Scenario: A measurement pins the fill on the status line
- **WHEN** a measurement whose `changed` names `context` reports 84000 tokens, 42%, of a 200000-token window
- **THEN** the status line is `ctx ████░░░░░░ 42% · 84k/200k`

#### Scenario: Before the first response the line says there is no reading
- **WHEN** the session starts and `$.session.usage()` reports a 200000-token window with no tokens
- **THEN** the status line is `ctx ░░░░░░░░░░ –/200k`

#### Scenario: A main-loop request moves the line within a turn
- **WHEN** a main-loop request resolves with 2000 input, 40000 cache-read and 8000 cache-write tokens and the window is 200000
- **THEN** the status line is `ctx ███░░░░░░░ 25% · 50k/200k`

#### Scenario: A measurement that moved only the cost leaves the line alone
- **WHEN** a measurement whose `changed` is only `cost` arrives
- **THEN** the module pins no new status line

#### Scenario: A million-token window is written in M
- **WHEN** a measurement reports 250000 tokens, 25%, of a 1000000-token window
- **THEN** the status line is `ctx ███░░░░░░░ 25% · 250k/1M`

### Requirement: Crossing a threshold toasts once
The thresholds SHALL be 70% and 85%. When the main window's percent reaches a threshold higher
than the highest one already toasted, the module SHALL show one toast naming the percent,
`/compact` and `ratchet task handoff`. A reading below a toasted threshold, or with no percent
(a compaction, a `/clear`), SHALL re-arm every threshold above it.

#### Scenario: Each threshold toasts once, and a drop re-arms it
- **WHEN** measurements report 50, 72, 75, 88 and 90 percent, then one with no tokens, then 20 and 71
- **THEN** exactly three toasts are shown, the first naming `72%`, the second `88%`, the third `71%`, each naming `/compact` and `ratchet task handoff`

### Requirement: Subagents' windows are tracked from their requests
After each model request of a subagent (a `turn.step` with an `agentId` whose result carries
`usage`) the module SHALL record, for that agent, the request's tokens (the same sum as the
main loop's), its peak, its request count and the model; it SHALL keep the twenty agents most
recently updated. The status line SHALL end with ` · <n> agent(s) (max <tokens>)` — `agent` for
one, `agents` otherwise — over the agents with a reading that `$.agent.list()` reports as
`running`, and SHALL have no such tail when none is. A subagent's window is not reported by the
engine: it SHALL be taken as the session model's window as `$.session.usage()` reports it
(`context.window`), or the agent's peak when that is larger.

#### Scenario: A running subagent's requests tail the status line
- **WHEN** the main window reads 84000 tokens, 42%, of 200000, and a request of running subagent `agent-1` resolves with 2000 input, 40000 cache-read and 8000 cache-write tokens
- **THEN** the status line is `ctx ████░░░░░░ 42% · 84k/200k · 1 agent (max 50k)`

#### Scenario: A finished subagent leaves the status line
- **WHEN** the same request comes from `agent-1` but `$.agent.list()` reports it `completed`
- **THEN** the status line is `ctx ████░░░░░░ 42% · 84k/200k`

### Requirement: /ctx opens a live breakdown pane
In an opted-in session the module SHALL register the `ctx` command, which opens the pane
`context-meter`. The pane SHALL draw from `$.session.usage({ breakdown: 'summary' })`, never
`full`, and SHALL redraw when a new main reading or subagent reading is recorded. It SHALL show
a header `<percentage>% · <totalTokens> / <rawMaxTokens>` followed by the model, a bar of the
categories across the pane's width, one row per category that is not `deferred` with its tokens
and its share of `rawMaxTokens` to one decimal, `auto-compact at <threshold>` when auto-compaction
is on, and, under a `Subagents` heading, two lines per agent with a reading, most recent first.
The first is `<mark> <type>` followed by the agent's description, where `<mark>` is `●` while
`$.agent.list()` reports the agent `running` and `○` otherwise. The second is a bar of the
agent's fill followed by ` <percent>% · <tokens>/<window> · <requests> req`, with
` · peak <peak>` added when the peak is above the agent's current tokens. The bar takes the
columns the second line leaves, at most 20 cells and at least 6, `round(percent × cells / 100)`
of them `█` and the rest `░`, drawn in the theme's `success` colour below 70%, `warning` from
70% and `error` from 85%, and dim for an agent that is not running. The engine drops an agent
from `$.agent.list()` once it is done, so the type and description SHALL be captured from that
list when a reading is recorded and kept with it. With no breakdown yet the
pane SHALL say `No breakdown yet`. It SHALL draw on the terminal and the desktop.

#### Scenario: The pane draws the breakdown on every surface
- **WHEN** the pane is drawn on the terminal and on the desktop over a breakdown of 86000 of 200000 tokens (43%), with a `Messages` row of 80000, a `deferred` row, and auto-compaction at 167000
- **THEN** each drawing shows `43% · 86k / 200k`, a `Messages` row with `80k` and `40.0%`, no row for the deferred category, and `auto-compact at 167k`

#### Scenario: The pane lists subagents with their fill
- **WHEN** the pane is drawn, with a 200000-token session window, after one request of running subagent `agent-1` (type `ratchet:reader`, description `Read the big file`) resolved with 50000 tokens
- **THEN** it shows `● ratchet:reader` with `Read the big file`, and a line led by a bar in the `success` colour reading `25% · 50k/200k · 1 req`

#### Scenario: A finished subagent stays in the pane
- **WHEN** one request of running subagent `agent-1` (type `ratchet:reader`, description `Read the big file`) resolved with 50000 tokens, and `$.agent.list()` no longer names it when the pane is drawn with a 200000-token session window
- **THEN** the pane still shows `○ ratchet:reader` with `Read the big file`, and a dim bar line reading `25% · 50k/200k · 1 req`

#### Scenario: A subagent past most of its window is drawn in the error colour with its peak
- **WHEN** a running subagent's requests resolved with 180000 then 175000 tokens, and the pane is drawn with a 200000-token session window
- **THEN** its bar line is drawn in the `error` colour and reads `88% · 175k/200k · 2 req · peak 180k`

#### Scenario: The pane says so before the first response
- **WHEN** the pane is drawn and `$.session.usage()` returns no breakdown
- **THEN** it shows `No breakdown yet`

### Requirement: The meter never fails the session
A failure while reading or recording usage SHALL be swallowed: the `turn.step` result SHALL
reach the engine unchanged, and every other hook SHALL still call `next`.

#### Scenario: A failing reading never fails a model request
- **WHEN** a main-loop request resolves while `$.session.usage()` rejects
- **THEN** the step's result reaches the engine unchanged
