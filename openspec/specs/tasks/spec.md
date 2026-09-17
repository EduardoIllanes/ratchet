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
