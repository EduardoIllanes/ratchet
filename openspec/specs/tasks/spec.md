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

### Requirement: Checking several items in one call
`ratchet task check <id> <n> [<n> ...]` SHALL accept one or more item positions and mark each one
done, in the order given, appending one `checklist.done` event per item (the same holds for
`--undo` and `checklist.undone`). If any position given does not exist on the checklist, the whole
call SHALL be refused exactly as marking that one position alone would be, listing the available
items, and no item in the call SHALL be marked. A call naming exactly one position behaves exactly
as before this requirement existed.

#### Scenario: Checking several items in one call
- **WHEN** `ratchet task check <id> 1 2 3` is run on a task with at least three items
- **THEN** items 1, 2 and 3 are done, each recorded against the calling session, and the history
  gained three `checklist.done` events, one per item, in that order

#### Scenario: A bad number in a batch refuses the whole call
- **WHEN** `ratchet task check <id> 1 9` is run on a task with fewer than 9 items
- **THEN** the operation is refused, listing the available items, and item 1 is left unmarked

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

### Requirement: Recording several notes in one call
`ratchet task note <id> "<text>" ["<text>" ...]` SHALL accept one or more texts and append one
`note` event per text, in the order given. A call naming exactly one text behaves exactly as
before this requirement existed.

#### Scenario: Recording several notes in one call
- **WHEN** `ratchet task note <id> "first" "second"` is run
- **THEN** two `note` events are appended, in that order, each carrying its own text

### Requirement: A handoff can carry a status transition
`ratchet task handoff <id> "<text>" --status <state>` SHALL record the handoff and then attempt
the same transition `ratchet task status <id> <state>` would, including its `--why` and
`--unreviewed` and the done gate of "Done requires an independent review". The handoff SHALL be
recorded regardless of the transition's outcome. A refused transition SHALL exit with that
transition's own error; a handoff given with no `--status` behaves exactly as before this
requirement existed.

#### Scenario: A handoff moves the task when the transition is valid
- **WHEN** `ratchet task handoff <id> "…" --status review` is run on a task that is `in_progress`
- **THEN** the handoff is recorded and the task is now `review`

#### Scenario: A refused transition after a handoff still records the handoff
- **WHEN** `ratchet task handoff <id> "…" --status done` is run on a task with pending checklist
  items
- **THEN** the operation is refused with the same message `ratchet task status <id> done` would
  give, and the handoff is recorded regardless

### Requirement: Review verdicts
An independent review of a task SHALL be recorded as a verdict — `approve` or `changes` — with
free text, appended as a `review.verdict` event attributed to the recording session. Recording a
verdict SHALL NOT change the task's status. The recording session need not be registered (unlike
claiming a task): a reviewer profile records its own, unregistered session identifier. An unknown
task or an unknown verdict word SHALL be refused, listing the two valid words. Recording is
unconditional on registration: an unregistered session's verdict is appended and shown exactly
like any other, even though it will not by itself satisfy the done gate below. When the
recording session's identity is attributed to a subagent (see sessions' "A subagent's board
write is attributed to it"), the event also carries that agent's identifier and its agent type,
and `ratchet task show` and its history show that agent's type and the first eight characters of
its identifier alongside the session.

#### Scenario: Verdict recorded
- **WHEN** a review verdict of `approve` with text `"looks right"` is recorded against a task
- **THEN** one `review.verdict` event is appended carrying that verdict and that text, and the
  task's status is unchanged

#### Scenario: An unregistered verdict is still recorded and listed
- **WHEN** a review verdict is recorded by a session identifier the registry does not know
- **THEN** the `review.verdict` event is appended and appears in `ratchet task show` and the
  task's history like any other

#### Scenario: task show shows the agent on an attributed event
- **WHEN** a review verdict is recorded by a call attributed to a subagent
- **THEN** `ratchet task show` shows that agent's type and the first eight characters of its
  identifier on that verdict's line

### Requirement: Done requires an independent review
Moving a task to `done` SHALL be refused unless its most recent `review.verdict` event is
`approve`, recorded by a session that ratchet itself registered — one recorded by the
session-start hook — and that is neither the task's current holder nor any session that recorded
a `task.claimed` or a `checklist.done` event on it. The refusal SHALL name the exact
`ratchet task review` command that supplies a valid verdict, and the session to record it from
other than: the disqualified verdict's own session when there is one to point at (whether it was
disqualified for holding the task or for a claimed/checklist.done entry in the task's history
that is not the current holder), and the current holder when there is no verdict at all. When the
most recent `approve` instead came from a session the registry does not know, the refusal SHALL
say that the reviewer's session is not one ratchet registered, that the review must be recorded
from a Claude Code session opened in this repo, and that `--unreviewed` remains available to the
task's owner. `ratchet task status <id> done --unreviewed` SHALL bypass this requirement — the
checklist requirement of "States and transitions" still applies — and SHALL record a note reading
"done without independent review", so the bypass stays visible on the board.

When the most recent `approve` is attributed to a session paired with an agent identity, the
gate SHALL compare identities as that pair: it SHALL pass when neither a `task.claimed` nor a
`checklist.done` event on the task carries that same pair, when a `subagent.start` event
recorded that agent starting in that session, and when that pair is not the task's current
holder. An approve attributed to the bare session — no agent identity — SHALL still need a
session ratchet registered that is neither the holder nor the session of a disqualifying
`task.claimed` or `checklist.done` event, exactly as above. Before the task moves to `done`, the
approving identity SHALL also need a Claude Code transcript — the session's own transcript for a
bare-session approve, or that agent's `agent-<id>.jsonl` transcript for a paired one, located the
way `ratchet usage` locates them — containing a `Bash` tool call whose command contains
`task review <id>`. Without that transcript the move SHALL be refused, saying the approving
identity's transcript carries no matching review command.

#### Scenario: Done refused with no verdict
- **WHEN** a task with a complete checklist and no `review.verdict` event is moved to `done`
- **THEN** the operation is refused, naming the missing verdict and the `ratchet task review`
  command that records one

#### Scenario: Done refused when the approve came from the holding session
- **WHEN** a task's only `review.verdict` event is `approve`, recorded by the session that
  currently holds the task, and the task is moved to `done`
- **THEN** the operation is refused for the same reason as a missing verdict, naming that
  holding session

#### Scenario: Done refused when the approve came from a session that checked off an item
- **WHEN** a task's most recent `review.verdict` event is `approve`, recorded by a session
  ratchet registered that never held the task but earlier recorded a `checklist.done` event on
  it, and the task is moved to `done`
- **THEN** the operation is refused, naming that session — not the task's current holder, who
  never reviewed anything

#### Scenario: Done refused when the approve came from an unregistered session
- **WHEN** a task's most recent `review.verdict` event is `approve`, recorded by a session
  identifier the registry does not know, and the task is moved to `done`
- **THEN** the operation is refused, saying that session is not one ratchet registered, that the
  review must be recorded from a Claude Code session opened in this repo, and naming
  `--unreviewed` as the owner's option

#### Scenario: Done allowed after an approve from another session
- **WHEN** a task's checklist is complete, its most recent `review.verdict` event is `approve`,
  recorded by a session ratchet registered that never claimed the task and never checked off one
  of its items, and that session's own transcript contains a `Bash` tool call whose command
  contains `task review <id>`
- **THEN** the task moves to `done`

#### Scenario: Done refused for a registered bare session whose transcript lacks the review
- **WHEN** a task's most recent `review.verdict` event is `approve`, recorded by a session
  ratchet registered — hand-registered through the session-start hook counts — that is
  independent by every other rule, but no transcript located for that session contains a `Bash`
  tool call whose command contains `task review <id>`
- **THEN** the operation is refused, saying the approving identity's transcript carries no
  matching review command

#### Scenario: Done refused when a changes verdict is newer than the approve
- **WHEN** a task received an `approve` from an independent session and then a later `changes`
  verdict, and the task is moved to `done`
- **THEN** the operation is refused, naming the `changes` verdict and when it was recorded, and
  the exact `ratchet task review` command for a fresh `approve`

#### Scenario: --unreviewed succeeds and records the note
- **WHEN** `ratchet task status <id> done --unreviewed` is run on a task with a complete checklist
  and no review verdict
- **THEN** the task moves to `done` and a note reading "done without independent review" is
  recorded

#### Scenario: Done allowed for a reviewer subagent that never worked the task
- **WHEN** a task's checklist is complete, a `subagent.start` event recorded a reviewer agent
  starting in the holding session, that agent's approve is the task's most recent
  `review.verdict`, neither a `task.claimed` nor a `checklist.done` event on the task carries
  that session-and-agent pair, and a transcript for that agent contains a `Bash` tool call whose
  command contains `task review <id>`
- **THEN** the task moves to `done`

#### Scenario: Done refused for the implementer subagent that checked items
- **WHEN** a subagent checked off an item on a task, attributed to a session-and-agent pair, and
  that same pair later records the task's most recent `approve`
- **THEN** the operation is refused for the same reason as a missing verdict, naming that agent

#### Scenario: Done refused for the orchestrator (bare session) that claimed
- **WHEN** the bare session that claimed a task — no agent identity, the main thread itself —
  later records the task's most recent `approve` from that same bare session
- **THEN** the operation is refused for the same reason as a missing verdict, naming that
  holding session

#### Scenario: Done refused when the reviewer identity has no transcript containing the review command
- **WHEN** a task's most recent `review.verdict` event is `approve`, attributed to a session and
  agent pair that is independent by every other rule, but no transcript located for that
  identity contains a `Bash` tool call whose command contains `task review <id>`
- **THEN** the operation is refused, saying the approving identity's transcript carries no
  matching review command

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
