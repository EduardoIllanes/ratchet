# sessions

Which agent sessions exist, which are alive, and what happens to the work of the ones that
died. Group 1 covers the state layer and the six hooks that are not `PreToolUse`; the task
board, the briefing and the handoff rule arrive with group 2.

## Purpose

An agent session is the unit of accountability: a task is claimed by a session, and a
session that disappears must not keep work hostage. The registry is written by the hooks
themselves, so it cannot be skipped from the prompt, and it never invents an identity of its
own.

## Requirements

### Requirement: One local state database
All state SHALL live in a single database file under the state directory, created on first
use. The schema SHALL be versioned and applied by ordered migrations embedded in the binary;
applying them SHALL be idempotent and all-or-nothing per migration. Only the session-start
hook and the explicit migrate command SHALL migrate; any other face SHALL refuse to work on
an older schema, naming the command that fixes it, and in a hook that refusal SHALL be
silent success plus one log line. The path of the database SHALL be printable.

#### Scenario: First hook creates the database
- **WHEN** the session-start hook runs in an opted-in repo and no database exists yet
- **THEN** the database file exists afterwards and the session is registered

#### Scenario: Migrations apply once
- **WHEN** the migrate command runs twice in a row
- **THEN** the first run reports the migrations it applied and the second reports that nothing was pending

#### Scenario: The database path is printable
- **WHEN** the owner asks for the database path
- **THEN** one line with the absolute path inside the state directory is printed

#### Scenario: A stale schema is reported, not migrated
- **WHEN** a command that only reads state runs against a database whose schema is older than the binary expects
- **THEN** it fails with a message naming the migrate command, and the schema is left untouched

#### Scenario: The guardrail hook opens no database
- **WHEN** the pre-tool hook evaluates a tool call in an opted-in repo with no database yet
- **THEN** no database file is created

### Requirement: Session registry
A session SHALL be registered from the hooks with the identifier the agent harness gives
them; ratchet SHALL NOT invent another. The registry SHALL record the repo it opted into,
the working directory, the worktree when the directory is one, the branch when it can be
read, the mode (interactive or headless), who launched it (the user or the platform), the
start, the last signal and the end. Registering the same identifier twice SHALL keep one
session and refresh it instead of creating a second. The identifier SHALL be published to
the session's shell environment when the harness offers a file for it. In a directory with
no marker above it, nothing SHALL be registered.

#### Scenario: Idempotent registration
- **WHEN** the session-start hook receives the same session identifier twice, the second time from another directory of the same repo
- **THEN** exactly one session exists with that identifier, its last signal is the later one, and two start events are recorded

#### Scenario: A session in a worktree records branch and worktree
- **WHEN** a session starts inside a linked worktree of an opted-in repo
- **THEN** the session records that directory as its worktree and the branch checked out there

#### Scenario: A headless session launched by the platform
- **WHEN** a session starts with the environment declaring headless mode and the platform as launcher
- **THEN** the session is registered with that mode and that launcher, and with the identifier the environment fixed

#### Scenario: The session id reaches the shell
- **WHEN** the session-start hook runs and the harness offers an environment file
- **THEN** that file gains one line exporting the session identifier

#### Scenario: No marker, no session
- **WHEN** the session-start hook runs in a directory with no marker above it
- **THEN** it exits successfully, prints nothing, and no database is created

### Requirement: Heartbeat
Every user prompt, every end of a response, every subagent start, every subagent end and
every compaction SHALL refresh the last signal of the session. A prompt, an end of response,
a subagent start and a subagent end SHALL also record an event; a compaction SHALL NOT. A
hook that arrives for a session that is not registered yet SHALL register it instead of
failing.

#### Scenario: A prompt updates the last signal
- **WHEN** a prompt arrives for a registered session
- **THEN** its last signal moves forward and a prompt event is recorded

#### Scenario: Compaction updates the last signal with no event
- **WHEN** a compaction arrives for a registered session
- **THEN** its last signal moves forward and no extra event is recorded for it

#### Scenario: A hook of an unregistered session registers it
- **WHEN** the first hook to arrive for a session identifier is a prompt, not a session start
- **THEN** the session is registered and its last signal is set

### Requirement: Subagent start and stop
A subagent start SHALL record a `subagent.start` event and a subagent end a `subagent.stop`
event, both attributed to the session the hook arrived for. When that session holds tasks in
progress, the event SHALL carry the first of them as its task; when it holds none, the event
SHALL carry no task. The payload SHALL carry the agent identifier, its type and description
when known, and the transcript path when the input offers one; a stop SHALL also carry the
exit status when the input offers one. When the input carries an agent identifier but no
type, the hook SHALL read the file `agent-<id>.meta.json` from the `subagents` directory of
that session next to the transcript — the directory named after the session beside the
transcript path — without ever writing to it, and take the type and description from its
`agentType` and `description` fields when present. A missing or unreadable meta file SHALL
NOT fail the hook: the event is recorded with whatever identity is known.

#### Scenario: A subagent start records a start event
- **WHEN** a subagent start with an agent identifier, a type and a transcript path arrives for a session holding a task in progress
- **THEN** its last signal moves forward and one `subagent.start` event is recorded with that task and a payload carrying the agent identifier, the type and the transcript path

#### Scenario: A subagent stop records a stop event
- **WHEN** a subagent stop with an agent identifier, a type, a transcript path and an exit status arrives for a session holding a task in progress
- **THEN** one `subagent.stop` event is recorded with that task and a payload carrying the agent identifier, the type, the transcript path and the exit status

#### Scenario: A stop with no held task records a session-level event
- **WHEN** a subagent stop arrives for a session holding no task in progress
- **THEN** one `subagent.stop` event is recorded with no task

#### Scenario: A missing type falls back to the meta file
- **WHEN** a subagent stop carries an agent identifier and a transcript path but no type, and the meta file for that agent beside that transcript names a type and a description
- **THEN** the recorded `subagent.stop` event carries that type and that description

#### Scenario: A missing meta file still records
- **WHEN** a subagent stop carries an agent identifier but no type and no meta file exists for it
- **THEN** the hook exits successfully and the `subagent.stop` event is recorded with the identifier alone

### Requirement: Derived session state
The state of a session SHALL be derived, never stored: ended when it has an end; otherwise
live while the last signal is recent, idle after the live threshold, and orphaned after the
idle threshold. Both thresholds SHALL be configurable per repo in the marker.

#### Scenario: A session with no signal for ninety minutes is orphaned
- **WHEN** a session has no end and its last signal was ninety minutes ago
- **THEN** its state is orphaned

#### Scenario: An ended session stays ended
- **WHEN** a session recorded its end one minute ago
- **THEN** its state is ended even though its last signal is recent

#### Scenario: The repo sets its own thresholds
- **WHEN** the marker sets a live threshold of one minute and an idle threshold of two, and the last signal was ninety seconds ago
- **THEN** the state is idle rather than live

### Requirement: Session end
The session-end hook SHALL record the end of the session and SHALL NOT block. Ending a
session that was never registered SHALL be silent success.

#### Scenario: Session end marks the session ended
- **WHEN** the session-end hook arrives for a live session
- **THEN** the session has an end, its state is ended, and an end event is recorded

### Requirement: Work of a dead session goes back
A task in progress claimed by a session that is ended or orphaned, or by a session the
registry has no row for at all, SHALL be released: it goes back to ready, without a session,
keeping its history. The release SHALL happen at the end of the session that held it and,
for sessions that died without a hook, at the next session start in that repo. A task
claimed by a live session SHALL NOT be released. Every release SHALL record the change of
state and a note saying it was released and by whom.

#### Scenario: Session end returns its tasks
- **WHEN** the session-end hook arrives for a session holding a task in progress
- **THEN** the task is ready, holds no session, and its history shows the release

#### Scenario: A new session releases tasks of a dead one
- **WHEN** a session starts in a repo where a task in progress is held by a session whose last signal is ninety minutes old
- **THEN** that task is ready and holds no session

#### Scenario: A task held by a live session is not released
- **WHEN** a session starts in a repo where a task in progress is held by another session that signalled a minute ago
- **THEN** that task is still in progress and still held by that session

#### Scenario: A task claimed by a session the registry never saw is released
- **WHEN** a session starts in a repo where a task in progress is claimed by a session identifier the registry has no row for
- **THEN** that task is ready, holds no session, and its history shows the release

### Requirement: Sessions can be listed and shown
Sessions SHALL be listable with their derived state, filterable to the live ones and by
repo, and one session SHALL be showable in detail with the tasks it holds. Asking for a
session without naming one SHALL resolve it: an explicit identifier first, then the one in
the environment, then the most recent live session whose directory or worktree covers the
current directory, most specific first. An identifier that does not exist SHALL fail with a
message, not silently.

#### Scenario: Listing shows the derived state
- **WHEN** two sessions exist, one signalling now and one ninety minutes ago, and the list is asked for
- **THEN** one line per session shows its identifier and its state, and the live filter leaves only the recent one

#### Scenario: Showing an unknown session fails with a message
- **WHEN** a session identifier that was never registered is shown
- **THEN** the command fails with a message naming that identifier

#### Scenario: Showing without an id resolves the session of the directory
- **WHEN** a session is shown without naming one, from a directory covered by a live session
- **THEN** the detail of that session is printed

### Requirement: State errors never break a session
A hook that cannot use the state database SHALL exit successfully, print nothing on the
context, and append one line to the log. It SHALL never block a tool call and SHALL never
interrupt the agent.

#### Scenario: An unusable database does not break the session
- **WHEN** the state directory holds a database file whose contents are not a database, and a prompt hook arrives
- **THEN** the hook exits successfully, prints nothing, and the log gains one line for the event
