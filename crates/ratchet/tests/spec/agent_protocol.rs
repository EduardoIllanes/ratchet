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
