# agent-protocol

How a Claude Code session interacts with ratchet through hooks: repo opt-in, guardrails, the
never-break rule, the briefing at start, the reminder on every prompt, the handoff rule when the
session closes, and the shape of what the CLI writes back.

## Purpose

Rules that live in the harness are rules the agent cannot skip from the prompt. Each block
carries the alternative the agent should use instead, so it corrects itself in one attempt.

## Requirements

### Requirement: Repo opt-in by marker
A repo SHALL opt in to ratchet by having a `ratchet.toml` at its root. Every hook SHALL
resolve the repo from the working directory of the tool call by walking up to the nearest
marker; when the working directory is a linked git worktree, the repo root SHALL be the main
checkout that owns it. Without a marker, every hook SHALL exit 0 immediately without reading
any other file. An unreadable or invalid marker SHALL be logged and treated as "no marker".

#### Scenario: No marker, hooks do nothing
- **WHEN** the pre-tool hook receives `python scripts/x.py` from a directory with no `ratchet.toml` above it
- **THEN** the hook exits 0 and prints nothing

#### Scenario: Marker found from a subdirectory
- **WHEN** the working directory is `src/deep` inside a repo that has `.venv` and `ratchet.toml` at its root and the tool call is `python scripts/x.py`
- **THEN** the hook blocks with the python-venv rule

#### Scenario: Invalid marker is logged and ignored
- **WHEN** `ratchet.toml` is not valid TOML and the tool call is `python scripts/x.py`
- **THEN** the hook exits 0 and the log file contains one line naming `ratchet.toml`

### Requirement: Marker can be generated
`ratchet config init` SHALL write a commented `ratchet.toml` at the root of the git repository
containing the current directory, and SHALL refuse to overwrite an existing marker unless
`--force` is given. The generated file SHALL be valid for the hooks as written.

#### Scenario: Init writes a marker at the repo root
- **WHEN** `ratchet config init` runs from a subdirectory of a git repo with no `ratchet.toml`
- **THEN** it exits 0, prints `wrote <root>/ratchet.toml`, and the file parses with `default_branch = "main"`

#### Scenario: Init refuses to overwrite without force
- **WHEN** `ratchet config init` runs in a repo that already has a `ratchet.toml`
- **THEN** it exits 1, the file is unchanged, and stderr mentions `--force`

#### Scenario: Init outside a git repo fails
- **WHEN** `ratchet config init` runs in a directory that is not inside a git repository
- **THEN** it exits 1 and stderr says `not a git repository`

### Requirement: Guardrails before the action
In PreToolUse inside an opted-in repo, ratchet SHALL block, with a message that states the
rule and the alternative, prefixed `[ratchet guardrail:<id>]`: (a) `python`, `pip`,
`pytest`, `uvicorn`, `mypy` or `ruff` not run through the repo's virtualenv (`uv run` or the
`.venv` interpreter) when the repo has a `.venv`; (b) `git push --force`, `git reset --hard`,
`git checkout -- .`, `git clean -f` and recursive forced deletion, except a deletion whose
every target is under the session scratchpad; (c) writes to `.env` files; (d) writes to a
tracked file of the main tree of the repo, from any session, including one running in a
worktree — whether the write is an `Edit`, `Write`, `NotebookEdit` or `MultiEdit` call, or a
`Bash`/`PowerShell` command segment of a recognised writing shape whose target is such a file:
a `>` or `>>` redirection, `tee`, `sed -i`, the destination of `cp`, `mv` or `rsync`, and the
paths given to `git checkout --` or `git restore`. A command that writes through an
interpreter (`python`, `node`, `perl`, a heredoc script) is NOT recognised before the fact; it
is caught after the fact (see *Main-tree writes detected after the fact*). Command rules
SHALL evaluate each command segment separately, splitting on `;`,
`&&`, `||`, `|` and newlines only outside quotes. The same rules SHALL apply with the same
message whether the command arrived through `Bash` or `PowerShell`. Rules SHALL be
extensible from a machine-wide file and from a repo file with the same schema; a rule with
the id of an existing one SHALL replace it; a repo SHALL be able to disable a rule by id.

#### Scenario: Python outside the venv
- **WHEN** the tool call is `python scripts/x.py` through `Bash` in a repo with `.venv`
- **THEN** the hook blocks with rule `python-venv` and the message proposes `uv run`

#### Scenario: Python through PowerShell, same message
- **WHEN** the tool call is `python -c "print(1)"` through `PowerShell` in a repo with `.venv`
- **THEN** the hook blocks with exactly the same stderr line as through `Bash`

#### Scenario: Quoted text does not split a command
- **WHEN** the tool call is `uv run ratchet task new -t x -c "done; mypy clean"` in a repo with `.venv`
- **THEN** the hook allows it

#### Scenario: Git destructive
- **WHEN** the tool call is `git reset --hard HEAD~1`
- **THEN** the hook blocks with rule `git-destructive` and the message says the owner runs it

#### Scenario: Recursive delete under the scratchpad is allowed
- **WHEN** the tool call is `rm -rf <scratchpad>/tmp` and the scratchpad directory is known from the environment
- **THEN** the hook allows it, while `rm -rf <repo>/src` is blocked

#### Scenario: Env file write blocked
- **WHEN** a `Write` targets `.env.local` inside the repo
- **THEN** the hook blocks with rule `env-files`

#### Scenario: Write to the main tree blocked
- **WHEN** an `Edit` targets a tracked file of the main tree and the working directory is the main tree
- **THEN** the hook blocks with rule `main-tree` and the message proposes a worktree

#### Scenario: Write to the main tree blocked from a worktree session
- **WHEN** an `Edit` targets a tracked file of the main tree and the working directory is a linked worktree of that repo
- **THEN** the hook blocks with rule `main-tree`

#### Scenario: Write inside a worktree allowed
- **WHEN** an `Edit` targets a file inside the repo's worktrees directory
- **THEN** the hook allows it

#### Scenario: Untracked file in the main tree allowed
- **WHEN** a `Write` targets a path in the main tree that git does not track
- **THEN** the hook allows it

#### Scenario: Bash redirection into a tracked main-tree file blocked
- **WHEN** the tool call is `echo x > README.md` through `Bash` and the working directory is the main tree, where `README.md` is tracked
- **THEN** the hook blocks with rule `main-tree` and the same message an `Edit` gets

#### Scenario: Bash writing shapes into the main tree blocked
- **WHEN** each of `printf y >> src/main.rs`, `tee src/main.rs`, `sed -i '' 's/a/b/' src/main.rs`, `cp /tmp/x src/main.rs`, `mv /tmp/x src/main.rs`, `rsync /tmp/x src/main.rs`, `git checkout -- src/main.rs` and `git restore src/main.rs` is the tool call through `Bash` in the main tree, where `src/main.rs` is tracked
- **THEN** every one of them blocks with rule `main-tree`, and the same set through `PowerShell` blocks with the same stderr lines

#### Scenario: Bash writing shapes elsewhere allowed
- **WHEN** the tool call is `echo x > notes.txt` (an untracked path in the main tree), `echo x > <scratchpad>/x`, or `echo x > <worktrees dir>/wt/README.md`, through `Bash`
- **THEN** the hook allows each of them

#### Scenario: Interpreter write is not blocked before the fact
- **WHEN** the tool call is a `python - <<'EOF' … EOF` heredoc that rewrites a tracked main-tree file, through `Bash`
- **THEN** the pre-tool hook allows it

#### Scenario: Rule disabled per repo
- **WHEN** `ratchet.toml` lists `python-venv` under `guardrails.off` and the tool call is `python scripts/x.py`
- **THEN** the hook allows it

#### Scenario: Custom content rule from the repo
- **WHEN** the repo's extra rules file defines a `content` rule blocking `\.purge_all\s*\(` and a `Write` has that text in its content
- **THEN** the hook blocks with the id and message of that rule

#### Scenario: Machine-wide rule overrides a built-in
- **WHEN** the machine rules file redefines `git-destructive` with a different message and the tool call is `git reset --hard`
- **THEN** the hook blocks with the machine message

### Requirement: Main-tree writes detected after the fact
In PostToolUse for `Bash` and `PowerShell` inside an opted-in repo, ratchet SHALL compare the
set of modified tracked files of the main tree (`git status --porcelain --untracked-files=no`
at the main root) with the set recorded by the matching PreToolUse, and when a tracked file of
the main tree changed during the command, SHALL print one stderr line prefixed
`[ratchet guardrail:main-tree]` naming the changed file(s) and the worktree alternative, and
SHALL record one event of kind `guardrail.main_tree_write` with the session, the command and
the files. It SHALL never block (exit 0, no decision), SHALL stay silent when nothing tracked
changed, and SHALL stay within the hook's time budget: at most one `git status` per hook, the
"before" set being taken by the PreToolUse that already runs for the command.

#### Scenario: A Bash command that changed a tracked main-tree file is reported after the fact
- **WHEN** the pre-tool hook ran for a `python` heredoc, the command rewrote a tracked file of the main tree, and the post-tool hook runs with the same session and command
- **THEN** the post-tool hook exits 0 with no decision, stderr has exactly one line containing `[ratchet guardrail:main-tree]` and the file's path, and the events log holds one `guardrail.main_tree_write` event for that session naming the file

#### Scenario: Post-check is silent when nothing tracked changed
- **WHEN** the pre-tool hook ran for a command, the command only created an untracked file in the main tree, and the post-tool hook runs
- **THEN** stdout and stderr are empty, exit 0, and no event is recorded

#### Scenario: Post-check without a prior snapshot is silent
- **WHEN** the post-tool hook runs for a `Bash` call whose PreToolUse never recorded a snapshot for this session
- **THEN** stdout and stderr are empty and exit 0

### Requirement: Big reads go to a cheap reader, not into the orchestrator's context
In PreToolUse inside an opted-in repo, the built-in rule `big-read` SHALL block a `Read` whose
`file_path` is a regular file of more than `[guardrails] big_read_lines` lines (default 350)
when the call gives neither `offset` nor `limit`, and SHALL block a `Bash`/`PowerShell`
segment whose command is `cat`, `head`, `tail`, `less` or `more` over such a file when the
segment is not part of a pipe. The message SHALL name three alternatives: `Read` with
`offset` and `limit`, the `reader` agent with the file and a question, and `grep` for the
lines wanted. The rule SHALL NOT apply to a file inside the repo's worktrees directory (that is
where implementers read whole files), to a file that does not exist, or to a segment with a
pipe. Like every built-in it SHALL be disableable by id and its threshold SHALL be
overridable per repo. Counting lines SHALL cost one read of the file and SHALL not run for
files under the threshold size in bytes (`big_read_lines × 16`), so the hot path stays under
the latency ceiling.

#### Scenario: Read of a big main-tree file blocked
- **WHEN** a `Read` targets a tracked main-tree file of 400 lines with no `offset` or `limit`
- **THEN** the hook blocks with rule `big-read` and the message names `offset`, `limit` and the `reader` agent

#### Scenario: Read with a window allowed
- **WHEN** a `Read` targets the same 400-line file with `limit: 80`
- **THEN** the hook allows it

#### Scenario: Small file allowed
- **WHEN** a `Read` targets a file of 349 lines with no window
- **THEN** the hook allows it

#### Scenario: cat of a big file blocked, piped cat allowed
- **WHEN** the tool call is `cat src/big.rs` through `Bash` where `src/big.rs` has 400 lines
- **THEN** the hook blocks with rule `big-read`, while `cat src/big.rs | grep fn` and `head -40 src/big.rs | cat` are allowed

#### Scenario: Big file inside a worktree allowed
- **WHEN** a `Read` targets a 400-line file under the repo's worktrees directory with no window
- **THEN** the hook allows it

#### Scenario: Threshold overridden per repo
- **WHEN** `ratchet.toml` sets `big_read_lines = 1000` under `[guardrails]` and a `Read` targets a 400-line main-tree file with no window
- **THEN** the hook allows it, and with `big_read_lines = 100` it blocks

### Requirement: Hooks never break a session
On any internal error (invalid config, malformed payload, unexpected failure) a hook SHALL
exit 0 without blocking and SHALL append one line to the log file under the state
directory. An unknown event SHALL exit 0. A pre-tool evaluation SHALL answer well under
the Claude Code hook timeout; the target is milliseconds, not seconds.

#### Scenario: Malformed payload exits 0 and is logged
- **WHEN** the pre-tool hook receives stdin that is not JSON
- **THEN** the hook exits 0, prints nothing on stdout, and the log file has one line for the event

#### Scenario: Unknown event exits 0
- **WHEN** the hook is invoked with an event name that does not exist
- **THEN** the hook exits 0

#### Scenario: Pre-tool answers fast
- **WHEN** the pre-tool hook is invoked twenty times on a blocked command in an opted-in repo
- **THEN** the median wall time is under 200 ms in a debug build

### Requirement: Active rules can be listed and dry-run
The active rule set for the current directory SHALL be listable, showing id, kind, tools,
source (built-in, machine or repo) and whether the repo disabled it. One tool call SHALL be
evaluable from the command line, with the same exit code and message a hook would give.

#### Scenario: List shows built-ins and disabled state
- **WHEN** `ratchet guardrails list` runs in a repo whose marker disables `python-venv`
- **THEN** the output has the four built-in ids and marks `python-venv` as off

#### Scenario: Dry-run reproduces the block
- **WHEN** `ratchet guardrails test Bash '{"command":"python x.py"}'` runs in a repo with `.venv`
- **THEN** it exits 2 and prints the same `[ratchet guardrail:python-venv]` line a hook would

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
