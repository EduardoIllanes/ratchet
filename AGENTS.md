# Agent doctrine

ratchet ships eight agent profiles and two working rules. The rules are what make the profiles
worth having.

## Rule 1: guardrails live in the harness, not in the prompt

Anything you ask for in a prompt and do not enforce gets violated sooner or later. A hook that
blocks is worth more than ten instructions that ask. Every block carries the alternative the
agent should use, so it corrects itself in one attempt instead of improvising.

## Rule 2: whoever writes the tests is not whoever implements, and whoever reviews is neither

- **spec-test-author** reads only the specs and the public signatures it is given. It writes one
  scenario test per `#### Scenario`, leaves them red-clean (assertion failures, never import
  or collection errors) and does not commit.
- **implementer** codes against those tests in a worktree. It never edits a scenario test; if
  one is impossible or contradictory it stops and reports.
- **reviewer** reads the diff against the spec, the checklist and the handoff, runs
  adversarial probes for real (always with doubles), runs the gate, and hunts side effects
  outside the declared scope. It describes a blocking finding; it never fixes it. Its verdict is
  recorded, not narrated — see "Hard stops" below.
- **refactorer** changes how code reads, never what it does: runs the gate before touching
  anything, works in small steps with the gate after each, reverts a step that goes red, and
  deletes only code with no reference in the crate and no mention in README, specs or skills.
  A behaviour change it would need is reported as a proposal, not made.
- **analyst** answers read-only questions (spec vs code, design comparisons) as board notes.
- **researcher** extracts text from local PDFs with `ratchet pdf` and answers with citations.
- **mapper** describes header-less files for `ratchet map`, one sentence each, through
  `ratchet map note` — the only file it ever touches is `.ratchet/map.notes`, and only through
  that command, never by editing it directly.
- **reader** is the built-in `big-read` guardrail's answer to "then how do I read it": a file
  over the line threshold, or a bare `cat`/`head`/`tail`/`less`/`more` of one, is blocked with
  three ways out — `Read` with `offset`/`limit`, `grep` for the lines wanted, or this agent. Big
  files are read by a cheap reader with a question, never by the orchestrator: it runs on
  `haiku` at low effort, takes a file (or files) plus a question, and answers in structured
  bullets only — no prose, and it never edits.

An orchestrating session dispatches them and reads their reports; it does not implement. That
is the only way its context stays useful at the end of the day.

## Hard stops: what a worker with no hooks checks by hand

A tool with hooks (PreToolUse, Stop) gets some of this enforced for it. A tool without hooks
gets nothing for free — this checklist is what the hooks would have done, with the exact
`ratchet` command for each. This list exists because skipping it once already produced a
self-reviewed, placeholder-shipping, branch-left-red change (T-0006, 2026-09-22).

- [ ] **Session start**: `ratchet task list` and read the last handoff of anything you hold or
      are about to take. Never start blind to what the last session said.
- [ ] **Claim**: `ratchet task claim <id>` before touching a file for that task. One call already
      leaves it `in_progress` (through `ready` first if it was in `backlog`) — no separate
      `status` call. An unclaimed task has no session to hold it accountable.
- [ ] **Progress**: `ratchet task check <id> <n> [<n> …]` the moment one or more checklist items
      are actually met — batch them in one call, in order — and `ratchet task note <id> "…" ["…"
      …]` for every non-obvious decision, also batchable — before you act on it, not after.
- [ ] **Before `status <id> review`**: run the gate (`cargo fmt --check`, `cargo clippy
      --all-targets -- -D warnings`, `cargo test`, or whatever the repo documents) on the
      **target branch, after merging your work into it** — not only inside your worktree. A
      worktree can be green while the branch it merges into is red.
- [ ] **The review is never your own note.** `ratchet task note` from the session that did the
      work is not a review. An independent reviewer — a different session, its own
      `--session` — records the verdict: `ratchet task review <id> approve "…"` or
      `ratchet task review <id> changes "…"`.
- [ ] **Before `status <id> done`**: an `approve` verdict from that independent session must
      already exist. The CLI refuses `done` on its own when it is missing, naming what to run —
      the check here is so you never find that out from a refusal mid-close.
- [ ] **Nothing named `scratch`, `probe`, or `placeholder` is committed.** A probe lives in the
      job's temp directory or the session scratchpad, never in the repo, never in a commit.
- [ ] **End**: `ratchet task handoff <id> "…" --status review` — what is left, where the work is,
      what not to do, how to resume, and the move to `review` in the same call (`--status`
      applies the same transition rules `status` does, including `--why` and `--unreviewed`; a
      refused transition still leaves the handoff recorded). A session that closes an
      `in_progress` task with nothing recorded leaves the next one guessing.

The owner alone may skip the review with `ratchet task status <id> done --unreviewed`; it still
requires a complete checklist, and it leaves a note on the board saying so. Nobody else uses it —
the point of the checklist above is that nobody needs to.

## How a task moves

```
backlog → ready → in_progress → review → done        (blocked is a side state)
```

- Progress is derived from the checklist. Nobody declares "60 % done".
- Every state change, check, note, review verdict and handoff is an append-only event.
- A session that ends with an `in_progress` task and no record from that turn is asked, once,
  for a handoff. A handoff says what is left, where the work is, what not to do, and how to
  resume.
- Tasks held by a session that died return to `ready` at the next session start, with a note.

## Where the roles come from

These profiles were extracted from a private platform where they ran for months on a
spec-driven flow (OpenSpec proposals → scenario tests → implementation → review). The
numbers in them (budgets, line limits) are the ones that survived that use. Change them in your
copy of the profile if your repo needs different ones.
