//! context-meter: the module's scenarios are TypeScript tests run by `claude plugin test`;
//! each function here asserts that one cached run of them printed `(pass) <scenario title>`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

const INSTALL: &str = "npm install -g @anthropic-ai/claude-code@2.1.288";

fn repo_root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let s = root.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => root,
    }
}

/// Runs `claude <args> <repo root>`; stdout and stderr together, and the exit success.
fn claude(args: &[&str], target: &str) -> (bool, String) {
    let mut last_err = None;
    for name in ["claude", "claude.cmd"] {
        match Command::new(name)
            .args(args)
            .arg(repo_root().join(target))
            .output()
        {
            Ok(out) => {
                let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&out.stderr));
                return (out.status.success(), text);
            }
            Err(e) => last_err = Some(e),
        }
    }
    panic!(
        "cannot run `claude` ({}): install the version the module is tested against with `{INSTALL}`",
        last_err.unwrap()
    );
}

fn test_run() -> &'static str {
    static RUN: OnceLock<String> = OnceLock::new();
    RUN.get_or_init(|| claude(&["plugin", "test"], "").1)
}

fn assert_passed(title: &str) {
    let out = test_run();
    let want = format!("(pass) {title}");
    // The runner appends ` [12.3ms]` to the name; a longer title would continue otherwise.
    let printed = out
        .lines()
        .any(|l| match l.trim_start().strip_prefix(&want) {
            Some(rest) => rest.is_empty() || rest.starts_with(" ["),
            None => false,
        });
    assert!(
        printed,
        "`claude plugin test` did not print `{want}`; output:\n{out}"
    );
}

#[test]
fn context_meter__the_plugin_validates_with_the_module_declared() {
    let (ok, out) = claude(&["plugin", "validate"], ".claude-plugin/plugin.json");
    assert!(ok, "`claude plugin validate` failed:\n{out}");
    for needle in [
        "session.start",
        "session.measure",
        "turn.step",
        "ui.render{component=Pane",
    ] {
        assert!(
            out.contains(needle),
            "validate output does not name `{needle}`:\n{out}"
        );
    }
}

#[test]
fn context_meter__without_ratchet_toml_the_meter_stays_silent() {
    assert_passed("Without ratchet.toml the meter stays silent");
}

#[test]
fn context_meter__a_ratchet_toml_in_an_ancestor_opts_the_session_in() {
    assert_passed("A ratchet.toml in an ancestor opts the session in");
}

#[test]
fn context_meter__a_directory_named_ratchet_toml_does_not_opt_in() {
    assert_passed("A directory named ratchet.toml does not opt in");
}

#[test]
fn context_meter__a_measurement_pins_the_fill_on_the_status_line() {
    assert_passed("A measurement pins the fill on the status line");
}

#[test]
fn context_meter__before_the_first_response_the_line_says_there_is_no_reading() {
    assert_passed("Before the first response the line says there is no reading");
}

#[test]
fn context_meter__a_main_loop_request_moves_the_line_within_a_turn() {
    assert_passed("A main-loop request moves the line within a turn");
}

#[test]
fn context_meter__a_measurement_that_moved_only_the_cost_leaves_the_line_alone() {
    assert_passed("A measurement that moved only the cost leaves the line alone");
}

#[test]
fn context_meter__a_million_token_window_is_written_in_m() {
    assert_passed("A million-token window is written in M");
}

#[test]
fn context_meter__each_threshold_toasts_once_and_a_drop_re_arms_it() {
    assert_passed("Each threshold toasts once, and a drop re-arms it");
}

#[test]
fn context_meter__a_running_subagent_s_requests_tail_the_status_line() {
    assert_passed("A running subagent's requests tail the status line");
}

#[test]
fn context_meter__a_finished_subagent_leaves_the_status_line() {
    assert_passed("A finished subagent leaves the status line");
}

#[test]
fn context_meter__the_pane_draws_the_breakdown_on_every_surface() {
    assert_passed("The pane draws the breakdown on every surface");
}

#[test]
fn context_meter__the_pane_lists_subagents_with_their_fill() {
    assert_passed("The pane lists subagents with their fill");
}

#[test]
fn context_meter__a_finished_subagent_stays_in_the_pane() {
    assert_passed("A finished subagent stays in the pane");
}

#[test]
fn context_meter__a_subagent_past_most_of_its_window_is_drawn_in_the_error_colour_with_its_peak() {
    assert_passed("A subagent past most of its window is drawn in the error colour with its peak");
}

#[test]
fn context_meter__the_pane_says_so_before_the_first_response() {
    assert_passed("The pane says so before the first response");
}

#[test]
fn context_meter__a_failing_reading_never_fails_a_model_request() {
    assert_passed("A failing reading never fails a model request");
}
