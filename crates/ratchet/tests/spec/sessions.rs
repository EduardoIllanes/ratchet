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
    // Derive the reached version from the first run's own output instead of hardcoding the
    // latest migration number, so adding a migration doesn't make this scenario stale.
    let first_out = stdout(&first);
    let reached_version = first_out
        .lines()
        .find_map(|l| l.strip_prefix("now at version "))
        .unwrap_or_else(|| panic!("no \"now at version\" line: {first_out}"));

    let second = cli(&sb, &["db", "migrate"], &sb.root(), &[]);
    assert_eq!(code(&second), 0);
    assert_eq!(
        stdout(&second).trim(),
        format!("already at version {reached_version}"),
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
fn sessions__compaction_updates_the_last_signal_with_no_event() {
    let sb = sandbox();
    start_session(&sb, "s-12", &sb.root(), T0);
    let out = hook_env(
        &sb,
        "pre-compact",
        &session_payload("s-12", &sb.root()),
        &sb.root(),
        &[("RATCHET_NOW", &at(10))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(last_seen(&sb, "s-12"), at(10));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind <> 'session.start'",
            &["s-12"]
        ),
        0
    );
}

// --- Subagent start and stop ---------------------------------------------------------------

/// Payload of a subagent hook: the session-scoped base plus the agent identity the hook
/// carries. `extra` holds the agent keys (`agent_type`, `description`, `transcript_path`,
/// `exit_status`) the scenario under test offers.
fn subagent_payload(
    session: &str,
    root: &std::path::Path,
    agent_id: &str,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut payload = json!({
        "session_id": session,
        "cwd": root.to_string_lossy(),
        "agent_id": agent_id,
    });
    for (k, v) in extra.as_object().unwrap() {
        payload[k] = v.clone();
    }
    payload
}

fn recorded_payload(sb: &Sandbox, session: &str, kind: &str) -> serde_json::Value {
    let payloads = session_event_payloads(sb, session, kind);
    assert_eq!(payloads.len(), 1, "{kind}: {payloads:?}");
    serde_json::from_str(&payloads[0]).expect("recorded payload is JSON")
}

#[test]
fn sessions__a_subagent_start_records_a_start_event() {
    let session = "s-sub-start";
    let sb = board(session);
    let id = new_task(&sb, "held work", &["a"], session, 1);
    assert_eq!(code(&task(&sb, &["claim", &id], session, 2)), 0);
    let transcript = sb.scratchpad.path().join("transcripts").join("t.jsonl");
    let agent = "a1b2c3d4e5f6";
    let when = at(3);
    let root = sb.root();
    let payload = subagent_payload(
        session,
        &root,
        agent,
        json!({
            "agent_type": "explore",
            "description": "reconnoitre",
            "transcript_path": transcript.to_string_lossy(),
        }),
    );
    let out = hook_env(
        &sb,
        "subagent-start",
        &payload,
        &root,
        &[("RATCHET_NOW", &when)],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(last_seen(&sb, session), when);
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'subagent.start'",
            &[session]
        ),
        1
    );
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'subagent.start' AND task_id = ?2",
            &[session, id.as_str()]
        ),
        1,
        "the start event was not attributed to the held task"
    );
    let recorded = recorded_payload(&sb, session, "subagent.start");
    assert_eq!(recorded["agent_id"], agent);
    assert_eq!(recorded["agent_type"], "explore");
    assert_eq!(recorded["description"], "reconnoitre");
    assert!(
        recorded["transcript_path"]
            .as_str()
            .unwrap()
            .contains("t.jsonl"),
        "{recorded}"
    );
}

#[test]
fn sessions__a_subagent_stop_records_a_stop_event() {
    let session = "s-sub-stop";
    let sb = board(session);
    let id = new_task(&sb, "held work", &["a"], session, 1);
    assert_eq!(code(&task(&sb, &["claim", &id], session, 2)), 0);
    let transcript = sb.scratchpad.path().join("transcripts").join("t.jsonl");
    let agent = "b1c2d3e4f5a6";
    let when = at(3);
    let root = sb.root();
    let payload = subagent_payload(
        session,
        &root,
        agent,
        json!({
            "agent_type": "explore",
            "description": "reconnoitre",
            "transcript_path": transcript.to_string_lossy(),
            "exit_status": 0,
        }),
    );
    let out = hook_env(
        &sb,
        "subagent-stop",
        &payload,
        &root,
        &[("RATCHET_NOW", &when)],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(last_seen(&sb, session), when);
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'subagent.stop' AND task_id = ?2",
            &[session, id.as_str()]
        ),
        1,
        "the stop event was not attributed to the held task"
    );
    let recorded = recorded_payload(&sb, session, "subagent.stop");
    assert_eq!(recorded["agent_id"], agent);
    assert_eq!(recorded["agent_type"], "explore");
    assert!(
        recorded["transcript_path"]
            .as_str()
            .unwrap()
            .contains("t.jsonl"),
        "{recorded}"
    );
    assert_eq!(recorded["exit_status"], 0);
}

#[test]
fn sessions__a_stop_with_no_held_task_records_a_session_level_event() {
    let session = "s-sub-lonely";
    let sb = board(session);
    let _ = new_task(&sb, "nobody claimed me", &[], session, 1);
    let transcript = sb.scratchpad.path().join("transcripts").join("t.jsonl");
    let agent = "c1d2e3f4a5b6";
    let root = sb.root();
    let payload = subagent_payload(
        session,
        &root,
        agent,
        json!({
            "agent_type": "explore",
            "transcript_path": transcript.to_string_lossy(),
            "exit_status": 0,
        }),
    );
    let out = hook_env(
        &sb,
        "subagent-stop",
        &payload,
        &root,
        &[("RATCHET_NOW", &at(2))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'subagent.stop'",
            &[session]
        ),
        1
    );
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'subagent.stop' AND task_id IS NULL",
            &[session]
        ),
        1,
        "the stop event should carry no task"
    );
}

#[test]
fn sessions__a_missing_type_falls_back_to_the_meta_file() {
    let session = "s-sub-meta";
    let sb = board(session);
    let id = new_task(&sb, "held work", &["a"], session, 1);
    assert_eq!(code(&task(&sb, &["claim", &id], session, 2)), 0);
    let agent = "d1e2f3a4b5c6";
    let transcript_dir = sb.scratchpad.path().join("transcripts");
    let transcript = transcript_dir.join("t.jsonl");
    let meta_path = transcript_dir
        .join(session)
        .join("subagents")
        .join(format!("agent-{agent}.meta.json"));
    std::fs::create_dir_all(meta_path.parent().unwrap()).unwrap();
    std::fs::write(
        &meta_path,
        r#"{"agentType": "bash-runner", "description": "shell work"}"#,
    )
    .unwrap();
    let root = sb.root();
    // No type and no description on the input: both must come from the meta file.
    let payload = subagent_payload(
        session,
        &root,
        agent,
        json!({ "transcript_path": transcript.to_string_lossy(), "exit_status": 0 }),
    );
    let out = hook_env(
        &sb,
        "subagent-stop",
        &payload,
        &root,
        &[("RATCHET_NOW", &at(3))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let recorded = recorded_payload(&sb, session, "subagent.stop");
    assert_eq!(recorded["agent_id"], agent);
    assert_eq!(recorded["agent_type"], "bash-runner", "{recorded}");
    assert_eq!(recorded["description"], "shell work", "{recorded}");
    assert_eq!(
        std::fs::read_to_string(&meta_path).unwrap(),
        r#"{"agentType": "bash-runner", "description": "shell work"}"#,
        "the hook must only read the meta file, never write it"
    );
}

#[test]
fn sessions__a_missing_meta_file_still_records() {
    let session = "s-sub-nometa";
    let sb = board(session);
    let id = new_task(&sb, "held work", &["a"], session, 1);
    assert_eq!(code(&task(&sb, &["claim", &id], session, 2)), 0);
    let agent = "e1f2a3b4c5d6";
    // A transcript directory with no meta file anywhere beside it.
    let transcript = sb.scratchpad.path().join("empty").join("t.jsonl");
    let root = sb.root();
    let payload = subagent_payload(
        session,
        &root,
        agent,
        json!({ "transcript_path": transcript.to_string_lossy(), "exit_status": 0 }),
    );
    let out = hook_env(
        &sb,
        "subagent-stop",
        &payload,
        &root,
        &[("RATCHET_NOW", &at(3))],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let recorded = recorded_payload(&sb, session, "subagent.stop");
    assert_eq!(recorded["agent_id"], agent);
    assert!(
        recorded
            .get("agent_type")
            .map(|v| v.is_null())
            .unwrap_or(true),
        "no type is known: {recorded}"
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

// --- A subagent's board write is attributed to it -------------------------------------------

/// A PreToolUse-shaped `Bash` payload for a board write: `tool_input.command` is the write's
/// full command text, `tool_use_id` is the call's own identifier (the join key pre-tool and
/// post-tool share), and `agent` carries `(agent_id, agent_type)` for a subagent's call or is
/// `None` for the main thread's own call. `pub(crate)` so `tasks.rs` can drive the same
/// mechanism for the done-gate scenarios without duplicating it.
pub(crate) fn board_write_call(
    command: &str,
    cwd: &std::path::Path,
    tool_use_id: &str,
    agent: Option<(&str, &str)>,
) -> serde_json::Value {
    let mut payload = json!({
        "tool_name": "Bash",
        "tool_input": { "command": command },
        "cwd": cwd.to_string_lossy(),
        "tool_use_id": tool_use_id,
    });
    if let Some((agent_id, agent_type)) = agent {
        payload["agent_id"] = json!(agent_id);
        payload["agent_type"] = json!(agent_type);
    }
    payload
}

#[test]
fn sessions__a_board_write_inside_a_single_matching_subagent_call_is_attributed_to_it() {
    let session = "s-attr-1";
    let sb = board(session);
    let id = new_task(
        &sb,
        "attributed to its subagent",
        &["only criterion"],
        session,
        1,
    );
    assert_eq!(code(&task(&sb, &["claim", &id], session, 2)), 0);
    let root = sb.root();
    let agent_id = "a1b2c3d4e5f6";
    let call = board_write_call(
        &format!("ratchet task check {id} 1"),
        &root,
        "tu-1",
        Some((agent_id, "ratchet:implementer")),
    );
    let pre = hook_env(
        &sb,
        "pre-tool",
        &call,
        &root,
        &[("RATCHET_SESSION_ID", session), ("RATCHET_NOW", &at(3))],
    );
    assert_eq!(code(&pre), 0, "pre-tool stderr: {}", stderr(&pre));
    let out = task(&sb, &["check", &id, "1"], session, 4);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let payloads = payloads_of(&sb, &id, "checklist.done");
    assert_eq!(payloads.len(), 1);
    assert!(
        payloads[0].contains(agent_id),
        "not attributed to the subagent: {payloads:?}"
    );
    assert!(payloads[0].contains("ratchet:implementer"), "{payloads:?}");
}

#[test]
fn sessions__a_main_thread_call_stays_attributed_to_the_bare_session() {
    let session = "s-attr-2";
    let sb = board(session);
    let id = new_task(&sb, "main thread write", &["only criterion"], session, 1);
    assert_eq!(code(&task(&sb, &["claim", &id], session, 2)), 0);
    let root = sb.root();
    let call = board_write_call(&format!("ratchet task check {id} 1"), &root, "tu-2", None);
    let pre = hook_env(
        &sb,
        "pre-tool",
        &call,
        &root,
        &[("RATCHET_SESSION_ID", session), ("RATCHET_NOW", &at(3))],
    );
    assert_eq!(code(&pre), 0, "pre-tool stderr: {}", stderr(&pre));
    let out = task(&sb, &["check", &id, "1"], session, 4);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let payloads = payloads_of(&sb, &id, "checklist.done");
    assert_eq!(payloads.len(), 1);
    assert!(
        !payloads[0].contains("agent_id"),
        "wrongly attributed to an agent: {payloads:?}"
    );
}

#[test]
fn sessions__two_open_matching_calls_stay_attributed_to_the_bare_session() {
    let session = "s-attr-3";
    let sb = board(session);
    let id = new_task(&sb, "ambiguous write", &["only criterion"], session, 1);
    assert_eq!(code(&task(&sb, &["claim", &id], session, 2)), 0);
    let root = sb.root();
    let call_a = board_write_call(
        &format!("ratchet task check {id} 1"),
        &root,
        "tu-3a",
        Some(("agent-aaa111", "ratchet:implementer")),
    );
    let call_b = board_write_call(
        &format!("ratchet task check {id} 1"),
        &root,
        "tu-3b",
        Some(("agent-bbb222", "ratchet:reviewer")),
    );
    assert_eq!(
        code(&hook_env(
            &sb,
            "pre-tool",
            &call_a,
            &root,
            &[("RATCHET_SESSION_ID", session), ("RATCHET_NOW", &at(3))]
        )),
        0
    );
    assert_eq!(
        code(&hook_env(
            &sb,
            "pre-tool",
            &call_b,
            &root,
            &[("RATCHET_SESSION_ID", session), ("RATCHET_NOW", &at(4))]
        )),
        0
    );
    let out = task(&sb, &["check", &id, "1"], session, 5);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let payloads = payloads_of(&sb, &id, "checklist.done");
    assert_eq!(payloads.len(), 1);
    assert!(
        !payloads[0].contains("agent_id"),
        "ambiguous match wrongly attributed: {payloads:?}"
    );
}

#[test]
fn sessions__a_post_tool_call_clears_its_pending_call() {
    let session = "s-attr-4";
    let sb = board(session);
    let id = new_task(
        &sb,
        "cleared before the write",
        &["only criterion"],
        session,
        1,
    );
    assert_eq!(code(&task(&sb, &["claim", &id], session, 2)), 0);
    let root = sb.root();
    let call = board_write_call(
        &format!("ratchet task check {id} 1"),
        &root,
        "tu-4",
        Some(("agent-ccc333", "ratchet:implementer")),
    );
    assert_eq!(
        code(&hook_env(
            &sb,
            "pre-tool",
            &call,
            &root,
            &[("RATCHET_SESSION_ID", session), ("RATCHET_NOW", &at(3))]
        )),
        0
    );
    assert_eq!(
        code(&hook_env(
            &sb,
            "post-tool",
            &post_tool(&call),
            &root,
            &[("RATCHET_SESSION_ID", session), ("RATCHET_NOW", &at(4))]
        )),
        0
    );
    let out = task(&sb, &["check", &id, "1"], session, 5);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let payloads = payloads_of(&sb, &id, "checklist.done");
    assert_eq!(payloads.len(), 1);
    assert!(
        !payloads[0].contains("agent-ccc333"),
        "stale pending call still credited: {payloads:?}"
    );
}

#[test]
fn sessions__a_session_identifier_cannot_express_an_agent_identity() {
    let sb = board("s-attr-5");
    let id = new_task(&sb, "no forged pair", &[], "s-attr-5", 1);
    let root = sb.root();

    let via_flag = task(
        &sb,
        &["claim", &id, "--session", "s-attr-5/agent-x"],
        "s-attr-5",
        2,
    );
    assert_eq!(code(&via_flag), 1, "stdout: {}", stdout(&via_flag));
    let flag_err = stderr(&via_flag).to_lowercase();
    assert!(flag_err.contains("cannot contain"), "{flag_err}");
    // Pins that this is its own, dedicated refusal -- not merely today's "is not registered"
    // message happening to echo a value that contains a slash.
    assert!(!flag_err.contains("is not registered"), "{flag_err}");
    assert_eq!(task_state(&sb, &id).1, None, "flag form wrote a claim");

    let via_env = cli(
        &sb,
        &["task", "claim", &id],
        &root,
        &[
            ("RATCHET_SESSION_ID", "s-attr-5/agent-x"),
            ("RATCHET_NOW", &at(3)),
        ],
    );
    assert_eq!(code(&via_env), 1, "stdout: {}", stdout(&via_env));
    let env_err = stderr(&via_env).to_lowercase();
    assert!(env_err.contains("cannot contain"), "{env_err}");
    assert!(!env_err.contains("is not registered"), "{env_err}");
    assert_eq!(task_state(&sb, &id).1, None, "env form wrote a claim");
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
