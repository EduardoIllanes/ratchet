//! One test per `#### Scenario:` of `openspec/specs/sessions/spec.md`, named `sessions__<slug>`.
//! Every test drives the real binary as a subprocess, exactly as the harness would.

use crate::support::*;
use serde_json::json;

// --- One local state database --------------------------------------------------------------

#[test]
fn sessions__first_hook_creates_the_database() {
    let sb = sandbox();
    assert!(!db_file(&sb).exists());
    let out = start_session(&sb, "s-1", &sb.root(), T0);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(db_file(&sb).is_file(), "no database after session-start");
    assert_eq!(
        count(&sb, "SELECT COUNT(*) FROM sessions WHERE id = ?1", &["s-1"]),
        1
    );
}

#[test]
fn sessions__migrations_apply_once() {
    let sb = sandbox();
    let first = cli(&sb, &["db", "migrate"], &sb.root(), &[]);
    assert_eq!(code(&first), 0, "stderr: {}", stderr(&first));
    assert!(
        stdout(&first).contains("0001_init"),
        "got: {}",
        stdout(&first)
    );
    let second = cli(&sb, &["db", "migrate"], &sb.root(), &[]);
    assert_eq!(code(&second), 0);
    assert!(
        stdout(&second).contains("already at version 1"),
        "got: {}",
        stdout(&second)
    );
}

#[test]
fn sessions__the_database_path_is_printable() {
    let sb = sandbox();
    let out = cli(&sb, &["db", "path"], &sb.root(), &[]);
    assert_eq!(code(&out), 0);
    let printed = stdout(&out).trim().to_lowercase();
    assert!(printed.ends_with("ratchet.db"), "got: {printed}");
    assert!(
        printed.contains(&sb.home.path().to_string_lossy().to_lowercase()),
        "got: {printed}"
    );
}

#[test]
fn sessions__a_stale_schema_is_reported_not_migrated() {
    let sb = sandbox();
    // An empty file that is a valid, empty database: schema version 0, older than this build.
    drop(rusqlite::Connection::open(db_file(&sb)).unwrap());
    let out = cli(&sb, &["session", "list"], &sb.root(), &[]);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(
        stderr(&out).contains("ratchet db migrate"),
        "got: {}",
        stderr(&out)
    );
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
            &["sessions"]
        ),
        0,
        "the stale database was migrated anyway"
    );
}

#[test]
fn sessions__the_guardrail_hook_opens_no_database() {
    let sb = sandbox();
    let out = hook_env(
        &sb,
        "pre-tool",
        &bash("python scripts/x.py", &sb.root()),
        &sb.root(),
        &[],
    );
    assert_eq!(code(&out), 2, "stderr: {}", stderr(&out));
    assert!(!db_file(&sb).exists(), "pre-tool created the database");
}

// --- Session registry ----------------------------------------------------------------------

#[test]
fn sessions__idempotent_registration() {
    let sb = sandbox();
    start_session(&sb, "s-6", &sb.root(), T0);
    let deep = sb.root().join("src").join("deep");
    let out = start_session(&sb, "s-6", &deep, &at(5));
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(
        count(&sb, "SELECT COUNT(*) FROM sessions WHERE id = ?1", &["s-6"]),
        1
    );
    assert_eq!(last_seen(&sb, "s-6"), at(5));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'session.start'",
            &["s-6"]
        ),
        2
    );
}

#[test]
fn sessions__a_session_in_a_worktree_records_branch_and_worktree() {
    let sb = sandbox();
    let wt = worktree(&sb);
    let out = start_session(&sb, "s-7", &wt, T0);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let shown = stdout(&cli(&sb, &["session", "show", "s-7"], &sb.root(), &[])).to_lowercase();
    assert!(shown.contains("branch wt"), "got: {shown}");
    assert!(
        shown.contains(&wt.to_string_lossy().to_lowercase()),
        "got: {shown}"
    );
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM sessions WHERE id = ?1 AND worktree IS NOT NULL",
            &["s-7"]
        ),
        1
    );
}

#[test]
fn sessions__a_headless_session_launched_by_the_platform() {
    let sb = sandbox();
    // No session_id in the payload: the launcher fixed the identity in the environment.
    let out = hook_env(
        &sb,
        "session-start",
        &json!({ "cwd": sb.root().to_string_lossy() }),
        &sb.root(),
        &[
            ("RATCHET_NOW", T0),
            ("RATCHET_SESSION_ID", "s-8"),
            ("RATCHET_SESSION_MODE", "headless"),
            ("RATCHET_LAUNCHED_BY", "platform"),
        ],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let shown = stdout(&cli(&sb, &["session", "show", "s-8"], &sb.root(), &[]));
    assert!(shown.contains("headless"), "got: {shown}");
    assert!(shown.contains("platform"), "got: {shown}");
}

#[test]
fn sessions__the_session_id_reaches_the_shell() {
    let sb = sandbox();
    let env_file = sb.scratchpad.path().join("env.sh");
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-9", &sb.root()),
        &sb.root(),
        &[
            ("RATCHET_NOW", T0),
            ("CLAUDE_ENV_FILE", &env_file.to_string_lossy()),
        ],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let text = std::fs::read_to_string(&env_file).unwrap();
    assert!(
        text.contains("export RATCHET_SESSION_ID=s-9"),
        "got: {text}"
    );
}

#[test]
fn sessions__no_marker_no_session() {
    let sb = sandbox();
    let outside = unmanaged_dir();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-10", outside.path()),
        outside.path(),
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(!db_file(&sb).exists());
}

// --- Heartbeat -----------------------------------------------------------------------------

#[test]
fn sessions__a_prompt_updates_the_last_signal() {
    let sb = sandbox();
    start_session(&sb, "s-11", &sb.root(), T0);
    let out = hook_env(
        &sb,
        "prompt",
        &session_payload("s-11", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", &at(5))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(last_seen(&sb, "s-11"), at(5));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'session.prompt'",
            &["s-11"]
        ),
        1
    );
}

#[test]
fn sessions__compaction_and_subagent_end_update_the_last_signal() {
    let sb = sandbox();
    start_session(&sb, "s-12", &sb.root(), T0);
    for (event, when) in [("pre-compact", at(10)), ("subagent-stop", at(15))] {
        let out = hook_env(
            &sb,
            event,
            &session_payload("s-12", &sb.root()),
            &sb.root(),
            &[("RATCHET_NOW", &when)],
        );
        assert_eq!(code(&out), 0, "{event}: {}", stderr(&out));
    }
    assert_eq!(last_seen(&sb, "s-12"), at(15));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind <> 'session.start'",
            &["s-12"]
        ),
        0
    );
}

#[test]
fn sessions__a_hook_of_an_unregistered_session_registers_it() {
    let sb = sandbox();
    // Another session created the database first; this one never saw a session-start.
    start_session(&sb, "s-13a", &sb.root(), T0);
    let out = hook_env(
        &sb,
        "prompt",
        &session_payload("s-13b", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", &at(2))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM sessions WHERE id = ?1",
            &["s-13b"]
        ),
        1
    );
    assert_eq!(last_seen(&sb, "s-13b"), at(2));
}

// --- Derived session state ------------------------------------------------------------------

#[test]
fn sessions__a_session_with_no_signal_for_ninety_minutes_is_orphaned() {
    let sb = sandbox();
    start_session(&sb, "s-14", &sb.root(), T0);
    let out = cli(
        &sb,
        &["session", "list"],
        &sb.root(),
        &[("RATCHET_NOW", &at(90))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("orphaned"), "got: {}", stdout(&out));
}

#[test]
fn sessions__an_ended_session_stays_ended() {
    let sb = sandbox();
    start_session(&sb, "s-15", &sb.root(), T0);
    hook_env(
        &sb,
        "session-end",
        &session_payload("s-15", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", &at(1))],
    );
    let out = cli(
        &sb,
        &["session", "list"],
        &sb.root(),
        &[("RATCHET_NOW", &at(2))],
    );
    assert!(stdout(&out).contains("ended"), "got: {}", stdout(&out));
    assert!(!stdout(&out).contains("live"), "got: {}", stdout(&out));
}

#[test]
fn sessions__the_repo_sets_its_own_thresholds() {
    let sb = sandbox();
    sb.write_marker(
        "[repo]\nworktrees_dir = \".worktrees\"\n[thresholds]\nlive_minutes = 1\nidle_minutes = 2\n",
    );
    start_session(&sb, "s-16", &sb.root(), T0);
    let out = cli(
        &sb,
        &["session", "list"],
        &sb.root(),
        &[("RATCHET_NOW", &at_secs(90))],
    );
    assert!(stdout(&out).contains("idle"), "got: {}", stdout(&out));
}

// --- Session end ----------------------------------------------------------------------------

#[test]
fn sessions__session_end_marks_the_session_ended() {
    let sb = sandbox();
    start_session(&sb, "s-17", &sb.root(), T0);
    let out = hook_env(
        &sb,
        "session-end",
        &session_payload("s-17", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", &at(3))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM sessions WHERE id = ?1 AND ended_at IS NOT NULL",
            &["s-17"]
        ),
        1
    );
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'session.end'",
            &["s-17"]
        ),
        1
    );
}

// --- Work of a dead session goes back ---------------------------------------------------------

#[test]
fn sessions__session_end_returns_its_tasks() {
    let sb = sandbox();
    start_session(&sb, "s-18", &sb.root(), T0);
    seed_task(&sb, "T-0001", "in_progress", Some("s-18"));
    let out = hook_env(
        &sb,
        "session-end",
        &session_payload("s-18", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", &at(1))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(task_row(&sb, "T-0001"), ("ready".to_string(), None));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE task_id = ?1 AND kind = 'task.status'",
            &["T-0001"]
        ),
        1
    );
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE task_id = ?1 AND kind = 'note'",
            &["T-0001"]
        ),
        1
    );
}

#[test]
fn sessions__a_new_session_releases_tasks_of_a_dead_one() {
    let sb = sandbox();
    start_session(&sb, "s-19a", &sb.root(), T0);
    seed_task(&sb, "T-0002", "in_progress", Some("s-19a"));
    let out = start_session(&sb, "s-19b", &sb.root(), &at(90));
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(task_row(&sb, "T-0002"), ("ready".to_string(), None));
}

#[test]
fn sessions__a_task_held_by_a_live_session_is_not_released() {
    let sb = sandbox();
    start_session(&sb, "s-20a", &sb.root(), T0);
    seed_task(&sb, "T-0003", "in_progress", Some("s-20a"));
    let out = start_session(&sb, "s-20b", &sb.root(), &at(1));
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(
        task_row(&sb, "T-0003"),
        ("in_progress".to_string(), Some("s-20a".to_string()))
    );
}

#[test]
fn sessions__a_task_claimed_by_a_session_the_registry_never_saw_is_released() {
    let sb = sandbox();
    start_session(&sb, "s-25a", &sb.root(), T0);
    // "s-ghost" has no row in the registry at all: never registered, not ended, not orphaned by
    // signal age -- just gone. The sweep at the next session's start must treat that the same as
    // the other two dead cases.
    seed_task(&sb, "T-0004", "in_progress", Some("s-ghost"));
    let out = start_session(&sb, "s-25b", &sb.root(), &at(1));
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(task_row(&sb, "T-0004"), ("ready".to_string(), None));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE task_id = ?1 AND kind = 'task.status'",
            &["T-0004"]
        ),
        1
    );
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE task_id = ?1 AND kind = 'note'",
            &["T-0004"]
        ),
        1
    );
}

// --- Listing and showing ----------------------------------------------------------------------

#[test]
fn sessions__listing_shows_the_derived_state() {
    let sb = sandbox();
    start_session(&sb, "s-21a", &sb.root(), T0);
    start_session(&sb, "s-21b", &sb.root(), &at(90));
    let all = cli(
        &sb,
        &["session", "list"],
        &sb.root(),
        &[("RATCHET_NOW", &at(90))],
    );
    let text = stdout(&all);
    assert!(text.contains("s-21a"), "got: {text}");
    assert!(text.contains("s-21b"), "got: {text}");
    assert!(text.contains("orphaned"), "got: {text}");
    let live = cli(
        &sb,
        &["session", "list", "--live"],
        &sb.root(),
        &[("RATCHET_NOW", &at(90))],
    );
    let text = stdout(&live);
    assert!(text.contains("s-21b"), "got: {text}");
    assert!(!text.contains("s-21a"), "got: {text}");
}

#[test]
fn sessions__showing_an_unknown_session_fails_with_a_message() {
    let sb = sandbox();
    start_session(&sb, "s-22", &sb.root(), T0);
    let out = cli(&sb, &["session", "show", "nope"], &sb.root(), &[]);
    assert_eq!(code(&out), 1, "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("nope"), "got: {}", stderr(&out));
}

#[test]
fn sessions__showing_without_an_id_resolves_the_session_of_the_directory() {
    let sb = sandbox();
    start_session(&sb, "s-23", &sb.root(), T0);
    let deep = sb.root().join("src").join("deep");
    let out = cli(&sb, &["session", "show"], &deep, &[("RATCHET_NOW", &at(1))]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("s-23"), "got: {}", stdout(&out));
}

// --- Never break a session ----------------------------------------------------------------------

#[test]
fn sessions__an_unusable_database_does_not_break_the_session() {
    let sb = sandbox();
    std::fs::create_dir_all(sb.home.path()).unwrap();
    std::fs::write(db_file(&sb), b"this is not a database").unwrap();
    let out = hook_env(
        &sb,
        "prompt",
        &session_payload("s-24", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(
        sb.log_text().lines().count() >= 1,
        "log: {:?}",
        sb.log_text()
    );
}
