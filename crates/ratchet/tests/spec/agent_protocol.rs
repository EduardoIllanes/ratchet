//! One test per `#### Scenario` of openspec/specs/agent-protocol/spec.md, named by slug.

use std::fs;
use std::time::Instant;

use crate::support::*;

// --- Requirement: Repo opt-in by marker -------------------------------------------

#[test]
fn agent_protocol__no_marker_hooks_do_nothing() {
    let sb = sandbox();
    let dir = unmanaged_dir();
    let out = hook_in(
        &sb,
        "pre-tool",
        &bash("python scripts/x.py", dir.path()),
        dir.path(),
    );
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), "");
}

#[test]
fn agent_protocol__marker_found_from_a_subdirectory() {
    let sb = sandbox();
    let deep = sb.root().join("src/deep");
    // The process itself is spawned from the (unmanaged) scratchpad, not from `deep`, so a
    // pass here proves the hook resolves the repo from the payload's `cwd` field rather than
    // its own OS current directory.
    let out = hook_in_from(
        &sb,
        "pre-tool",
        &bash("python scripts/x.py", &deep),
        sb.scratchpad.path(),
    );
    assert_eq!(code(&out), 2, "stderr: {}", stderr(&out));
    assert!(stderr(&out).starts_with("[ratchet guardrail:python-venv]"));
}

#[test]
fn agent_protocol__invalid_marker_is_logged_and_ignored() {
    let sb = sandbox();
    sb.write_marker("[repo\nthis is = not toml");
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &bash("python scripts/x.py", &root), &root);
    assert_eq!(code(&out), 0);
    assert!(
        sb.log_text().contains("ratchet.toml"),
        "log: {}",
        sb.log_text()
    );
}

// --- Requirement: Marker can be generated -----------------------------------------

#[test]
fn agent_protocol__init_writes_a_marker_at_the_repo_root() {
    let sb = sandbox();
    let root = sb.root();
    fs::remove_file(root.join("ratchet.toml")).unwrap();
    let deep = root.join("src/deep");
    let out = cli(&sb, &["config", "init"], &deep, &[]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let expected_path = root.join("ratchet.toml");
    assert_eq!(
        stdout(&out).trim(),
        format!("wrote {}", expected_path.display())
    );
    assert_eq!(stderr(&out), "");
    let text = fs::read_to_string(&expected_path).unwrap();
    let parsed: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(
        parsed["repo"]["default_branch"].as_str(),
        Some("main"),
        "{text}"
    );
}

#[test]
fn agent_protocol__init_refuses_to_overwrite_without_force() {
    let sb = sandbox();
    let root = sb.root();
    sb.write_marker("[repo]\ndefault_branch = \"sentinel\"\n");
    let before = fs::read_to_string(root.join("ratchet.toml")).unwrap();
    let out = cli(&sb, &["config", "init"], &root, &[]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("--force"), "{}", stderr(&out));
    let after = fs::read_to_string(root.join("ratchet.toml")).unwrap();
    assert_eq!(before, after);
}

#[test]
fn agent_protocol__init_outside_a_git_repo_fails() {
    let sb = sandbox();
    let dir = unmanaged_dir();
    let out = cli(&sb, &["config", "init"], dir.path(), &[]);
    assert_eq!(code(&out), 1);
    assert!(
        stderr(&out).contains("not a git repository"),
        "{}",
        stderr(&out)
    );
    assert!(!dir.path().join("ratchet.toml").exists());
}

// --- Requirement: Guardrails before the action -----------------------------------

#[test]
fn agent_protocol__python_outside_the_venv() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &bash("python scripts/x.py", &root), &root);
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(err.starts_with("[ratchet guardrail:python-venv]"), "{err}");
    assert!(err.contains("uv run"), "{err}");
    let ok = hook_in(
        &sb,
        "pre-tool",
        &bash("uv run python scripts/x.py", &root),
        &root,
    );
    assert_eq!(code(&ok), 0);
}

#[test]
fn agent_protocol__python_through_powershell_same_message() {
    let sb = sandbox();
    let root = sb.root();
    let via_bash = hook_in(
        &sb,
        "pre-tool",
        &bash("python -c \"print(1)\"", &root),
        &root,
    );
    let via_ps = hook_in(
        &sb,
        "pre-tool",
        &powershell("python -c \"print(1)\"", &root),
        &root,
    );
    assert_eq!(code(&via_ps), 2);
    assert_eq!(stderr(&via_ps), stderr(&via_bash));
}

#[test]
fn agent_protocol__quoted_text_does_not_split_a_command() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(
        &sb,
        "pre-tool",
        &bash(
            "uv run ratchet task new -t x -c \"done; mypy clean\"",
            &root,
        ),
        &root,
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
}

#[test]
fn agent_protocol__git_destructive() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(
        &sb,
        "pre-tool",
        &bash("git reset --hard HEAD~1", &root),
        &root,
    );
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(
        err.starts_with("[ratchet guardrail:git-destructive]"),
        "{err}"
    );
    assert!(err.to_lowercase().contains("owner"), "{err}");
    let lease = hook_in(
        &sb,
        "pre-tool",
        &bash("git push --force-with-lease", &root),
        &root,
    );
    assert_eq!(code(&lease), 0);
}

#[test]
fn agent_protocol__recursive_delete_under_the_scratchpad_is_allowed() {
    let sb = sandbox();
    let root = sb.root();
    let inside = sb.scratchpad.path().join("tmp");
    fs::create_dir_all(&inside).unwrap();
    let ok = hook_in(
        &sb,
        "pre-tool",
        &bash(&format!("rm -rf {}", inside.display()), &root),
        &root,
    );
    assert_eq!(code(&ok), 0, "stderr: {}", stderr(&ok));
    let bad = hook_in(
        &sb,
        "pre-tool",
        &bash(&format!("rm -rf {}", root.join("src").display()), &root),
        &root,
    );
    assert_eq!(code(&bad), 2);
}

#[test]
fn agent_protocol__env_file_write_blocked() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(
        &sb,
        "pre-tool",
        &write(&root.join(".env.local"), "KEY=1", &root),
        &root,
    );
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).starts_with("[ratchet guardrail:env-files]"));
}

#[test]
fn agent_protocol__write_to_the_main_tree_blocked() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(
        &sb,
        "pre-tool",
        &edit(&root.join("tracked.txt"), "bye", &root),
        &root,
    );
    assert_eq!(code(&out), 2, "stderr: {}", stderr(&out));
    let err = stderr(&out);
    assert!(err.starts_with("[ratchet guardrail:main-tree]"), "{err}");
    assert!(err.contains("worktree"), "{err}");
}

#[test]
fn agent_protocol__write_to_the_main_tree_blocked_from_a_worktree_session() {
    let sb = sandbox();
    let root = sb.root();
    let wt = root.join(".worktrees/wt");
    let out = hook_in(
        &sb,
        "pre-tool",
        &edit(&root.join("tracked.txt"), "bye", &wt),
        &wt,
    );
    assert_eq!(code(&out), 2, "stderr: {}", stderr(&out));
    assert!(stderr(&out).starts_with("[ratchet guardrail:main-tree]"));
}

#[test]
fn agent_protocol__write_inside_a_worktree_allowed() {
    let sb = sandbox();
    let root = sb.root();
    let wt = root.join(".worktrees/wt");
    let out = hook_in(
        &sb,
        "pre-tool",
        &edit(&wt.join("tracked.txt"), "bye", &wt),
        &wt,
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    // Companion assertion: the same session (cwd = the worktree) still gets blocked editing
    // the main tree's tracked file, so a broken resolver that just always allows can't pass.
    let blocked = hook_in(
        &sb,
        "pre-tool",
        &edit(&root.join("tracked.txt"), "bye", &wt),
        &wt,
    );
    assert_eq!(code(&blocked), 2, "stderr: {}", stderr(&blocked));
    assert!(
        stderr(&blocked).starts_with("[ratchet guardrail:main-tree]"),
        "{}",
        stderr(&blocked)
    );
}

#[test]
fn agent_protocol__untracked_file_in_the_main_tree_allowed() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(
        &sb,
        "pre-tool",
        &write(&root.join("notes/new.md"), "draft", &root),
        &root,
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    // Companion assertion: a tracked file in the same main tree still blocks, so a resolver
    // that always allows (rather than correctly distinguishing tracked/untracked) can't pass.
    let blocked = hook_in(
        &sb,
        "pre-tool",
        &edit(&root.join("tracked.txt"), "bye", &root),
        &root,
    );
    assert_eq!(code(&blocked), 2, "stderr: {}", stderr(&blocked));
    assert!(
        stderr(&blocked).starts_with("[ratchet guardrail:main-tree]"),
        "{}",
        stderr(&blocked)
    );
}

#[test]
fn agent_protocol__bash_redirection_into_a_tracked_main_tree_file_blocked() {
    let sb = sandbox();
    let root = sb.root();
    let via_bash = hook_in(&sb, "pre-tool", &bash("echo x > tracked.txt", &root), &root);
    assert_eq!(code(&via_bash), 2, "stderr: {}", stderr(&via_bash));
    let err = stderr(&via_bash);
    assert!(err.starts_with("[ratchet guardrail:main-tree]"), "{err}");
    // Same message an `Edit` targeting the same tracked file gets.
    let via_edit = hook_in(
        &sb,
        "pre-tool",
        &edit(&root.join("tracked.txt"), "bye", &root),
        &root,
    );
    assert_eq!(code(&via_edit), 2, "stderr: {}", stderr(&via_edit));
    assert_eq!(err, stderr(&via_edit));
}

#[test]
fn agent_protocol__bash_writing_shapes_into_the_main_tree_blocked() {
    let sb = sandbox();
    let root = sb.root();
    track_file(&sb, "src/main.rs", "fn main() {}\n");
    let shapes = [
        "printf y >> src/main.rs",
        "tee src/main.rs",
        "sed -i '' 's/a/b/' src/main.rs",
        "cp /tmp/x src/main.rs",
        "mv /tmp/x src/main.rs",
        "rsync /tmp/x src/main.rs",
        "git checkout -- src/main.rs",
        "git restore src/main.rs",
    ];
    for shape in shapes {
        let via_bash = hook_in(&sb, "pre-tool", &bash(shape, &root), &root);
        assert_eq!(
            code(&via_bash),
            2,
            "bash {shape:?}: stderr {}",
            stderr(&via_bash)
        );
        assert!(
            stderr(&via_bash).starts_with("[ratchet guardrail:main-tree]"),
            "bash {shape:?}: {}",
            stderr(&via_bash)
        );
        let via_ps = hook_in(&sb, "pre-tool", &powershell(shape, &root), &root);
        assert_eq!(
            code(&via_ps),
            2,
            "powershell {shape:?}: stderr {}",
            stderr(&via_ps)
        );
        assert_eq!(
            stderr(&via_bash),
            stderr(&via_ps),
            "shape {shape:?} differs between Bash and PowerShell"
        );
    }
}

#[test]
fn agent_protocol__bash_writing_shapes_elsewhere_allowed() {
    let sb = sandbox();
    let root = sb.root();
    let wt = worktree(&sb);
    let cases = [
        "echo x > notes.txt".to_string(),
        format!("echo x > {}", sb.scratchpad.path().join("x").display()),
        format!("echo x > {}", wt.join("README.md").display()),
    ];
    for cmd in &cases {
        let out = hook_in(&sb, "pre-tool", &bash(cmd, &root), &root);
        assert_eq!(code(&out), 0, "{cmd:?}: stderr {}", stderr(&out));
    }
    // Companion assertion: the same recognised shape against a tracked main-tree path still
    // blocks, so a resolver that just always allows `>` redirections can't pass.
    let blocked = hook_in(&sb, "pre-tool", &bash("echo x > tracked.txt", &root), &root);
    assert_eq!(code(&blocked), 2, "stderr: {}", stderr(&blocked));
    assert!(
        stderr(&blocked).starts_with("[ratchet guardrail:main-tree]"),
        "{}",
        stderr(&blocked)
    );
}

#[test]
fn agent_protocol__interpreter_write_is_not_blocked_before_the_fact() {
    let sb = sandbox();
    let root = sb.root();
    // Spec text: `python - <<'EOF' … EOF`. Bare `python` in a repo with `.venv` trips the
    // separate `python-venv` rule before the main-tree question is even reached, so this uses
    // `uv run python` to isolate the shape under test (interpreter writes vs. recognised
    // redirection/`sed -i`/etc. shapes) from that unrelated rule.
    let cmd = "uv run python - <<'EOF'\nopen('tracked.txt', 'w').write('bye')\nEOF";
    let out = hook_in(&sb, "pre-tool", &bash(cmd, &root), &root);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    // Companion assertion: a recognised shape against the very same file still blocks, so the
    // allow above is specific to the interpreter shape, not a blanket pass on that path.
    let blocked = hook_in(
        &sb,
        "pre-tool",
        &bash("sed -i '' 's/a/b/' tracked.txt", &root),
        &root,
    );
    assert_eq!(code(&blocked), 2, "stderr: {}", stderr(&blocked));
    assert!(
        stderr(&blocked).starts_with("[ratchet guardrail:main-tree]"),
        "{}",
        stderr(&blocked)
    );
}

#[test]
fn agent_protocol__rule_disabled_per_repo() {
    let sb = sandbox();
    sb.write_marker(
        "[repo]\nworktrees_dir = \".worktrees\"\n[guardrails]\noff = [\"python-venv\"]\n",
    );
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &bash("python scripts/x.py", &root), &root);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    // Companion assertion: a different, still-enabled rule keeps blocking in the same repo,
    // so a resolver that just silently disables everything can't pass.
    let blocked = hook_in(&sb, "pre-tool", &bash("git reset --hard", &root), &root);
    assert_eq!(code(&blocked), 2, "stderr: {}", stderr(&blocked));
    assert!(
        stderr(&blocked).starts_with("[ratchet guardrail:git-destructive]"),
        "{}",
        stderr(&blocked)
    );
}

#[test]
fn agent_protocol__custom_content_rule_from_the_repo() {
    let sb = sandbox();
    sb.write_marker("[repo]\nworktrees_dir = \".worktrees\"\n[guardrails]\nextra = \"ratchet/guardrails.toml\"\n");
    sb.write(
        "ratchet/guardrails.toml",
        "[[rules]]\nid = \"db-readonly\"\ntools = [\"Bash\", \"PowerShell\", \"Edit\", \"Write\"]\nkind = \"content\"\npattern = '\\.purge_all\\s*\\('\nmessage = \"The database is read-only.\"\nalternative = \"Read through the data layer.\"\n",
    );
    let root = sb.root();
    let out = hook_in(
        &sb,
        "pre-tool",
        &write(&root.join("notes/s.py"), "coll.purge_all({})", &root),
        &root,
    );
    assert_eq!(code(&out), 2, "stderr: {}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.starts_with("[ratchet guardrail:db-readonly] The database is read-only."),
        "{err}"
    );
}

#[test]
fn agent_protocol__machine_wide_rule_overrides_a_built_in() {
    let sb = sandbox();
    fs::create_dir_all(sb.home.path()).unwrap();
    fs::write(
        sb.home.path().join("config.toml"),
        "[guardrails]\nextra = \"guardrails.toml\"\n",
    )
    .unwrap();
    fs::write(
        sb.home.path().join("guardrails.toml"),
        "[[rules]]\nid = \"git-destructive\"\ntools = [\"Bash\", \"PowerShell\"]\nkind = \"command\"\npattern = 'git\\s+reset\\s+--hard'\nmessage = \"Machine says no.\"\nalternative = \"Use git stash.\"\n",
    ).unwrap();
    let root = sb.root();
    let out = hook_in(&sb, "pre-tool", &bash("git reset --hard", &root), &root);
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).starts_with("[ratchet guardrail:git-destructive] Machine says no."),
        "{}",
        stderr(&out)
    );
}

// --- Requirement: Main-tree writes detected after the fact -------------------------

#[test]
fn agent_protocol__a_bash_command_that_changed_a_tracked_main_tree_file_is_reported_after_the_fact()
{
    let session = "s-mt-reported";
    // `board`, not `sandbox`, so the state database (and its `events` table) already exists —
    // the scenario is about what gets recorded once a snapshot exists, not about bootstrapping.
    let sb = board(session);
    let root = sb.root();
    let payload = bash(
        "uv run python - <<'EOF'\nopen('tracked.txt', 'w').write('bye')\nEOF",
        &root,
    );
    let pre = hook_env(
        &sb,
        "pre-tool",
        &payload,
        &root,
        &[("RATCHET_SESSION_ID", session), ("RATCHET_NOW", &at(1))],
    );
    assert_eq!(code(&pre), 0, "pre-tool stderr: {}", stderr(&pre));
    // Simulate the command's effect: an interpreter heredoc rewrote the tracked file, which the
    // pre-tool hook does not (and cannot) block before the fact.
    fs::write(root.join("tracked.txt"), "bye").unwrap();
    let post = hook_env(
        &sb,
        "post-tool",
        &post_tool(&payload),
        &root,
        &[("RATCHET_SESSION_ID", session), ("RATCHET_NOW", &at(2))],
    );
    assert_eq!(code(&post), 0, "post-tool stderr: {}", stderr(&post));
    assert_eq!(stdout(&post), "");
    let err = stderr(&post);
    assert_eq!(err.lines().count(), 1, "expected exactly one line: {err:?}");
    assert!(err.contains("[ratchet guardrail:main-tree]"), "{err}");
    assert!(err.contains("tracked.txt"), "{err}");
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'guardrail.main_tree_write'",
            &[session]
        ),
        1
    );
    let payloads = session_event_payloads(&sb, session, "guardrail.main_tree_write");
    assert_eq!(payloads.len(), 1);
    assert!(payloads[0].contains("tracked.txt"), "{:?}", payloads[0]);
}

#[test]
fn agent_protocol__post_check_is_silent_when_nothing_tracked_changed() {
    let session = "s-mt-silent-untracked";
    let sb = board(session);
    let root = sb.root();
    let payload = bash(
        "uv run python - <<'EOF'\nopen('scratch.md', 'w').write('draft')\nEOF",
        &root,
    );
    let pre = hook_env(
        &sb,
        "pre-tool",
        &payload,
        &root,
        &[("RATCHET_SESSION_ID", session), ("RATCHET_NOW", &at(1))],
    );
    assert_eq!(code(&pre), 0, "pre-tool stderr: {}", stderr(&pre));
    // Simulate the command's effect: only a new, untracked file appeared in the main tree.
    fs::write(root.join("scratch.md"), "draft").unwrap();
    let post = hook_env(
        &sb,
        "post-tool",
        &post_tool(&payload),
        &root,
        &[("RATCHET_SESSION_ID", session), ("RATCHET_NOW", &at(2))],
    );
    assert_eq!(code(&post), 0, "post-tool stderr: {}", stderr(&post));
    assert_eq!(stdout(&post), "");
    assert_eq!(stderr(&post), "");
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'guardrail.main_tree_write'",
            &[session]
        ),
        0
    );
}

#[test]
fn agent_protocol__post_check_without_a_prior_snapshot_is_silent() {
    let session = "s-mt-no-snapshot";
    // The session is registered (so the database exists), but no `pre-tool` hook call ever ran
    // for this Bash command in that session, so no "before" snapshot was recorded for it.
    let sb = board(session);
    let root = sb.root();
    let payload = bash(
        "uv run python - <<'EOF'\nopen('tracked.txt', 'w').write('bye')\nEOF",
        &root,
    );
    // Even though the tracked file really did change:
    fs::write(root.join("tracked.txt"), "bye").unwrap();
    let post = hook_env(
        &sb,
        "post-tool",
        &post_tool(&payload),
        &root,
        &[("RATCHET_SESSION_ID", session), ("RATCHET_NOW", &at(1))],
    );
    assert_eq!(code(&post), 0, "post-tool stderr: {}", stderr(&post));
    assert_eq!(stdout(&post), "");
    assert_eq!(stderr(&post), "");
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'guardrail.main_tree_write'",
            &[session]
        ),
        0
    );
}

// --- Requirement: Hooks never break a session -------------------------------------

#[test]
fn agent_protocol__malformed_payload_exits_0_and_is_logged() {
    let sb = sandbox();
    let root = sb.root();
    let out = run_hook(sb.home.path(), None, "pre-tool", "this is not json", &root);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "");
    assert!(sb.log_text().contains("pre-tool"), "log: {}", sb.log_text());
}

#[test]
fn agent_protocol__unknown_event_exits_0() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_in(&sb, "no-such-event", &bash("echo hi", &root), &root);
    assert_eq!(code(&out), 0);
}

#[test]
fn agent_protocol__pre_tool_answers_fast() {
    let sb = sandbox();
    let root = sb.root();
    let payload = bash("python scripts/x.py", &root);
    let mut times = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        let out = hook_in(&sb, "pre-tool", &payload, &root);
        times.push(t.elapsed().as_millis());
        assert_eq!(code(&out), 2);
    }
    times.sort();
    let median = times[times.len() / 2];
    eprintln!("pre-tool median {median} ms (debug build)");
    assert!(median < 200, "median {median} ms");
}

// --- Requirement: Active rules can be listed and dry-run ---------------------------

#[test]
fn agent_protocol__list_shows_built_ins_and_disabled_state() {
    let sb = sandbox();
    sb.write_marker(
        "[repo]\nworktrees_dir = \".worktrees\"\n[guardrails]\noff = [\"python-venv\"]\n",
    );
    let root = sb.root();
    let out = guardrails(&sb, &["list"], &root);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    for id in ["python-venv", "git-destructive", "env-files", "main-tree"] {
        assert!(text.contains(id), "{text}");
    }
    let line = text.lines().find(|l| l.contains("python-venv")).unwrap();
    assert!(line.contains("off"), "{line}");
}

#[test]
fn agent_protocol__dry_run_reproduces_the_block() {
    let sb = sandbox();
    let root = sb.root();
    let out = guardrails(
        &sb,
        &["test", "Bash", r#"{"command":"python x.py"}"#],
        &root,
    );
    assert_eq!(code(&out), 2);
    assert!(
        stdout(&out).contains("[ratchet guardrail:python-venv]")
            || stderr(&out).contains("[ratchet guardrail:python-venv]")
    );
}

// --- Requirement: Task reminder names subagents ------------------------------------------

/// A subagent stop for `agent` in `session`, carrying its own type so no meta file lookup
/// is involved. The agent keys ride on the hook input; `Payload` parses them once task B
/// lands.
fn record_stop(sb: &Sandbox, session: &str, agent: &str, minutes: i64) -> std::process::Output {
    let root = sb.root();
    let transcript = sb.scratchpad.path().join("transcripts").join("t.jsonl");
    let payload = serde_json::json!({
        "session_id": session,
        "cwd": root.to_string_lossy(),
        "agent_id": agent,
        "agent_type": "explore",
        "description": "reconnoitre",
        "transcript_path": transcript.to_string_lossy(),
        "exit_status": 0,
    });
    let when = at(minutes);
    hook_env(
        sb,
        "subagent-stop",
        &payload,
        &root,
        &[("RATCHET_NOW", &when)],
    )
}

/// A subagent start for `agent` in `session` with no stop after it.
fn record_start(sb: &Sandbox, session: &str, agent: &str, minutes: i64) -> std::process::Output {
    let root = sb.root();
    let transcript = sb.scratchpad.path().join("transcripts").join("t.jsonl");
    let payload = serde_json::json!({
        "session_id": session,
        "cwd": root.to_string_lossy(),
        "agent_id": agent,
        "agent_type": "explore",
        "description": "reconnoitre",
        "transcript_path": transcript.to_string_lossy(),
    });
    let when = at(minutes);
    hook_env(
        sb,
        "subagent-start",
        &payload,
        &root,
        &[("RATCHET_NOW", &when)],
    )
}

fn prompt_at(sb: &Sandbox, session: &str, minutes: i64) -> std::process::Output {
    let root = sb.root();
    let when = at(minutes);
    hook_env(
        sb,
        "prompt",
        &session_payload(session, &root),
        &root,
        &[("RATCHET_NOW", &when)],
    )
}

/// A session holding one claimed task, for the subagent reminder scenarios.
fn held_session(session: &str) -> (Sandbox, String) {
    let sb = board(session);
    let id = new_task(&sb, "held work", &["a"], session, 1);
    assert_eq!(code(&task(&sb, &["claim", &id], session, 2)), 0);
    (sb, id)
}

#[test]
fn agent_protocol__a_subagent_stopped_with_no_record_adds_a_line() {
    let session = "s-rem-stop";
    let (sb, id) = held_session(session);
    let agent = "a1b2c3d4e5f60718";
    let stopped = record_stop(&sb, session, agent, 3);
    assert_eq!(code(&stopped), 0, "{}", stderr(&stopped));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'subagent.stop'",
            &[session]
        ),
        1
    );
    let out = prompt_at(&sb, session, 4);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    assert_eq!(printed.len(), 2, "{printed:?}");
    assert!(printed[0].contains(&id), "{}", printed[0]);
    assert_eq!(
        printed[1],
        format!(
            "[ratchet] {id} · subagent {} stopped with no record",
            &agent[..8]
        )
    );
}

#[test]
fn agent_protocol__a_subagent_stopped_with_a_record_adds_no_line() {
    let session = "s-rem-noted";
    let (sb, id) = held_session(session);
    let agent = "b1c2d3e4f5a60718";
    let stopped = record_stop(&sb, session, agent, 3);
    assert_eq!(code(&stopped), 0, "{}", stderr(&stopped));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'subagent.stop'",
            &[session]
        ),
        1
    );
    assert_eq!(
        code(&task(
            &sb,
            &["note", &id, "saw the subagent stop"],
            session,
            4
        )),
        0
    );
    let out = prompt_at(&sb, session, 5);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    assert_eq!(printed.len(), 1, "{printed:?}");
    assert!(printed[0].contains(&id), "{}", printed[0]);
    assert!(
        !printed[0].contains("subagent"),
        "a note was recorded since the stop: {}",
        printed[0]
    );
}

#[test]
fn agent_protocol__a_running_subagent_adds_a_line() {
    let session = "s-rem-running";
    let (sb, id) = held_session(session);
    let agent = "c1d2e3f4a5b60718";
    let started = record_start(&sb, session, agent, 3);
    assert_eq!(code(&started), 0, "{}", stderr(&started));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'subagent.start'",
            &[session]
        ),
        1
    );
    let out = prompt_at(&sb, session, 4);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let printed = lines(&out);
    assert_eq!(printed.len(), 2, "{printed:?}");
    assert!(printed[0].contains(&id), "{}", printed[0]);
    assert_eq!(
        printed[1],
        format!("[ratchet] {id} · subagent {} running", &agent[..8])
    );
}

#[test]
fn agent_protocol__subagent_events_appear_in_the_detail_view() {
    let session = "s-rem-shown";
    let (sb, id) = held_session(session);
    let stopped = record_stop(&sb, session, "d1e2f3a4b5c60718", 3);
    assert_eq!(code(&stopped), 0, "{}", stderr(&stopped));
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE task_id = ?1 AND kind = 'subagent.stop'",
            &[id.as_str()]
        ),
        1
    );
    let shown = stdout(&task(&sb, &["show", &id], session, 4));
    assert!(shown.contains("subagent.stop"), "{shown}");
}
