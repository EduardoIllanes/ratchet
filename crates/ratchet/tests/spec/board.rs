//! One test per `#### Scenario` of the four board requirements of
//! openspec/specs/agent-protocol/spec.md, named by slug.

use crate::support::*;

// --- Requirement: Briefing at session start -------------------------------------------------

#[test]
fn agent_protocol__briefing_with_orphans_and_ready_tasks() {
    let sb = board("s-old");
    let held = new_task(&sb, "the abandoned one", &["a", "b"], "s-old", 1);
    assert_eq!(code(&task(&sb, &["claim", &held], "s-old", 2)), 0);
    assert_eq!(
        code(&task(
            &sb,
            &[
                "handoff",
                &held,
                "stopped at the parser; resume with the fixtures"
            ],
            "s-old",
            3
        )),
        0
    );
    for title in ["first ready", "second ready", "third ready"] {
        let id = new_task(&sb, title, &[], "s-old", 4);
        assert_eq!(code(&task(&sb, &["status", &id, "ready"], "s-old", 5)), 0);
    }
    // Ninety minutes with no signal: s-old is orphaned when the next session starts.
    let out = join(&sb, "s-new", 90);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.starts_with("[ratchet] repo "), "{text}");
    let orphan_heading = text
        .lines()
        .position(|l| l.to_lowercase().contains("dead session"))
        .expect("no heading for tasks held by dead sessions");
    let ready_heading = text
        .lines()
        .position(|l| l.to_lowercase().contains("ready"))
        .expect("no heading for ready tasks");
    assert!(orphan_heading < ready_heading, "{text}");
    assert!(text.contains(&held), "{text}");
    assert!(text.contains("resume with the fixtures"), "{text}");
    assert!(text.contains("first ready"), "{text}");
    assert!(text.contains("third ready"), "{text}");
    assert!(
        text.lines().last().unwrap().contains("ratchet-tasks"),
        "last line: {:?}",
        text.lines().last()
    );
    // The sweep runs after the briefing, so the task is back in the queue now.
    assert_eq!(task_state(&sb, &held).0, "ready");
}

#[test]
fn agent_protocol__no_tasks_one_line() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-alone", &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    assert_eq!(printed.len(), 1, "{printed:?}");
    assert!(printed[0].starts_with("[ratchet] repo "), "{}", printed[0]);
    assert!(printed[0].contains("session"), "{}", printed[0]);
    assert!(printed[0].contains("branch"), "{}", printed[0]);
}

#[test]
fn agent_protocol__the_briefing_never_exceeds_forty_lines() {
    let sb = board("s-crowded");
    // Sixty tasks in progress held by a session that was never registered: every one of them is
    // orphaned, and that list has no cap of its own.
    for n in 1..=60 {
        seed_task(&sb, &format!("T-{n:04}"), "in_progress", Some("s-ghost"));
    }
    let out = join(&sb, "s-reader", 1);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    assert!(printed.len() <= 40, "{} lines", printed.len());
    assert!(printed.len() > 30, "suspiciously short: {printed:?}");
    assert!(
        printed[printed.len() - 2].contains("ratchet task list"),
        "{:?}",
        printed[printed.len() - 2]
    );
    assert!(
        printed[printed.len() - 1].contains("ratchet-tasks"),
        "{:?}",
        printed[printed.len() - 1]
    );
}

// --- Requirement: Task reminder on every prompt ---------------------------------------------

#[test]
fn agent_protocol__with_a_claimed_task() {
    let sb = board("s-22");
    let id = new_task(
        &sb,
        "the claimed one",
        &["a", "b", "c", "d", "e"],
        "s-22",
        1,
    );
    assert_eq!(code(&task(&sb, &["claim", &id], "s-22", 2)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-22", 3)), 0);
    assert_eq!(code(&task(&sb, &["check", &id, "2"], "s-22", 4)), 0);
    let when = at(5);
    let root = sb.root();
    let out = hook_env(
        &sb,
        "prompt",
        &session_payload("s-22", &root),
        &root,
        &[("RATCHET_NOW", &when)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    assert_eq!(printed.len(), 1, "{printed:?}");
    assert!(printed[0].starts_with("[ratchet] "), "{}", printed[0]);
    assert!(printed[0].contains(&id), "{}", printed[0]);
    assert!(printed[0].contains("in_progress"), "{}", printed[0]);
    assert!(printed[0].contains("(2/5)"), "{}", printed[0]);
}

#[test]
fn agent_protocol__without_a_claimed_task() {
    let sb = board("s-23");
    let _ = new_task(&sb, "nobody claimed me", &[], "s-23", 1);
    let when = at(2);
    let root = sb.root();
    let out = hook_env(
        &sb,
        "prompt",
        &session_payload("s-23", &root),
        &root,
        &[("RATCHET_NOW", &when)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
}

// --- Requirement: Handoff rule when the session closes --------------------------------------

/// A prompt, then a stop, both at explicit instants: the rule's window is "since the last prompt".
fn prompt_then_stop(
    sb: &Sandbox,
    session_id: &str,
    prompt_at: i64,
    stop_at: i64,
    retry: bool,
) -> std::process::Output {
    let root = sb.root();
    let p = at(prompt_at);
    let out = hook_env(
        sb,
        "prompt",
        &session_payload(session_id, &root),
        &root,
        &[("RATCHET_NOW", &p)],
    );
    assert_eq!(code(&out), 0, "prompt failed: {}", stderr(&out));
    let s = at(stop_at);
    let payload = serde_json::json!({
        "session_id": session_id,
        "cwd": root.to_string_lossy(),
        "stop_hook_active": retry,
    });
    hook_env(sb, "stop", &payload, &root, &[("RATCHET_NOW", &s)])
}

#[test]
fn agent_protocol__closing_with_nothing_recorded() {
    let sb = board("s-24");
    let id = new_task(&sb, "claimed and abandoned", &["a"], "s-24", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-24", 2)), 0);
    let out = prompt_then_stop(&sb, "s-24", 3, 4, false);
    assert_eq!(code(&out), 2, "stdout: {}", stdout(&out));
    let err = stderr(&out);
    assert!(err.starts_with("[ratchet] "), "{err}");
    assert!(err.contains(&id), "{err}");
    assert!(err.contains("handoff"), "{err}");
    assert!(err.contains("check"), "{err}");
    assert!(err.contains("note"), "{err}");
    assert!(err.contains("status"), "{err}");
}

#[test]
fn agent_protocol__closing_with_something_recorded() {
    let sb = board("s-25");
    let id = new_task(&sb, "claimed and worked", &["a"], "s-25", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-25", 2)), 0);
    let root = sb.root();
    let p = at(3);
    assert_eq!(
        code(&hook_env(
            &sb,
            "prompt",
            &session_payload("s-25", &root),
            &root,
            &[("RATCHET_NOW", &p)]
        )),
        0
    );
    assert_eq!(code(&task(&sb, &["check", &id, "1"], "s-25", 4)), 0);
    let s = at(5);
    let payload = serde_json::json!({
        "session_id": "s-25",
        "cwd": root.to_string_lossy(),
        "stop_hook_active": false,
    });
    let out = hook_env(&sb, "stop", &payload, &root, &[("RATCHET_NOW", &s)]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(stderr(&out), "");
}

#[test]
fn agent_protocol__retrying_the_close() {
    let sb = board("s-26");
    let id = new_task(&sb, "claimed and abandoned", &["a"], "s-26", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-26", 2)), 0);
    let blocked = prompt_then_stop(&sb, "s-26", 3, 4, false);
    assert_eq!(code(&blocked), 2);
    let root = sb.root();
    let s = at(5);
    let payload = serde_json::json!({
        "session_id": "s-26",
        "cwd": root.to_string_lossy(),
        "stop_hook_active": true,
    });
    let retry = hook_env(&sb, "stop", &payload, &root, &[("RATCHET_NOW", &s)]);
    assert_eq!(code(&retry), 0, "stderr: {}", stderr(&retry));
    assert_eq!(task_state(&sb, &id).0, "in_progress");
}

#[test]
fn agent_protocol__closing_a_headless_session_with_nothing_recorded() {
    let sb = sandbox();
    let root = sb.root();
    let started = hook_env(
        &sb,
        "session-start",
        &session_payload("s-27", &root),
        &root,
        &[
            ("RATCHET_NOW", T0),
            ("RATCHET_SESSION_MODE", "headless"),
            ("RATCHET_LAUNCHED_BY", "platform"),
        ],
    );
    assert_eq!(code(&started), 0, "{}", stderr(&started));
    let id = new_task(&sb, "claimed by a robot", &["a"], "s-27", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-27", 2)), 0);
    let out = prompt_then_stop(&sb, "s-27", 3, 4, false);
    assert_eq!(
        code(&out),
        0,
        "a headless close was blocked: {}",
        stderr(&out)
    );
}

// --- Requirement: Compact output for agents -------------------------------------------------

#[test]
fn agent_protocol__long_output_goes_to_a_file() {
    let sb = board("s-28");
    for n in 1..=70 {
        seed_task(&sb, &format!("T-{n:04}"), "ready", None);
    }
    let out = task(&sb, &["list"], "s-28", 1);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    // The requirement is exact, not just a cap: "the terminal SHALL receive the first 20 lines
    // plus the path of that file" — 20 content lines + 1 path line, always, once output passes
    // the 60-line threshold. 70 seeded lines clears that threshold, so anything other than 21 is
    // a violation, not merely "close enough".
    assert_eq!(
        printed.len(),
        21,
        "expected the first 20 lines plus the path line (21 total), got {}: {:?}",
        printed.len(),
        printed
    );
    let files = out_files(&sb);
    assert_eq!(files.len(), 1, "{files:?}");
    let body = std::fs::read_to_string(&files[0]).unwrap();
    assert_eq!(body.lines().count(), 70, "the file lost lines");
    assert!(
        printed
            .last()
            .unwrap()
            .contains(&files[0].display().to_string()),
        "the path is not on the terminal: {:?}",
        printed.last()
    );
}

#[test]
fn agent_protocol__a_task_listing_is_one_line_per_task() {
    let sb = board("s-29");
    let first = new_task(&sb, "first ready", &["a", "b"], "s-29", 1);
    assert_eq!(code(&task(&sb, &["status", &first, "ready"], "s-29", 2)), 0);
    let second = new_task(&sb, "second ready", &[], "s-29", 3);
    assert_eq!(
        code(&task(&sb, &["status", &second, "ready"], "s-29", 4)),
        0
    );
    let out = task(&sb, &["list", "--status", "ready"], "s-29", 5);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    assert_eq!(printed.len(), 2, "{printed:?}");
    let line = printed.iter().find(|l| l.contains(&first)).unwrap();
    assert!(line.contains("ready"), "{line}");
    assert!(line.contains("p3"), "{line}");
    assert!(line.contains("first ready"), "{line}");
    assert!(line.contains("(0/2)"), "{line}");
    assert!(out_files(&sb).is_empty(), "a short listing went to a file");
}
