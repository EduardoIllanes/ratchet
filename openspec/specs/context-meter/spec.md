# context-meter

How full the context windows of a Claude Code session are, shown while the session works: the
main window on the status line with its last move, a toast when it crosses a threshold, every
subagent's window beside it, and a pane that breaks the main window down by category, draws its
trend toward auto-compaction, names the heaviest tool results and what the held task has cost.

## Purpose

The doctrine keeps the orchestrator's context useful by delegating; nothing showed whether it
was working. The meter is a function-hook module (Claude Code's early-access plugin API) that
ships inside the plugin beside the command hooks. It reads what the engine reports to hooks —
the status line's figures, each model request's token counts, the subagent list, the session's
messages — and the board through two read-only ratchet commands; it writes nothing to disk. It
runs only in repos that opted in to ratchet.

## Requirements

### Requirement: The module ships inside the plugin and validates
`hooks/hooks.json` SHALL keep its command hooks and SHALL also name the module under
`"modules": ["./context-meter/register.tsx"]`; `.claude-plugin/plugin.json` SHALL name the
module's state contract under `"types"` and SHALL still carry no `version`. The module's state
SHALL live under `PluginState['ratchet']`. `claude plugin validate .claude-plugin/plugin.json`
SHALL pass (on the repository root it validates only `marketplace.json`). Every scenario of this spec but this requirement's SHALL be a test of the module under
`claude plugin test`, named exactly as the scenario, and its Rust spec function SHALL assert
that `claude plugin test` on the repository root printed `(pass) <scenario title>`. With no
`claude` on the path those Rust functions SHALL fail naming
`npm install -g @anthropic-ai/claude-code@2.1.288`, the version the module is tested against,
which CI SHALL install before `cargo test` on every platform.

#### Scenario: The plugin validates with the module declared
- **WHEN** `claude plugin validate` runs on the repository's `.claude-plugin/plugin.json`
- **THEN** it exits 0, and its output names the module's hooks `session.start`,
  `session.measure`, `session.end`, `turn.step` and `ui.render` for the `Pane` component

### Requirement: Only a repo that opted in is metered
On `session.start` the module SHALL walk the session's `cwd` and each of its ancestors, nearest
first, splitting on both `/` and `\`, and SHALL treat the session as opted in when one of them
holds `ratchet.toml` as a regular file (`$.fs.stat` resolving with `kind: 'file'`; a stat that
rejects counts as absent) — the rule `repo::find_repo` applies. A session that did not opt in
SHALL get no status line, no toast, no `/ctx` command, no pane and no ratchet command run from
the module, and every hook SHALL pass its event on unchanged.

#### Scenario: Without ratchet.toml the meter stays silent
- **WHEN** the session starts in `/work/plain/src` with no `ratchet.toml` in it or any ancestor, and a measurement then reports 90% of a 200000-token window
- **THEN** the module pins no status line, shows no toast, registers no command, and runs no process

#### Scenario: A ratchet.toml in an ancestor opts the session in
- **WHEN** the session starts in `/work/repo/crates/core` and only `/work/repo/ratchet.toml` exists, as a file
- **THEN** the module registers the `ctx` command and pins a status line

#### Scenario: A directory named ratchet.toml does not opt in
- **WHEN** the session starts in `/work/repo` and `/work/repo/ratchet.toml` is a directory
- **THEN** the module registers no command and pins no status line

### Requirement: The status line shows the main window's fill and its last move
In an opted-in session the module SHALL pin one status line,
`ctx <bar> <percent>% · <tokens>/<window>`, where `<bar>` is ten cells, `round(percent / 10)`
of them `█` and the rest `░`, clamped to ten. A token count `n` is written as the integer below
1000; as `round(n / 1000)` followed by `k` while that is below 1000; and otherwise as
`n / 1000000` to one decimal followed by `M`, the decimal dropped when it is `0` — so 999600
and 1040000 are both `1M`. Before any response of the live window (`tokens` absent) the line
SHALL be `ctx ░░░░░░░░░░ –/<window>`. It SHALL be refreshed on each `session.measure` whose
`changed` names `context`, from that measurement's `context`, and after each model request of
the main loop (a `turn.step` with no `agentId` whose result carries `usage`), with
`tokens = input_tokens + cache_read_input_tokens + cache_creation_input_tokens`, the window from
`$.session.usage()`, and `percent = round(tokens / window × 100)`.

The module SHALL keep a history of the main window: the tokens of each `session.measure` whose
`changed` names `context` and that reports tokens, the sixty most recent. When the last two
entries differ, the line SHALL be followed by their difference, ` +<tokens>` for growth and
` -<tokens>` for a drop.

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

#### Scenario: Counts that round to a thousand k are written in M
- **WHEN** a measurement reports 999600 tokens, 50%, of a 2000000-token window, and another then reports 1040000 tokens, 52%
- **THEN** the first status line is `ctx █████░░░░░ 50% · 1M/2M` and the second `ctx █████░░░░░ 52% · 1M/2M +40k`

#### Scenario: Each turn's move follows the fill
- **WHEN** measurements report 84000 tokens (42%), then 96000 (48%), then 66000 (33%), of a 200000-token window
- **THEN** the status line after the second is `ctx █████░░░░░ 48% · 96k/200k +12k` and after the third `ctx ███░░░░░░░ 33% · 66k/200k -30k`

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
recently updated, the one just updated always among them. A subagent's window is not reported
by the engine: it SHALL be taken as the session model's window as `$.session.usage()` reports
it (`context.window`), or the agent's peak when that is larger.

The status line SHALL end with ` │ ` followed by one entry per running subagent — an agent with
a reading that `$.agent.list()` reports as `running` — joined by ` · `: its type with the text
up to and including the first `:` dropped, a five-cell bar (`round(percent × 5 / 100)` of them
`█`, the rest `░`) and `<percent>%`, where `percent = round(tokens / window × 100)`. At most the
three with the most tokens SHALL be shown, fullest first, followed by ` +<n>` for the rest.
With no running subagent there SHALL be no tail.

#### Scenario: A running subagent's requests tail the status line
- **WHEN** the main window reads 84000 tokens, 42%, of 200000, and a request of running subagent `agent-1` (type `ratchet:implementer`) resolves with 2000 input, 40000 cache-read and 8000 cache-write tokens
- **THEN** the status line is `ctx ████░░░░░░ 42% · 84k/200k │ implementer █░░░░ 25%`

#### Scenario: Past three running subagents the tail counts the rest
- **WHEN** the main window reads 84000 tokens, 42%, of 200000, and running subagents `agent-1` (type `ratchet:reader`), `agent-2` (`Explore`), `agent-3` (`ratchet:implementer`) and `agent-4` (`general-purpose`) last resolved with 20000, 60000, 160000 and 100000 tokens
- **THEN** the status line is `ctx ████░░░░░░ 42% · 84k/200k │ implementer ████░ 80% · general-purpose ███░░ 50% · Explore ██░░░ 30% +1`

#### Scenario: A finished subagent leaves the status line
- **WHEN** the same request comes from `agent-1` but `$.agent.list()` reports it `completed`
- **THEN** the status line is `ctx ████░░░░░░ 42% · 84k/200k`

#### Scenario: The agent just updated is kept when twenty share its time
- **WHEN** twenty-one subagents `agent-1` to `agent-21` each resolve one request at the same clock time, `agent-21` last, and the pane is drawn
- **THEN** it lists twenty agents, `agent-21` among them

#### Scenario: /clear forgets the subagents and the trend
- **WHEN** a subagent's request was recorded and two measurements of 40000 and 50000 tokens were taken, then the session ends with reason `clear`, and the pane is drawn
- **THEN** it shows no `Subagents` heading and no `Trend` heading

### Requirement: /ctx opens a live breakdown pane
In an opted-in session the module SHALL register the `ctx` command, which opens the pane
`context-meter`. The pane SHALL draw from `$.session.usage({ breakdown: 'summary' })`, never
`full`, and SHALL redraw when a main reading, a subagent reading, the history or the held task's
figures change. It SHALL draw on the terminal and the desktop, in the Celestial palette:

- a fill colour per percent, `#29D398` below 70%, `#FAB795` from 70% and `#E95678` from 85%;
- the series `#26BBD9`, `#B877DB`, `#F09483`, `#EE64AC`, `#59E1E3`, `#FAB795`, `#29D398`, taken
  in order and repeated;
- `#B877DB` for headings and `#6C6F93` for what is not used.

It SHALL show a header `<percentage>% · <totalTokens> / <rawMaxTokens>`, the percentage in its
fill colour, followed by the model; a bar of the categories across the pane's width; one row per
category that is not `deferred` with a mark, its tokens and its share of `rawMaxTokens` to one
decimal, each `used` category's bar cells and mark in the series colours taken from the first and the `buffer`
and `free` ones in `#6C6F93`; and `auto-compact at <threshold>` when auto-compaction is on.
With no breakdown yet the pane SHALL say `No breakdown yet`.

Under a bold `Subagents` heading in `#B877DB` it SHALL draw two lines per agent with a reading,
most recent first. The first is `<mark> <type>` followed by the agent's description, where
`<mark>` is `●` while `$.agent.list()` reports the agent `running` and `○` otherwise. The second
is a bar of the agent's fill followed by ` <percent>% · <tokens>/<window> · <requests> req`,
with ` · peak <peak>` added when the peak is above the agent's current tokens. The bar takes the
columns the second line leaves, at most 20 cells and at least 6, `round(percent × cells / 100)`
of them `█` and the rest `░`, drawn in its fill colour, and dim for an agent that is not
running. The engine drops an agent from `$.agent.list()` once it is done, so the type and
description SHALL be captured from that list when a reading is recorded and kept with it.

#### Scenario: The pane draws the breakdown on every surface
- **WHEN** the pane is drawn on the terminal and on the desktop over a breakdown of 86000 of 200000 tokens (43%), with a `Messages` row of 80000, a `deferred` row, and auto-compaction at 167000
- **THEN** each drawing shows `43% · 86k / 200k`, a `Messages` row with `80k` and `40.0%`, no row for the deferred category, and `auto-compact at 167k`

#### Scenario: The pane colours the breakdown in the Celestial palette
- **WHEN** the pane is drawn over a breakdown of 150000 of 200000 tokens (75%) with `used` rows `System prompt` then `Messages`, a `buffer` row and a `free` row
- **THEN** the header's `75%` is drawn in `#FAB795`, the `System prompt` row's mark in `#26BBD9`, the `Messages` row's in `#B877DB`, and the buffer and free rows' marks in `#6C6F93`

#### Scenario: The pane lists subagents with their fill
- **WHEN** the pane is drawn, with a 200000-token session window, after one request of running subagent `agent-1` (type `ratchet:reader`, description `Read the big file`) resolved with 50000 tokens
- **THEN** it shows `● ratchet:reader` with `Read the big file`, and a line led by a bar in `#29D398` reading `25% · 50k/200k · 1 req`

#### Scenario: A finished subagent stays in the pane
- **WHEN** one request of running subagent `agent-1` (type `ratchet:reader`, description `Read the big file`) resolved with 50000 tokens, and `$.agent.list()` no longer names it when the pane is drawn with a 200000-token session window
- **THEN** the pane still shows `○ ratchet:reader` with `Read the big file`, and a dim bar line reading `25% · 50k/200k · 1 req`

#### Scenario: A subagent past most of its window is drawn in the hot colour with its peak
- **WHEN** a running subagent's requests resolved with 180000 then 175000 tokens, and the pane is drawn with a 200000-token session window
- **THEN** its bar line is drawn in `#E95678` and reads `88% · 175k/200k · 2 req · peak 180k`

#### Scenario: The pane says so before the first response
- **WHEN** the pane is drawn and `$.session.usage()` returns no breakdown
- **THEN** it shows `No breakdown yet`

### Requirement: The pane draws the main window's trend toward auto-compaction
With two or more entries in the history, the pane SHALL draw, under a `Trend` heading, a
sparkline of as many of the most recent entries as fit on its line beside the move — one block per entry from
`▁▂▃▄▅▆▇█`, the one at index `min(7, floor(tokens / window × 8))` — in the fill colour of the
breakdown's percentage, followed by the status line's move (` +<tokens>` or ` -<tokens>`, none
when the last two entries are equal). When auto-compaction is on and its threshold is known, it
SHALL add a forecast line. The pace is the mean of the differences between consecutive entries,
newest first, at most five, stopping at the first that is negative (a compaction or a `/clear`
shrank the window there); the forecast is `at the auto-compact threshold` when the last entry
is at or past the threshold, `not growing` when the pace is not above zero, and otherwise
`~<n> turns to auto-compact at this pace` (`turn` for one) with
`n = ceil((threshold − last) / pace)`.

#### Scenario: The pane draws the trend and the turns left
- **WHEN** measurements report 40000, 50000, 60000 and 70000 tokens of a 200000-token window, and the pane is drawn over a breakdown with auto-compaction at 167000
- **THEN** it shows a `Trend` heading, the sparkline `▂▃▃▃` followed by `+10k`, and `~10 turns to auto-compact at this pace`

#### Scenario: A shrunk window restarts the pace
- **WHEN** measurements report 100000, 150000, 60000 and 70000 tokens of a 200000-token window, and the pane is drawn over a breakdown with auto-compaction at 167000
- **THEN** it shows `~10 turns to auto-compact at this pace`

#### Scenario: A flat window is not growing
- **WHEN** measurements report 60000 and 60000 tokens of a 200000-token window, and the pane is drawn over a breakdown with auto-compaction at 167000
- **THEN** it shows a `Trend` heading and `not growing`

### Requirement: The pane names the heaviest tool results
The pane SHALL read `$.session.messages({ as: 'api' })` — the messages the next request is built
from, so a result a compaction replaced is not among them — pair each `tool_use` block of an
assistant message with the `tool_result` block of a user message whose `tool_use_id` is its
`id`, and take as the result's text its `content` when that is a string, else the `text` of its
`text` blocks joined. Under a `Top consumers` heading it SHALL list the five results whose text
is longest, largest first: the `tool_use`'s `name`, an estimate of `ceil(length / 4)` tokens
written as the status line writes tokens, and a label from the `tool_use`'s `input`. The label is
the last three segments, split on `/` or `\`, of the input's `file_path`, `notebook_path` or
`path`; else the first line of its `command`, cut to 47 characters followed by `…` when longer
than 48; else its `pattern`, `url`, `description`, `subagent_type` or `query`, the first that is
a string. When a `Read` among the five is estimated at 5000 tokens or more, the pane SHALL add
the line `→ reads this size can go to ratchet:reader`. With no answered tool use in those
messages there SHALL be no such section.

#### Scenario: The pane names the heaviest tool results
- **WHEN** the session's messages hold a `Read` of `/repo/crates/ratchet/src/usage/transcript.rs` answered with 40000 characters, a `Bash` of `cargo test --workspace` answered with 8000, and a `Grep` for `fn parse` answered with 400, and the pane is drawn
- **THEN** it shows `Top consumers`, then `Read` with `10k` and `src/usage/transcript.rs`, then `Bash` with `2k` and `cargo test --workspace`, then `Grep` with `100` and `fn parse`, and `→ reads this size can go to ratchet:reader`

#### Scenario: Only the five heaviest tool results are listed
- **WHEN** the session's messages hold six answered `Grep` uses for patterns `p1` to `p6` answered with 600, 500, 400, 300, 200 and 100 characters, and the pane is drawn
- **THEN** it lists `p1` to `p5` and not `p6`

#### Scenario: A read under 5k tokens brings no reader hint
- **WHEN** the session's messages hold one `Read` of `/work/repo/README.md` answered with 4000 characters, and the pane is drawn
- **THEN** it lists `Read` with `1k` and `work/repo/README.md`, and no line naming `ratchet:reader`

### Requirement: The pane shows what the held task has cost so far
In an opted-in session the module SHALL read the held task's tokens on `session.start`, every
60 seconds after it, and on `/ctx`, without any hook waiting for it: it SHALL run the plugin's
own binary — `<$.plugin.root>/bin/ratchet.exe` when `$.fs.stat` resolves it with `kind: 'file'`,
else `<$.plugin.root>/bin/ratchet` — as `task list --mine --json --session <$.session.id()>`,
take the first task whose `status` is `in_progress`, and run `usage <id> --json`. Both commands
print through ratchet's output discipline, which caps stdout even when it is piped: past 60
lines it prints the first 20 and then the line `… (<n> lines in <path>)`. When the last line of
stdout has that form the module SHALL read the whole output from `<path>` with `$.fs.read` and
parse that instead. The task's
figures are its `buckets` summed by `role` over `input`, `cache_write`, `cache_read` and
`output` (thinking is inside output), roles ordered by tokens, largest first; they are tokens,
not dollars. With no held task the figures SHALL be cleared; a run that rejects, exits non-zero,
prints what does not parse or reports nothing for the task SHALL leave the last figures as they
were.

When figures with more than zero tokens are held, the pane SHALL draw, in `#B877DB`, the task's
id followed by `· <total> tokens so far`; a bar across the pane's width split among the roles in
proportion, each role's cells in the series colours taken from the first; and a legend of the four largest roles,
`<role> <percent>%` joined by ` · `, each in its role's colour, the role with the text up to and
including the first `:` dropped and `percent = round(tokens / total × 100)`.

#### Scenario: The pane shows what the held task has cost by role
- **WHEN** the session holds `T-0099`, and `usage T-0099 --json` reports buckets `implementer` of 60000 tokens, `orchestrator` of 30000 and `ratchet:reviewer` of 10000, and `/ctx` opens the pane
- **THEN** both commands were run with `<plugin root>/bin/ratchet`, and the pane shows `T-0099` with `100k tokens so far` and the legend `implementer 60% · orchestrator 30% · reviewer 10%`

#### Scenario: On Windows the binary is ratchet.exe
- **WHEN** `<plugin root>/bin/ratchet.exe` exists as a file and `/ctx` runs
- **THEN** every command the module runs starts with `<plugin root>/bin/ratchet.exe`

#### Scenario: Without a held task the pane shows no task cost
- **WHEN** `task list --mine --json` lists only tasks whose status is not `in_progress`, and `/ctx` opens the pane
- **THEN** the module does not run `usage`, and the pane shows no `tokens so far` line

#### Scenario: A long report is read from the file its last line names
- **WHEN** the session holds `T-0099`, and `usage T-0099 --json` prints the first 20 lines of a 70-line report followed by `… (70 lines in /home/u/.ratchet/out/20261004T143413-usage-T-0099.txt)`, that file holding the whole report with buckets `implementer` of 60000 tokens, `orchestrator` of 30000 and `ratchet:reviewer` of 10000, and `/ctx` opens the pane
- **THEN** the pane shows `T-0099` with `100k tokens so far` and the legend `implementer 60% · orchestrator 30% · reviewer 10%`

#### Scenario: A failed refresh keeps the last figures
- **WHEN** the figures of `T-0099` were read once with 100000 tokens, and on the next `/ctx` `usage T-0099 --json` exits 1
- **THEN** the pane still shows `T-0099` with `100k tokens so far`

### Requirement: The meter never fails the session
A failure while reading or recording usage SHALL be swallowed: the `turn.step` result SHALL
reach the engine unchanged, and every other hook SHALL still call `next` — `session.start`
included when `$.command.register` or a state update rejects. When `$.agent.list()` rejects, the
status line SHALL be pinned without its subagent tail, a subagent's request SHALL still be
recorded (its type and description left as they were, `agent` and empty for a new one), and the
pane SHALL draw every agent as not running. When `$.session.usage()` rejects while the pane
draws, or anything else fails while it draws, the pane SHALL say `No breakdown yet`; when
`$.session.messages()` rejects, the pane SHALL draw the rest without top consumers. A failure
recording the history SHALL NOT keep a measurement from refreshing the status line.

#### Scenario: A failing reading never fails a model request
- **WHEN** a main-loop request resolves while `$.session.usage()` rejects
- **THEN** the step's result reaches the engine unchanged

#### Scenario: A failing agent list leaves the line without its tail
- **WHEN** `$.agent.list()` rejects and a measurement reports 84000 tokens, 42%, of a 200000-token window
- **THEN** the status line is `ctx ████░░░░░░ 42% · 84k/200k`

#### Scenario: A failing command registration still meters the session
- **WHEN** the session starts in an opted-in repo, `$.command.register` rejects, and `$.session.usage()` reports 84000 tokens, 42%, of a 200000-token window
- **THEN** the start resolves, and the status line is `ctx ████░░░░░░ 42% · 84k/200k`

#### Scenario: A failing read still draws the pane
- **WHEN** the pane is drawn while `$.session.usage()` rejects
- **THEN** it shows `No breakdown yet`

#### Scenario: A failing agent list still records a subagent's request
- **WHEN** `$.agent.list()` rejects while a request of subagent `agent-1` resolves with 50000 tokens, and the pane is then drawn with a 200000-token session window
- **THEN** it shows `○ agent` and a dim bar line reading `25% · 50k/200k · 1 req`

#### Scenario: A failing messages read draws the pane without top consumers
- **WHEN** `$.session.messages()` rejects and the pane is drawn over a breakdown of 86000 of 200000 tokens (43%)
- **THEN** it shows `43% · 86k / 200k` and no `Top consumers` heading
