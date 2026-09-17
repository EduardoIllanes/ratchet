---
name: ratchet-tasks
description: How to work the `ratchet task list` board from a Claude Code session — look before claiming, claim, advance by checklist, leave notes, and write a handoff that the next session can act on. Use when the `[ratchet]` briefing appears at session start or when the Stop hook asks for a handoff.
---

# Tasks in a ratchet session

At session start the hook prints a briefing `[ratchet] repo … · session … · branch …` with the
repo's orphaned tasks, the ones ready to take, and your tasks in progress. Everything runs
through the `ratchet` CLI (the plugin puts it on your path; `ratchet --help` lists the commands).

## Minimum cycle

1. **Look before you claim**: `ratchet task list` is the repo's board (`--mine`, `--status ready`,
   `--tag <t>`); `ratchet task show T-0042` gives one task's body, checklist, last handoff and
   last ten events.
2. **Claim**: `ratchet task claim T-0042`. It becomes `in_progress` under your session. If another
   live session holds it, the CLI says so: do not force it, pick another or tell the owner.
3. **Advance by checklist**: each criterion met → `ratchet task check T-0042 <n>`. Progress is
   *only* that; there is no "about 60 % done".
4. **Leave a trail**: `ratchet task note T-0042 "found X, decided Y"` whenever you take a decision
   the next person needs. If you are stuck: `ratchet task status T-0042 blocked --why "…"`.
5. **Before ending your reply**: if the task is still `in_progress` and you recorded nothing this
   turn, the Stop hook will ask for a handoff once. Write a good one (below).
6. **Close**: `ratchet task status T-0042 review` (or `done` when the checklist is complete;
   without a checklist, `done --why "…"`).

## A useful handoff

`ratchet task handoff T-0042 "…"` is the first thing the next session reads (you tomorrow, or
another agent). In 2-5 lines it answers:

- **What is left**, concretely: "edge-case test in `x.py` missing; 3/5 of the checklist".
- **Where the work is**: branch/worktree, files touched, whether there is a commit.
- **What NOT to do / what was tried and failed**: "don't use `foo()`: breaks on naive dates".
- **How to resume**: the exact next command or step.

Bad: "continue the task". Good: "Missing the scenario coverage check for platform-ui (item 4).
Everything in `.worktrees/core`, no commit since `f5e4e3d`. Next: `cargo test --test spec ui`".

## Orphaned tasks

If the briefing lists a task "claimed by a dead session", read its last handoff and decide:
resume it (`ratchet task claim` transfers it to your session and leaves a note) or leave it for
the owner. Never mark it `done` without meeting its checklist.

When the owner has reviewed a task that is `done`, `ratchet task archive T-0042` takes it off the
board without losing anything: `ratchet task list --all` still lists it, `ratchet task show` still
has its whole history, and `ratchet task unarchive T-0042` brings it back. Archiving is the owner's
call, not yours — ask before you tidy.

## Session identity

The CLI resolves your session on its own (`RATCHET_SESSION_ID`, or the live session whose
directory covers your cwd). If it says "no session resolved", pass `--session <id>` (the id is in
the briefing); it is accepted anywhere on the command line, `ratchet --session <id> task claim
T-0042` and `ratchet task claim T-0042 --session <id>` alike. A `claim` with no session fails
outright; a `note`, `check`, `status` or `handoff` is recorded with no session and warns once, so
never ignore that warning — an unattributed record is a record nobody can be asked about.

## Guardrails

`ratchet guardrails list` shows the active rules (Python through the venv, no destructive git,
no `.env` writes, no writes to the main tree, plus whatever the repo or machine added). When a hook
blocks you, the message carries the alternative: use it, do not look for a way around.
