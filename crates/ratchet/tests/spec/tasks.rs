//! One test per `#### Scenario` of openspec/specs/tasks/spec.md, named by slug. These tests drive
//! the real binary as a subprocess; they read and seed the database directly because they stand
//! outside the binary, which production code never does outside `services/`.

use crate::sessions::board_write_call;
use crate::support::*;
use serde_json::json;
use std::io::Write;

// --- Requirement: Readable, stable identifier -----------------------------------------------

#[test]
fn tasks__creating_a_task_assigns_the_next_identifier() {
    let sb = board("s-1");
    let first = new_task(&sb, "first", &[], "s-1", 1);
    assert_eq!(first, "T-0001");
    let second = new_task(&sb, "second", &[], "s-1", 2);
    assert_eq!(second, "T-0002");
    // Closing and archiving the first one must not free its number.
    assert_eq!(code(&task(&sb, &["claim", &first], "s-1", 3)), 0);
    assert_eq!(
        code(&task(
            &sb,
            &[
                "status",
                &first,
                "done",
                "--why",
                "no criteria",
                "--unreviewed"
            ],
            "s-1",
            4
        )),
        0
    );
    assert_eq!(code(&task(&sb, &["archive", &first], "s-1", 5)), 0);
    let third = new_task(&sb, "third", &[], "s-1", 6);
    assert_eq!(third, "T-0003");
}

// --- Requirement: Fields of a task ----------------------------------------------------------

#[test]
fn tasks__one_level_of_subtasks() {
    let sb = board("s-2");
    let parent = new_task(&sb, "parent", &[], "s-2", 1);
    let child = task(&sb, &["new", "child", "--parent", &parent], "s-2", 2);
    assert_eq!(code(&child), 0, "{}", stderr(&child));
    let child_id = stdout(&child)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    let grandchild = task(&sb, &["new", "grandchild", "--parent", &child_id], "s-2", 3);
    assert_eq!(code(&grandchild), 1);
    assert!(
        stderr(&grandchild).to_lowercase().contains("one level"),
        "{}",
        stderr(&grandchild)
    );
    assert_eq!(count(&sb, "SELECT COUNT(*) FROM tasks", &[]), 2);
}

#[test]
fn tasks__a_task_belongs_to_the_repo_it_was_created_in() {
    let sb = board("s-3");
    let inside = new_task(&sb, "inside", &[], "s-3", 1);
    let repo_root: String = db(&sb)
        .query_row(
            "SELECT repo_root FROM tasks WHERE id = ?1",
            rusqlite::params![inside],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(repo_root, repo_root_key(&sb));

    let outside = unmanaged_dir();
    let when = at(2);
    let out = cli(
        &sb,
        &["task", "new", "orphan of no repo"],
        outside.path(),
        &[("RATCHET_SESSION_ID", "s-3"), ("RATCHET_NOW", &when)],
    );
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("ratchet.toml"), "{}", stderr(&out));
    assert_eq!(count(&sb, "SELECT COUNT(*) FROM tasks", &[]), 1);
}

#[test]
fn tasks__priority_outside_the_range() {
    let sb = board("s-4");
    let out = task(&sb, &["new", "too urgent", "--priority", "7"], "s-4", 1);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("1"), "{}", stderr(&out));
    assert!(stderr(&out).contains("4"), "{}", stderr(&out));
    assert_eq!(count(&sb, "SELECT COUNT(*) FROM tasks", &[]), 0);
}

// --- Requirement: States and transitions ----------------------------------------------------

#[test]
fn tasks__invalid_transition() {
    let sb = board("s-5");
    let id = new_task(&sb, "straight to done", &[], "s-5", 1);
    assert_eq!(code(&task(&sb, &["status", &id, "ready"], "s-5", 2)), 0);
    let out = task(&sb, &["status", &id, "done"], "s-5", 3);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("in_progress"), "allowed list missing: {err}");
    assert!(err.contains("backlog"), "allowed list missing: {err}");
    assert_eq!(task_state(&sb, &id).0, "ready");
}

#[test]
fn tasks__done_needs_a_complete_checklist() {
    let sb = board("s-6");
    let id = new_task(&sb, "two criteria", &["write it", "test it"], "s-6", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-6", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-6", 3)), 0);
    let out = task(&sb, &["status", &id, "done"], "s-6", 4);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("test it"), "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

#[test]
fn tasks__done_without_a_checklist_needs_a_reason() {
    let sb = board("s-7");
    let id = new_task(&sb, "no criteria", &[], "s-7", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-7", 2)), 0);
    let refused = task(&sb, &["status", &id, "done"], "s-7", 3);
    assert_eq!(code(&refused), 1, "stdout: {}", stdout(&refused));
    assert!(
        stderr(&refused).to_lowercase().contains("why"),
        "{}",
        stderr(&refused)
    );
    let accepted = task(
        &sb,
        &["status", &id, "done", "--why", "obsolete", "--unreviewed"],
        "s-7",
        4,
    );
    assert_eq!(code(&accepted), 0, "{}", stderr(&accepted));
    assert_eq!(task_state(&sb, &id).0, "done");
    let payloads = payloads_of(&sb, &id, "task.status");
    assert!(
        payloads.last().unwrap().contains("obsolete"),
        "{payloads:?}"
    );
}

#[test]
fn tasks__a_valid_transition_leaves_an_event() {
    let sb = board("s-8");
    let id = new_task(&sb, "blocked work", &[], "s-8", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-8", 2)), 0);
    let out = task(
        &sb,
        &["status", &id, "blocked", "--why", "waiting for credentials"],
        "s-8",
        3,
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).0, "blocked");
    let last = payloads_of(&sb, &id, "task.status").pop().unwrap();
    assert!(last.contains("in_progress"), "origin missing: {last}");
    assert!(last.contains("blocked"), "destination missing: {last}");
    assert!(
        last.contains("waiting for credentials"),
        "reason missing: {last}"
    );
}

// --- Requirement: Claiming a task -----------------------------------------------------------

#[test]
fn tasks__claim_from_backlog() {
    let sb = board("s-9");
    let id = new_task(&sb, "to claim", &[], "s-9", 1);
    assert_eq!(task_state(&sb, &id).0, "backlog");
    let out = task(&sb, &["claim", &id], "s-9", 2);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let (status, holder, _) = task_state(&sb, &id);
    assert_eq!(status, "in_progress");
    assert_eq!(holder.as_deref(), Some("s-9"));
    let kinds = kinds_of(&sb, &id);
    assert_eq!(
        kinds,
        vec![
            "task.created".to_string(),
            "task.status".to_string(),
            "task.status".to_string(),
            "task.claimed".to_string(),
        ],
        "history was {kinds:?}"
    );
}

#[test]
fn tasks__claim_with_an_unregistered_session() {
    let sb = board("s-10");
    let id = new_task(&sb, "to claim", &[], "s-10", 1);
    let out = task(&sb, &["claim", &id, "--session", "s-ghost"], "s-10", 2);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(
        stderr(&out).to_lowercase().contains("not registered"),
        "{}",
        stderr(&out)
    );
    let (status, holder, _) = task_state(&sb, &id);
    assert_eq!(status, "backlog");
    assert_eq!(holder, None);
}

#[test]
fn tasks__claim_over_a_live_session() {
    let sb = board("s-11a");
    assert_eq!(code(&join(&sb, "s-11b", 1)), 0);
    let id = new_task(&sb, "contested", &[], "s-11a", 2);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-11a", 3)), 0);
    let out = task(&sb, &["claim", &id], "s-11b", 4);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("s-11a"), "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).1.as_deref(), Some("s-11a"));
}

#[test]
fn tasks__claim_over_an_orphaned_session() {
    let sb = board("s-12a");
    assert_eq!(code(&join(&sb, "s-12b", 1)), 0);
    let id = new_task(&sb, "abandoned", &[], "s-12a", 2);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-12a", 3)), 0);
    // Ninety minutes later, with no signal from either session, both are orphaned; a CLI call
    // does not sweep, so the task is still held when the second session asks for it.
    let out = task(&sb, &["claim", &id], "s-12b", 90);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).1.as_deref(), Some("s-12b"));
    let notes = payloads_of(&sb, &id, "note");
    assert!(
        notes
            .iter()
            .any(|n| n.contains("s-12a") && n.contains("s-12b")),
        "transfer note missing: {notes:?}"
    );
}

// --- Requirement: The checklist is the acceptance criteria ----------------------------------

#[test]
fn tasks__check_by_position() {
    let sb = board("s-13");
    let id = new_task(&sb, "three criteria", &["one", "two", "three"], "s-13", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-13", 2)), 0);
    let out = task(&sb, &["check", &id, "3"], "s-13", 3);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let (done, by): (i64, Option<String>) = db(&sb)
        .query_row(
            "SELECT done, done_by_session FROM checklist_items WHERE task_id = ?1 AND position = 3",
            rusqlite::params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(done, 1);
    assert_eq!(by.as_deref(), Some("s-13"));
    assert!(
        payloads_of(&sb, &id, "checklist.done")
            .last()
            .unwrap()
            .contains("three"),
        "event does not name the item"
    );
}

#[test]
fn tasks__a_position_that_does_not_exist() {
    let sb = board("s-14");
    let id = new_task(&sb, "five criteria", &["a", "b", "c", "d", "e"], "s-14", 1);
    let out = task(&sb, &["check", &id, "9"], "s-14", 2);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("1"), "{err}");
    assert!(err.contains("5"), "{err}");
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM checklist_items WHERE done = 1",
            &[]
        ),
        0
    );
}

// --- Requirement: Checking several items in one call -----------------------------------------

#[test]
fn tasks__checking_several_items_in_one_call() {
    let sb = board("s-30");
    let id = new_task(&sb, "three criteria", &["one", "two", "three"], "s-30", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-30", 2)), 0);
    let out = task(&sb, &["check", &id, "1", "2", "3"], "s-30", 3);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    for pos in 1..=3 {
        let (done, by): (i64, Option<String>) = db(&sb)
            .query_row(
                "SELECT done, done_by_session FROM checklist_items WHERE task_id = ?1 AND position = ?2",
                rusqlite::params![id, pos],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(done, 1, "item {pos} not done");
        assert_eq!(by.as_deref(), Some("s-30"), "item {pos}");
    }
    let events = payloads_of(&sb, &id, "checklist.done");
    assert_eq!(events.len(), 3, "{events:?}");
    // In order: position 1's payload first, then 2, then 3.
    assert!(events[0].contains("\"position\":1"), "{events:?}");
    assert!(events[1].contains("\"position\":2"), "{events:?}");
    assert!(events[2].contains("\"position\":3"), "{events:?}");
}

#[test]
fn tasks__a_bad_number_in_a_batch_refuses_the_whole_call() {
    let sb = board("s-31");
    let id = new_task(&sb, "three criteria", &["one", "two", "three"], "s-31", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-31", 2)), 0);
    let out = task(&sb, &["check", &id, "1", "9"], "s-31", 3);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("1"), "{err}");
    assert!(err.contains("3"), "{err}");
    let (done, _): (i64, Option<String>) = db(&sb)
        .query_row(
            "SELECT done, done_by_session FROM checklist_items WHERE task_id = ?1 AND position = 1",
            rusqlite::params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(done, 0, "item 1 was marked despite the refusal");
    assert!(payloads_of(&sb, &id, "checklist.done").is_empty());
}

// --- Requirement: Progress is derived -------------------------------------------------------

#[test]
fn tasks__with_a_checklist() {
    let sb = board("s-15");
    let id = new_task(&sb, "five items", &["a", "b", "c", "d", "e"], "s-15", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-15", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-15", 3)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "2"], "s-15", 4)), 0);
    let shown = stdout(&task(&sb, &["show", &id], "s-15", 5));
    assert!(shown.contains("2/5"), "{shown}");
    let listed = stdout(&task(&sb, &["list"], "s-15", 6));
    assert!(listed.contains("2/5"), "{listed}");
}

#[test]
fn tasks__without_a_checklist() {
    let sb = board("s-16");
    let id = new_task(&sb, "no items", &[], "s-16", 1);
    let shown = stdout(&task(&sb, &["show", &id], "s-16", 2));
    assert!(shown.contains("backlog"), "{shown}");
    assert!(!shown.contains("0/0"), "reported a zero progress: {shown}");
    let listed = stdout(&task(&sb, &["list"], "s-16", 3));
    assert!(
        !listed.contains("0/0"),
        "reported a zero progress: {listed}"
    );
}

// --- Requirement: Notes and handoffs --------------------------------------------------------

#[test]
fn tasks__the_last_handoff() {
    let sb = board("s-17");
    let id = new_task(&sb, "two handoffs", &[], "s-17", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-17", 2)), 0);
    assert_eq!(
        code(&task(
            &sb,
            &["handoff", &id, "the ten o'clock one"],
            "s-17",
            3
        )),
        0
    );
    assert_eq!(
        code(&task(
            &sb,
            &["handoff", &id, "the twelve o'clock one"],
            "s-17",
            4
        )),
        0
    );
    let shown = stdout(&task(&sb, &["show", &id], "s-17", 5));
    assert!(shown.contains("the twelve o'clock one"), "{shown}");
    let last_line = shown
        .lines()
        .rev()
        .find(|l| l.to_lowercase().contains("handoff"))
        .unwrap_or_else(|| panic!("no line names a handoff: {shown}"))
        .to_string();
    // Positive, not just negative: the matched line must BE the second handoff, not merely fail
    // to mention the first one. A negative-only check passes on any wrong line (the task's own
    // title contains the substring "handoff" too), so it must name the text that identifies
    // handoff #2 in this fixture and reject the text that identifies handoff #1.
    assert!(
        last_line.contains("the twelve o'clock one"),
        "the last handoff line is not the second handoff: {last_line}"
    );
    assert!(
        !last_line.contains("the ten o'clock one"),
        "the older handoff is shown as the last one: {last_line}"
    );
}

#[test]
fn tasks__an_empty_handoff_is_refused() {
    let sb = board("s-18");
    let id = new_task(&sb, "needs a handoff", &[], "s-18", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-18", 2)), 0);
    let out = task(&sb, &["handoff", &id, "   "], "s-18", 3);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(
        stderr(&out).to_lowercase().contains("resume"),
        "{}",
        stderr(&out)
    );
    assert!(payloads_of(&sb, &id, "handoff").is_empty());
}

// --- Requirement: Recording several notes in one call -----------------------------------------

#[test]
fn tasks__recording_several_notes_in_one_call() {
    let sb = board("s-32");
    let id = new_task(&sb, "needs two notes", &[], "s-32", 1);
    let out = task(&sb, &["note", &id, "first", "second"], "s-32", 2);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let notes = payloads_of(&sb, &id, "note");
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].contains("first"), "{notes:?}");
    assert!(notes[1].contains("second"), "{notes:?}");
}

// --- Requirement: A handoff can carry a status transition --------------------------------------

#[test]
fn tasks__a_handoff_moves_the_task_when_the_transition_is_valid() {
    let sb = board("s-33");
    let id = new_task(&sb, "handoff to review", &[], "s-33", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-33", 2)), 0);
    let out = task(
        &sb,
        &["handoff", &id, "moving to review", "--status", "review"],
        "s-33",
        3,
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).0, "review");
    let handoffs = payloads_of(&sb, &id, "handoff");
    assert_eq!(handoffs.len(), 1);
    assert!(handoffs[0].contains("moving to review"), "{handoffs:?}");
}

#[test]
fn tasks__a_refused_transition_after_a_handoff_still_records_the_handoff() {
    let sb = board("s-34");
    let id = new_task(&sb, "two criteria", &["write it", "test it"], "s-34", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-34", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-34", 3)), 0);
    let out = task(
        &sb,
        &["handoff", &id, "not done yet", "--status", "done"],
        "s-34",
        4,
    );
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("test it"), "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).0, "in_progress");
    let handoffs = payloads_of(&sb, &id, "handoff");
    assert_eq!(handoffs.len(), 1, "handoff was not recorded: {handoffs:?}");
    assert!(handoffs[0].contains("not done yet"), "{handoffs:?}");
}

// --- Requirement: Review verdicts -------------------------------------------------------------

#[test]
fn tasks__verdict_recorded() {
    let sb = board("s-22");
    let id = new_task(&sb, "to review", &[], "s-22", 1);
    let out = task(
        &sb,
        &[
            "review",
            &id,
            "approve",
            "looks right",
            "--session",
            "s-reviewer",
        ],
        "s-22",
        2,
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let payloads = payloads_of(&sb, &id, "review.verdict");
    assert_eq!(payloads.len(), 1);
    assert!(payloads[0].contains("approve"), "{payloads:?}");
    assert!(payloads[0].contains("looks right"), "{payloads:?}");
    // Recording a verdict never moves the task.
    assert_eq!(task_state(&sb, &id).0, "backlog");
}

#[test]
fn tasks__an_unregistered_verdict_is_still_recorded_and_listed() {
    let sb = board("s-30");
    let id = new_task(&sb, "ghost reviewed", &[], "s-30", 1);
    // s-ghost-reviewer is never registered (no join()) -- recording is unconditional on
    // registration, only the done gate cares.
    let out = task(
        &sb,
        &[
            "review",
            &id,
            "approve",
            "looks fine to a stranger",
            "--session",
            "s-ghost-reviewer",
        ],
        "s-30",
        2,
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let payloads = payloads_of(&sb, &id, "review.verdict");
    assert_eq!(payloads.len(), 1);
    assert!(payloads[0].contains("approve"), "{payloads:?}");
    let shown = stdout(&task(&sb, &["show", &id], "s-30", 3));
    assert!(shown.contains("looks fine to a stranger"), "{shown}");
}

#[test]
fn tasks__task_show_shows_the_agent_on_an_attributed_event() {
    let sb = board("s-44");
    let id = new_task(&sb, "attributed verdict", &[], "s-44", 1);
    let root = sb.root();
    let agent_id = "rev-3a1b2c3d";
    let call = board_write_call(
        &format!("ratchet task review {id} approve \"looks right\""),
        &root,
        "tu-r5",
        Some((agent_id, "ratchet:reviewer")),
    );
    assert_eq!(
        code(&hook_env(
            &sb,
            "pre-tool",
            &call,
            &root,
            &[("RATCHET_SESSION_ID", "s-44"), ("RATCHET_NOW", &at(2))]
        )),
        0
    );
    assert_eq!(
        code(&task(
            &sb,
            &["review", &id, "approve", "looks right"],
            "s-44",
            3
        )),
        0
    );
    let shown = stdout(&task(&sb, &["show", &id], "s-44", 4));
    assert!(shown.contains("ratchet:reviewer"), "{shown}");
    assert!(shown.contains(&agent_id[..8]), "{shown}");
}

// --- Requirement: Done requires an independent review ---------------------------------------

#[test]
fn tasks__done_refused_with_no_verdict() {
    let sb = board("s-23");
    let id = new_task(&sb, "needs review", &["only criterion"], "s-23", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-23", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-23", 3)), 0);
    let out = task(&sb, &["status", &id, "done"], "s-23", 4);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("no independent review"), "{err}");
    assert!(err.contains("ratchet task review"), "{err}");
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

#[test]
fn tasks__done_refused_when_the_approve_came_from_the_holding_session() {
    let sb = board("s-24");
    let id = new_task(&sb, "self reviewed", &["only criterion"], "s-24", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-24", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-24", 3)), 0);
    // No --session override: this verdict is recorded by s-24, which is also the holder.
    assert_eq!(
        code(&task(
            &sb,
            &["review", &id, "approve", "self-approved"],
            "s-24",
            4
        )),
        0
    );
    let out = task(&sb, &["status", &id, "done"], "s-24", 5);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("no independent review"), "{err}");
    // The disqualified verdict came from the holder itself, so naming the holder is naming the
    // verdict's own session here.
    assert!(err.contains("s-24"), "{err}");
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

#[test]
fn tasks__done_refused_when_the_approve_came_from_a_session_that_checked_off_an_item() {
    let sb = board("s-29");
    // s-prior is registered (join()) so this pins the "worked on it" refusal specifically, not
    // the separate "not registered" refusal added for T-0016.
    assert_eq!(code(&join(&sb, "s-prior", 1)), 0);
    let id = new_task(&sb, "history disqualifies", &["only criterion"], "s-29", 2);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-29", 3)), 0);
    // s-prior never holds the task, but it recorded the checklist.done earlier (e.g. before a
    // transfer) — close enough to the work to disqualify it as an independent reviewer.
    assert_eq!(
        code(&task(
            &sb,
            &["check", &id, "1", "--session", "s-prior"],
            "s-29",
            4
        )),
        0
    );
    assert_eq!(
        code(&task(
            &sb,
            &["review", &id, "approve", "fine", "--session", "s-prior"],
            "s-29",
            5
        )),
        0
    );
    let out = task(&sb, &["status", &id, "done"], "s-29", 6);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("no independent review"), "{err}");
    // The bug this pins: the message must name s-prior, the session that actually recorded the
    // disqualified verdict — not s-29, the current holder, who never reviewed anything.
    assert!(err.contains("s-prior"), "{err}");
    assert!(!err.contains("other than s-29"), "{err}");
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

#[test]
fn tasks__done_refused_when_the_approve_came_from_an_unregistered_session() {
    let sb = board("s-31");
    let id = new_task(&sb, "ghost approved", &["only criterion"], "s-31", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-31", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-31", 3)), 0);
    // s-ghost is never registered (no join()) — a hand-typed --session can still record a
    // verdict, but it must not satisfy the done gate (T-0016: no minting an arbitrary session
    // to self-approve).
    assert_eq!(
        code(&task(
            &sb,
            &[
                "review",
                &id,
                "approve",
                "looks fine",
                "--session",
                "s-ghost"
            ],
            "s-31",
            4
        )),
        0
    );
    let out = task(&sb, &["status", &id, "done"], "s-31", 5);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.to_lowercase().contains("not registered"), "{err}");
    assert!(err.to_lowercase().contains("claude code"), "{err}");
    assert!(err.contains("--unreviewed"), "{err}");
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

#[test]
fn tasks__done_allowed_after_an_approve_from_another_session() {
    let sb = board("s-25");
    // The reviewer session must be one ratchet itself registered (T-0016) — a hand-typed
    // --session that was never seen by the session-start hook does not satisfy the gate.
    assert_eq!(code(&join(&sb, "s-reviewer", 1)), 0);
    let id = new_task(&sb, "properly reviewed", &["only criterion"], "s-25", 2);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-25", 3)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-25", 4)), 0);
    let review_cmd = format!("ratchet task review {id} approve \"clean diff, gate green\"");
    assert_eq!(
        code(&task(
            &sb,
            &[
                "review",
                &id,
                "approve",
                "clean diff, gate green",
                "--session",
                "s-reviewer"
            ],
            "s-25",
            5
        )),
        0
    );
    // A bare-session approve needs the same transcript proof (H3) as a paired one — the spec's
    // own prose already required it for every approving identity (T-0016 review verdict).
    let root = sb.root();
    let tb = TranscriptBuilder::new();
    write_review_call_transcript(&tb, &root, "s-reviewer", None, &at(6), &review_cmd);
    let projects = tb.root.path().to_string_lossy().to_string();
    let out = cli(
        &sb,
        &["task", "status", &id, "done"],
        &root,
        &[
            ("RATCHET_SESSION_ID", "s-25"),
            ("RATCHET_NOW", &at(7)),
            ("RATCHET_CLAUDE_PROJECTS", &projects),
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).0, "done");
}

#[test]
fn tasks__done_refused_for_a_registered_bare_session_whose_transcript_lacks_the_review() {
    let sb = board("s-45");
    // A bare session hand-registered through the real session-start hook — exactly the shape
    // T-0016's review flagged: no Agent SDK process, no transcript, ever behind it. With no
    // agent identity, H3 (require_reviewer_transcript in cli/task_cmd.rs) returns early today
    // and never checks a bare identity's transcript at all, so this currently reaches `done`
    // with zero transcript ever written — the assertion below pins the required fix, not the
    // current behavior.
    assert_eq!(code(&join(&sb, "s-reviewer2", 1)), 0);
    let id = new_task(
        &sb,
        "bare reviewer with no proof",
        &["only criterion"],
        "s-45",
        2,
    );
    assert_eq!(code(&task(&sb, &["claim", &id], "s-45", 3)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-45", 4)), 0);
    assert_eq!(
        code(&task(
            &sb,
            &[
                "review",
                &id,
                "approve",
                "looks fine",
                "--session",
                "s-reviewer2"
            ],
            "s-45",
            5
        )),
        0
    );
    // No transcript at all is written for s-reviewer2 -- H3 has nothing to find.
    let tb = TranscriptBuilder::new();
    let projects = tb.root.path().to_string_lossy().to_string();
    let root = sb.root();
    let out = cli(
        &sb,
        &["task", "status", &id, "done"],
        &root,
        &[
            ("RATCHET_SESSION_ID", "s-45"),
            ("RATCHET_NOW", &at(6)),
            ("RATCHET_CLAUDE_PROJECTS", &projects),
        ],
    );
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.to_lowercase().contains("transcript"), "{err}");
    assert!(err.contains("task review"), "{err}");
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

#[test]
fn tasks__done_refused_when_a_changes_verdict_is_newer_than_the_approve() {
    let sb = board("s-26");
    let id = new_task(&sb, "flip flopped", &["only criterion"], "s-26", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-26", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-26", 3)), 0);
    assert_eq!(
        code(&task(
            &sb,
            &[
                "review",
                &id,
                "approve",
                "first pass ok",
                "--session",
                "s-reviewer"
            ],
            "s-26",
            4
        )),
        0
    );
    assert_eq!(
        code(&task(
            &sb,
            &[
                "review",
                &id,
                "changes",
                "actually, fix the edge case",
                "--session",
                "s-reviewer"
            ],
            "s-26",
            5
        )),
        0
    );
    let out = task(&sb, &["status", &id, "done"], "s-26", 6);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("\"changes\""), "{err}");
    // Spells out the literal next command, like the other two refusal branches.
    assert!(err.contains("ratchet task review"), "{err}");
    assert!(err.contains("approve"), "{err}");
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

#[test]
fn tasks__unreviewed_succeeds_and_records_the_note() {
    let sb = board("s-27");
    let id = new_task(&sb, "owner bypass", &["only criterion"], "s-27", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-27", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-27", 3)), 0);
    let out = task(&sb, &["status", &id, "done", "--unreviewed"], "s-27", 4);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).0, "done");
    let notes = payloads_of(&sb, &id, "note");
    assert!(
        notes
            .iter()
            .any(|n| n.contains("done without independent review")),
        "{notes:?}"
    );
}

/// Slug ratchet uses for a working directory under the Claude "projects" tree — mirrors
/// `TranscriptBuilder`'s own, private slug (`support.rs`); duplicated here rather than exposed
/// from there, since a scenario-test file only adds its own tests, never touches shared fixture
/// code.
fn projects_slug(cwd: &std::path::Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Appends one assistant record whose only content is a `Bash` `tool_use` carrying `command` in
/// `input.command`, to the session's own transcript (`agent_id: None`) or to a subagent's
/// (`<slug>/<session>/subagents/agent-<id>.jsonl`) — the shape H3 reads (a real Claude Code
/// transcript's tool_use block carries its `input`; `TranscriptBuilder::call_with_tool` does not,
/// since no scenario before this one needed the command text itself).
fn write_review_call_transcript(
    tb: &TranscriptBuilder,
    cwd: &std::path::Path,
    session_id: &str,
    agent_id: Option<&str>,
    ts: &str,
    command: &str,
) {
    let slug = projects_slug(cwd);
    let path = match agent_id {
        None => tb
            .root
            .path()
            .join(&slug)
            .join(format!("{session_id}.jsonl")),
        Some(a) => tb
            .root
            .path()
            .join(&slug)
            .join(session_id)
            .join("subagents")
            .join(format!("agent-{a}.jsonl")),
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let rec = json!({
        "type": "assistant",
        "timestamp": ts,
        "sessionId": session_id,
        "cwd": cwd.to_string_lossy(),
        "gitBranch": "main",
        "version": "1.2.3",
        "message": {
            "model": "claude-sonnet-5",
            "content": [{
                "type": "tool_use",
                "id": "tu-transcript",
                "name": "Bash",
                "input": { "command": command },
            }],
            "usage": {
                "input_tokens": 1,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
                "output_tokens": 1,
            },
        },
    });
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(f, "{rec}").unwrap();
}

#[test]
fn tasks__done_allowed_for_a_reviewer_subagent_that_never_worked_the_task() {
    let sb = board("s-40");
    let id = new_task(
        &sb,
        "reviewed by a subagent",
        &["only criterion"],
        "s-40",
        1,
    );
    assert_eq!(code(&task(&sb, &["claim", &id], "s-40", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-40", 3)), 0);
    // A reviewer subagent starts in the holding session -- it never claims or checks anything.
    seed_subagent_event(
        &sb,
        "subagent.start",
        "s-40",
        "rev-1",
        "ratchet:reviewer",
        "review pass",
        &id,
        &at(4),
    );
    let root = sb.root();
    let review_cmd = format!("ratchet task review {id} approve \"looks good\"");
    let call = board_write_call(
        &review_cmd,
        &root,
        "tu-r1",
        Some(("rev-1", "ratchet:reviewer")),
    );
    assert_eq!(
        code(&hook_env(
            &sb,
            "pre-tool",
            &call,
            &root,
            &[("RATCHET_SESSION_ID", "s-40"), ("RATCHET_NOW", &at(5))]
        )),
        0
    );
    assert_eq!(
        code(&task(
            &sb,
            &["review", &id, "approve", "looks good"],
            "s-40",
            6
        )),
        0
    );
    let payloads = payloads_of(&sb, &id, "review.verdict");
    assert!(
        payloads[0].contains("rev-1"),
        "verdict not attributed to the subagent: {payloads:?}"
    );

    let tb = TranscriptBuilder::new();
    write_review_call_transcript(&tb, &root, "s-40", Some("rev-1"), &at(7), &review_cmd);
    let projects = tb.root.path().to_string_lossy().to_string();

    let out = cli(
        &sb,
        &["task", "status", &id, "done"],
        &root,
        &[
            ("RATCHET_SESSION_ID", "s-40"),
            ("RATCHET_NOW", &at(8)),
            ("RATCHET_CLAUDE_PROJECTS", &projects),
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).0, "done");
}

#[test]
fn tasks__done_refused_for_the_implementer_subagent_that_checked_items() {
    let sb = board("s-41");
    let id = new_task(
        &sb,
        "self reviewed by its own subagent",
        &["only criterion"],
        "s-41",
        1,
    );
    assert_eq!(code(&task(&sb, &["claim", &id], "s-41", 2)), 0);
    let root = sb.root();
    // The implementer subagent checks the item -- attributed to (s-41, impl-1).
    let check_call = board_write_call(
        &format!("ratchet task check {id} 1"),
        &root,
        "tu-c1",
        Some(("impl-1", "ratchet:implementer")),
    );
    assert_eq!(
        code(&hook_env(
            &sb,
            "pre-tool",
            &check_call,
            &root,
            &[("RATCHET_SESSION_ID", "s-41"), ("RATCHET_NOW", &at(3))]
        )),
        0
    );
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-41", 4)), 0);
    assert!(payloads_of(&sb, &id, "checklist.done")[0].contains("impl-1"));

    // The same subagent identity later "approves" its own work.
    let review_cmd = format!("ratchet task review {id} approve \"fine\"");
    let review_call = board_write_call(
        &review_cmd,
        &root,
        "tu-r2",
        Some(("impl-1", "ratchet:implementer")),
    );
    assert_eq!(
        code(&hook_env(
            &sb,
            "pre-tool",
            &review_call,
            &root,
            &[("RATCHET_SESSION_ID", "s-41"), ("RATCHET_NOW", &at(5))]
        )),
        0
    );
    assert_eq!(
        code(&task(&sb, &["review", &id, "approve", "fine"], "s-41", 6)),
        0
    );
    assert!(payloads_of(&sb, &id, "review.verdict")[0].contains("impl-1"));

    let tb = TranscriptBuilder::new();
    write_review_call_transcript(&tb, &root, "s-41", Some("impl-1"), &at(7), &review_cmd);
    let projects = tb.root.path().to_string_lossy().to_string();

    let out = cli(
        &sb,
        &["task", "status", &id, "done"],
        &root,
        &[
            ("RATCHET_SESSION_ID", "s-41"),
            ("RATCHET_NOW", &at(8)),
            ("RATCHET_CLAUDE_PROJECTS", &projects),
        ],
    );
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("no independent review"), "{err}");
    assert!(
        err.contains("impl-1"),
        "must name the disqualified agent: {err}"
    );
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

#[test]
fn tasks__done_refused_for_the_orchestrator_bare_session_that_claimed() {
    let sb = board("s-42");
    let id = new_task(
        &sb,
        "orchestrator did everything itself",
        &["only criterion"],
        "s-42",
        1,
    );
    let root = sb.root();
    // The claim itself runs from the main thread: its own pre-tool call carries no agent identity.
    let claim_call = board_write_call(&format!("ratchet task claim {id}"), &root, "tu-c2", None);
    assert_eq!(
        code(&hook_env(
            &sb,
            "pre-tool",
            &claim_call,
            &root,
            &[("RATCHET_SESSION_ID", "s-42"), ("RATCHET_NOW", &at(2))]
        )),
        0
    );
    assert_eq!(code(&task(&sb, &["claim", &id], "s-42", 3)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-42", 4)), 0);

    // The orchestrator itself later tries to approve its own task, again from the main thread.
    let review_call = board_write_call(
        &format!("ratchet task review {id} approve \"self-approved\""),
        &root,
        "tu-r3",
        None,
    );
    assert_eq!(
        code(&hook_env(
            &sb,
            "pre-tool",
            &review_call,
            &root,
            &[("RATCHET_SESSION_ID", "s-42"), ("RATCHET_NOW", &at(5))]
        )),
        0
    );
    assert_eq!(
        code(&task(
            &sb,
            &["review", &id, "approve", "self-approved"],
            "s-42",
            6
        )),
        0
    );
    assert!(!payloads_of(&sb, &id, "review.verdict")[0].contains("agent_id"));

    let tb = TranscriptBuilder::new();
    let projects = tb.root.path().to_string_lossy().to_string();
    let out = cli(
        &sb,
        &["task", "status", &id, "done"],
        &root,
        &[
            ("RATCHET_SESSION_ID", "s-42"),
            ("RATCHET_NOW", &at(7)),
            ("RATCHET_CLAUDE_PROJECTS", &projects),
        ],
    );
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("no independent review"), "{err}");
    assert!(err.contains("s-42"), "{err}");
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

#[test]
fn tasks__done_refused_when_the_reviewer_identity_has_no_transcript_containing_the_review_command()
{
    let sb = board("s-43");
    let id = new_task(
        &sb,
        "reviewer with no proof",
        &["only criterion"],
        "s-43",
        1,
    );
    assert_eq!(code(&task(&sb, &["claim", &id], "s-43", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-43", 3)), 0);
    seed_subagent_event(
        &sb,
        "subagent.start",
        "s-43",
        "rev-2",
        "ratchet:reviewer",
        "review pass",
        &id,
        &at(4),
    );
    let root = sb.root();
    let call = board_write_call(
        &format!("ratchet task review {id} approve \"looks fine\""),
        &root,
        "tu-r4",
        Some(("rev-2", "ratchet:reviewer")),
    );
    assert_eq!(
        code(&hook_env(
            &sb,
            "pre-tool",
            &call,
            &root,
            &[("RATCHET_SESSION_ID", "s-43"), ("RATCHET_NOW", &at(5))]
        )),
        0
    );
    assert_eq!(
        code(&task(
            &sb,
            &["review", &id, "approve", "looks fine"],
            "s-43",
            6
        )),
        0
    );
    assert!(payloads_of(&sb, &id, "review.verdict")[0].contains("rev-2"));

    // No transcript at all is written for rev-2 -- H3 has nothing to find.
    let tb = TranscriptBuilder::new();
    let projects = tb.root.path().to_string_lossy().to_string();
    let out = cli(
        &sb,
        &["task", "status", &id, "done"],
        &root,
        &[
            ("RATCHET_SESSION_ID", "s-43"),
            ("RATCHET_NOW", &at(7)),
            ("RATCHET_CLAUDE_PROJECTS", &projects),
        ],
    );
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.to_lowercase().contains("transcript"), "{err}");
    assert!(err.contains("task review"), "{err}");
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

// --- Requirement: Archiving hides, it never deletes -----------------------------------------

#[test]
fn tasks__only_a_done_task_is_archived() {
    let sb = board("s-19");
    let id = new_task(&sb, "still open", &[], "s-19", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-19", 2)), 0);
    let out = task(&sb, &["archive", &id], "s-19", 3);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("done"), "{}", stderr(&out));
    assert_eq!(task_state(&sb, &id).2, None);
}

#[test]
fn tasks__an_archived_task_leaves_the_listing() {
    let sb = board("s-20");
    let id = new_task(&sb, "finished work", &[], "s-20", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-20", 2)), 0);
    assert_eq!(
        code(&task(
            &sb,
            &["status", &id, "done", "--why", "shipped", "--unreviewed"],
            "s-20",
            3
        )),
        0
    );
    assert_eq!(code(&task(&sb, &["archive", &id], "s-20", 4)), 0);
    assert!(task_state(&sb, &id).2.is_some());
    let default_list = stdout(&task(&sb, &["list"], "s-20", 5));
    assert!(!default_list.contains(&id), "{default_list}");
    let all = stdout(&task(&sb, &["list", "--all"], "s-20", 6));
    assert!(all.contains(&id), "{all}");
    let shown = stdout(&task(&sb, &["show", &id], "s-20", 7));
    assert!(shown.contains("shipped"), "history lost: {shown}");
}

// --- Requirement: Every change leaves an event ----------------------------------------------

#[test]
fn tasks__checking_an_item_leaves_an_event() {
    let sb = board("s-21");
    let id = new_task(&sb, "one item", &["only criterion"], "s-21", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-21", 2)), 0);
    let before = kinds_of(&sb, &id);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-21", 3)), 0);
    assert_eq!(
        code(&task(&sb, &["check", &id, "1", "--undo"], "s-21", 4)),
        0
    );
    let after = kinds_of(&sb, &id);
    assert_eq!(after[..before.len()], before[..], "history was rewritten");
    assert_eq!(
        after[before.len()..],
        ["checklist.done".to_string(), "checklist.undone".to_string()]
    );
}
