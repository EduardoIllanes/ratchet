//! One test per `#### Scenario` of openspec/specs/usage/spec.md, named by slug. `RATCHET_HOME`
//! isolates the database (as every other spec test file does); `RATCHET_CLAUDE_PROJECTS` (set by
//! `support::usage`) isolates the transcript tree in the same way.
//!
//! NOTE (Task 1 tension, flagged for the owner): the plan's brief names a token-tuple builder
//! and the `ratchet usage` process runner both `usage` in `support.rs`, which cannot coexist in
//! one Rust module (E0428 -- no overloading). The token-tuple builder is called `tokens(...)`
//! here instead; `usage(...)` stays the process runner, matching its own doc comment ("Run
//! `ratchet usage <args>` ... every `usage__*` scenario test goes through this helper"). See
//! `support.rs` for the same note at the definition site.

use crate::support::*;
use serde_json::Value;

// --- Requirement: Transcripts are located per session from its working directory -----------

#[test]
fn usage__a_worktree_session_is_found_under_its_own_slug() {
    let sb = sandbox();
    let wt = worktree(&sb);
    let out = start_session(&sb, "s-1", &wt, T0);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let id = new_task(&sb, "wt task", &[], "s-1", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-1", 2)), 0);

    let tb = TranscriptBuilder::new();
    tb.call(
        &wt,
        "s-1",
        &at(3),
        "claude-sonnet-5",
        &tokens(100, 0, 0, 50),
    );

    let out = usage(&sb, &tb, &[], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(!stdout(&out).contains("no transcript"), "{}", stdout(&out));
    assert!(stdout(&out).contains(&id), "{}", stdout(&out));

    let detail = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&detail), 0, "{}", stderr(&detail));
    assert!(
        stdout(&detail).contains("orchestrator"),
        "{}",
        stdout(&detail)
    );
    assert!(
        !stdout(&detail).contains("no transcript"),
        "{}",
        stdout(&detail)
    );
}

#[test]
fn usage__a_session_without_a_transcript_is_reported_not_an_error() {
    let sb = board("s-2");
    let id = new_task(&sb, "no transcript task", &[], "s-2", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-2", 2)), 0);
    let tb = TranscriptBuilder::new(); // nothing ever written for s-2
    let out = usage(&sb, &tb, &[], &sb.root(), &[("RATCHET_NOW", &at(3))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("no transcript"), "{}", stdout(&out));
    assert!(stdout(&out).contains("s-2"), "{}", stdout(&out));
}

#[test]
fn usage__missing_projects_directory_fails_naming_the_path() {
    let sb = board("s-3");
    let missing = sb.scratchpad.path().join("does-not-exist");
    let out = cli(
        &sb,
        &["usage"],
        &sb.root(),
        &[("RATCHET_CLAUDE_PROJECTS", &missing.to_string_lossy())],
    );
    assert_eq!(code(&out), 1, "{}", stdout(&out));
    assert!(
        stderr(&out).contains(&missing.to_string_lossy().to_string()),
        "{}",
        stderr(&out)
    );
}

// --- Requirement: Reading is tolerant and reports what it did not understand ----------------

#[test]
fn usage__garbage_lines_and_a_record_without_usage_are_counted() {
    let sb = board("s-4");
    let id = new_task(&sb, "tolerant", &[], "s-4", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-4", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-4",
        &at(3),
        "claude-sonnet-5",
        &tokens(100, 0, 0, 50),
    )
    .garbage(&sb.root(), "s-4")
    .call_no_usage(&sb.root(), "s-4", &at(4), "claude-sonnet-5")
    .call(
        &sb.root(),
        "s-4",
        &at(5),
        "claude-sonnet-5",
        &tokens(200, 0, 0, 60),
    );

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(6))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("skipped 1"), "{text}");
    assert!(text.contains("partial 1"), "{text}");
    assert!(text.contains("1.2.3"), "{text}"); // the highest version seen
    assert!(text.contains("300"), "{text}"); // 100 + 200, the two understood calls
}

#[test]
fn usage__a_transcript_with_nothing_understood_is_skipped_once() {
    let sb = board("s-5");
    let id = new_task(&sb, "nothing understood", &[], "s-5", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-5", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.non_assistant(&sb.root(), "s-5");
    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(3))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("skipped 1"), "{}", stdout(&out));
}

// --- Requirement: A call belongs to the task its session held at that instant ---------------

#[test]
fn usage__one_task_one_session_orchestrator_only() {
    let sb = board("s-6");
    let id = new_task(&sb, "one task", &[], "s-6", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-6", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-6",
        &at(3),
        "claude-sonnet-5",
        &tokens(100, 0, 0, 50),
    )
    .call(
        &sb.root(),
        "s-6",
        &at(4),
        "claude-sonnet-5",
        &tokens(100, 0, 0, 50),
    )
    .call(
        &sb.root(),
        "s-6",
        &at(5),
        "claude-sonnet-5",
        &tokens(100, 0, 0, 50),
    );
    assert_eq!(code(&task(&sb, &["status", &id, "review"], "s-6", 6)), 0);

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(7))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("orchestrator"), "{text}");
    assert!(text.contains("300"), "{text}"); // 3 x 100 input, summed
    assert_eq!(
        text.matches("orchestrator").count(),
        1,
        "one row per model, not one per call: {text}"
    );
}

#[test]
fn usage__two_tasks_held_in_sequence_split_the_session() {
    let sb = board("s-7");
    let t1 = new_task(&sb, "first", &[], "s-7", 1);
    let t2 = new_task(&sb, "second", &[], "s-7", 2);
    assert_eq!(code(&task(&sb, &["claim", &t1], "s-7", 3)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-7",
        &at(4),
        "claude-sonnet-5",
        &tokens(100, 0, 0, 10),
    )
    .call(
        &sb.root(),
        "s-7",
        &at(5),
        "claude-sonnet-5",
        &tokens(100, 0, 0, 10),
    );
    assert_eq!(
        code(&task(
            &sb,
            &["status", &t1, "done", "--why", "closing for the test"],
            "s-7",
            6
        )),
        0
    );
    assert_eq!(code(&task(&sb, &["claim", &t2], "s-7", 7)), 0);
    tb.call(
        &sb.root(),
        "s-7",
        &at(8),
        "claude-sonnet-5",
        &tokens(50, 0, 0, 5),
    );

    let d1 = usage(&sb, &tb, &[&t1], &sb.root(), &[("RATCHET_NOW", &at(9))]);
    assert_eq!(code(&d1), 0, "{}", stderr(&d1));
    assert!(stdout(&d1).contains("200"), "{}", stdout(&d1)); // 100 + 100
    assert!(!stdout(&d1).contains("250"), "{}", stdout(&d1)); // never the sum of both tasks

    let d2 = usage(&sb, &tb, &[&t2], &sb.root(), &[("RATCHET_NOW", &at(9))]);
    assert_eq!(code(&d2), 0, "{}", stderr(&d2));
    assert!(stdout(&d2).contains("50"), "{}", stdout(&d2));
    assert!(!stdout(&d2).contains("150"), "{}", stdout(&d2));
}

#[test]
fn usage__calls_outside_any_held_task_are_unassigned() {
    let sb = board("s-8");
    let id = new_task(&sb, "unassigned test", &[], "s-8", 1);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-8",
        &at(2),
        "claude-sonnet-5",
        &tokens(10, 0, 0, 1),
    )
    .call(
        &sb.root(),
        "s-8",
        &at(3),
        "claude-sonnet-5",
        &tokens(10, 0, 0, 1),
    );
    assert_eq!(code(&task(&sb, &["claim", &id], "s-8", 4)), 0);
    tb.call(
        &sb.root(),
        "s-8",
        &at(5),
        "claude-sonnet-5",
        &tokens(100, 0, 0, 10),
    );
    assert_eq!(
        code(&task(
            &sb,
            &["status", &id, "done", "--why", "closing"],
            "s-8",
            6
        )),
        0
    );
    tb.call(
        &sb.root(),
        "s-8",
        &at(7),
        "claude-sonnet-5",
        &tokens(20, 0, 0, 2),
    );

    let out = usage(
        &sb,
        &tb,
        &["--by", "session"],
        &sb.root(),
        &[("RATCHET_NOW", &at(8))],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("unassigned"), "{text}");
    assert!(text.contains("40"), "{text}"); // 10 + 10 + 20, outside the hold
}

// --- Requirement: Subagent calls are attributed through ratchet's own events first ----------

#[test]
fn usage__a_subagent_is_attributed_through_its_start_event() {
    let sb = board("s-9");
    let id = new_task(&sb, "subagent via event", &[], "s-9", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-9", 2)), 0);
    seed_subagent_event(
        &sb,
        "subagent.start",
        "s-9",
        "a1",
        "ratchet:reviewer",
        "",
        &id,
        &at(3),
    );
    let tb = TranscriptBuilder::new();
    // No agent-a1.meta.json at all -- the event alone must be enough.
    tb.subagent(
        &sb.root(),
        "s-9",
        "a1",
        &at(3),
        "claude-opus-4",
        &tokens(500, 0, 0, 80),
    );

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("ratchet:reviewer"),
        "{}",
        stdout(&out)
    );
    assert!(stdout(&out).contains("500"), "{}", stdout(&out));
}

#[test]
fn usage__a_subagent_is_attributed_through_tooluseid_when_no_event_exists() {
    let sb = board("s-10");
    let id = new_task(&sb, "subagent via tool use id", &[], "s-10", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-10", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call_with_tool(
        &sb.root(),
        "s-10",
        &at(3),
        "claude-sonnet-5",
        &tokens(20, 0, 0, 5),
        "Agent",
        "tu-9",
    )
    .subagent(
        &sb.root(),
        "s-10",
        "a2",
        &at(3),
        "claude-sonnet-5",
        &tokens(300, 0, 0, 40),
    );
    tb.meta(
        &sb.root(),
        "s-10",
        "a2",
        "ratchet:implementer",
        "irrelevant",
        "claude-sonnet-5",
        Some("tu-9"),
    );
    // No subagent.start/stop event for "a2" at all.

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("ratchet:implementer"),
        "{}",
        stdout(&out)
    );
    assert!(stdout(&out).contains("300"), "{}", stdout(&out));
}

#[test]
fn usage__a_subagent_is_attributed_by_first_timestamp_when_both_are_missing() {
    let sb = board("s-11");
    let id = new_task(&sb, "subagent by timestamp", &[], "s-11", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-11", 2)), 0);
    let tb = TranscriptBuilder::new();
    // No parent Agent tool_use, no meta file, no event -- only the subagent transcript exists,
    // and its first record's timestamp falls while the task is held.
    tb.subagent(
        &sb.root(),
        "s-11",
        "a3",
        &at(3),
        "claude-sonnet-5",
        &tokens(70, 0, 0, 9),
    );

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(stdout(&out).contains("subagent"), "{}", stdout(&out));
    assert!(stdout(&out).contains("70"), "{}", stdout(&out));
}

#[test]
fn usage__a_general_purpose_subagent_shows_its_description() {
    let sb = board("s-12");
    let id = new_task(&sb, "general purpose subagent", &[], "s-12", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-12", 2)), 0);
    let description = "search the whole repository for every remaining caller of the old helper";
    seed_subagent_event(
        &sb,
        "subagent.start",
        "s-12",
        "a4",
        "general-purpose",
        description,
        &id,
        &at(3),
    );
    let tb = TranscriptBuilder::new();
    tb.subagent(
        &sb.root(),
        "s-12",
        "a4",
        &at(3),
        "claude-sonnet-5",
        &tokens(40, 0, 0, 5),
    );

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    // Computed by the same rule production code uses ("first 40 characters"), not hand-counted —
    // see Assumption 5 at the top of this plan for why the spec's own prose example is not
    // reproduced verbatim here.
    let clipped: String = description.chars().take(40).collect();
    assert!(
        stdout(&out).contains(&format!("general-purpose: {clipped}")),
        "{}",
        stdout(&out)
    );
    assert!(
        !stdout(&out).contains("caller of the old helper"),
        "{}",
        stdout(&out)
    );
}

// --- Requirement: Orientation is the orchestrator's cost before real work starts ------------

#[test]
fn usage__calls_before_the_first_claim_are_orientation() {
    let sb = board("s-13");
    let id = new_task(&sb, "orientation via claim", &[], "s-13", 1);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-13",
        &at(2),
        "claude-sonnet-5",
        &tokens(30, 0, 0, 4),
    )
    .call(
        &sb.root(),
        "s-13",
        &at(3),
        "claude-sonnet-5",
        &tokens(30, 0, 0, 4),
    );
    assert_eq!(code(&task(&sb, &["claim", &id], "s-13", 4)), 0);
    tb.call(
        &sb.root(),
        "s-13",
        &at(5),
        "claude-sonnet-5",
        &tokens(90, 0, 0, 10),
    )
    .call(
        &sb.root(),
        "s-13",
        &at(6),
        "claude-sonnet-5",
        &tokens(90, 0, 0, 10),
    )
    .call(
        &sb.root(),
        "s-13",
        &at(7),
        "claude-sonnet-5",
        &tokens(90, 0, 0, 10),
    );

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(8))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("orientation"), "{text}");
    assert!(text.contains("60"), "{text}"); // 30 + 30, the two calls before the claim
}

#[test]
fn usage__a_dispatch_ends_orientation_without_a_claim() {
    let sb = board("s-14");
    let id = new_task(&sb, "orientation via dispatch", &[], "s-14", 1);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-14",
        &at(2),
        "claude-sonnet-5",
        &tokens(25, 0, 0, 3),
    )
    .call_with_tool(
        &sb.root(),
        "s-14",
        &at(3),
        "claude-sonnet-5",
        &tokens(25, 0, 0, 3),
        "Agent",
        "tu-1",
    )
    .call(
        &sb.root(),
        "s-14",
        &at(4),
        "claude-sonnet-5",
        &tokens(80, 0, 0, 9),
    )
    .call(
        &sb.root(),
        "s-14",
        &at(5),
        "claude-sonnet-5",
        &tokens(80, 0, 0, 9),
    );
    assert_eq!(code(&task(&sb, &["claim", &id], "s-14", 6)), 0);

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(7))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("orientation"), "{text}");
    assert!(text.contains("50"), "{text}"); // 25 + 25 -- the dispatching call itself is included
}

#[test]
fn usage__a_fix_round_after_a_review_verdict_still_belongs_to_the_task() {
    let sb = board("s-27");
    let id = new_task(&sb, "fix round", &[], "s-27", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-27", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-27",
        &at(3),
        "claude-sonnet-5",
        &tokens(10, 0, 0, 1),
    );
    assert_eq!(code(&task(&sb, &["status", &id, "review"], "s-27", 4)), 0);
    // No reclaim, no status change: this is how fix rounds actually happen on the board.
    tb.call(
        &sb.root(),
        "s-27",
        &at(5),
        "claude-sonnet-5",
        &tokens(30, 0, 0, 3),
    )
    .call(
        &sb.root(),
        "s-27",
        &at(6),
        "claude-sonnet-5",
        &tokens(30, 0, 0, 3),
    );

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(7))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("70"), "{text}"); // 10 + 30 + 30, all on the task
    assert!(!text.contains("unassigned"), "{text}");
}

// --- Requirement: Review rounds are counted from status events ------------------------------

#[test]
fn usage__two_review_rounds_and_tokens_per_round() {
    let sb = board("s-15");
    let id = new_task(&sb, "rounds", &[], "s-15", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-15", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-15",
        &at(3),
        "claude-sonnet-5",
        &tokens(40, 0, 0, 4),
    );
    assert_eq!(code(&task(&sb, &["status", &id, "review"], "s-15", 4)), 0);
    // The fix round runs with the task still in `review`: no reclaim, no status change
    // (Assumption 3 -- `review` does not close the hold). The bare `status ... in_progress`
    // afterwards only exists so the second `status ... review` is a real transition.
    tb.call(
        &sb.root(),
        "s-15",
        &at(5),
        "claude-sonnet-5",
        &tokens(70, 0, 0, 7),
    )
    .call(
        &sb.root(),
        "s-15",
        &at(6),
        "claude-sonnet-5",
        &tokens(70, 0, 0, 7),
    );
    assert_eq!(
        code(&task(&sb, &["status", &id, "in_progress"], "s-15", 7)),
        0
    );
    assert_eq!(code(&task(&sb, &["status", &id, "review"], "s-15", 8)), 0);

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(9))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("rounds 2"), "{text}");
    assert!(text.contains("140"), "{text}"); // 70 + 70, round 2's tokens
}

// --- Requirement: Weights add a cost column, and only then ----------------------------------

#[test]
fn usage__weights_present_add_cost() {
    let sb = board("s-16");
    let id = new_task(&sb, "weighted", &[], "s-16", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-16", 2)), 0);
    std::fs::write(
        sb.home.path().join("config.toml"),
        "[usage.weights.\"claude-sonnet\"]\ninput = 3.0\ncache_write = 3.75\ncache_read = 0.3\noutput = 15.0\n",
    )
    .unwrap();
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-16",
        &at(3),
        "claude-sonnet-5",
        &tokens(1_000_000, 0, 0, 1_000_000),
    );

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    // 1M input @ $3/M + 1M output @ $15/M = $18.00
    assert!(stdout(&out).contains("cost"), "{}", stdout(&out));
    assert!(stdout(&out).contains("18"), "{}", stdout(&out));
}

#[test]
fn usage__no_weights_no_cost() {
    let sb = board("s-17");
    let id = new_task(&sb, "unweighted", &[], "s-17", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-17", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-17",
        &at(3),
        "claude-sonnet-5",
        &tokens(100, 0, 0, 10),
    );

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert!(
        !stdout(&out).to_lowercase().contains("cost"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn usage__the_longest_matching_prefix_wins() {
    let sb = board("s-18");
    let id = new_task(&sb, "prefix match", &[], "s-18", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-18", 2)), 0);
    std::fs::write(
        sb.home.path().join("config.toml"),
        "[usage.weights.\"claude\"]\ninput = 1.0\ncache_write = 1.0\ncache_read = 1.0\noutput = 1.0\n\
         [usage.weights.\"claude-opus\"]\ninput = 100.0\ncache_write = 0\ncache_read = 0\noutput = 0\n",
    )
    .unwrap();
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-18",
        &at(3),
        "claude-opus-5",
        &tokens(1_000_000, 0, 0, 0),
    );

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    // "claude" ($1/M) would give $1.00; "claude-opus" ($100/M) gives $100.00.
    assert!(stdout(&out).contains("100"), "{}", stdout(&out));
}

// --- Requirement: Reports come in four shapes and a time window -----------------------------

#[test]
fn usage__default_listing_shows_tasks_of_the_last_seven_days() {
    let sb = board("s-19");
    let recent = new_task(&sb, "recent", &[], "s-19", 1);
    assert_eq!(code(&task(&sb, &["claim", &recent], "s-19", 2)), 0);
    let old = new_task(&sb, "old", &[], "s-19", 3);
    assert_eq!(code(&task(&sb, &["claim", &old], "s-19", 4)), 0);
    // Backdating the old task's only events is simpler than fighting the CLI's own clock to make
    // "20 days ago" happen for real.
    db(&sb)
        .execute(
            "UPDATE events SET ts = ?1 WHERE task_id = ?2",
            rusqlite::params!["2026-08-27T12:00:00Z", old],
        )
        .unwrap();
    let tb = TranscriptBuilder::new();
    let out = usage(&sb, &tb, &[], &sb.root(), &[("RATCHET_NOW", &at(5))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains(&recent), "{text}");
    assert!(!text.contains(&old), "{text}");
}

#[test]
fn usage__since_widens_the_window() {
    let sb = board("s-20");
    let recent = new_task(&sb, "recent2", &[], "s-20", 1);
    assert_eq!(code(&task(&sb, &["claim", &recent], "s-20", 2)), 0);
    let old = new_task(&sb, "old2", &[], "s-20", 3);
    assert_eq!(code(&task(&sb, &["claim", &old], "s-20", 4)), 0);
    db(&sb)
        .execute(
            "UPDATE events SET ts = ?1 WHERE task_id = ?2",
            rusqlite::params!["2026-08-27T12:00:00Z", old],
        )
        .unwrap();
    let tb = TranscriptBuilder::new();
    let narrow = usage(&sb, &tb, &[], &sb.root(), &[("RATCHET_NOW", &at(5))]);
    assert!(!stdout(&narrow).contains(&old), "{}", stdout(&narrow));
    let wide = usage(
        &sb,
        &tb,
        &["--since", "30d"],
        &sb.root(),
        &[("RATCHET_NOW", &at(5))],
    );
    assert_eq!(code(&wide), 0, "{}", stderr(&wide));
    assert!(stdout(&wide).contains(&recent), "{}", stdout(&wide));
    assert!(stdout(&wide).contains(&old), "{}", stdout(&wide));
}

#[test]
fn usage__by_role_aggregates_across_tasks() {
    let sb = board("s-21");
    let t1 = new_task(&sb, "role agg 1", &[], "s-21", 1);
    let t2 = new_task(&sb, "role agg 2", &[], "s-21", 2);
    assert_eq!(code(&task(&sb, &["claim", &t1], "s-21", 3)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-21",
        &at(4),
        "claude-sonnet-5",
        &tokens(50, 0, 0, 5),
    );
    seed_subagent_event(
        &sb,
        "subagent.start",
        "s-21",
        "b1",
        "ratchet:reviewer",
        "",
        &t1,
        &at(4),
    );
    tb.subagent(
        &sb.root(),
        "s-21",
        "b1",
        &at(4),
        "claude-sonnet-5",
        &tokens(60, 0, 0, 6),
    );
    assert_eq!(
        code(&task(
            &sb,
            &["status", &t1, "done", "--why", "closing"],
            "s-21",
            5
        )),
        0
    );

    assert_eq!(code(&task(&sb, &["claim", &t2], "s-21", 6)), 0);
    tb.call(
        &sb.root(),
        "s-21",
        &at(7),
        "claude-sonnet-5",
        &tokens(70, 0, 0, 7),
    );
    seed_subagent_event(
        &sb,
        "subagent.start",
        "s-21",
        "b2",
        "ratchet:reviewer",
        "",
        &t2,
        &at(7),
    );
    tb.subagent(
        &sb.root(),
        "s-21",
        "b2",
        &at(7),
        "claude-sonnet-5",
        &tokens(80, 0, 0, 8),
    );

    let out = usage(
        &sb,
        &tb,
        &["--by", "role"],
        &sb.root(),
        &[("RATCHET_NOW", &at(8))],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("orchestrator"), "{text}");
    assert!(text.contains("120"), "{text}"); // 50 + 70
    assert!(text.contains("ratchet:reviewer"), "{text}");
    assert!(text.contains("140"), "{text}"); // 60 + 80
    assert!(text.contains("review rounds per task"), "{text}");
    assert!(text.contains("orientation per session"), "{text}");
}

#[test]
fn usage__numbers_are_abbreviated() {
    let sb = board("s-22");
    let id = new_task(&sb, "abbreviated", &[], "s-22", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-22", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-22",
        &at(3),
        "claude-sonnet-5",
        &tokens(1234, 0, 2_500_000, 1),
    );

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("1.2k"), "{text}");
    assert!(text.contains("2.5M"), "{text}");
}

// --- Requirement: `--json` exposes the same data for scripts ---------------------------------

#[test]
fn usage__json_shape() {
    let sb = board("s-23");
    let id = new_task(&sb, "json", &[], "s-23", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-23", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-23",
        &at(3),
        "claude-sonnet-5",
        &tokens(10, 0, 0, 1),
    );

    let out = usage(
        &sb,
        &tb,
        &[&id, "--json"],
        &sb.root(),
        &[("RATCHET_NOW", &at(4))],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["tasks"][0]["id"], id);
    assert!(v["tasks"][0]["buckets"][0]["tokens"]["input"].is_number());
    assert!(v["tasks"][0]["buckets"][0].get("cost").is_none());
    assert!(v.get("skipped").is_some());
    assert!(v.get("partial").is_some());
}

// --- Requirement: `--note` writes the one-task summary to the board and nothing else --------

#[test]
fn usage__note_appends_the_summary() {
    let sb = board("s-24");
    let id = new_task(&sb, "note me", &[], "s-24", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-24", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-24",
        &at(3),
        "claude-sonnet-5",
        &tokens(100, 0, 0, 10),
    );

    let out = usage(
        &sb,
        &tb,
        &[&id, "--note"],
        &sb.root(),
        &[("RATCHET_SESSION_ID", "s-24"), ("RATCHET_NOW", &at(4))],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let shown = stdout(&task(&sb, &["show", &id], "s-24", 5));
    assert!(shown.contains("usage:"), "{shown}");
    assert!(shown.contains("100"), "{shown}");
    assert!(shown.contains("rounds"), "{shown}");
}

#[test]
fn usage__note_on_a_missing_task_is_refused() {
    let sb = board("s-25");
    let tb = TranscriptBuilder::new();
    let expected = task(&sb, &["note", "T-9999", "x"], "s-25", 1);
    let out = usage(
        &sb,
        &tb,
        &["T-9999", "--note"],
        &sb.root(),
        &[("RATCHET_SESSION_ID", "s-25"), ("RATCHET_NOW", &at(2))],
    );
    assert_eq!(code(&out), 1, "{}", stdout(&out));
    assert_eq!(
        stderr(&out),
        stderr(&expected),
        "must refuse exactly like `ratchet task note` does"
    );
    assert_eq!(
        count(
            &sb,
            "SELECT COUNT(*) FROM events WHERE task_id = 'T-9999'",
            &[]
        ),
        0
    );
}

#[test]
fn usage__without_note_nothing_is_written() {
    let sb = board("s-26");
    let id = new_task(&sb, "no note", &[], "s-26", 1);
    assert_eq!(code(&task(&sb, &["claim", &id], "s-26", 2)), 0);
    let tb = TranscriptBuilder::new();
    tb.call(
        &sb.root(),
        "s-26",
        &at(3),
        "claude-sonnet-5",
        &tokens(10, 0, 0, 1),
    );
    let before: i64 = count(&sb, "SELECT COUNT(*) FROM events", &[]);

    let out = usage(&sb, &tb, &[&id], &sb.root(), &[("RATCHET_NOW", &at(4))]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        count(&sb, "SELECT COUNT(*) FROM events", &[]),
        before,
        "no event was written"
    );
}
