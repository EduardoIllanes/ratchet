//! One test per `#### Scenario` of openspec/specs/tasks/spec.md, named by slug. These tests drive
//! the real binary as a subprocess; they read and seed the database directly because they stand
//! outside the binary, which production code never does outside `services/`.

use crate::support::*;

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
            &["status", &first, "done", "--why", "no criteria"],
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
    let accepted = task(&sb, &["status", &id, "done", "--why", "obsolete"], "s-7", 4);
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
            &["status", &id, "done", "--why", "shipped"],
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
