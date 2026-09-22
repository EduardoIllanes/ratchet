# usage

Token cost per task, role and model, read from the transcripts Claude Code already writes to
disk and joined with the sessions and task events ratchet already records. Read-only, no
network, no price list.

## Purpose

Claude Code reports tokens per session and per model; an owner deciding "reviewer on Sonnet or
Opus", "does the repo map cut orientation cost" or "how much does one review round cost" needs
numbers per task and per role. `ratchet usage` answers those questions from what is already on
disk, and writes nothing except an explicit `--note`.

## Requirements

### Requirement: Transcripts are located per session from its working directory
For each session of the repo, ratchet SHALL look for `<projects>/<slug>/<session id>.jsonl`,
where `<projects>` is `~/.claude/projects` unless `RATCHET_CLAUDE_PROJECTS` is set, and
`<slug>` is the session's recorded `cwd` with every path separator replaced by `-`. Subagent
transcripts of that session SHALL be read from `<projects>/<slug>/<session id>/subagents/`.
A session whose transcript is absent SHALL appear as one `no transcript` row and SHALL NOT
fail the report. A missing `<projects>` directory SHALL fail with one stderr line naming the
path looked at and exit 1.

#### Scenario: A worktree session is found under its own slug
- **WHEN** a session of the repo was started in `<repo>/.worktrees/t-0001` and its transcript sits under the slug of that directory, not the repo's
- **THEN** `ratchet usage` includes its tokens and no `no transcript` row is printed for it

#### Scenario: A session without a transcript is reported, not an error
- **WHEN** a session of the repo has no transcript under its slug
- **THEN** `ratchet usage` exits 0 and prints one row containing `no transcript` and the session id

#### Scenario: Missing projects directory fails naming the path
- **WHEN** `RATCHET_CLAUDE_PROJECTS` points at a directory that does not exist
- **THEN** `ratchet usage` exits 1 and stderr has one line containing that path

### Requirement: Reading is tolerant and reports what it did not understand
The reader SHALL skip records whose `type` is not `assistant`, SHALL count a line that is not
valid JSON as `skipped`, and SHALL count an `assistant` record without `message.usage` as
`partial` with all its token classes at zero. When either count is non-zero the report SHALL
end with one line naming both counts and the highest `version` seen. A transcript with zero
understood records SHALL add one to `skipped` and be reported once, never as an error.

#### Scenario: Garbage lines and a record without usage are counted
- **WHEN** a transcript holds two well-formed assistant records, one line of garbage and one assistant record without `message.usage`
- **THEN** the report totals the two records, and its last line names `skipped 1`, `partial 1` and the highest `version` seen

#### Scenario: A transcript with nothing understood is skipped once
- **WHEN** a session's transcript holds only `user` records
- **THEN** the report exits 0 and counts that transcript once in `skipped`

### Requirement: A call belongs to the task its session held at that instant
A session holds a task from the timestamp of its `task.claimed` event until the first
`task.status` event that leaves `in_progress` (`review`, `done`, `blocked`, `ready`) or the
session's `ended_at`, whichever comes first. A call at exactly the claim timestamp belongs to
the task. When a session holds several tasks at once the most recently claimed wins. Calls
outside any held task go to an `unassigned` bucket for that session.

#### Scenario: One task, one session, orchestrator only
- **WHEN** a session claims `T-0001`, makes three assistant calls, then moves it to `review`
- **THEN** `ratchet usage T-0001` shows one `orchestrator` row per model with the sum of the three calls' `input`, `cache w`, `cache r` and `out`

#### Scenario: Two tasks held in sequence split the session
- **WHEN** a session claims `T-0001`, calls twice, moves it to `done`, claims `T-0002` and calls once
- **THEN** `T-0001` totals the first two calls and `T-0002` the third, and nothing is double counted

#### Scenario: Calls outside any held task are unassigned
- **WHEN** a session makes two calls before its first claim and one after releasing its last task
- **THEN** `ratchet usage --by session` shows those three calls under `unassigned` for that session

### Requirement: Subagent calls are attributed through ratchet's own events first
A subagent transcript `agent-<id>.jsonl` SHALL be attributed, in order: to the task named by
the `subagent.start` (or `subagent.stop`) event whose `agent_id` is `<id>`; failing that, to
the task the parent held when the parent's `Agent` `tool_use` whose id equals the meta file's
`toolUseId` was issued; failing that, to the task the parent held at the subagent's first
record timestamp. Its role SHALL be the event's `agent_type`, else the meta's `agentType`,
else `subagent`; a `general-purpose` role SHALL be shown with the first 40 characters of its
`description`. The parent's own calls have role `orchestrator`. Tokens SHALL always come from
the transcript, never from events.

#### Scenario: A subagent is attributed through its start event
- **WHEN** the parent session holds `T-0001` and a `subagent.start` event names `agent_id` `a1` with `agent_type` `ratchet:reviewer`, and `agent-a1.jsonl` exists with no meta file
- **THEN** `ratchet usage T-0001` shows a `ratchet:reviewer` row with the subagent's tokens

#### Scenario: A subagent is attributed through toolUseId when no event exists
- **WHEN** no subagent event exists, the parent's transcript has an `Agent` `tool_use` with id `tu-9` issued while `T-0001` was held, and `agent-a2.meta.json` has `toolUseId` `tu-9` and `agentType` `ratchet:implementer`
- **THEN** the subagent's tokens appear under `T-0001` with role `ratchet:implementer`

#### Scenario: A subagent is attributed by first timestamp when both are missing
- **WHEN** no event and no meta file exist for `agent-a3.jsonl`, and its first record's timestamp falls while the parent held `T-0002`
- **THEN** the subagent's tokens appear under `T-0002` with role `subagent`

#### Scenario: A general-purpose subagent shows its description
- **WHEN** a subagent's role is `general-purpose` and its description is `find every caller of parse_repo_config in the tree and list them`
- **THEN** its row reads `general-purpose: find every caller of parse_repo_config in` and nothing more of the description

### Requirement: Orientation is the orchestrator's cost before real work starts
Per session, orientation SHALL be the orchestrator's tokens from the session's first record
until the earliest of: its first `task.claimed`, its first `Agent` `tool_use`, its first
`Edit`, `Write` or `NotebookEdit` `tool_use`. It SHALL be shown per task as the average over
the task's sessions and under `--by role` as the average per session.

#### Scenario: Calls before the first claim are orientation
- **WHEN** a session makes two calls, claims `T-0001`, then makes three more
- **THEN** `ratchet usage T-0001` shows an `orientation` line equal to the first two calls' tokens

#### Scenario: A dispatch ends orientation without a claim
- **WHEN** a session makes one call, then a call whose content has an `Agent` `tool_use`, then two more calls before its first claim
- **THEN** orientation for that session equals the first two calls' tokens

### Requirement: Review rounds are counted from status events
A task's rounds SHALL be the number of its `task.status` events whose new status is `review`.
Tokens per round SHALL be the task's tokens between consecutive entries into `review`, the
first round counting from the first claim.

#### Scenario: Two review rounds and tokens per round
- **WHEN** `T-0001` goes `review`, back to `in_progress` with two more calls, then `review` again
- **THEN** `ratchet usage T-0001` shows `rounds 2` and two per-round lines, the second equal to the two calls in between

### Requirement: Weights add a cost column, and only then
`[usage.weights."<model prefix>"]` tables in `~/.ratchet/config.toml` with `input`,
`cache_write`, `cache_read` and `output`, all per million tokens, SHALL make every report add
a `cost` column computed with the longest configured prefix that matches the model. Without a
matching weight the report SHALL show tokens only and SHALL NOT print a cost.

#### Scenario: Weights present add cost
- **WHEN** the machine config holds weights for prefix `claude-sonnet` and every call uses model `claude-sonnet-5`
- **THEN** the report has a `cost` column whose value is the weighted sum of the four token classes

#### Scenario: No weights, no cost
- **WHEN** the machine config has no `[usage.weights]` table
- **THEN** the report has no `cost` column and the word `cost` does not appear

#### Scenario: The longest matching prefix wins
- **WHEN** weights exist for `claude` and for `claude-opus`, and the model is `claude-opus-5`
- **THEN** the cost uses the `claude-opus` weights

### Requirement: Reports come in four shapes and a time window
`ratchet usage` SHALL list the repo's tasks touched in the last 7 days, one line each with id,
title, status, rounds and the four token classes. `ratchet usage <id>` SHALL show one task: a
line per role × model, then orientation, rounds, orchestrator share and cache efficiency.
`--by task|role|model|session` SHALL aggregate across the window; `--since <7d|30d|date>`
SHALL set the window; `--all-repos` SHALL drop the repo filter. Numbers SHALL use `k` and
`M` with one decimal. Output over the terminal budget follows the long-output rule.

#### Scenario: Default listing shows tasks of the last seven days
- **WHEN** the repo has a task touched 3 days ago and one touched 20 days ago
- **THEN** `ratchet usage` lists the first and not the second, with `rounds` and the four token columns

#### Scenario: Since widens the window
- **WHEN** the same repo is queried with `--since 30d`
- **THEN** both tasks are listed

#### Scenario: By role aggregates across tasks
- **WHEN** two tasks each have an `orchestrator` row and a `ratchet:reviewer` row
- **THEN** `ratchet usage --by role` shows one line per role with the sums, plus `review rounds per task` and `orientation per session`

#### Scenario: Numbers are abbreviated
- **WHEN** a bucket totals 1234 input tokens and 2500000 cache-read tokens
- **THEN** the report prints `1.2k` and `2.5M` for them

### Requirement: `--json` exposes the same data for scripts
`--json` SHALL print one JSON object with `tasks` (an array of objects with `id`, `title`,
`status`, `rounds`, `orientation`, `buckets`) and top-level `skipped`, `partial` and `version`.
Each bucket SHALL have `session`, `role`, `model` and `tokens` with `input`, `cache_write`,
`cache_read`, `output` and `thinking`, plus `cost` only when weights matched.

#### Scenario: JSON shape
- **WHEN** `ratchet usage T-0001 --json` runs for a task with one orchestrator bucket and no weights
- **THEN** stdout parses as JSON with `tasks[0].id` equal to `T-0001`, a `buckets[0].tokens.input` number, no `cost` key, and top-level `skipped` and `partial`

### Requirement: `--note` writes the one-task summary to the board and nothing else
`ratchet usage <id> --note` SHALL append a task note whose text starts with `usage:` and holds
the one-task summary on one paragraph. Without `--note` the command SHALL write nothing. On a
task that does not exist `--note` SHALL be refused exactly as `ratchet task note` refuses it.

#### Scenario: Note appends the summary
- **WHEN** `ratchet usage T-0001 --note` runs
- **THEN** `ratchet task show T-0001` lists a new note starting with `usage:` that contains the task's total tokens and rounds

#### Scenario: Note on a missing task is refused
- **WHEN** `ratchet usage T-9999 --note` runs
- **THEN** it fails with the same message `ratchet task note T-9999` gives, and no note is written anywhere

#### Scenario: Without note nothing is written
- **WHEN** `ratchet usage T-0001` runs
- **THEN** the events table has no new row and no file changed under the state directory
