//! Group 1 owns exactly one thing about tasks: giving back the work of a session that died.
//! The board (create, claim, checklist, notes, handoffs) arrives with group 2 and extends this
//! module; the events written here are the contract that must not change.

use chrono::{DateTime, Utc};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, TransactionBehavior};
use serde_json::json;

use super::{events, sessions, ServiceError};
use crate::clock;
use crate::config::Thresholds;
use crate::model::{
    allowed_transitions, ChecklistItem, Event, EventKind, SessionState, Source, Task, TaskStatus,
};

// Consumed by tasks::release_dead below and by cli::task_cmd / group 2's board (Task 11/group 2).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    pub task_id: String,
    pub title: String,
    pub claimed_by: String,
}

/// Tasks in progress whose holder is ended, orphaned, or gone from the registry. Pure: an orphan
/// stays listed until something releases it, which is what makes the briefing of group 2 able to
/// show it once with its last handoff before it goes back to `ready`.
// Consumed by tasks::release_dead below and by cli::task_cmd (Task 11) to list orphans.
#[allow(dead_code)]
pub fn orphaned(
    conn: &Connection,
    repo_root: &str,
    th: &Thresholds,
    now: DateTime<Utc>,
) -> Result<Vec<Orphan>, ServiceError> {
    let mut stmt = conn.prepare(
        "SELECT id, title, claimed_by FROM tasks \
         WHERE repo_root = ?1 AND status = ?2 AND claimed_by IS NOT NULL AND archived_at IS NULL \
         ORDER BY id",
    )?;
    let rows = stmt.query_map(params![repo_root, TaskStatus::InProgress.as_str()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (task_id, title, claimed_by) = row?;
        let dead = match sessions::get(conn, &claimed_by)? {
            None => true,
            Some(holder) => matches!(
                sessions::state(&holder, th, now),
                SessionState::Orphaned | SessionState::Ended
            ),
        };
        if dead {
            out.push(Orphan {
                task_id,
                title,
                claimed_by,
            });
        }
    }
    Ok(out)
}

/// Back to `ready`, without a session, with the change of state and a note saying who released
/// it. History is untouched: the last handoff stays where it was.
// Consumed by tasks::release_dead below, by hooks::dispatch (Task 10), and by group 2's
// tasks::transition, which will subsume its internals while keeping the same two events.
#[allow(dead_code)]
pub fn release(
    conn: &mut Connection,
    task_id: &str,
    by: &str,
    source: Source,
    now: DateTime<Utc>,
) -> Result<(), ServiceError> {
    // The precondition read and the write must happen against the same transaction, not the
    // bare connection: opening the IMMEDIATE transaction first and reading through `&tx` closes
    // the check-then-act window where two racing callers could both see a holder and both
    // release the same task (same defect class, same fix shape, as sessions::upsert_start).
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let task = get(&tx, task_id)?;
    let holder = task.claimed_by.clone().ok_or_else(|| {
        ServiceError::Invalid(format!(
            "{task_id} holds no session; there is nothing to release"
        ))
    })?;
    let why = format!("released by {by}");
    set_status_in(&tx, &task, TaskStatus::Ready, source, None, Some(&why), now)?;
    tx.execute(
        "UPDATE tasks SET claimed_by = NULL WHERE id = ?1",
        params![task_id],
    )?;
    events::emit(
        &tx,
        EventKind::Note,
        &json!({"text": format!("{why}; it was held by session {holder}")}),
        source,
        None,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    Ok(())
}

/// One mechanism, two triggers: the session-end hook calls it for a tidy close, and the
/// session-start hook calls it as a lazy sweep for sessions that died without a hook. A row that
/// no longer needs releasing by the time its turn comes — another racing sweep already got to it
/// — is skipped, not treated as a reason to give up on the rest of the sweep; only a genuine
/// database error aborts it.
///
/// `by` names the trigger for the `note` event's text (e.g. `"end of session"` or `"the next
/// session's start"`) — callers pass what actually happened so the note is never wrong about why
/// a task came back.
// Consumed by hooks::dispatch (Task 10) on both `session-end` and `session-start`.
#[allow(dead_code)]
pub fn release_dead(
    conn: &mut Connection,
    repo_root: &str,
    th: &Thresholds,
    by: &str,
    now: DateTime<Utc>,
) -> Result<Vec<String>, ServiceError> {
    let dead = orphaned(conn, repo_root, th, now)?;
    let mut released = Vec::new();
    for orphan in dead {
        match release(conn, &orphan.task_id, by, Source::Hook, now) {
            Ok(()) => released.push(orphan.task_id),
            Err(ServiceError::Invalid(_)) | Err(ServiceError::NotFound(_)) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(released)
}

// Consumed by cli::session_cmd (Task 11, `session list|show`) to show a session's work in progress.
#[allow(dead_code)]
pub fn claimed_ids(conn: &Connection, session_id: &str) -> Result<Vec<String>, ServiceError> {
    let mut stmt = conn.prepare(
        "SELECT id FROM tasks WHERE claimed_by = ?1 AND status = ?2 AND archived_at IS NULL ORDER BY id",
    )?;
    let rows = stmt.query_map(params![session_id, TaskStatus::InProgress.as_str()], |r| {
        r.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// The one definition of "the first held task": the lowest identifier among the tasks in
/// progress this session holds, or `None` when it holds none. `hooks::dispatch::subagent_event`
/// (write time, to pick the task a `subagent.start`/`subagent.stop` event is attributed to) and
/// `hooks::briefing::prompt_line` (read time, to pick the task the prompt reminder names) both
/// call this so the two can never disagree — see the agent-protocol spec's "Task reminder on
/// every prompt" requirement.
pub fn first_held_id(conn: &Connection, session_id: &str) -> Result<Option<String>, ServiceError> {
    Ok(claimed_ids(conn, session_id)?.into_iter().next())
}

/// What a listing asks for. `repo_root` is the scoping key (G1-R1); `repo` filters on the display
/// name and exists for "everything called `web` on this machine".
// Consumed by cli::task_cmd (Task 8, `task list`) and hooks (Task 9's briefing).
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct Filter<'a> {
    pub repo_root: Option<&'a str>,
    pub repo: Option<&'a str>,
    pub statuses: &'a [TaskStatus],
    pub claimed_by: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub include_archived: bool,
}

// Consumed by tasks::{claim, transition, checklist_done/undone} (Task 5/6) for their precondition
// reads, and by cli::task_cmd (Task 8, `task show`).
#[allow(dead_code)]
pub fn get(conn: &Connection, task_id: &str) -> Result<Task, ServiceError> {
    let mut stmt = conn.prepare("SELECT * FROM tasks WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![task_id], Task::from_row)?;
    match rows.next() {
        Some(row) => Ok(row?),
        None => Err(ServiceError::NotFound(format!(
            "task {task_id} does not exist"
        ))),
    }
}

/// Priority first (1 is the most urgent), then identifier, so a listing reads like a queue.
/// `tag` is filtered in Rust: tags are a JSON array in one column and SQLite has no operator for
/// it that is worth a dependency.
// Consumed by cli::task_cmd (Task 8, `task list`) and hooks::dispatch (Task 9's briefing).
#[allow(dead_code)]
pub fn list(conn: &Connection, filter: &Filter<'_>) -> Result<Vec<Task>, ServiceError> {
    let mut sql = String::from("SELECT * FROM tasks WHERE 1 = 1");
    let mut args: Vec<String> = Vec::new();
    if !filter.include_archived {
        sql.push_str(" AND archived_at IS NULL");
    }
    if let Some(root) = filter.repo_root {
        sql.push_str(" AND repo_root = ?");
        args.push(root.to_string());
    }
    if let Some(name) = filter.repo {
        sql.push_str(" AND repo = ?");
        args.push(name.to_string());
    }
    if !filter.statuses.is_empty() {
        let holes = vec!["?"; filter.statuses.len()].join(",");
        sql.push_str(&format!(" AND status IN ({holes})"));
        args.extend(filter.statuses.iter().map(|s| s.as_str().to_string()));
    }
    if let Some(session_id) = filter.claimed_by {
        sql.push_str(" AND claimed_by = ?");
        args.push(session_id.to_string());
    }
    sql.push_str(" ORDER BY priority ASC, id ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), Task::from_row)?;
    let mut out = Vec::new();
    for row in rows {
        let task = row?;
        if let Some(tag) = filter.tag {
            if !task.tags.iter().any(|t| t == tag) {
                continue;
            }
        }
        out.push(task);
    }
    Ok(out)
}

// Consumed by cli::task_cmd (Task 8, `task show`) and hooks::dispatch (Task 9's briefing).
#[allow(dead_code)]
pub fn checklist(conn: &Connection, task_id: &str) -> Result<Vec<ChecklistItem>, ServiceError> {
    let mut stmt =
        conn.prepare("SELECT * FROM checklist_items WHERE task_id = ?1 ORDER BY position")?;
    let rows = stmt.query_map(params![task_id], ChecklistItem::from_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Items done over items total, computed here every time. `None` means the task has no checklist:
/// that is *no progress*, not zero, and no face may turn it into a number (spec §4.5).
// Consumed by cli::task_cmd (Task 8, `task show`/`task list`) and hooks::dispatch (Task 9's
// briefing).
#[allow(dead_code)]
pub fn progress(conn: &Connection, task_id: &str) -> Result<Option<(i64, i64)>, ServiceError> {
    let (done, total): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(done), 0), COUNT(*) FROM checklist_items WHERE task_id = ?1",
        params![task_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(if total == 0 {
        None
    } else {
        Some((done, total))
    })
}

// Consumed by cli::task_cmd (Task 8, `task show`) and hooks::dispatch (Task 9/10, the Stop
// handoff rule and the session-start briefing).
#[allow(dead_code)]
pub fn last_handoff(conn: &Connection, task_id: &str) -> Result<Option<Event>, ServiceError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM events WHERE task_id = ?1 AND kind = ?2 ORDER BY id DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map(params![task_id, EventKind::Handoff.as_str()], |r| {
        Event::from_row(r)
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// What `create` needs. `repo`/`repo_root` come from the marker the face resolved: ratchet has no
/// registry of repos, so a task cannot be created outside one (G2-R1).
// Consumed by cli::task_cmd (Task 8, `task new`), which constructs one from the parsed CLI args.
#[allow(dead_code)]
pub struct NewTask<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub repo: &'a str,
    pub repo_root: &'a str,
    pub priority: i64,
    pub parent_id: Option<&'a str>,
    pub tags: &'a [String],
    pub checklist: &'a [String],
}

/// Creates a task in `backlog` with its checklist, inside one transaction, and appends one
/// `task.created` event. The identifier comes from the `task_seq` table, so a deleted or archived
/// task never gives its number back.
// Consumed by cli::task_cmd (Task 8, `task new`).
#[allow(dead_code)]
pub fn create(
    conn: &mut Connection,
    new: NewTask<'_>,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Task, ServiceError> {
    if new.title.trim().is_empty() {
        return Err(ServiceError::Invalid("the title cannot be empty".into()));
    }
    if !(1..=4).contains(&new.priority) {
        return Err(ServiceError::Invalid(
            "the priority goes from 1 (most urgent) to 4".into(),
        ));
    }
    let ts = clock::iso(now);
    let tags = serde_json::to_string(new.tags).unwrap_or_else(|_| "[]".to_string());
    // G2-P8: the transaction opens first; the parent lookup below is `create`'s one precondition
    // read from the database, and it runs through `&tx`, not the bare connection, so nothing else
    // can turn a valid parent into a grandchild between the check and the insert.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(parent_id) = new.parent_id {
        let parent = get(&tx, parent_id)?;
        if parent.parent_id.is_some() {
            return Err(ServiceError::Invalid(format!(
                "{parent_id} is already a subtask; only one level of nesting is allowed"
            )));
        }
    }
    tx.execute("INSERT INTO task_seq DEFAULT VALUES", [])?;
    // `last_insert_rowid()` is the connection's, not the statement's: this read must stay here,
    // immediately after the sequence insert and before any other write in this transaction.
    // Moving it below the `tasks` insert would silently number the task after the wrong row.
    let task_id = format!("T-{:04}", tx.last_insert_rowid());
    tx.execute(
        "INSERT INTO tasks(id,title,body,repo,repo_root,status,priority,parent_id,tags,claimed_by,\
         created_at,updated_at,archived_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,?10,?10,NULL)",
        params![
            task_id,
            new.title.trim(),
            new.body,
            new.repo,
            new.repo_root,
            TaskStatus::Backlog.as_str(),
            new.priority,
            new.parent_id,
            tags,
            ts
        ],
    )?;
    for (position, text) in new.checklist.iter().enumerate() {
        tx.execute(
            "INSERT INTO checklist_items(task_id, position, text) VALUES (?1, ?2, ?3)",
            params![task_id, (position + 1) as i64, text],
        )?;
    }
    events::emit(
        &tx,
        EventKind::TaskCreated,
        &json!({
            "title": new.title.trim(),
            "repo": new.repo,
            "priority": new.priority,
            "parent_id": new.parent_id,
            "tags": new.tags,
            "checklist": new.checklist.len(),
        }),
        source,
        session_id,
        Some(&task_id),
        now,
    )?;
    tx.commit()?;
    get(conn, &task_id)
}

/// Moves a task, checking the table and the evidence `done` needs. One transaction, one event.
/// `unreviewed` is the owner's escape hatch (`--unreviewed`): it skips the independent-review
/// check below (the checklist evidence still applies) and leaves a note behind so the bypass is
/// visible on the board.
// Consumed by cli::task_cmd (Task 8, `task status`).
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub fn transition(
    conn: &mut Connection,
    task_id: &str,
    to: TaskStatus,
    source: Source,
    session_id: Option<&str>,
    why: Option<&str>,
    unreviewed: bool,
    now: DateTime<Utc>,
) -> Result<Task, ServiceError> {
    // G2-P8: transaction first; every precondition — the status/archived checks below, the
    // checklist read inside `require_done_evidence`, and the event history read inside
    // `require_independent_review` — reads through `&tx`, the same locked snapshot the write
    // commits from.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let task = get(&tx, task_id)?;
    if task.archived_at.is_some() {
        return Err(ServiceError::Invalid(format!(
            "{task_id} is archived; unarchive it (ratchet task unarchive {task_id}) before moving it"
        )));
    }
    let allowed = allowed_transitions(task.status);
    if !allowed.contains(&to) {
        let listed = allowed
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ServiceError::Invalid(format!(
            "{task_id} cannot go from {} to {}; allowed from {}: {listed}",
            task.status.as_str(),
            to.as_str(),
            task.status.as_str()
        )));
    }
    if to == TaskStatus::Done {
        require_done_evidence(&tx, &task, why)?;
        if !unreviewed {
            require_independent_review(&tx, &task)?;
        }
    }
    set_status_in(&tx, &task, to, source, session_id, why, now)?;
    if to == TaskStatus::Done && unreviewed {
        events::emit(
            &tx,
            EventKind::Note,
            &json!({ "text": "done without independent review" }),
            source,
            session_id,
            Some(task_id),
            now,
        )?;
    }
    tx.commit()?;
    get(conn, task_id)
}

// Consumed by cli::task_cmd (Task 8, `task archive`).
#[allow(dead_code)]
pub fn archive(
    conn: &mut Connection,
    task_id: &str,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Task, ServiceError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let task = get(&tx, task_id)?;
    if task.status != TaskStatus::Done {
        return Err(ServiceError::Invalid(format!(
            "only a done task can be archived; {task_id} is {}",
            task.status.as_str()
        )));
    }
    if task.archived_at.is_some() {
        return Err(ServiceError::Invalid(format!(
            "{task_id} is already archived"
        )));
    }
    let ts = clock::iso(now);
    // The read above holds through commit: an IMMEDIATE transaction takes the write lock before
    // it reads, so no other writer can archive or delete this row out from under it. The
    // affected-row check is the belt to that lock's suspenders (G2-P8): if it ever trips, this
    // must not emit `task.archived` for a fact that did not happen.
    let affected = tx.execute(
        "UPDATE tasks SET archived_at = ?1, updated_at = ?1 WHERE id = ?2",
        params![ts, task_id],
    )?;
    if affected == 0 {
        return Err(ServiceError::NotFound(format!(
            "task {task_id} does not exist"
        )));
    }
    events::emit(
        &tx,
        EventKind::TaskArchived,
        &json!({}),
        source,
        session_id,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    get(conn, task_id)
}

// Consumed by cli::task_cmd (Task 8, `task unarchive`).
#[allow(dead_code)]
pub fn unarchive(
    conn: &mut Connection,
    task_id: &str,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Task, ServiceError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let task = get(&tx, task_id)?;
    let Some(archived_at) = task.archived_at else {
        return Err(ServiceError::Invalid(format!("{task_id} is not archived")));
    };
    let ts = clock::iso(now);
    let affected = tx.execute(
        "UPDATE tasks SET archived_at = NULL, updated_at = ?1 WHERE id = ?2",
        params![ts, task_id],
    )?;
    if affected == 0 {
        return Err(ServiceError::NotFound(format!(
            "task {task_id} does not exist"
        )));
    }
    events::emit(
        &tx,
        EventKind::TaskUnarchived,
        &json!({ "archived_at": clock::iso(archived_at) }),
        source,
        session_id,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    get(conn, task_id)
}

/// The only place a status is written. Takes the open connection (a `&Transaction` derefs into
/// one), so a caller that changes a status twice — `claim` from `backlog` — does it all inside one
/// transaction. Entering `done` gives the claim back: a finished task with a holder is dirty data,
/// and the event records which session it was.
// Called only from `release` and `transition` above; Task 6 calls it directly from inside its own
// transaction (checklist mutations), which is when it stops being unreachable from `main`.
#[allow(dead_code)]
fn set_status_in(
    conn: &Connection,
    task: &Task,
    to: TaskStatus,
    source: Source,
    session_id: Option<&str>,
    why: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), ServiceError> {
    let ts = clock::iso(now);
    let released = if to == TaskStatus::Done {
        task.claimed_by.clone()
    } else {
        None
    };
    let affected = if released.is_some() {
        conn.execute(
            "UPDATE tasks SET status = ?1, claimed_by = NULL, updated_at = ?2 WHERE id = ?3",
            params![to.as_str(), ts, task.id],
        )?
    } else {
        conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![to.as_str(), ts, task.id],
        )?
    };
    if affected == 0 {
        return Err(ServiceError::NotFound(format!(
            "task {} does not exist",
            task.id
        )));
    }
    events::emit(
        conn,
        EventKind::TaskStatus,
        &json!({
            "from": task.status.as_str(),
            "to": to.as_str(),
            "why": why,
            "claim_released": released,
        }),
        source,
        session_id,
        Some(&task.id),
        now,
    )?;
    Ok(())
}

/// `done` is the one status that needs evidence: every item checked, or an explicit reason when
/// there is no checklist to check.
// Called only from `transition` above; Task 8 wires `transition` into the CLI, which is when this
// stops being unreachable from `main`.
#[allow(dead_code)]
fn require_done_evidence(
    conn: &Connection,
    task: &Task,
    why: Option<&str>,
) -> Result<(), ServiceError> {
    let items = checklist(conn, &task.id)?;
    if items.is_empty() {
        if why.map(|w| w.trim().is_empty()).unwrap_or(true) {
            return Err(ServiceError::Invalid(format!(
                "{} has no checklist: give a reason (--why) to close it as done",
                task.id
            )));
        }
        return Ok(());
    }
    let pending: Vec<String> = items
        .iter()
        .filter(|i| !i.done)
        .map(|i| format!("{}. {}", i.position, i.text))
        .collect();
    if pending.is_empty() {
        return Ok(());
    }
    Err(ServiceError::Invalid(format!(
        "cannot close {}: checklist pending: {}",
        task.id,
        pending.join("; ")
    )))
}

/// `done`'s other piece of evidence: a review verdict from a session that did not do the work.
/// Reads the most recent `review.verdict` event and refuses unless it is `approve` from a session
/// that is neither the current holder nor any session that ever claimed the task or checked off
/// one of its items — a self-review by any name.
// Called only from `transition` above.
#[allow(dead_code)]
fn require_independent_review(conn: &Connection, task: &Task) -> Result<(), ServiceError> {
    // `who` is the session to tell the reviewer to avoid: the verdict's own session when there
    // is a disqualified verdict to point at (whether it was disqualified for being the current
    // holder or for a claimed/checklist.done entry in the task's history), and the current
    // holder only when there is no verdict at all to draw a session from.
    let missing = |who: Option<&str>| {
        let who = who
            .map(short_session)
            .unwrap_or_else(|| "the session that holds it".to_string());
        ServiceError::Invalid(format!(
            "{}: no independent review verdict — have the reviewer run: ratchet task review {} approve \"...\" (from a session other than {who})",
            task.id, task.id
        ))
    };
    let Some(ev) = latest_review_verdict(conn, &task.id)? else {
        return Err(missing(task.claimed_by.as_deref()));
    };
    let verdict = ev
        .payload
        .get("verdict")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if verdict != "approve" {
        return Err(ServiceError::Invalid(format!(
            "{}: last verdict is \"{verdict}\" ({}) — fix, then have the reviewer run: ratchet task review {} approve \"...\"",
            task.id,
            clock::iso(ev.ts),
            task.id
        )));
    }
    let reviewer = ev.session_id.as_deref();
    let disqualified = match reviewer {
        Some(r) => session_worked_on(conn, &task.id, task.claimed_by.as_deref(), r)?,
        None => true,
    };
    if disqualified {
        return Err(missing(reviewer.or(task.claimed_by.as_deref())));
    }
    Ok(())
}

/// The most recent `review.verdict` event of a task, if any.
fn latest_review_verdict(conn: &Connection, task_id: &str) -> Result<Option<Event>, ServiceError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM events WHERE task_id = ?1 AND kind = ?2 ORDER BY id DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map(
        params![task_id, EventKind::ReviewVerdict.as_str()],
        Event::from_row,
    )?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Whether `session` is the task's current holder, or ever recorded a `task.claimed` or
/// `checklist.done` event on it — the set of sessions too close to the work to review it.
fn session_worked_on(
    conn: &Connection,
    task_id: &str,
    holder: Option<&str>,
    session: &str,
) -> Result<bool, ServiceError> {
    if holder == Some(session) {
        return Ok(true);
    }
    let mut stmt = conn.prepare(
        "SELECT 1 FROM events WHERE task_id = ?1 AND session_id = ?2 AND kind IN (?3, ?4) LIMIT 1",
    )?;
    Ok(stmt
        .query_row(
            params![
                task_id,
                session,
                EventKind::TaskClaimed.as_str(),
                EventKind::ChecklistDone.as_str()
            ],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Eight characters are enough to name a session in a refusal message; shorter ids stay whole.
/// Mirrors `hooks::briefing::short` — kept local so `services` never depends on `hooks`.
fn short_session(session_id: &str) -> String {
    if session_id.chars().count() > 8 {
        format!("{}…", session_id.chars().take(8).collect::<String>())
    } else {
        session_id.to_string()
    }
}

/// Puts the task in `in_progress` under `session_id`. A task in `backlog` goes through `ready`
/// (two status events) so the history reads the same whichever door it came in by. The claiming
/// session must be registered; a task held by a session that is still live or idle is not taken
/// from it, and one held by a dead session is, with a note.
// Consumed by cli::task_cmd (Task 8, `task claim`).
#[allow(dead_code)]
pub fn claim(
    conn: &mut Connection,
    task_id: &str,
    session_id: &str,
    th: &Thresholds,
    source: Source,
    now: DateTime<Utc>,
) -> Result<Task, ServiceError> {
    // G2-P8: the transaction opens first; every precondition below — the task's own state, the
    // claiming session's registration, and the current holder's liveness — reads through `&tx`,
    // the same locked snapshot the writes commit from. Two concurrent claims of the same task
    // serialize on this lock: the loser re-reads the holder the winner just set, so exactly one
    // of them can ever see the task as free.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let task = get(&tx, task_id)?;
    if task.archived_at.is_some() {
        return Err(ServiceError::Invalid(format!(
            "{task_id} is archived; unarchive it before claiming it"
        )));
    }
    if task.status == TaskStatus::Done {
        return Err(ServiceError::Invalid(format!(
            "{task_id} is done; a done task cannot be claimed"
        )));
    }
    if sessions::get(&tx, session_id)?.is_none() {
        return Err(ServiceError::Invalid(format!(
            "session {session_id} is not registered; {task_id} was not claimed"
        )));
    }
    let previous = task.claimed_by.clone();
    let mut transferred_from: Option<(String, SessionState)> = None;
    if let Some(holder_id) = previous.as_deref() {
        if holder_id != session_id {
            let holder_state = match sessions::get(&tx, holder_id)? {
                Some(holder) => sessions::state(&holder, th, now),
                None => SessionState::Orphaned,
            };
            if matches!(holder_state, SessionState::Live | SessionState::Idle) {
                let since = claimed_since(&tx, task_id, holder_id)?
                    .unwrap_or_else(|| clock::iso(task.updated_at));
                return Err(ServiceError::Invalid(format!(
                    "{task_id} is held by session {holder_id} ({}) since {since}",
                    holder_state.as_str()
                )));
            }
            transferred_from = Some((holder_id.to_string(), holder_state));
        }
    }
    let ts = clock::iso(now);
    let mut current = task.clone();
    if current.status == TaskStatus::Backlog {
        set_status_in(
            &tx,
            &current,
            TaskStatus::Ready,
            source,
            Some(session_id),
            None,
            now,
        )?;
        current.status = TaskStatus::Ready;
    }
    if current.status != TaskStatus::InProgress {
        set_status_in(
            &tx,
            &current,
            TaskStatus::InProgress,
            source,
            Some(session_id),
            None,
            now,
        )?;
        current.status = TaskStatus::InProgress;
    }
    tx.execute(
        "UPDATE tasks SET claimed_by = ?1, updated_at = ?2 WHERE id = ?3",
        params![session_id, ts, task_id],
    )?;
    events::emit(
        &tx,
        EventKind::TaskClaimed,
        &json!({ "previous": previous }),
        source,
        Some(session_id),
        Some(task_id),
        now,
    )?;
    if let Some((holder_id, holder_state)) = transferred_from {
        events::emit(
            &tx,
            EventKind::Note,
            &json!({
                "text": format!(
                    "transferred from session {holder_id} ({}) to {session_id}",
                    holder_state.as_str()
                ),
                // Ruling G2-P19: this note is part of the claim, not a record of any work. The
                // marker is what the Stop handoff rule exempts; never a match on `text`.
                "claim": true
            }),
            source,
            Some(session_id),
            Some(task_id),
            now,
        )?;
    }
    tx.commit()?;
    get(conn, task_id)
}

// Consumed by cli::task_cmd (Task 8, `task check`).
#[allow(dead_code)]
pub fn check(
    conn: &mut Connection,
    task_id: &str,
    position: i64,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ChecklistItem, ServiceError> {
    set_item(conn, task_id, position, true, source, session_id, now)
}

// Consumed by cli::task_cmd (Task 8, `task check --undo`).
#[allow(dead_code)]
pub fn uncheck(
    conn: &mut Connection,
    task_id: &str,
    position: i64,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ChecklistItem, ServiceError> {
    set_item(conn, task_id, position, false, source, session_id, now)
}

/// Marks or unmarks one item. The `updated_at` bump of the task belongs to the same fact as the
/// item write, so the pair emits exactly one event.
// Called only from `check` and `uncheck` above; Task 8's CLI wiring is when it stops being
// unreachable from `main`.
#[allow(dead_code)]
fn set_item(
    conn: &mut Connection,
    task_id: &str,
    position: i64,
    done: bool,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ChecklistItem, ServiceError> {
    // G2-P8: the transaction opens first; the checklist read that locates the item, and the task
    // read that produces a NotFound when the task itself does not exist, both run through `&tx`.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let items = checklist(&tx, task_id)?;
    if items.is_empty() {
        get(&tx, task_id)?; // NotFound if the task itself is the problem
        return Err(ServiceError::Invalid(format!("{task_id} has no checklist")));
    }
    let Some(item) = items.iter().find(|i| i.position == position) else {
        let listed = items
            .iter()
            .map(|i| format!("{}. {}", i.position, i.text))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ServiceError::Invalid(format!(
            "{task_id} has no item {position}; available: {listed}"
        )));
    };
    let ts = clock::iso(now);
    // Keyed by (task_id, position), not `item.id`, so the affected-row count below is a genuine
    // check of the fact this call is about to claim happened, not a tautology (G2-P8): 0 rows
    // means the item vanished between the read above and this write, and no event is emitted for
    // a fact that did not happen.
    let affected = tx.execute(
        "UPDATE checklist_items SET done = ?1, done_by_session = ?2, done_at = ?3 \
         WHERE task_id = ?4 AND position = ?5",
        params![
            i64::from(done),
            if done { session_id } else { None },
            if done { Some(ts.as_str()) } else { None },
            task_id,
            position
        ],
    )?;
    if affected == 0 {
        return Err(ServiceError::NotFound(format!(
            "{task_id} has no item {position}"
        )));
    }
    tx.execute(
        "UPDATE tasks SET updated_at = ?1 WHERE id = ?2",
        params![ts, task_id],
    )?;
    events::emit(
        &tx,
        if done {
            EventKind::ChecklistDone
        } else {
            EventKind::ChecklistUndone
        },
        &json!({ "item_id": item.id, "position": position, "text": item.text }),
        source,
        session_id,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    let mut stmt = conn.prepare("SELECT * FROM checklist_items WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![item.id], ChecklistItem::from_row)?;
    match rows.next() {
        Some(row) => Ok(row?),
        None => Err(ServiceError::NotFound(format!(
            "item {position} of {task_id} disappeared while writing it"
        ))),
    }
}

// Consumed by cli::task_cmd (Task 8, `task note`).
#[allow(dead_code)]
pub fn note(
    conn: &mut Connection,
    task_id: &str,
    text: &str,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Event, ServiceError> {
    // G2-P8: the transaction opens first; the task's existence is read through `&tx`.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    get(&tx, task_id)?;
    let ev = events::emit(
        &tx,
        EventKind::Note,
        &json!({ "text": text }),
        source,
        session_id,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    Ok(ev)
}

/// The text the next session reads first, so it is never allowed to be empty.
// Consumed by cli::task_cmd (Task 8, `task handoff`) and hooks::dispatch (Task 9/10, the Stop
// handoff rule).
#[allow(dead_code)]
pub fn handoff(
    conn: &mut Connection,
    task_id: &str,
    text: &str,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Event, ServiceError> {
    if text.trim().is_empty() {
        return Err(ServiceError::Invalid(
            "a handoff cannot be empty: what is left, and how to resume".into(),
        ));
    }
    // G2-P8: the transaction opens first; the task's existence is read through `&tx`.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    get(&tx, task_id)?;
    let ev = events::emit(
        &tx,
        EventKind::Handoff,
        &json!({ "text": text }),
        source,
        session_id,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    Ok(ev)
}

/// Records an independent review verdict (`approve` or `changes`) with free text, as a
/// `review.verdict` event. Never changes the task's status — `transition` is the only place that
/// does that, and it is what reads this event back when `done` is asked for. The recording
/// session need not be registered: a reviewer profile mints its own session identifier so it is
/// never mistaken for the implementer's (agent-protocol convention, not a session-registry rule).
// Consumed by cli::task_cmd (`task review`).
#[allow(dead_code)]
pub fn review(
    conn: &mut Connection,
    task_id: &str,
    verdict: &str,
    text: &str,
    source: Source,
    session_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Event, ServiceError> {
    // G2-P8: the transaction opens first; the task's existence is read through `&tx`.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    get(&tx, task_id)?;
    let ev = events::emit(
        &tx,
        EventKind::ReviewVerdict,
        &json!({ "verdict": verdict, "text": text }),
        source,
        session_id,
        Some(task_id),
        now,
    )?;
    tx.commit()?;
    Ok(ev)
}

/// When this session took the task, for the message that refuses a second claim.
// Called only from `claim` above; Task 8's CLI wiring is when it stops being unreachable from
// `main`.
#[allow(dead_code)]
fn claimed_since(
    conn: &Connection,
    task_id: &str,
    session_id: &str,
) -> Result<Option<String>, ServiceError> {
    Ok(conn
        .query_row(
            "SELECT ts FROM events WHERE task_id = ?1 AND session_id = ?2 AND kind = ?3 \
             ORDER BY id DESC LIMIT 1",
            params![task_id, session_id, EventKind::TaskClaimed.as_str()],
            |r| r.get::<_, String>(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::model::{LaunchedBy, SessionMode};
    use crate::services::sessions::{self, StartInput};

    fn at(s: &str) -> DateTime<Utc> {
        clock::parse(s).unwrap()
    }

    fn setup(session_at: &str) -> Connection {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        sessions::upsert_start(
            &mut c,
            StartInput {
                session_id: "s-holder",
                repo: "demo",
                repo_root: "root",
                cwd: "root",
                worktree: None,
                branch: None,
                mode: SessionMode::Interactive,
                launched_by: LaunchedBy::User,
            },
            at(session_at),
        )
        .unwrap();
        c.execute(
            "INSERT INTO tasks(id,title,body,repo,repo_root,status,priority,tags,claimed_by,created_at,updated_at) \
             VALUES ('T-0001','build it','','demo','root','in_progress',3,'[]','s-holder',?1,?1)",
            rusqlite::params![session_at],
        )
        .unwrap();
        c
    }

    #[test]
    fn a_task_of_a_live_session_is_not_orphaned() {
        let c = setup("2026-09-16T12:00:00Z");
        let found = orphaned(
            &c,
            "root",
            &Thresholds::default(),
            at("2026-09-16T12:05:00Z"),
        )
        .unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn a_task_of_an_orphaned_session_is_listed_but_not_released_by_listing() {
        let c = setup("2026-09-16T12:00:00Z");
        let found = orphaned(
            &c,
            "root",
            &Thresholds::default(),
            at("2026-09-16T13:30:00Z"),
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].task_id, "T-0001");
        assert_eq!(found[0].claimed_by, "s-holder");
        let status: String = c
            .query_row("SELECT status FROM tasks WHERE id = 'T-0001'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "in_progress", "listing must be pure");
    }

    #[test]
    fn a_task_claimed_by_a_session_the_registry_never_saw_is_orphaned() {
        let c = setup("2026-09-16T12:00:00Z");
        c.execute(
            "INSERT INTO tasks(id,title,body,repo,repo_root,status,priority,tags,claimed_by,created_at,updated_at) \
             VALUES ('T-0002','ghost work','','demo','root','in_progress',3,'[]','s-ghost','2026-09-16T12:00:00Z','2026-09-16T12:00:00Z')",
            [],
        )
        .unwrap();
        // s-holder is still live at this `now`; only the row claimed by the unregistered
        // s-ghost must come back.
        let found = orphaned(
            &c,
            "root",
            &Thresholds::default(),
            at("2026-09-16T12:05:00Z"),
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].task_id, "T-0002");
        assert_eq!(found[0].claimed_by, "s-ghost");
    }

    #[test]
    fn release_returns_the_task_and_writes_two_events() {
        let mut c = setup("2026-09-16T12:00:00Z");
        release(
            &mut c,
            "T-0001",
            "end of session",
            Source::Hook,
            at("2026-09-16T13:30:00Z"),
        )
        .unwrap();
        let (status, holder): (String, Option<String>) = c
            .query_row(
                "SELECT status, claimed_by FROM tasks WHERE id = 'T-0001'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "ready");
        assert_eq!(holder, None);
        let kinds: Vec<String> = crate::services::events::for_task(&c, "T-0001", 10)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(kinds, vec!["task.status", "note"]);
    }

    #[test]
    fn releasing_an_unclaimed_task_is_rejected() {
        let mut c = setup("2026-09-16T12:00:00Z");
        c.execute("UPDATE tasks SET claimed_by = NULL WHERE id = 'T-0001'", [])
            .unwrap();
        let err = release(
            &mut c,
            "T-0001",
            "x",
            Source::Cli,
            at("2026-09-16T12:01:00Z"),
        )
        .unwrap_err();
        assert!(matches!(err, ServiceError::Invalid(_)), "{err:?}");
    }

    #[test]
    fn releasing_a_task_that_does_not_exist_is_not_found() {
        let mut c = setup("2026-09-16T12:00:00Z");
        let err = release(
            &mut c,
            "T-9999",
            "x",
            Source::Cli,
            at("2026-09-16T12:01:00Z"),
        )
        .unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)), "{err:?}");
    }

    #[test]
    fn release_dead_sweeps_only_the_dead() {
        let mut c = setup("2026-09-16T12:00:00Z");
        let none = release_dead(
            &mut c,
            "root",
            &Thresholds::default(),
            "end of session",
            at("2026-09-16T12:05:00Z"),
        )
        .unwrap();
        assert!(none.is_empty());
        let swept = release_dead(
            &mut c,
            "root",
            &Thresholds::default(),
            "end of session",
            at("2026-09-16T13:30:00Z"),
        )
        .unwrap();
        assert_eq!(swept, vec!["T-0001".to_string()]);
    }

    #[test]
    fn release_dead_note_names_the_trigger_it_was_called_with() {
        // release_dead has two callers (hooks::dispatch): session-end passes "end of session"
        // for a tidy close, session-start passes the lazy-sweep wording. The note must say
        // whichever one actually happened, not a hardcoded string.
        let mut c = setup("2026-09-16T12:00:00Z");
        release_dead(
            &mut c,
            "root",
            &Thresholds::default(),
            "the start of the next session",
            at("2026-09-16T13:30:00Z"),
        )
        .unwrap();
        let text: String = c
            .query_row(
                "SELECT payload FROM events WHERE task_id = 'T-0001' AND kind = 'note'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            text.contains("released by the start of the next session"),
            "{text}"
        );
        assert!(!text.contains("end of session"), "{text}");
    }

    #[test]
    fn release_dead_releases_every_dead_task_in_a_multi_row_sweep() {
        // A dead session ordinarily holds more than one task; the sweep must not stop at the
        // first row it releases.
        let mut c = setup("2026-09-16T12:00:00Z");
        sessions::upsert_start(
            &mut c,
            StartInput {
                session_id: "s-holder-2",
                repo: "demo",
                repo_root: "root",
                cwd: "root",
                worktree: None,
                branch: None,
                mode: SessionMode::Interactive,
                launched_by: LaunchedBy::User,
            },
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        for (id, holder) in [("T-0002", "s-holder"), ("T-0003", "s-holder-2")] {
            c.execute(
                "INSERT INTO tasks(id,title,body,repo,repo_root,status,priority,tags,claimed_by,created_at,updated_at) \
                 VALUES (?1,'more work','','demo','root','in_progress',3,'[]',?2,'2026-09-16T12:00:00Z','2026-09-16T12:00:00Z')",
                rusqlite::params![id, holder],
            )
            .unwrap();
        }
        let swept = release_dead(
            &mut c,
            "root",
            &Thresholds::default(),
            "end of session",
            at("2026-09-16T13:30:00Z"),
        )
        .unwrap();
        assert_eq!(
            swept,
            vec![
                "T-0001".to_string(),
                "T-0002".to_string(),
                "T-0003".to_string(),
            ]
        );
        for id in ["T-0001", "T-0002", "T-0003"] {
            let (status, holder): (String, Option<String>) = c
                .query_row(
                    "SELECT status, claimed_by FROM tasks WHERE id = ?1",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(status, "ready", "{id}");
            assert_eq!(holder, None, "{id}");
        }
    }

    #[test]
    fn claimed_ids_lists_the_work_in_progress_of_a_session() {
        let c = setup("2026-09-16T12:00:00Z");
        assert_eq!(
            claimed_ids(&c, "s-holder").unwrap(),
            vec!["T-0001".to_string()]
        );
        assert!(claimed_ids(&c, "nobody").unwrap().is_empty());
    }

    /// Seeds one task directly. Task 5 gives this module a real `create`; until it exists the
    /// reads need rows from somewhere, and a test inside the binary may use its own SQL.
    fn seed(conn: &Connection, id: &str, status: TaskStatus, priority: i64, tags: &str) {
        conn.execute(
            "INSERT INTO tasks(id,title,body,repo,repo_root,status,priority,tags,claimed_by,\
             created_at,updated_at) VALUES (?1,?2,'','demo','root',?3,?4,?5,NULL,?6,?6)",
            params![
                id,
                format!("title of {id}"),
                status.as_str(),
                priority,
                tags,
                "2026-09-16T12:00:00Z"
            ],
        )
        .unwrap();
    }

    #[test]
    fn get_finds_a_task_and_says_so_when_it_does_not_exist() {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        seed(&c, "T-0001", TaskStatus::Ready, 3, "[\"one\"]");
        let task = get(&c, "T-0001").unwrap();
        assert_eq!(task.title, "title of T-0001");
        assert_eq!(task.tags, vec!["one".to_string()]);
        let err = get(&c, "T-9999").unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)), "{err:?}");
        assert!(err.to_string().contains("T-9999"), "{err}");
    }

    #[test]
    fn list_orders_by_priority_then_id_and_filters() {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        seed(&c, "T-0001", TaskStatus::Ready, 3, "[]");
        seed(&c, "T-0002", TaskStatus::Ready, 1, "[\"urgent\"]");
        seed(&c, "T-0003", TaskStatus::Backlog, 2, "[]");
        let all = list(&c, &Filter::default()).unwrap();
        assert_eq!(
            all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["T-0002", "T-0003", "T-0001"]
        );
        let ready = list(
            &c,
            &Filter {
                statuses: &[TaskStatus::Ready],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ready.len(), 2);
        let tagged = list(
            &c,
            &Filter {
                tag: Some("urgent"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(tagged.len(), 1);
        let elsewhere = list(
            &c,
            &Filter {
                repo_root: Some("another root"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(elsewhere.is_empty());
    }

    #[test]
    fn list_hides_archived_tasks_unless_asked() {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        seed(&c, "T-0001", TaskStatus::Done, 3, "[]");
        c.execute(
            "UPDATE tasks SET archived_at = '2026-09-16T12:00:00Z' WHERE id = 'T-0001'",
            [],
        )
        .unwrap();
        assert!(list(&c, &Filter::default()).unwrap().is_empty());
        let with_archived = list(
            &c,
            &Filter {
                include_archived: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(with_archived.len(), 1);
    }

    #[test]
    fn progress_is_derived_and_absent_without_a_checklist() {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        seed(&c, "T-0001", TaskStatus::InProgress, 3, "[]");
        assert_eq!(progress(&c, "T-0001").unwrap(), None);
        for (pos, text) in [(1, "a"), (2, "b"), (3, "c")] {
            c.execute(
                "INSERT INTO checklist_items(task_id, position, text) VALUES ('T-0001', ?1, ?2)",
                params![pos, text],
            )
            .unwrap();
        }
        assert_eq!(progress(&c, "T-0001").unwrap(), Some((0, 3)));
        c.execute(
            "UPDATE checklist_items SET done = 1 WHERE task_id = 'T-0001' AND position = 2",
            [],
        )
        .unwrap();
        assert_eq!(progress(&c, "T-0001").unwrap(), Some((1, 3)));
        let items = checklist(&c, "T-0001").unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].position, 1);
        assert!(items[1].done);
    }

    #[test]
    fn last_handoff_is_the_most_recent_one() {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        seed(&c, "T-0001", TaskStatus::InProgress, 3, "[]");
        assert!(last_handoff(&c, "T-0001").unwrap().is_none());
        for (minute, text) in [("10", "the early one"), ("12", "the late one")] {
            events::emit(
                &c,
                EventKind::Handoff,
                &json!({ "text": text }),
                Source::Cli,
                Some("s-1"),
                Some("T-0001"),
                at(&format!("2026-09-16T{minute}:00:00Z")),
            )
            .unwrap();
        }
        let last = last_handoff(&c, "T-0001").unwrap().unwrap();
        assert_eq!(last.payload["text"], "the late one");
    }

    fn fresh() -> Connection {
        let mut c = db::open_memory().unwrap();
        db::migrate(&mut c).unwrap();
        c
    }

    fn simple<'a>(title: &'a str, checklist: &'a [String]) -> NewTask<'a> {
        NewTask {
            title,
            body: "",
            repo: "demo",
            repo_root: "root",
            priority: 3,
            parent_id: None,
            tags: &[],
            checklist,
        }
    }

    #[test]
    fn create_numbers_tasks_in_sequence_and_never_reuses_a_number() {
        let mut c = fresh();
        let items = vec!["one".to_string(), "two".to_string()];
        let first = create(
            &mut c,
            simple("first", &items),
            Source::Cli,
            Some("s-1"),
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        assert_eq!(first.id, "T-0001");
        assert_eq!(first.status, TaskStatus::Backlog);
        assert_eq!(checklist(&c, &first.id).unwrap().len(), 2);
        c.execute("DELETE FROM checklist_items WHERE task_id = 'T-0001'", [])
            .unwrap();
        c.execute("DELETE FROM tasks WHERE id = 'T-0001'", [])
            .unwrap();
        let second = create(
            &mut c,
            simple("second", &[]),
            Source::Cli,
            None,
            at("2026-09-16T12:01:00Z"),
        )
        .unwrap();
        assert_eq!(second.id, "T-0002", "a number was reused");
        assert_eq!(
            events::for_task(&c, "T-0002", 10).unwrap()[0].kind,
            "task.created"
        );
    }

    #[test]
    fn create_refuses_an_empty_title_a_bad_priority_and_a_grandchild() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let blank = NewTask {
            title: "   ",
            ..simple("x", &[])
        };
        assert!(matches!(
            create(&mut c, blank, Source::Cli, None, ts).unwrap_err(),
            ServiceError::Invalid(_)
        ));
        let urgent = NewTask {
            priority: 7,
            ..simple("x", &[])
        };
        let err = create(&mut c, urgent, Source::Cli, None, ts).unwrap_err();
        assert!(err.to_string().contains('4'), "{err}");
        let parent = create(&mut c, simple("parent", &[]), Source::Cli, None, ts).unwrap();
        let child = create(
            &mut c,
            NewTask {
                parent_id: Some(&parent.id),
                ..simple("child", &[])
            },
            Source::Cli,
            None,
            ts,
        )
        .unwrap();
        let err = create(
            &mut c,
            NewTask {
                parent_id: Some(&child.id),
                ..simple("grandchild", &[])
            },
            Source::Cli,
            None,
            ts,
        )
        .unwrap_err();
        assert!(err.to_string().contains("one level"), "{err}");
        assert_eq!(list(&c, &Filter::default()).unwrap().len(), 2);
    }

    #[test]
    fn transition_follows_the_table_and_records_origin_and_destination() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        let err = transition(
            &mut c,
            &task.id,
            TaskStatus::Done,
            Source::Cli,
            None,
            None,
            true,
            ts,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("ready"),
            "allowed list missing: {err}"
        );
        transition(
            &mut c,
            &task.id,
            TaskStatus::Ready,
            Source::Cli,
            None,
            None,
            false,
            ts,
        )
        .unwrap();
        let moved = transition(
            &mut c,
            &task.id,
            TaskStatus::InProgress,
            Source::Cli,
            Some("s-1"),
            None,
            false,
            ts,
        )
        .unwrap();
        assert_eq!(moved.status, TaskStatus::InProgress);
        let blocked = transition(
            &mut c,
            &task.id,
            TaskStatus::Blocked,
            Source::Cli,
            Some("s-1"),
            Some("waiting"),
            false,
            ts,
        )
        .unwrap();
        assert_eq!(blocked.status, TaskStatus::Blocked);
        let last = events::for_task(&c, &task.id, 10).unwrap().pop().unwrap();
        assert_eq!(last.kind, "task.status");
        assert_eq!(last.payload["from"], "in_progress");
        assert_eq!(last.payload["to"], "blocked");
        assert_eq!(last.payload["why"], "waiting");
    }

    #[test]
    fn done_needs_the_checklist_complete_or_a_reason() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let items = vec!["one".to_string(), "two".to_string()];
        let with = create(
            &mut c,
            simple("with a checklist", &items),
            Source::Cli,
            None,
            ts,
        )
        .unwrap();
        transition(
            &mut c,
            &with.id,
            TaskStatus::Ready,
            Source::Cli,
            None,
            None,
            false,
            ts,
        )
        .unwrap();
        transition(
            &mut c,
            &with.id,
            TaskStatus::InProgress,
            Source::Cli,
            None,
            None,
            false,
            ts,
        )
        .unwrap();
        let err = transition(
            &mut c,
            &with.id,
            TaskStatus::Done,
            Source::Cli,
            None,
            None,
            true,
            ts,
        )
        .unwrap_err();
        assert!(err.to_string().contains("two"), "{err}");
        let without = create(&mut c, simple("no checklist", &[]), Source::Cli, None, ts).unwrap();
        transition(
            &mut c,
            &without.id,
            TaskStatus::Ready,
            Source::Cli,
            None,
            None,
            false,
            ts,
        )
        .unwrap();
        transition(
            &mut c,
            &without.id,
            TaskStatus::InProgress,
            Source::Cli,
            None,
            None,
            false,
            ts,
        )
        .unwrap();
        assert!(transition(
            &mut c,
            &without.id,
            TaskStatus::Done,
            Source::Cli,
            None,
            None,
            true,
            ts
        )
        .is_err());
        let done = transition(
            &mut c,
            &without.id,
            TaskStatus::Done,
            Source::Cli,
            None,
            Some("obsolete"),
            true,
            ts,
        )
        .unwrap();
        assert_eq!(done.status, TaskStatus::Done);
    }

    #[test]
    fn entering_done_gives_back_the_claim_and_says_so() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let task = create(&mut c, simple("held", &[]), Source::Cli, None, ts).unwrap();
        transition(
            &mut c,
            &task.id,
            TaskStatus::Ready,
            Source::Cli,
            None,
            None,
            false,
            ts,
        )
        .unwrap();
        transition(
            &mut c,
            &task.id,
            TaskStatus::InProgress,
            Source::Cli,
            None,
            None,
            false,
            ts,
        )
        .unwrap();
        c.execute(
            "UPDATE tasks SET claimed_by = 's-1' WHERE id = ?1",
            params![task.id],
        )
        .unwrap();
        let done = transition(
            &mut c,
            &task.id,
            TaskStatus::Done,
            Source::Cli,
            Some("s-1"),
            Some("shipped"),
            true,
            ts,
        )
        .unwrap();
        assert_eq!(done.claimed_by, None);
        // `--unreviewed` (passed above) appends a note after the status event, so pick the
        // `task.status` event by kind rather than assuming it is the last one in the history.
        let status_event = events::for_task(&c, &task.id, 10)
            .unwrap()
            .into_iter()
            .rev()
            .find(|e| e.kind == "task.status")
            .unwrap();
        assert_eq!(status_event.payload["claim_released"], "s-1");
    }

    #[test]
    fn archiving_hides_a_done_task_and_refuses_anything_else() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        let err = archive(&mut c, &task.id, Source::Cli, None, ts).unwrap_err();
        assert!(err.to_string().contains("done"), "{err}");
        transition(
            &mut c,
            &task.id,
            TaskStatus::Ready,
            Source::Cli,
            None,
            None,
            false,
            ts,
        )
        .unwrap();
        transition(
            &mut c,
            &task.id,
            TaskStatus::InProgress,
            Source::Cli,
            None,
            None,
            false,
            ts,
        )
        .unwrap();
        transition(
            &mut c,
            &task.id,
            TaskStatus::Done,
            Source::Cli,
            None,
            Some("shipped"),
            true,
            ts,
        )
        .unwrap();
        let archived = archive(&mut c, &task.id, Source::Cli, None, ts).unwrap();
        assert!(archived.archived_at.is_some());
        assert!(list(&c, &Filter::default()).unwrap().is_empty());
        assert!(archive(&mut c, &task.id, Source::Cli, None, ts).is_err());
        let back = unarchive(&mut c, &task.id, Source::Cli, None, ts).unwrap();
        assert!(back.archived_at.is_none());
        assert_eq!(list(&c, &Filter::default()).unwrap().len(), 1);
        let kinds: Vec<String> = events::for_task(&c, &task.id, 20)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert!(kinds.contains(&"task.archived".to_string()));
        assert!(kinds.contains(&"task.unarchived".to_string()));
    }

    #[test]
    fn an_archived_task_cannot_be_moved_until_it_comes_back() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        transition(
            &mut c,
            &task.id,
            TaskStatus::Ready,
            Source::Cli,
            None,
            None,
            false,
            ts,
        )
        .unwrap();
        transition(
            &mut c,
            &task.id,
            TaskStatus::InProgress,
            Source::Cli,
            None,
            None,
            false,
            ts,
        )
        .unwrap();
        transition(
            &mut c,
            &task.id,
            TaskStatus::Done,
            Source::Cli,
            None,
            Some("shipped"),
            true,
            ts,
        )
        .unwrap();
        archive(&mut c, &task.id, Source::Cli, None, ts).unwrap();
        let err = transition(
            &mut c,
            &task.id,
            TaskStatus::Backlog,
            Source::Cli,
            None,
            None,
            false,
            ts,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unarchive"), "{err}");
    }

    fn with_session(id: &str, at_iso: &str) -> Connection {
        let mut c = fresh();
        sessions::upsert_start(
            &mut c,
            StartInput {
                session_id: id,
                repo: "demo",
                repo_root: "root",
                cwd: "root",
                worktree: None,
                branch: None,
                mode: SessionMode::Interactive,
                launched_by: LaunchedBy::User,
            },
            at(at_iso),
        )
        .unwrap();
        c
    }

    #[test]
    fn claim_walks_a_backlog_task_all_the_way_to_in_progress() {
        let mut c = with_session("s-1", "2026-09-16T12:00:00Z");
        let ts = at("2026-09-16T12:01:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        let claimed = claim(
            &mut c,
            &task.id,
            "s-1",
            &Thresholds::default(),
            Source::Cli,
            ts,
        )
        .unwrap();
        assert_eq!(claimed.status, TaskStatus::InProgress);
        assert_eq!(claimed.claimed_by.as_deref(), Some("s-1"));
        let kinds: Vec<String> = events::for_task(&c, &task.id, 10)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(
            kinds,
            vec!["task.created", "task.status", "task.status", "task.claimed"]
        );
        let statuses: Vec<Event> = events::for_task(&c, &task.id, 10)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "task.status")
            .collect();
        assert_eq!(statuses[0].payload["from"], "backlog");
        assert_eq!(statuses[0].payload["to"], "ready");
        assert_eq!(statuses[1].payload["from"], "ready");
        assert_eq!(statuses[1].payload["to"], "in_progress");
    }

    #[test]
    fn claim_refuses_an_unregistered_session_and_a_done_task() {
        let mut c = with_session("s-1", "2026-09-16T12:00:00Z");
        let ts = at("2026-09-16T12:01:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        let err = claim(
            &mut c,
            &task.id,
            "s-ghost",
            &Thresholds::default(),
            Source::Cli,
            ts,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not registered"), "{err}");
        assert_eq!(get(&c, &task.id).unwrap().status, TaskStatus::Backlog);
        claim(
            &mut c,
            &task.id,
            "s-1",
            &Thresholds::default(),
            Source::Cli,
            ts,
        )
        .unwrap();
        transition(
            &mut c,
            &task.id,
            TaskStatus::Done,
            Source::Cli,
            Some("s-1"),
            Some("shipped"),
            true,
            ts,
        )
        .unwrap();
        let err = claim(
            &mut c,
            &task.id,
            "s-1",
            &Thresholds::default(),
            Source::Cli,
            ts,
        )
        .unwrap_err();
        assert!(err.to_string().contains("done"), "{err}");
    }

    #[test]
    fn a_live_holder_keeps_the_task_and_a_dead_one_hands_it_over() {
        let mut c = with_session("s-a", "2026-09-16T12:00:00Z");
        sessions::upsert_start(
            &mut c,
            StartInput {
                session_id: "s-b",
                repo: "demo",
                repo_root: "root",
                cwd: "root",
                worktree: None,
                branch: None,
                mode: SessionMode::Interactive,
                launched_by: LaunchedBy::User,
            },
            at("2026-09-16T12:00:00Z"),
        )
        .unwrap();
        let ts = at("2026-09-16T12:01:00Z");
        let th = Thresholds::default();
        let task = create(&mut c, simple("contested", &[]), Source::Cli, None, ts).unwrap();
        claim(&mut c, &task.id, "s-a", &th, Source::Cli, ts).unwrap();
        let err = claim(&mut c, &task.id, "s-b", &th, Source::Cli, ts).unwrap_err();
        assert!(err.to_string().contains("s-a"), "{err}");
        // Ninety minutes on, s-a is orphaned: the claim goes through and leaves a note.
        let later = at("2026-09-16T13:31:00Z");
        let moved = claim(&mut c, &task.id, "s-b", &th, Source::Cli, later).unwrap();
        assert_eq!(moved.claimed_by.as_deref(), Some("s-b"));
        let notes: Vec<String> = events::for_task(&c, &task.id, 20)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "note")
            .map(|e| e.payload["text"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(
            notes.iter().any(|n| n.contains("s-a") && n.contains("s-b")),
            "{notes:?}"
        );
    }

    #[test]
    fn checking_an_item_records_who_and_when_and_emits_one_event() {
        let mut c = with_session("s-1", "2026-09-16T12:00:00Z");
        let ts = at("2026-09-16T12:01:00Z");
        let items = vec!["one".to_string(), "two".to_string()];
        let task = create(&mut c, simple("work", &items), Source::Cli, None, ts).unwrap();
        let before = events::for_task(&c, &task.id, 50).unwrap().len();
        let item = check(&mut c, &task.id, 2, Source::Cli, Some("s-1"), ts).unwrap();
        assert!(item.done);
        assert_eq!(item.done_by_session.as_deref(), Some("s-1"));
        assert_eq!(progress(&c, &task.id).unwrap(), Some((1, 2)));
        let after = events::for_task(&c, &task.id, 50).unwrap();
        assert_eq!(after.len(), before + 1, "more than one event for one fact");
        assert_eq!(after.last().unwrap().kind, "checklist.done");
        let undone = uncheck(&mut c, &task.id, 2, Source::Cli, Some("s-1"), ts).unwrap();
        assert!(!undone.done);
        assert_eq!(undone.done_by_session, None);
        assert_eq!(progress(&c, &task.id).unwrap(), Some((0, 2)));
    }

    #[test]
    fn a_position_that_does_not_exist_lists_the_ones_that_do() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let items = vec!["one".to_string()];
        let task = create(&mut c, simple("work", &items), Source::Cli, None, ts).unwrap();
        let err = check(&mut c, &task.id, 9, Source::Cli, None, ts).unwrap_err();
        assert!(err.to_string().contains("1. one"), "{err}");
        let bare = create(&mut c, simple("no items", &[]), Source::Cli, None, ts).unwrap();
        let err = check(&mut c, &bare.id, 1, Source::Cli, None, ts).unwrap_err();
        assert!(err.to_string().contains("no checklist"), "{err}");
    }

    #[test]
    fn notes_are_free_text_and_a_handoff_cannot_be_blank() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        let ev = note(
            &mut c,
            &task.id,
            "found X, decided Y",
            Source::Cli,
            Some("s-1"),
            ts,
        )
        .unwrap();
        assert_eq!(ev.payload["text"], "found X, decided Y");
        let err = handoff(&mut c, &task.id, "   ", Source::Cli, Some("s-1"), ts).unwrap_err();
        assert!(err.to_string().contains("resume"), "{err}");
        assert!(last_handoff(&c, &task.id).unwrap().is_none());
        handoff(
            &mut c,
            &task.id,
            "half done; resume at item 2",
            Source::Cli,
            Some("s-1"),
            ts,
        )
        .unwrap();
        assert_eq!(
            last_handoff(&c, &task.id).unwrap().unwrap().payload["text"],
            "half done; resume at item 2"
        );
        assert!(note(&mut c, "T-9999", "into the void", Source::Cli, None, ts).is_err());
    }

    // --- Review verdicts and the gate on done --------------------------------------------------

    /// A task claimed by `s-impl`, checklist complete, ready for a `done` attempt but for the
    /// review this whole block is about.
    fn reviewable(ts: DateTime<Utc>) -> (Connection, String) {
        let mut c = with_session("s-impl", "2026-09-16T12:00:00Z");
        let items = vec!["one".to_string()];
        let task = create(&mut c, simple("reviewable", &items), Source::Cli, None, ts).unwrap();
        claim(
            &mut c,
            &task.id,
            "s-impl",
            &Thresholds::default(),
            Source::Cli,
            ts,
        )
        .unwrap();
        check(&mut c, &task.id, 1, Source::Cli, Some("s-impl"), ts).unwrap();
        (c, task.id)
    }

    #[test]
    fn review_records_a_verdict_event_and_never_touches_status() {
        let mut c = fresh();
        let ts = at("2026-09-16T12:00:00Z");
        let task = create(&mut c, simple("work", &[]), Source::Cli, None, ts).unwrap();
        let ev = review(
            &mut c,
            &task.id,
            "approve",
            "looks right",
            Source::Cli,
            Some("s-reviewer"),
            ts,
        )
        .unwrap();
        assert_eq!(ev.kind, "review.verdict");
        assert_eq!(ev.payload["verdict"], "approve");
        assert_eq!(ev.payload["text"], "looks right");
        assert_eq!(get(&c, &task.id).unwrap().status, TaskStatus::Backlog);
        assert!(review(&mut c, "T-9999", "approve", "x", Source::Cli, None, ts).is_err());
    }

    #[test]
    fn done_is_refused_with_no_verdict_at_all() {
        let ts = at("2026-09-16T12:01:00Z");
        let (mut c, id) = reviewable(ts);
        let err = transition(
            &mut c,
            &id,
            TaskStatus::Done,
            Source::Cli,
            Some("s-impl"),
            None,
            false,
            ts,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no independent review"), "{err}");
        assert!(err.to_string().contains("ratchet task review"), "{err}");
        assert_eq!(get(&c, &id).unwrap().status, TaskStatus::InProgress);
    }

    #[test]
    fn done_is_refused_when_the_approve_came_from_the_holding_session() {
        let ts = at("2026-09-16T12:01:00Z");
        let (mut c, id) = reviewable(ts);
        review(
            &mut c,
            &id,
            "approve",
            "self-reviewed",
            Source::Cli,
            Some("s-impl"),
            ts,
        )
        .unwrap();
        let err = transition(
            &mut c,
            &id,
            TaskStatus::Done,
            Source::Cli,
            Some("s-impl"),
            None,
            false,
            ts,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no independent review"), "{err}");
        // The disqualified verdict came from the holder itself, so naming the holder and naming
        // the verdict's own session say the same thing here — but the message must still name a
        // session, not the placeholder fallback for "no verdict at all".
        assert!(err.to_string().contains("s-impl"), "{err}");
    }

    #[test]
    fn done_is_refused_when_the_approve_came_from_a_session_that_checked_off_an_item() {
        // s-check never claimed or holds the task, but it recorded a checklist.done on it
        // earlier (a transferred task's history), which is close enough to the work to disqualify
        // it as an independent reviewer.
        let ts = at("2026-09-16T12:01:00Z");
        let (mut c, id) = reviewable(ts);
        sessions::upsert_start(
            &mut c,
            StartInput {
                session_id: "s-check",
                repo: "demo",
                repo_root: "root",
                cwd: "root",
                worktree: None,
                branch: None,
                mode: SessionMode::Interactive,
                launched_by: LaunchedBy::User,
            },
            ts,
        )
        .unwrap();
        check(&mut c, &id, 1, Source::Cli, Some("s-check"), ts).unwrap();
        review(
            &mut c,
            &id,
            "approve",
            "fine",
            Source::Cli,
            Some("s-check"),
            ts,
        )
        .unwrap();
        let err = transition(
            &mut c,
            &id,
            TaskStatus::Done,
            Source::Cli,
            Some("s-impl"),
            None,
            false,
            ts,
        )
        .unwrap_err();
        let err = err.to_string();
        assert!(err.contains("no independent review"), "{err}");
        // The bug this pins: naming the *holder* (s-impl) here would be wrong — s-impl never
        // reviewed anything. The session that actually disqualified the verdict is s-check,
        // and the message must say so, not fall back to the task's current holder.
        assert!(err.contains("s-check"), "{err}");
        assert!(!err.contains("s-impl"), "{err}");
    }

    #[test]
    fn done_is_allowed_after_an_approve_from_another_session() {
        let ts = at("2026-09-16T12:01:00Z");
        let (mut c, id) = reviewable(ts);
        review(
            &mut c,
            &id,
            "approve",
            "clean diff, gate green",
            Source::Cli,
            Some("s-reviewer"),
            ts,
        )
        .unwrap();
        let done = transition(
            &mut c,
            &id,
            TaskStatus::Done,
            Source::Cli,
            Some("s-impl"),
            None,
            false,
            ts,
        )
        .unwrap();
        assert_eq!(done.status, TaskStatus::Done);
    }

    #[test]
    fn done_is_refused_when_a_changes_verdict_is_newer_than_the_approve() {
        let ts = at("2026-09-16T12:01:00Z");
        let (mut c, id) = reviewable(ts);
        review(
            &mut c,
            &id,
            "approve",
            "first pass ok",
            Source::Cli,
            Some("s-reviewer"),
            ts,
        )
        .unwrap();
        let later = at("2026-09-16T12:02:00Z");
        review(
            &mut c,
            &id,
            "changes",
            "actually, fix the edge case",
            Source::Cli,
            Some("s-reviewer"),
            later,
        )
        .unwrap();
        let err = transition(
            &mut c,
            &id,
            TaskStatus::Done,
            Source::Cli,
            Some("s-impl"),
            None,
            false,
            later,
        )
        .unwrap_err();
        let err = err.to_string();
        assert!(err.contains("\"changes\""), "{err}");
        // Spells out the literal next command, like the other two refusal branches, not just the
        // bare word "approve".
        assert!(err.contains("ratchet task review"), "{err}");
        assert!(err.contains("approve"), "{err}");
    }

    #[test]
    fn unreviewed_bypasses_the_gate_and_leaves_a_visible_note() {
        let ts = at("2026-09-16T12:01:00Z");
        let (mut c, id) = reviewable(ts);
        let done = transition(
            &mut c,
            &id,
            TaskStatus::Done,
            Source::Cli,
            Some("s-impl"),
            None,
            true,
            ts,
        )
        .unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        let notes = events::for_task(&c, &id, 20)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "note")
            .map(|e| e.payload["text"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>();
        assert!(
            notes.iter().any(|n| n == "done without independent review"),
            "{notes:?}"
        );
    }
}
