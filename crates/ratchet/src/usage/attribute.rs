//! Joins parsed transcript calls with the sessions/events ratchet already records, and derives
//! every number Requirement 3 through 8 of `openspec/specs/usage/spec.md` define: which task (if
//! any) a call belongs to, a subagent's task and role, orientation, review rounds, and the
//! abbreviated/weighted numbers a report prints. Pure: no filesystem, no clock, no database —
//! `cli::usage_cmd` (Task 4) reads everything this module needs and hands it over as plain data.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::config::ModelWeights;
use crate::usage::transcript::{self, Meta, Usage};
use crate::usage::weights;

/// Sum of four token classes plus thinking. `Serialize`s with exactly the field names
/// Requirement 9 (`--json`) specifies. `thinking` is already counted inside `output` on the wire
/// (design §4: "out, with thinking inside") — tracked separately here only for display, and
/// `all()` does not double-add it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Totals {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub thinking: u64,
}

impl Totals {
    pub fn add(&mut self, u: &Usage) {
        self.input += u.input;
        self.cache_write += u.cache_write;
        self.cache_read += u.cache_read;
        self.output += u.output;
        self.thinking += u.thinking;
    }

    /// Every class summed; `thinking` excluded on purpose (already inside `output`).
    pub fn all(&self) -> u64 {
        self.input + self.cache_write + self.cache_read + self.output
    }
}

/// `k`/`M` with one decimal, per design §4's column rule. Below 1000 prints as-is. The `k` form
/// is computed first and only used when it does not itself round to `1000.0` or more (e.g.
/// `999_950` prints `1.0M`, not `1000.0k`) — the fix for a rounding edge the first pass missed.
pub fn abbreviate(n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    if n < 1_000_000 {
        let k = n as f64 / 1_000.0;
        if (k * 10.0).round() / 10.0 < 1000.0 {
            return format!("{:.1}k", k);
        }
    }
    format!("{:.1}M", n as f64 / 1_000_000.0)
}

/// "7d" / "30d" / an RFC 3339 date (`2026-09-01`, midnight UTC). The caller supplies the default
/// (`"7d"`) when `--since` is absent — this function does not know about defaults.
pub fn since_cutoff(spec: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
    let spec = spec.trim();
    if let Some(days) = spec.strip_suffix('d') {
        let n: i64 = days
            .parse()
            .map_err(|_| format!("invalid --since: {spec:?}"))?;
        if n <= 0 {
            return Err(format!("invalid --since: {spec:?}"));
        }
        return Ok(now - Duration::days(n));
    }
    let with_time = format!("{spec}T00:00:00Z");
    crate::clock::parse(&with_time).ok_or_else(|| format!("invalid --since: {spec:?}"))
}

/// `general-purpose` shows with the first 40 characters of its description and nothing more of
/// it; every other role prints as-is (D-usage-subagents).
pub fn display_role(role: &str, description: Option<&str>) -> String {
    if role == "general-purpose" {
        let clipped: String = description.unwrap_or_default().chars().take(40).collect();
        format!("general-purpose: {clipped}")
    } else {
        role.to_string()
    }
}

/// The two event kinds `holds` needs, already extracted from `events.kind`/`events.payload` by
/// the caller (`cli::usage_cmd`, Task 4, via `services::events::for_session`): a `task.claimed`
/// row becomes `Claimed`, a `task.status` row becomes `Status` with `to` read from
/// `payload.to`.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEventKind {
    Claimed { task: String },
    Status { task: String, to: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionEvent {
    pub ts: DateTime<Utc>,
    pub kind: SessionEventKind,
}

/// One interval during which a session held one task, half-open `[start, end)`. `end: None`
/// means "still held" — `holds()` always closes these against `session_end` before returning.
#[derive(Debug, Clone, PartialEq)]
pub struct Hold {
    pub task: String,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
}

/// R3: a hold opens on `task.claimed`. A `task.status` event for that same task whose `to` is
/// `done`, `blocked` or `ready` closes EVERY hold currently open on that task at that timestamp
/// — not just the oldest one. A session may re-claim a task it already holds
/// (`services::tasks::claim` allows this, emitting a second `task.claimed` with no status event
/// in between), leaving two open holds on the same task; a single closing status event ends both
/// of them, so no duplicate hold is left open past that point (fix round 1: the earlier
/// oldest-match-only close left the duplicate open until `session_end`, misattributing every
/// later call to a task already marked done). Every hold still open once all events are
/// processed closes at `session_end`, whichever comes first for it. `review` does NOT close a
/// hold (Assumption 3, settled): fix rounds after a verdict happen with the task still in
/// `review` and belong to it. A `task.status` event to `in_progress` (or `review`) is a no-op
/// here. Several holds may be open at once for different tasks too (a session that claims a
/// second task before releasing the first); no hold's `end` is ever truncated by a later claim
/// on a different task — `task_at` below is what implements "the most recently claimed wins" for
/// an instant covered by more than one open hold, by construction, without this function needing
/// a stack-pop/reactivate step.
pub fn holds(events: &[SessionEvent], session_end: Option<DateTime<Utc>>) -> Vec<Hold> {
    let mut open: Vec<Hold> = Vec::new();
    let mut closed: Vec<Hold> = Vec::new();
    let mut ordered: Vec<&SessionEvent> = events.iter().collect();
    ordered.sort_by_key(|e| e.ts);
    for ev in ordered {
        match &ev.kind {
            SessionEventKind::Claimed { task } => {
                open.push(Hold {
                    task: task.clone(),
                    start: ev.ts,
                    end: None,
                });
            }
            SessionEventKind::Status { task, to }
                if matches!(to.as_str(), "done" | "blocked" | "ready") =>
            {
                let mut i = 0;
                while i < open.len() {
                    if &open[i].task == task {
                        let mut h = open.remove(i);
                        h.end = Some(ev.ts);
                        closed.push(h);
                    } else {
                        i += 1;
                    }
                }
            }
            SessionEventKind::Status { .. } => {}
        }
    }
    for mut h in open {
        h.end = session_end;
        closed.push(h);
    }
    closed.sort_by_key(|h| h.start);
    closed
}

/// The task `at` belongs to: the hold with the latest `start` whose window contains `at` (start
/// inclusive, end exclusive), or `None` for "unassigned". "Most recently claimed wins" falls out
/// of `max_by_key(start)` directly when more than one hold's window contains `at`.
pub fn task_at(holds: &[Hold], at: DateTime<Utc>) -> Option<&str> {
    holds
        .iter()
        .filter(|h| h.start <= at && h.end.map(|e| at < e).unwrap_or(true))
        .max_by_key(|h| h.start)
        .map(|h| h.task.as_str())
}

/// D-usage-orientation: the orchestrator's tokens from the session's first record until the
/// earliest of its first claim, or the call that first dispatches an `Agent`/`Edit`/`Write`/
/// `NotebookEdit` tool — that dispatching call itself is included (it is what ends orientation,
/// not what follows it). `calls_sorted` must be sorted by `ts` ascending and must be the
/// orchestrator's own calls only (role `orchestrator`) — a subagent's calls are never part of
/// orientation.
pub fn orientation(
    calls_sorted: &[&transcript::Call],
    first_claim: Option<DateTime<Utc>>,
) -> Totals {
    let mut totals = Totals::default();
    for c in calls_sorted {
        if let Some(claim_ts) = first_claim {
            if c.ts >= claim_ts {
                break;
            }
        }
        let dispatch = c
            .tools
            .iter()
            .any(|t| matches!(t.as_str(), "Agent" | "Edit" | "Write" | "NotebookEdit"));
        totals.add(&c.usage);
        if dispatch {
            break;
        }
    }
    totals
}

/// Component-wise average, rounded to the nearest token. Empty input averages to zero. Used for
/// "orientation shown per task as the average over the task's sessions" (design §4).
pub fn average_totals(items: &[Totals]) -> Totals {
    if items.is_empty() {
        return Totals::default();
    }
    let n = items.len() as f64;
    let avg = |sum: u64| (sum as f64 / n).round() as u64;
    Totals {
        input: avg(items.iter().map(|t| t.input).sum()),
        cache_write: avg(items.iter().map(|t| t.cache_write).sum()),
        cache_read: avg(items.iter().map(|t| t.cache_read).sum()),
        output: avg(items.iter().map(|t| t.output).sum()),
        thinking: avg(items.iter().map(|t| t.thinking).sum()),
    }
}

/// One `subagent.start`/`subagent.stop` event, already extracted by the caller (Assumption 1 at
/// the top of this plan): `agent_id`/`agent_type`/`description` come from `events.payload`;
/// `task` comes from the event row's own `task_id` column, never from the payload (which has
/// no `task_id` field).
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentEvent {
    pub agent_id: String,
    pub agent_type: Option<String>,
    pub description: Option<String>,
    pub task: Option<String>,
}

/// D-usage-subagents' three-step resolution for one subagent transcript. `event` is the
/// `subagent.start`/`.stop` event matching this transcript's `agent_id` (`None` when no
/// start/stop event fired for this call — the chain falls through exactly the same way
/// either way). `parent_agent_calls` are the parent's own calls that carry
/// an `Agent` `tool_use`, as `(tool_use_id, ts)` pairs. `parent_holds` are the parent session's
/// holds (`holds()`, above). Returns `(task, role)`; `role` is not yet display-formatted — pass
/// it through `display_role` before printing. `role` resolves on its own chain, independent of
/// which of the three steps below resolves `task` (spec Requirement 4): `event`'s `agent_type`,
/// else `meta`'s `agentType`, else `"subagent"` — so an event with no `agent_type` still falls
/// back to the meta file's, rather than jumping straight to `"subagent"` (fix round 1).
pub fn subagent_task_and_role(
    event: Option<&SubagentEvent>,
    meta: Option<&Meta>,
    parent_holds: &[Hold],
    parent_agent_calls: &[(String, DateTime<Utc>)],
    first_record_ts: Option<DateTime<Utc>>,
) -> (Option<String>, String) {
    let role = event
        .and_then(|ev| ev.agent_type.clone())
        .or_else(|| meta.and_then(|m| m.agent_type.clone()))
        .unwrap_or_else(|| "subagent".to_string());

    if let Some(ev) = event {
        return (ev.task.clone(), role);
    }
    if let Some(tu_id) = meta.and_then(|m| m.tool_use_id.as_deref()) {
        if let Some((_, ts)) = parent_agent_calls.iter().find(|(id, _)| id == tu_id) {
            let task = task_at(parent_holds, *ts).map(str::to_string);
            return (task, role);
        }
    }
    let task = first_record_ts
        .and_then(|ts| task_at(parent_holds, ts))
        .map(str::to_string);
    (task, role)
}

/// One call, already resolved to its task (`None` = unassigned), session, role and model. The
/// flat shape `cli::usage_cmd` (Task 4) builds by walking sessions/transcripts and classifying
/// every call through `task_at`/`subagent_task_and_role`; every reducer below (`group_totals`,
/// `buckets_of`, `totals_by_role_model`, `rounds_tokens`) works from this one shape so the CLI
/// face only needs to build it once per report. `role` here is the *raw* role (e.g.
/// `general-purpose`, not yet display-formatted) — callers pass it through `display_role` for
/// anything printed.
#[derive(Debug, Clone)]
pub struct AttributedCall {
    pub task: Option<String>,
    pub session: String,
    pub role: String,
    pub model: String,
    pub ts: DateTime<Utc>,
    pub usage: Usage,
}

/// One role×model×session row. `cost` is `None` unless `buckets_of` was given weights and a
/// prefix matched this bucket's `model`.
#[derive(Debug, Clone, Serialize)]
pub struct Bucket {
    pub session: String,
    pub role: String,
    pub model: String,
    pub tokens: Totals,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

/// Sums `calls` grouped by an arbitrary string key — the shared engine behind `--by
/// task|role|model|session` (Task 4 supplies the key closure per flag). `BTreeMap` keeps output
/// order deterministic without a separate sort step.
pub fn group_totals(
    calls: &[AttributedCall],
    key: impl Fn(&AttributedCall) -> String,
) -> BTreeMap<String, Totals> {
    let mut out: BTreeMap<String, Totals> = BTreeMap::new();
    for c in calls {
        out.entry(key(c)).or_default().add(&c.usage);
    }
    out
}

/// One task's calls, grouped into `(session, role, model)` rows — the granularity `--json`'s
/// `buckets` array uses (Requirement 9). `weights` is `None` when the machine config has no
/// `[usage.weights]` table at all; when `Some`, each bucket's `cost` is set from its own model,
/// independently — a bucket whose model matches no prefix simply has `cost: None`.
pub fn buckets_of(
    calls: &[AttributedCall],
    weights: Option<&HashMap<String, ModelWeights>>,
) -> Vec<Bucket> {
    let mut totals: BTreeMap<(String, String, String), Totals> = BTreeMap::new();
    for c in calls {
        totals
            .entry((c.session.clone(), c.role.clone(), c.model.clone()))
            .or_default()
            .add(&c.usage);
    }
    totals
        .into_iter()
        .map(|((session, role, model), tokens)| {
            let cost = weights.and_then(|w| {
                weights::cost(
                    &model,
                    w,
                    tokens.input,
                    tokens.cache_write,
                    tokens.cache_read,
                    tokens.output,
                )
            });
            Bucket {
                session,
                role,
                model,
                tokens,
                cost,
            }
        })
        .collect()
}

/// The same calls collapsed to `(role, model)` — what the task-detail *text* view shows (design
/// §4: "a line per role × model"; which sessions were involved is listed separately, not as a
/// row dimension here).
pub fn totals_by_role_model(calls: &[AttributedCall]) -> BTreeMap<(String, String), Totals> {
    let mut out: BTreeMap<(String, String), Totals> = BTreeMap::new();
    for c in calls {
        out.entry((c.role.clone(), c.model.clone()))
            .or_default()
            .add(&c.usage);
    }
    out
}

/// R6's two pieces: `rounds` is unchanged (exactly one `Totals` per entry into `review`, tokens
/// between consecutive entries, the first counting from the first claim) — its `len()` is what
/// `rounds N` prints, and stays `review_entries.len()` no matter what `current` resolves to.
/// `current` (T-0013) is the open fix round's tokens: `Some` only when the caller asked for one
/// (`rounds_tokens`'s `current_as_of`) AND at least one entry into `review` exists to span from.
#[derive(Debug, Clone, PartialEq)]
pub struct Rounds {
    pub rounds: Vec<Totals>,
    pub current: Option<Totals>,
}

/// R6: tokens between consecutive entries into `review`, the first round counting from
/// `first_claim`. `review_entries` are the task's `task.status → review` timestamps, in order;
/// `rounds` (the count Requirement 6 names) is simply `review_entries.len()`. `calls` must
/// already be filtered to one task (every `AttributedCall` with `task == Some(that_id)`).
///
/// `current_as_of`, when `Some(now)`, additionally spans [the last entry into `review`, `now`) —
/// the fix round still open while the task sits in `review` (T-0013: those tokens used to fall
/// out of every window above, since `bounds.windows(2)` never produced one past the last review
/// entry). The caller decides when a "current" window makes sense (in practice: the task's own
/// status is still `review`); this function only refuses one when there is no last entry to span
/// from at all, which cannot happen for a task actually sitting in `review` but keeps this
/// function correct standalone.
pub fn rounds_tokens(
    calls: &[AttributedCall],
    first_claim: DateTime<Utc>,
    review_entries: &[DateTime<Utc>],
    current_as_of: Option<DateTime<Utc>>,
) -> Rounds {
    let sum_between = |start: DateTime<Utc>, end: DateTime<Utc>| {
        let mut t = Totals::default();
        for c in calls {
            if c.ts >= start && c.ts < end {
                t.add(&c.usage);
            }
        }
        t
    };
    let mut bounds = vec![first_claim];
    bounds.extend(review_entries.iter().copied());
    let rounds = bounds.windows(2).map(|w| sum_between(w[0], w[1])).collect();
    let current =
        current_as_of.and_then(|end| review_entries.last().map(|&start| sum_between(start, end)));
    Rounds { rounds, current }
}

/// `orchestrator` tokens ÷ all tokens, `0.0` when `buckets` is empty.
pub fn orchestrator_share(buckets: &[Bucket]) -> f64 {
    let all: u64 = buckets.iter().map(|b| b.tokens.all()).sum();
    if all == 0 {
        return 0.0;
    }
    let orch: u64 = buckets
        .iter()
        .filter(|b| b.role == "orchestrator")
        .map(|b| b.tokens.all())
        .sum();
    orch as f64 / all as f64
}

/// `cache_read ÷ (input + cache_write + cache_read)`, `0.0` when that denominator is zero.
pub fn cache_efficiency(t: &Totals) -> f64 {
    let denom = t.input + t.cache_write + t.cache_read;
    if denom == 0 {
        return 0.0;
    }
    t.cache_read as f64 / denom as f64
}

/// A task-level aggregate `cost`: the sum of every bucket's own cost, but only when *all* of them
/// matched a weight (Assumption 4 at the top of this plan) — a partial sum would be misleading
/// for a task that used more than one model.
pub fn task_cost(buckets: &[Bucket]) -> Option<f64> {
    if buckets.is_empty() {
        return None;
    }
    buckets
        .iter()
        .map(|b| b.cost)
        .collect::<Option<Vec<f64>>>()
        .map(|v| v.iter().sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        crate::clock::parse(s).unwrap()
    }

    fn usage(input: u64, output: u64) -> Usage {
        Usage {
            input,
            cache_write: 0,
            cache_read: 0,
            output,
            thinking: 0,
        }
    }

    fn call(ts: &str, model: &str, u: Usage, tools: &[&str]) -> transcript::Call {
        transcript::Call {
            ts: at(ts),
            model: model.to_string(),
            usage: u,
            tools: tools.iter().map(|s| s.to_string()).collect(),
            tool_use_ids: Vec::new(),
        }
    }

    // --- holds / task_at -------------------------------------------------------------------

    #[test]
    fn a_call_at_exactly_the_claim_timestamp_belongs_to_the_task() {
        let events = vec![SessionEvent {
            ts: at("2026-09-16T12:00:00Z"),
            kind: SessionEventKind::Claimed {
                task: "T-0001".into(),
            },
        }];
        let h = holds(&events, None);
        assert_eq!(task_at(&h, at("2026-09-16T12:00:00Z")), Some("T-0001"));
        assert_eq!(task_at(&h, at("2026-09-16T11:59:59Z")), None);
    }

    #[test]
    fn a_call_at_exactly_the_release_timestamp_is_outside_the_window() {
        let events = vec![
            SessionEvent {
                ts: at("2026-09-16T12:00:00Z"),
                kind: SessionEventKind::Claimed {
                    task: "T-0001".into(),
                },
            },
            SessionEvent {
                ts: at("2026-09-16T12:10:00Z"),
                kind: SessionEventKind::Status {
                    task: "T-0001".into(),
                    to: "done".into(),
                },
            },
        ];
        let h = holds(&events, None);
        assert_eq!(task_at(&h, at("2026-09-16T12:09:59Z")), Some("T-0001"));
        assert_eq!(
            task_at(&h, at("2026-09-16T12:10:00Z")),
            None,
            "the release instant is not inside the closed window"
        );
    }

    #[test]
    fn review_does_not_close_the_hold() {
        let events = vec![
            SessionEvent {
                ts: at("2026-09-16T12:00:00Z"),
                kind: SessionEventKind::Claimed {
                    task: "T-0001".into(),
                },
            },
            SessionEvent {
                ts: at("2026-09-16T12:10:00Z"),
                kind: SessionEventKind::Status {
                    task: "T-0001".into(),
                    to: "review".into(),
                },
            },
            SessionEvent {
                ts: at("2026-09-16T12:20:00Z"),
                kind: SessionEventKind::Status {
                    task: "T-0001".into(),
                    to: "ready".into(),
                },
            },
        ];
        let h = holds(&events, None);
        assert_eq!(
            task_at(&h, at("2026-09-16T12:15:00Z")),
            Some("T-0001"),
            "a fix round while the task sits in review still belongs to it"
        );
        assert_eq!(
            task_at(&h, at("2026-09-16T12:20:00Z")),
            None,
            "ready closes it"
        );
    }

    #[test]
    fn most_recently_claimed_wins_while_two_tasks_are_held_at_once() {
        let events = vec![
            SessionEvent {
                ts: at("2026-09-16T12:00:00Z"),
                kind: SessionEventKind::Claimed {
                    task: "T-0001".into(),
                },
            },
            SessionEvent {
                ts: at("2026-09-16T12:05:00Z"),
                kind: SessionEventKind::Claimed {
                    task: "T-0002".into(),
                },
            },
            SessionEvent {
                ts: at("2026-09-16T12:10:00Z"),
                kind: SessionEventKind::Status {
                    task: "T-0002".into(),
                    to: "done".into(),
                },
            },
        ];
        let h = holds(&events, None);
        assert_eq!(
            task_at(&h, at("2026-09-16T12:06:00Z")),
            Some("T-0002"),
            "the more recently claimed task wins while both are open"
        );
        assert_eq!(
            task_at(&h, at("2026-09-16T12:11:00Z")),
            Some("T-0001"),
            "T-0001 resumes once T-0002 is released"
        );
    }

    #[test]
    fn an_open_hold_closes_at_session_end() {
        let events = vec![SessionEvent {
            ts: at("2026-09-16T12:00:00Z"),
            kind: SessionEventKind::Claimed {
                task: "T-0001".into(),
            },
        }];
        let h = holds(&events, Some(at("2026-09-16T13:00:00Z")));
        assert_eq!(task_at(&h, at("2026-09-16T12:59:59Z")), Some("T-0001"));
        assert_eq!(task_at(&h, at("2026-09-16T13:00:00Z")), None);
    }

    #[test]
    fn a_status_event_into_in_progress_neither_opens_nor_closes_a_hold() {
        let events = vec![
            SessionEvent {
                ts: at("2026-09-16T12:00:00Z"),
                kind: SessionEventKind::Claimed {
                    task: "T-0001".into(),
                },
            },
            SessionEvent {
                ts: at("2026-09-16T12:05:00Z"),
                kind: SessionEventKind::Status {
                    task: "T-0001".into(),
                    to: "in_progress".into(),
                },
            },
        ];
        let h = holds(&events, None);
        assert_eq!(
            h.len(),
            1,
            "the to=in_progress event produced no second hold"
        );
        assert_eq!(task_at(&h, at("2026-09-16T12:05:00Z")), Some("T-0001"));
    }

    #[test]
    fn a_reclaim_of_an_already_held_task_leaves_no_duplicate_hold_open_past_the_close() {
        let events = vec![
            SessionEvent {
                ts: at("2026-09-16T12:00:00Z"),
                kind: SessionEventKind::Claimed {
                    task: "T-0001".into(),
                },
            },
            SessionEvent {
                ts: at("2026-09-16T12:05:00Z"),
                kind: SessionEventKind::Claimed {
                    task: "T-0001".into(),
                },
            },
            SessionEvent {
                ts: at("2026-09-16T12:10:00Z"),
                kind: SessionEventKind::Status {
                    task: "T-0001".into(),
                    to: "done".into(),
                },
            },
        ];
        let h = holds(&events, None);
        assert_eq!(task_at(&h, at("2026-09-16T12:09:00Z")), Some("T-0001"));
        assert_eq!(task_at(&h, at("2026-09-16T12:10:00Z")), None);
        assert_eq!(
            task_at(&h, at("2026-09-17T12:00:00Z")),
            None,
            "the duplicate hold from the re-claim must not stay open indefinitely"
        );
    }

    #[test]
    fn a_reclaim_still_closes_fully_at_the_status_event_even_with_a_later_session_end() {
        let events = vec![
            SessionEvent {
                ts: at("2026-09-16T12:00:00Z"),
                kind: SessionEventKind::Claimed {
                    task: "T-0001".into(),
                },
            },
            SessionEvent {
                ts: at("2026-09-16T12:05:00Z"),
                kind: SessionEventKind::Claimed {
                    task: "T-0001".into(),
                },
            },
            SessionEvent {
                ts: at("2026-09-16T12:10:00Z"),
                kind: SessionEventKind::Status {
                    task: "T-0001".into(),
                    to: "done".into(),
                },
            },
        ];
        let h = holds(&events, Some(at("2026-09-16T12:50:00Z")));
        assert_eq!(task_at(&h, at("2026-09-16T12:09:00Z")), Some("T-0001"));
        assert_eq!(task_at(&h, at("2026-09-16T12:10:00Z")), None);
        assert_eq!(
            task_at(&h, at("2026-09-16T12:30:00Z")),
            None,
            "between the close and session_end, the duplicate must not have leaked through to session_end"
        );
        assert_eq!(task_at(&h, at("2026-09-17T12:00:00Z")), None);
    }

    // --- orientation -------------------------------------------------------------------------

    #[test]
    fn orientation_stops_before_the_first_claim() {
        let c1 = call("2026-09-16T12:00:00Z", "m", usage(30, 4), &[]);
        let c2 = call("2026-09-16T12:01:00Z", "m", usage(30, 4), &[]);
        let c3 = call("2026-09-16T12:02:00Z", "m", usage(90, 10), &[]);
        let sorted = [&c1, &c2, &c3];
        let t = orientation(&sorted, Some(at("2026-09-16T12:01:30Z")));
        assert_eq!(t.input, 60);
    }

    #[test]
    fn orientation_includes_the_dispatching_call_itself() {
        let c1 = call("2026-09-16T12:00:00Z", "m", usage(25, 3), &[]);
        let c2 = call("2026-09-16T12:01:00Z", "m", usage(25, 3), &["Agent"]);
        let c3 = call("2026-09-16T12:02:00Z", "m", usage(80, 9), &[]);
        let sorted = [&c1, &c2, &c3];
        let t = orientation(&sorted, None);
        assert_eq!(
            t.input, 50,
            "both calls up to and including the dispatch count"
        );
    }

    #[test]
    fn orientation_with_neither_a_claim_nor_a_dispatch_counts_everything() {
        let c1 = call("2026-09-16T12:00:00Z", "m", usage(10, 1), &[]);
        let c2 = call("2026-09-16T12:01:00Z", "m", usage(10, 1), &[]);
        let sorted = [&c1, &c2];
        let t = orientation(&sorted, None);
        assert_eq!(t.input, 20);
    }

    // --- rounds_tokens -----------------------------------------------------------------------

    #[test]
    fn rounds_tokens_splits_on_consecutive_review_entries() {
        let mk = |ts: &str, input: u64| AttributedCall {
            task: Some("T-0001".into()),
            session: "s-1".into(),
            role: "orchestrator".into(),
            model: "m".into(),
            ts: at(ts),
            usage: usage(input, input / 10),
        };
        let calls = vec![
            mk("2026-09-16T12:01:00Z", 40),
            mk("2026-09-16T12:06:00Z", 70),
            mk("2026-09-16T12:07:00Z", 70),
        ];
        let review_entries = [at("2026-09-16T12:04:00Z"), at("2026-09-16T12:08:00Z")];
        let rounds = rounds_tokens(&calls, at("2026-09-16T12:00:00Z"), &review_entries, None);
        assert_eq!(rounds.rounds.len(), 2);
        assert_eq!(rounds.rounds[0].input, 40);
        assert_eq!(
            rounds.rounds[1].input, 140,
            "the two calls between the two review entries"
        );
        assert!(rounds.current.is_none(), "no current_as_of was given");
    }

    #[test]
    fn rounds_tokens_current_round_spans_from_the_last_entry_to_now() {
        // T-0013: a task still sitting in `review` after its (only) entry keeps making calls
        // that used to fall out of every round window. `current_as_of` closes that gap.
        let mk = |ts: &str, input: u64| AttributedCall {
            task: Some("T-0001".into()),
            session: "s-1".into(),
            role: "orchestrator".into(),
            model: "m".into(),
            ts: at(ts),
            usage: usage(input, 0),
        };
        let calls = vec![
            mk("2026-09-16T12:01:00Z", 40), // before the (only) review entry: round 0
            mk("2026-09-16T12:06:00Z", 70), // after it, the open fix round
            mk("2026-09-16T12:07:00Z", 70),
        ];
        let review_entries = [at("2026-09-16T12:04:00Z")];
        let rounds = rounds_tokens(
            &calls,
            at("2026-09-16T12:00:00Z"),
            &review_entries,
            Some(at("2026-09-16T12:09:00Z")),
        );
        assert_eq!(
            rounds.rounds.len(),
            1,
            "the current window must not change what `rounds` counts"
        );
        assert_eq!(rounds.rounds[0].input, 40);
        assert_eq!(
            rounds.current.map(|t| t.input),
            Some(140),
            "the two calls after the only review entry"
        );
    }

    #[test]
    fn rounds_tokens_has_no_current_round_without_any_review_entry() {
        let calls: Vec<AttributedCall> = Vec::new();
        let rounds = rounds_tokens(
            &calls,
            at("2026-09-16T12:00:00Z"),
            &[],
            Some(at("2026-09-16T12:09:00Z")),
        );
        assert!(
            rounds.current.is_none(),
            "nothing to span from with zero entries into review"
        );
    }

    // --- abbreviate / since_cutoff / display_role / averages ---------------------------------

    #[test]
    fn abbreviate_uses_k_and_m_with_one_decimal() {
        assert_eq!(abbreviate(999), "999");
        assert_eq!(abbreviate(1234), "1.2k");
        assert_eq!(abbreviate(2_500_000), "2.5M");
    }

    #[test]
    fn abbreviate_promotes_to_m_when_the_rounded_k_value_reaches_1000() {
        assert_eq!(abbreviate(999_949), "999.9k");
        assert_eq!(
            abbreviate(999_950),
            "1.0M",
            "999.95k rounds to 1000.0k, which must print as 1.0M instead"
        );
        assert_eq!(abbreviate(999_999), "1.0M");
        assert_eq!(abbreviate(1_000_000), "1.0M");
    }

    #[test]
    fn since_cutoff_understands_days_and_dates_and_refuses_junk() {
        let now = at("2026-09-16T12:00:00Z");
        assert_eq!(since_cutoff("7d", now).unwrap(), now - Duration::days(7));
        assert_eq!(since_cutoff("30d", now).unwrap(), now - Duration::days(30));
        assert_eq!(
            since_cutoff("2026-09-01", now).unwrap(),
            at("2026-09-01T00:00:00Z")
        );
        assert!(since_cutoff("0d", now).is_err());
        assert!(since_cutoff("nonsense", now).is_err());
    }

    #[test]
    fn display_role_clips_general_purpose_and_passes_others_through() {
        let desc = "search the whole repository for every remaining caller of the old helper";
        let clipped: String = desc.chars().take(40).collect();
        assert_eq!(
            display_role("general-purpose", Some(desc)),
            format!("general-purpose: {clipped}")
        );
        assert_eq!(display_role("ratchet:reviewer", None), "ratchet:reviewer");
    }

    #[test]
    fn average_totals_is_component_wise_and_rounds() {
        let a = Totals {
            input: 10,
            cache_write: 0,
            cache_read: 0,
            output: 1,
            thinking: 0,
        };
        let b = Totals {
            input: 5,
            cache_write: 0,
            cache_read: 0,
            output: 2,
            thinking: 0,
        };
        let avg = average_totals(&[a, b]);
        assert_eq!(avg.input, 8, "(10+5)/2 = 7.5, rounds to 8");
        assert_eq!(avg.output, 2, "(1+2)/2 = 1.5, rounds to 2");
        assert_eq!(average_totals(&[]).input, 0);
    }

    // --- buckets_of / task_cost ----------------------------------------------------------------

    #[test]
    fn buckets_of_groups_by_session_role_and_model_and_costs_independently() {
        let calls = vec![
            AttributedCall {
                task: Some("T-0001".into()),
                session: "s-1".into(),
                role: "orchestrator".into(),
                model: "claude-sonnet-5".into(),
                ts: at("2026-09-16T12:00:00Z"),
                usage: usage(1_000_000, 0),
            },
            AttributedCall {
                task: Some("T-0001".into()),
                session: "s-1".into(),
                role: "orchestrator".into(),
                model: "claude-haiku-5".into(),
                ts: at("2026-09-16T12:01:00Z"),
                usage: usage(1_000_000, 0),
            },
        ];
        let mut w = HashMap::new();
        w.insert(
            "claude-sonnet".to_string(),
            ModelWeights {
                input: 3.0,
                cache_write: 0.0,
                cache_read: 0.0,
                output: 0.0,
            },
        );
        let buckets = buckets_of(&calls, Some(&w));
        assert_eq!(buckets.len(), 2);
        let sonnet = buckets
            .iter()
            .find(|b| b.model == "claude-sonnet-5")
            .unwrap();
        assert_eq!(sonnet.cost, Some(3.0));
        let haiku = buckets
            .iter()
            .find(|b| b.model == "claude-haiku-5")
            .unwrap();
        assert_eq!(haiku.cost, None);
        assert_eq!(
            task_cost(&buckets),
            None,
            "one unweighted bucket means no aggregate cost"
        );
    }

    #[test]
    fn task_cost_sums_when_every_bucket_matched() {
        let calls = vec![AttributedCall {
            task: Some("T-0001".into()),
            session: "s-1".into(),
            role: "orchestrator".into(),
            model: "claude-sonnet-5".into(),
            ts: at("2026-09-16T12:00:00Z"),
            usage: usage(1_000_000, 0),
        }];
        let mut w = HashMap::new();
        w.insert(
            "claude-sonnet".to_string(),
            ModelWeights {
                input: 3.0,
                cache_write: 0.0,
                cache_read: 0.0,
                output: 0.0,
            },
        );
        let buckets = buckets_of(&calls, Some(&w));
        assert_eq!(task_cost(&buckets), Some(3.0));
    }

    // --- subagent_task_and_role ------------------------------------------------------------------

    #[test]
    fn subagent_resolution_prefers_the_event_then_tool_use_id_then_first_timestamp() {
        let holds = vec![Hold {
            task: "T-0001".into(),
            start: at("2026-09-16T12:00:00Z"),
            end: None,
        }];

        let via_event = subagent_task_and_role(
            Some(&SubagentEvent {
                agent_id: "a1".into(),
                agent_type: Some("ratchet:reviewer".into()),
                description: None,
                task: Some("T-0002".into()),
            }),
            None,
            &holds,
            &[],
            None,
        );
        assert_eq!(
            via_event,
            (Some("T-0002".to_string()), "ratchet:reviewer".to_string())
        );

        let meta = Meta {
            agent_type: Some("ratchet:implementer".into()),
            description: None,
            model: None,
            tool_use_id: Some("tu-9".into()),
        };
        let via_tool = subagent_task_and_role(
            None,
            Some(&meta),
            &holds,
            &[("tu-9".to_string(), at("2026-09-16T12:05:00Z"))],
            None,
        );
        assert_eq!(
            via_tool,
            (
                Some("T-0001".to_string()),
                "ratchet:implementer".to_string()
            )
        );

        let via_ts =
            subagent_task_and_role(None, None, &holds, &[], Some(at("2026-09-16T12:05:00Z")));
        assert_eq!(via_ts, (Some("T-0001".to_string()), "subagent".to_string()));
    }

    #[test]
    fn subagent_role_falls_back_to_meta_independently_of_which_step_resolves_the_task() {
        let holds = vec![Hold {
            task: "T-0001".into(),
            start: at("2026-09-16T12:00:00Z"),
            end: None,
        }];
        let meta = Meta {
            agent_type: Some("ratchet:implementer".into()),
            description: None,
            model: None,
            tool_use_id: None,
        };

        let event_without_agent_type = SubagentEvent {
            agent_id: "a1".into(),
            agent_type: None,
            description: None,
            task: Some("T-1".into()),
        };
        let via_event_missing_type = subagent_task_and_role(
            Some(&event_without_agent_type),
            Some(&meta),
            &holds,
            &[],
            None,
        );
        assert_eq!(
            via_event_missing_type,
            (Some("T-1".to_string()), "ratchet:implementer".to_string()),
            "the event resolves the task but has no agent_type, so role falls back to meta"
        );

        let event_with_agent_type = SubagentEvent {
            agent_id: "a1".into(),
            agent_type: Some("ratchet:reviewer".into()),
            description: None,
            task: Some("T-1".into()),
        };
        let via_event_own_type =
            subagent_task_and_role(Some(&event_with_agent_type), Some(&meta), &holds, &[], None);
        assert_eq!(
            via_event_own_type,
            (Some("T-1".to_string()), "ratchet:reviewer".to_string()),
            "the event's own agent_type wins over a differing meta"
        );
    }

    // --- orchestrator_share / cache_efficiency ---------------------------------------------------

    #[test]
    fn orchestrator_share_and_cache_efficiency_are_ratios_of_all_tokens() {
        let buckets = vec![
            Bucket {
                session: "s-1".into(),
                role: "orchestrator".into(),
                model: "m".into(),
                tokens: Totals {
                    input: 100,
                    cache_write: 0,
                    cache_read: 0,
                    output: 0,
                    thinking: 0,
                },
                cost: None,
            },
            Bucket {
                session: "s-1".into(),
                role: "ratchet:reviewer".into(),
                model: "m".into(),
                tokens: Totals {
                    input: 300,
                    cache_write: 0,
                    cache_read: 0,
                    output: 0,
                    thinking: 0,
                },
                cost: None,
            },
        ];
        assert_eq!(orchestrator_share(&buckets), 0.25);
        assert_eq!(orchestrator_share(&[]), 0.0);
        let t = Totals {
            input: 70,
            cache_write: 0,
            cache_read: 30,
            output: 0,
            thinking: 0,
        };
        assert_eq!(cache_efficiency(&t), 0.3);
        assert_eq!(cache_efficiency(&Totals::default()), 0.0);
    }
}
