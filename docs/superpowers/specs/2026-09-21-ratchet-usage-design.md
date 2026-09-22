# ratchet usage — token cost per task, role and model

Date: 2026-09-21. Owner: Eduardo Illanes. Status: design, awaiting owner review. Extends the
plugin design (`2026-09-16-ratchet-plugin-design.md`) with a task group 7.

## 1. Purpose

Claude Code reports tokens per session and per model. A session mixes several tasks, the
orchestrator's own turns and every subagent it dispatched, so the only cost signal an owner
gets today is the big and obvious one (an expensive model somewhere). Decisions such as
"reviewer on Sonnet or Opus", "does the repo map cut orientation cost", "how much does one
review round cost" need numbers per task and per role, compared before and after a change.

`ratchet usage` reads the transcripts Claude Code already writes to disk, joins them with the
sessions and task events ratchet already records, and answers those questions. Read-only, no
network, nothing sent anywhere.

Not in scope: live dashboards, OpenTelemetry, prices (ratchet has no price list; the owner may
configure weights), anything that changes how Claude Code records usage.

## 2. Facts this design rests on (verified 2026-09-21 on this machine's transcripts)

- A session transcript is `~/.claude/projects/<slug>/<session id>.jsonl`, where `<slug>` is
  the session's working directory with every `/` replaced by `-`. Sessions started inside a
  worktree therefore land under a different slug than the main checkout; ratchet knows each
  session's `cwd`, so it derives the slug per session instead of assuming one per repo.
- Each line is a JSON record. Records with `"type": "assistant"` carry `message.model`,
  `message.usage` with `input_tokens`, `cache_creation_input_tokens`,
  `cache_read_input_tokens`, `output_tokens` and optionally
  `output_tokens_details.thinking_tokens`; plus `timestamp` (RFC 3339, UTC), `sessionId`,
  `cwd`, `gitBranch`, `version`. `message.content` holds `tool_use` blocks with a `name`
  (`Agent`, `Edit`, `Bash`, …).
- Subagents dispatched by a session live in `<slug>/<session id>/subagents/agent-<id>.jsonl`
  with a sibling `agent-<id>.meta.json` holding `agentType` (e.g. `ratchet:reviewer`,
  `general-purpose`), `description`, `model` and `toolUseId`.
- ratchet's session ids are Claude Code's session ids. `events` has `task.claimed` and
  `task.status` (with the new status in the payload) per session with timestamps; `sessions`
  has `cwd`, `started_at`, `ended_at`.
- The format is internal to Claude Code and undocumented. Everything above is treated as
  observed, not promised.

## 3. Decisions

- **D-usage-transcripts.** The source is the transcripts, not telemetry. Rationale: already on
  disk, retroactive, per call, with model and subagent identity; no collector to run.
- **D-usage-tolerant.** The reader never fails on a record it does not understand: unknown
  record types are skipped, missing usage fields count as zero and the record is counted as
  `partial`, unparseable lines are counted as `skipped`. Every report ends with those two
  counts when non-zero, and with the highest `version` seen, so a format change shows up as a
  visible number rather than as silent zeros.
- **D-usage-join.** A call is attributed to the task its session held at the call's timestamp:
  the task is held from `task.claimed` until a `task.status` event leaves `in_progress`
  (`review`, `done`, `blocked`, `ready`) or the session ends. A session holding several tasks
  at once attributes to the most recently claimed. Calls outside any held task go to
  `unassigned` for that session.
- **D-usage-subagents.** A subagent's calls are attributed to the task its parent session held
  when the subagent was dispatched (the parent's `Agent` `tool_use` whose id matches the
  meta's `toolUseId`; failing that, the subagent's first record timestamp). Its role is the
  meta's `agentType`, with `general-purpose` shown together with the first 40 characters of
  its `description`. The parent's own calls have role `orchestrator`.
- **D-usage-orientation.** Per session, "orientation" is the orchestrator's tokens from the
  session's first record until the earliest of: its first `task.claimed`, its first `Agent`
  dispatch, its first `Edit`/`Write`/`NotebookEdit`. It is the cost the repo map (group 6) is
  meant to cut, so it is a first-class number.
- **D-usage-rounds.** Review rounds of a task = number of `task.status` events to `review`.
  One round means no loop. Tokens per round = the task's tokens between consecutive entries
  into `review`.
- **D-usage-weights.** Optional `[usage.weights."<model prefix>"]` tables in
  `~/.ratchet/config.toml` with `input`, `cache_write`, `cache_read`, `output`, all "per
  million tokens" in whatever unit the owner likes. A model matches the longest prefix
  configured. With weights, reports add a `cost` column; without, they show tokens only and
  never invent a price.
- **D-usage-no-cache.** Transcripts are parsed on every run. A few megabytes parse in well
  under a second; an index is an open point until a real repo makes it slow.
- **D-usage-read-only.** The command writes nothing except through `--note`, which appends a
  task note like any other note, and the long-output file rule of `output.rs`.

## 4. Commands

- `ratchet usage` — tasks of this repo touched in the last 7 days, one line each: id, title,
  status, rounds, tokens by class, cost when weighted.
- `ratchet usage T-0012` — one task: a line per role × model, then orientation, rounds and
  tokens per round, orchestrator share, cache efficiency, sessions involved.
- `ratchet usage --by task|role|model|session [--since 7d|30d|2026-09-01] [--all-repos]` —
  aggregates. `--by role` adds "review rounds per task" and "orientation per session".
- `ratchet usage --json` — the same data as JSON for scripts.
- `ratchet usage T-0012 --note` — appends the one-task summary as a note on the task
  (`usage: …`), so the board keeps the cost next to the handoff.
- `RATCHET_CLAUDE_PROJECTS` overrides `~/.claude/projects` (tests, unusual layouts).

Columns, in order: `in` (input), `cache w`, `cache r`, `out` (with thinking inside, and
`(think n)` after it when the field exists). Numbers use `k` and `M` with one decimal.

Definitions shown by `--by role` and per task:

- **orchestrator share** = orchestrator tokens ÷ all tokens of the task (all classes).
- **cache efficiency** = `cache r` ÷ (`in` + `cache w` + `cache r`).
- **orientation** = D-usage-orientation, per session, averaged per task.

## 5. Components

- `usage::transcript` (pure): parse one JSONL file into `Call { ts, model, usage, tools:
  Vec<tool name>, tool_use_ids }`, plus `read_meta`. Takes bytes, returns calls and the
  skipped/partial counts. Slug derivation from a `cwd`.
- `usage::attribute` (pure): given calls, subagent groups, the session rows and the task
  events of the repo, produce `Bucket { task, session, role, model } → Totals` and the derived
  metrics of §4. Windows come from events only; no wall-clock reads.
- `usage::weights`: config parsing and prefix matching.
- `cli/usage_cmd.rs`: thin face; resolves the repo, loads sessions and events through the
  existing services (read-only), locates transcripts, prints through `output`.

Git is not consulted. The database is read through the existing read paths; no new tables.

## 6. Error handling

- No `~/.claude/projects` (or override) directory: one stderr line naming the path looked at,
  exit 1.
- A session in the database with no transcript on disk: reported as a `no transcript` row,
  never an error (transcripts can be deleted).
- A transcript with zero understood records: counted in `skipped`, reported once.
- `--note` on a task that does not exist: the same refusal `ratchet task note` gives.
- Long output follows `output.rs`: over 60 lines goes to a file, the terminal gets the head.

## 7. Testing

- **Scenario tests** in `crates/ratchet/tests/spec/usage.rs` from a new
  `openspec/specs/usage/spec.md`, driving the binary with `RATCHET_HOME` and
  `RATCHET_CLAUDE_PROJECTS` pointing at temp dirs holding synthesized transcripts (a fixture
  builder writes records with exactly the fields of §2; no real transcript is copied into the
  repo). Scenarios: one task, one session, orchestrator only; a subagent attributed through
  `toolUseId`; a subagent attributed by first timestamp when the id is missing; two tasks held
  in sequence; calls before the first claim counted as orientation; two review rounds and
  tokens per round; weights present and absent; a transcript with garbage lines and a record
  without usage (counts reported); `--json` shape; `--note` appends; a session without a
  transcript; a worktree session found under its own slug.
- **Unit tests** for slug derivation, prefix matching, window edges (a call at exactly the
  claim timestamp belongs to the task).
- Gate unchanged.

## 8. Delivery plan (group 7)

Same roles as the other groups; one review at the end.

1. `usage::transcript` and `usage::weights`, with the fixture builder in test support.
2. `usage::attribute` and the derived metrics.
3. `ratchet usage` faces (`--by`, `--since`, `--json`, `--all-repos`) and output.
4. `--note`, README section, one line in the agent doctrine ("cost is measured per task, not
   per session"), and the first real report on this repo pasted into the group's handoff.

## 9. Open points

- An index over parsed transcripts, if a repo with hundreds of sessions makes runs slow.
- Recording a `usage:` note automatically when a task reaches `done`; left explicit
  (`--note`) until the numbers have been trusted for a while.
- Attributing tokens of a session that never claims tasks (pure conversation) beyond the
  `unassigned` bucket.
