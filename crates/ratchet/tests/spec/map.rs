//! Scenario tests for `openspec/specs/map/spec.md`. Every `#### Scenario` there has exactly one
//! test here, named by slug. No test involves a model — `mapper` (haiku) is never exercised;
//! everything a model would otherwise do (recording a note) is driven directly through
//! `ratchet map note`.

use crate::support::{
    code, commit, hook_env, map, map_at, map_env, read_map, read_notes, sandbox, session_payload,
    stderr, stdout, unmanaged_dir, write_plain_file, write_python_module, write_rust_module,
    write_ts_module, T0,
};

// --- determinism, generation, headers (Task 2) ---------------------------------------------

#[test]
fn map__two_runs_produce_byte_identical_output() {
    let sb = sandbox();
    write_rust_module(&sb, "src/a.rs", Some("Does a thing."), "pub fn a() {}");
    commit(&sb, &["src/a.rs"], "add a");
    let out1 = map_env(&sb, &[], &[("RATCHET_NOW", T0)]);
    assert_eq!(code(&out1), 0, "{}", stderr(&out1));
    let first = read_map(&sb);
    let out2 = map_env(&sb, &[], &[("RATCHET_NOW", T0)]);
    assert_eq!(code(&out2), 0, "{}", stderr(&out2));
    let second = read_map(&sb);
    assert_eq!(
        first, second,
        "two runs over the same commit must be byte-identical"
    );
    assert!(!first.is_empty());
}

#[test]
fn map__generating_a_map_prints_the_wrote_line() {
    let sb = sandbox();
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let s = stdout(&out);
    let first_line = s.lines().next().unwrap();
    assert!(first_line.starts_with("wrote "), "{first_line}");
    assert!(first_line.contains(".ratchet/map.md"), "{first_line}");
    assert!(sb.root().join(".ratchet/map.md").is_file());
}

#[test]
fn map__map_generation_outside_a_marker_repo_fails() {
    let dir = unmanaged_dir();
    let home = tempfile::TempDir::new().unwrap();
    let out = map_at(dir.path(), home.path(), &[]);
    assert_eq!(code(&out), 1);
    assert!(
        stderr(&out).contains("not a ratchet-managed repo"),
        "{}",
        stderr(&out)
    );
    assert!(!dir.path().join(".ratchet/map.md").exists());
}

#[test]
fn map__an_unwired_repo_gets_a_hint_naming_wire() {
    let sb = sandbox();
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    // `stdout(&out)` is an owned `String`; bind it before slicing into `&str` lines so the
    // borrow outlives the `Vec<&str>` (a temporary here would be dropped at end of statement).
    let s = stdout(&out);
    let hints: Vec<&str> = s.lines().filter(|l| l.contains("--wire")).collect();
    assert_eq!(hints.len(), 2, "{:?}", s);
}

#[test]
fn map__a_rust_doc_comment_becomes_the_module_sentence() {
    let sb = sandbox();
    write_rust_module(
        &sb,
        "src/widget.rs",
        Some("Computes the widget checksum"),
        "pub fn f() {}",
    );
    commit(&sb, &["src/widget.rs"], "add widget");
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    let line = text
        .lines()
        .find(|l| l.contains("src/widget.rs"))
        .expect(&text);
    assert!(line.contains("Computes the widget checksum"), "{line}");
}

#[test]
fn map__a_python_module_docstring_becomes_the_module_sentence() {
    let sb = sandbox();
    write_python_module(
        &sb,
        "src/widget.py",
        Some("Computes the widget checksum"),
        "def f(): pass",
    );
    commit(&sb, &["src/widget.py"], "add widget");
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    let line = text
        .lines()
        .find(|l| l.contains("src/widget.py"))
        .expect(&text);
    assert!(line.contains("Computes the widget checksum"), "{line}");
}

#[test]
fn map__a_typescript_leading_comment_becomes_the_module_sentence() {
    let sb = sandbox();
    write_ts_module(
        &sb,
        "src/widget.ts",
        Some("Computes the widget checksum"),
        "export function f() {}",
    );
    commit(&sb, &["src/widget.ts"], "add widget");
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    let line = text
        .lines()
        .find(|l| l.contains("src/widget.ts"))
        .expect(&text);
    assert!(line.contains("Computes the widget checksum"), "{line}");
}

#[test]
fn map__a_module_with_no_header_shows_a_placeholder() {
    let sb = sandbox();
    write_rust_module(&sb, "src/bare.rs", None, "pub fn f() {}");
    commit(&sb, &["src/bare.rs"], "add bare");
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    let line = text
        .lines()
        .find(|l| l.contains("src/bare.rs"))
        .expect(&text);
    assert!(line.trim_end().ends_with('—'), "{line}");
}

// --- notes (Task 3) --------------------------------------------------------------------------

#[test]
fn map__a_note_describes_a_header_less_file() {
    let sb = sandbox();
    write_rust_module(&sb, "src/bare.rs", None, "pub fn f() {}");
    commit(&sb, &["src/bare.rs"], "add bare");
    let n = map(&sb, &["note", "src/bare.rs", "Handles the bare case."]);
    assert_eq!(code(&n), 0, "{}", stderr(&n));
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    let line = text
        .lines()
        .find(|l| l.contains("src/bare.rs"))
        .expect(&text);
    assert!(line.contains("Handles the bare case."), "{line}");
}

#[test]
fn map__a_header_always_wins_over_a_note() {
    let sb = sandbox();
    write_rust_module(
        &sb,
        "src/both.rs",
        Some("The real header sentence"),
        "pub fn f() {}",
    );
    commit(&sb, &["src/both.rs"], "add both");
    let n = map(
        &sb,
        &["note", "src/both.rs", "A note that should never show."],
    );
    assert_eq!(code(&n), 0, "{}", stderr(&n));
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    let line = text
        .lines()
        .find(|l| l.contains("src/both.rs"))
        .expect(&text);
    assert!(line.contains("The real header sentence"), "{line}");
    assert!(!line.contains("A note that should never show"), "{line}");
}

#[test]
fn map__a_stale_note_is_dropped_at_the_next_generation() {
    let sb = sandbox();
    write_rust_module(&sb, "src/gone.rs", None, "pub fn f() {}");
    commit(&sb, &["src/gone.rs"], "add gone");
    let n = map(&sb, &["note", "src/gone.rs", "Will be deleted."]);
    assert_eq!(code(&n), 0, "{}", stderr(&n));
    std::fs::remove_file(sb.root().join("src/gone.rs")).unwrap();
    crate::support::git(sb.repo.path(), &["rm", "-q", "src/gone.rs"]);
    crate::support::git(sb.repo.path(), &["commit", "-q", "-m", "remove gone"]);
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        !read_notes(&sb).contains("src/gone.rs"),
        "{}",
        read_notes(&sb)
    );
}

// --- `map note` (Task 3) ----------------------------------------------------------------------

#[test]
fn map__map_note_records_a_sentence_for_a_tracked_file() {
    let sb = sandbox();
    write_rust_module(&sb, "src/x.rs", None, "pub fn f() {}");
    commit(&sb, &["src/x.rs"], "add x");
    let out = map(&sb, &["note", "src/x.rs", "Does the x thing."]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(read_notes(&sb).contains("src/x.rs"), "{}", read_notes(&sb));
    assert!(
        read_notes(&sb).contains("Does the x thing."),
        "{}",
        read_notes(&sb)
    );
}

#[test]
fn map__map_note_replaces_an_existing_note_for_the_same_path() {
    let sb = sandbox();
    write_rust_module(&sb, "src/x.rs", None, "pub fn f() {}");
    commit(&sb, &["src/x.rs"], "add x");
    map(&sb, &["note", "src/x.rs", "First sentence."]);
    let out = map(&sb, &["note", "src/x.rs", "Second sentence."]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let notes = read_notes(&sb);
    assert!(!notes.contains("First sentence."), "{notes}");
    assert!(notes.contains("Second sentence."), "{notes}");
    assert_eq!(
        notes.lines().filter(|l| l.contains("src/x.rs")).count(),
        1,
        "{notes}"
    );
}

#[test]
fn map__map_note_refuses_a_path_that_is_not_tracked() {
    let sb = sandbox();
    let out = map(&sb, &["note", "src/never-added.rs", "Whatever."]);
    assert_eq!(code(&out), 1);
    assert!(
        stderr(&out).contains("not a tracked file"),
        "{}",
        stderr(&out)
    );
    assert!(read_notes(&sb).is_empty());
}

#[test]
fn map__map_note_refuses_an_invalid_sentence() {
    let sb = sandbox();
    write_rust_module(&sb, "src/x.rs", None, "pub fn f() {}");
    commit(&sb, &["src/x.rs"], "add x");
    let long = "a".repeat(121);
    let out1 = map(&sb, &["note", "src/x.rs", &long]);
    assert_eq!(code(&out1), 1);
    assert!(
        stderr(&out1).contains("at most 120 characters"),
        "{}",
        stderr(&out1)
    );
    let out2 = map(&sb, &["note", "src/x.rs", "line one\nline two"]);
    assert_eq!(code(&out2), 1);
    assert!(
        stderr(&out2).contains("at most 120 characters"),
        "{}",
        stderr(&out2)
    );
    assert!(read_notes(&sb).is_empty());
}

#[test]
fn map__map_note_refuses_an_empty_sentence() {
    let sb = sandbox();
    write_rust_module(&sb, "src/x.rs", None, "pub fn f() {}");
    commit(&sb, &["src/x.rs"], "add x");
    let out = map(&sb, &["note", "src/x.rs", ""]);
    assert_eq!(code(&out), 1);
    assert!(
        stderr(&out).contains("must not be empty"),
        "{}",
        stderr(&out)
    );
    assert!(read_notes(&sb).is_empty());
}

// --- `--missing` (Task 3) ----------------------------------------------------------------------

#[test]
fn map__missing_lists_every_file_with_no_header_and_no_note() {
    let sb = sandbox();
    write_rust_module(&sb, "src/a.rs", Some("Has a header."), "pub fn a() {}");
    write_rust_module(&sb, "src/b.rs", None, "pub fn b() {}");
    write_rust_module(&sb, "src/c.rs", None, "pub fn c() {}");
    commit(&sb, &["src/a.rs", "src/b.rs", "src/c.rs"], "add a b c");
    let out = map(&sb, &["--missing"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let s = stdout(&out);
    let lines: Vec<&str> = s.lines().collect();
    assert!(lines.contains(&"src/b.rs"), "{lines:?}");
    assert!(lines.contains(&"src/c.rs"), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("src/a.rs")), "{lines:?}");
}

#[test]
fn map__missing_narrows_to_files_changed_since_the_recorded_commit() {
    let sb = sandbox();
    write_rust_module(&sb, "src/old.rs", None, "pub fn a() {}");
    commit(&sb, &["src/old.rs"], "add old");
    let gen = map(&sb, &[]);
    assert_eq!(code(&gen), 0, "{}", stderr(&gen));
    write_rust_module(&sb, "src/new.rs", None, "pub fn b() {}");
    commit(&sb, &["src/new.rs"], "add new");
    let out = map(&sb, &["--missing"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let s = stdout(&out);
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines, vec!["src/new.rs"], "{lines:?}");
}

#[test]
fn map__missing_all_widens_back_to_every_undescribed_file() {
    let sb = sandbox();
    write_rust_module(&sb, "src/old.rs", None, "pub fn a() {}");
    commit(&sb, &["src/old.rs"], "add old");
    map(&sb, &[]);
    write_rust_module(&sb, "src/new.rs", None, "pub fn b() {}");
    commit(&sb, &["src/new.rs"], "add new");
    let out = map(&sb, &["--missing", "--all"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let s = stdout(&out);
    let lines: Vec<&str> = s.lines().collect();
    assert!(lines.contains(&"src/old.rs"), "{lines:?}");
    assert!(lines.contains(&"src/new.rs"), "{lines:?}");
}

// --- 150-line cap (Task 2) ---------------------------------------------------------------------

#[test]
fn map__a_module_list_over_the_cap_collapses_into_directory_counts() {
    let sb = sandbox();
    let mut files = Vec::new();
    for n in 0..200 {
        let rel = format!("src/gen/f{n:04}.rs");
        write_rust_module(&sb, &rel, None, "pub fn f() {}");
        files.push(rel);
    }
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    commit(&sb, &refs, "add 200 generated files");
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    assert!(
        text.lines().count() <= 150,
        "{} lines",
        text.lines().count()
    );
    assert!(
        text.lines()
            .any(|l| l.contains("src/gen/") && l.contains("files")),
        "expected a collapsed directory summary line:\n{text}"
    );
}

// --- `--wire` (Task 5) --------------------------------------------------------------------------

#[test]
fn map__wire_appends_the_claude_md_import_and_the_gitignore_entry() {
    let sb = sandbox();
    let out = map(&sb, &["--wire"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let claude = std::fs::read_to_string(sb.root().join("CLAUDE.md")).unwrap();
    assert!(claude.contains("@.ratchet/map.md"), "{claude}");
    let gitignore = std::fs::read_to_string(sb.root().join(".gitignore")).unwrap();
    assert!(gitignore.contains(".ratchet/"), "{gitignore}");
}

#[test]
fn map__wire_run_twice_changes_nothing_the_second_time() {
    let sb = sandbox();
    let first = map(&sb, &["--wire"]);
    assert_eq!(code(&first), 0, "{}", stderr(&first));
    let claude_after_first = std::fs::read_to_string(sb.root().join("CLAUDE.md")).unwrap();
    let gitignore_after_first = std::fs::read_to_string(sb.root().join(".gitignore")).unwrap();
    let second = map(&sb, &["--wire"]);
    assert_eq!(code(&second), 0, "{}", stderr(&second));
    assert_eq!(
        std::fs::read_to_string(sb.root().join("CLAUDE.md")).unwrap(),
        claude_after_first
    );
    assert_eq!(
        std::fs::read_to_string(sb.root().join(".gitignore")).unwrap(),
        gitignore_after_first
    );
    assert!(
        stdout(&second).contains("already wired"),
        "{}",
        stdout(&second)
    );
}

#[cfg(unix)]
#[test]
fn map__wire_refuses_a_symlinked_claude_md() {
    let sb = sandbox();
    let target = sb.repo.path().join("elsewhere.md");
    std::fs::write(&target, "not really CLAUDE.md").unwrap();
    crate::support::symlink_claude_md(&sb, &target);
    let out = map(&sb, &["--wire"]);
    assert_eq!(code(&out), 1);
    assert!(
        stderr(&out).contains("not a regular file"),
        "{}",
        stderr(&out)
    );
    assert!(
        !sb.root().join(".gitignore").exists(),
        "gitignore must not be touched when CLAUDE.md refuses"
    );
}

// --- briefing (Task 4) ---------------------------------------------------------------------------

#[test]
fn map__no_map_prints_the_map_none_line() {
    let sb = sandbox();
    let root = sb.root();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-none", &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("map: none — run /ratchet:map for the repo layout"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn map__a_map_behind_head_prints_the_commits_behind_line() {
    let sb = sandbox();
    write_rust_module(&sb, "src/a.rs", None, "pub fn a() {}");
    commit(&sb, &["src/a.rs"], "add a");
    map(&sb, &[]);
    write_rust_module(&sb, "src/b.rs", None, "pub fn b() {}");
    commit(&sb, &["src/b.rs"], "add b");
    let root = sb.root();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-behind", &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("map: 1 commits behind — run /ratchet:map"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn map__a_map_from_another_branch_prints_the_from_another_branch_line() {
    let sb = sandbox();
    write_rust_module(&sb, "src/a.rs", None, "pub fn a() {}");
    commit(&sb, &["src/a.rs"], "add a");
    map(&sb, &[]);
    // Amending the last commit orphans the recorded sha: it becomes a sibling, not an ancestor,
    // of the new HEAD — exactly what `git merge-base --is-ancestor` reports as `false`.
    crate::support::git(
        sb.repo.path(),
        &["commit", "--amend", "-q", "-m", "amended"],
    );
    let root = sb.root();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-other", &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("map: from another branch — run /ratchet:map"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn map__a_current_map_prints_no_line() {
    let sb = sandbox();
    write_rust_module(&sb, "src/a.rs", None, "pub fn a() {}");
    commit(&sb, &["src/a.rs"], "add a");
    map(&sb, &[]);
    let root = sb.root();
    let out = hook_env(
        &sb,
        "session-start",
        &session_payload("s-current", &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        !stdout(&out).lines().any(|l| l.contains("map:")),
        "{}",
        stdout(&out)
    );
}

// --- `map status` (Task 2) -----------------------------------------------------------------------

#[test]
fn map__map_status_prints_the_same_line_the_briefing_would_show() {
    let sb = sandbox();
    write_rust_module(&sb, "src/a.rs", None, "pub fn a() {}");
    commit(&sb, &["src/a.rs"], "add a");
    map(&sb, &[]);
    write_rust_module(&sb, "src/b.rs", None, "pub fn b() {}");
    commit(&sb, &["src/b.rs"], "add b");
    let root = sb.root();
    let briefing = hook_env(
        &sb,
        "session-start",
        &session_payload("s-cmp", &root),
        &root,
        &[("RATCHET_NOW", T0)],
    );
    let status = map(&sb, &["status"]);
    assert_eq!(code(&status), 0, "{}", stderr(&status));
    let briefing_line = stdout(&briefing)
        .lines()
        .find(|l| l.starts_with("map:"))
        .unwrap()
        .to_string();
    assert_eq!(stdout(&status).trim(), briefing_line);
}

#[test]
fn map__map_status_prints_nothing_outside_a_marker_repo() {
    let dir = unmanaged_dir();
    let home = tempfile::TempDir::new().unwrap();
    let out = map_at(dir.path(), home.path(), &["status"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).trim().is_empty(), "{}", stdout(&out));
    assert!(stderr(&out).trim().is_empty(), "{}", stderr(&out));
}

// --- `[map]` config (Task 2) ----------------------------------------------------------------------

#[test]
fn map__map_exclude_leaves_matching_files_out_of_the_map() {
    let sb = sandbox();
    write_rust_module(
        &sb,
        "vendor/blob.rs",
        Some("Vendored, should never appear."),
        "pub fn f() {}",
    );
    write_rust_module(
        &sb,
        "src/a.rs",
        Some("Ours, should appear."),
        "pub fn a() {}",
    );
    commit(&sb, &["vendor/blob.rs", "src/a.rs"], "add vendor and a");
    sb.write_marker(
        "[repo]\ndefault_branch = \"main\"\nworktrees_dir = \".worktrees\"\n\n[map]\nexclude = [\"vendor/*\"]\n",
    );
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    assert!(!text.contains("vendor/blob.rs"), "{text}");
    assert!(text.contains("src/a.rs"), "{text}");
}

#[test]
fn map__map_gate_replaces_detected_gate_commands_entirely() {
    let sb = sandbox();
    write_plain_file(
        &sb,
        "Cargo.toml",
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
    );
    commit(&sb, &["Cargo.toml"], "add Cargo.toml");
    sb.write_marker(
        "[repo]\ndefault_branch = \"main\"\nworktrees_dir = \".worktrees\"\n\n[map]\ngate = [\"make check\"]\n",
    );
    let out = map(&sb, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = read_map(&sb);
    assert!(text.contains("make check"), "{text}");
    assert!(!text.contains("cargo fmt"), "{text}");
}
