//! Types of the state layer and the exact strings they take in the database. Parsing is lenient
//! on purpose: a row written by a newer build must never make a hook fail (D-p3).

use chrono::{DateTime, Utc};
use rusqlite::Row;
use serde::Serialize;
use serde_json::Value;

use crate::clock;

// A plain `#[allow(dead_code)]` on a `db_enum!(...)` invocation is ignored by rustc (the
// built-in attribute does not propagate through a macro_rules! item invocation), so the
// allow lives inside the expansion itself, applied to the enum and its inherent impl —
// still item-level, never module-level. Consumed once services/faces wire each enum in:
// SessionMode/LaunchedBy by services::sessions (Task 7) and hooks::dispatch (Task 10);
// SessionState by services::sessions::state (Task 7) and cli::session_cmd (Task 11);
// Source by services::events::emit (Task 6); TaskStatus by services::tasks (Task 8);
// EventKind by services::events (Task 6) and hooks::dispatch (Task 10).
macro_rules! db_enum {
    ($name:ident, $default:ident, { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        #[serde(into = "String")]
        #[allow(dead_code)]
        pub enum $name {
            $($variant),+
        }

        #[allow(dead_code)]
        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self {
                    $($name::$variant => $text),+
                }
            }

            /// Lenient: anything unexpected becomes the default variant.
            pub fn from_db(s: &str) -> Self {
                match s {
                    $($text => $name::$variant,)+
                    _ => $name::$default,
                }
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> String {
                v.as_str().to_string()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

db_enum!(SessionMode, Interactive, { Interactive => "interactive", Headless => "headless" });
db_enum!(LaunchedBy, User, { User => "user", Platform => "platform" });
db_enum!(SessionState, Live, {
    Live => "live", Idle => "idle", Orphaned => "orphaned", Ended => "ended",
});
db_enum!(Source, Cli, { Hook => "hook", Cli => "cli" });
db_enum!(TaskStatus, Backlog, {
    Backlog => "backlog",
    Ready => "ready",
    InProgress => "in_progress",
    Blocked => "blocked",
    Review => "review",
    Done => "done",
});
db_enum!(EventKind, Note, {
    SessionStart => "session.start",
    SessionPrompt => "session.prompt",
    SessionStop => "session.stop",
    SessionEnd => "session.end",
    TaskCreated => "task.created",
    TaskClaimed => "task.claimed",
    TaskStatus => "task.status",
    TaskArchived => "task.archived",
    TaskUnarchived => "task.unarchived",
    ChecklistDone => "checklist.done",
    ChecklistUndone => "checklist.undone",
    Handoff => "handoff",
    Note => "note",
    GuardrailMainTreeWrite => "guardrail.main_tree_write",
});

// Consumed by services::sessions (Task 7) and cli::session_cmd (Task 11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Session {
    pub id: String,
    pub repo: String,
    pub repo_root: String,
    pub cwd: String,
    pub worktree: Option<String>,
    pub branch: Option<String>,
    pub mode: SessionMode,
    pub launched_by: LaunchedBy,
    pub started_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

impl Session {
    // Consumed by services::sessions (Task 7): get/list/resolve query rows through this.
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Session> {
        let started: String = row.get("started_at")?;
        let seen: String = row.get("last_seen")?;
        let ended: Option<String> = row.get("ended_at")?;
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch");
        Ok(Session {
            id: row.get("id")?,
            repo: row.get("repo")?,
            repo_root: row.get("repo_root")?,
            cwd: row.get("cwd")?,
            worktree: row.get("worktree")?,
            branch: row.get("branch")?,
            mode: SessionMode::from_db(&row.get::<_, String>("mode")?),
            launched_by: LaunchedBy::from_db(&row.get::<_, String>("launched_by")?),
            started_at: clock::parse(&started).unwrap_or(epoch),
            last_seen: clock::parse(&seen).unwrap_or(epoch),
            ended_at: ended.as_deref().and_then(clock::parse),
        })
    }
}

// Consumed by services::events (Task 6).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    pub id: i64,
    pub ts: DateTime<Utc>,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub kind: String,
    pub payload: Value,
    pub source: Source,
}

impl Event {
    // Consumed by services::events::for_session (Task 6): rows are read back through this.
    #[allow(dead_code)]
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Event> {
        let ts: String = row.get("ts")?;
        let payload: String = row.get("payload")?;
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch");
        Ok(Event {
            id: row.get("id")?,
            ts: clock::parse(&ts).unwrap_or(epoch),
            session_id: row.get("session_id")?,
            task_id: row.get("task_id")?,
            kind: row.get("kind")?,
            payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
            source: Source::from_db(&row.get::<_, String>("source")?),
        })
    }
}

/// Where a task may go from each status, in the order the message lists them. Ported from the
/// `tasks` spec: any status may go back to `backlog`, and `in_progress`, `blocked` and `review`
/// may go back to `ready` — that is what letting go of a task means.
// Consumed by services::tasks (Tasks 4 and 5).
#[allow(dead_code)]
pub fn allowed_transitions(from: TaskStatus) -> &'static [TaskStatus] {
    use TaskStatus::*;
    match from {
        Backlog => &[Ready],
        Ready => &[InProgress, Backlog],
        InProgress => &[Blocked, Review, Done, Ready, Backlog],
        Blocked => &[InProgress, Ready, Backlog],
        Review => &[InProgress, Done, Ready, Backlog],
        Done => &[Backlog],
    }
}

/// A unit of work. `repo` is the display name of the repo it was created in; `repo_root` is the
/// key every lookup scopes by (G1-R1). Progress is never a field: it is computed from the
/// checklist on every read.
// Consumed by services::tasks (Tasks 4 and 5).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub body: String,
    pub repo: String,
    pub repo_root: String,
    pub status: TaskStatus,
    pub priority: i64,
    pub parent_id: Option<String>,
    pub tags: Vec<String>,
    pub claimed_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

impl Task {
    // Consumed by services::tasks (Task 4): rows are read back through this.
    #[allow(dead_code)]
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Task> {
        let created: String = row.get("created_at")?;
        let updated: String = row.get("updated_at")?;
        let archived: Option<String> = row.get("archived_at")?;
        let tags: String = row.get("tags")?;
        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch");
        Ok(Task {
            id: row.get("id")?,
            title: row.get("title")?,
            body: row.get("body")?,
            repo: row.get("repo")?,
            repo_root: row.get("repo_root")?,
            status: TaskStatus::from_db(&row.get::<_, String>("status")?),
            priority: row.get("priority")?,
            parent_id: row.get("parent_id")?,
            // Lenient like every other read: a row a newer build wrote must not fail a hook.
            tags: serde_json::from_str(&tags).unwrap_or_default(),
            claimed_by: row.get("claimed_by")?,
            created_at: clock::parse(&created).unwrap_or(epoch),
            updated_at: clock::parse(&updated).unwrap_or(epoch),
            archived_at: archived.as_deref().and_then(clock::parse),
        })
    }
}

/// One acceptance criterion. `position` is 1-based and is what the CLI names.
// Consumed by services::tasks (Tasks 4 and 6).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChecklistItem {
    pub id: i64,
    pub task_id: String,
    pub position: i64,
    pub text: String,
    pub done: bool,
    pub done_by_session: Option<String>,
    pub done_at: Option<DateTime<Utc>>,
}

impl ChecklistItem {
    // Consumed by services::tasks (Task 4): rows are read back through this.
    #[allow(dead_code)]
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<ChecklistItem> {
        let done_at: Option<String> = row.get("done_at")?;
        Ok(ChecklistItem {
            id: row.get("id")?,
            task_id: row.get("task_id")?,
            position: row.get("position")?,
            text: row.get("text")?,
            done: row.get::<_, i64>("done")? != 0,
            done_by_session: row.get("done_by_session")?,
            done_at: done_at.as_deref().and_then(clock::parse),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_round_trip_through_their_database_form() {
        assert_eq!(SessionMode::from_db("headless"), SessionMode::Headless);
        assert_eq!(SessionMode::Headless.as_str(), "headless");
        assert_eq!(LaunchedBy::from_db("platform"), LaunchedBy::Platform);
        assert_eq!(TaskStatus::from_db("in_progress"), TaskStatus::InProgress);
        assert_eq!(TaskStatus::InProgress.as_str(), "in_progress");
        assert_eq!(EventKind::SessionStart.as_str(), "session.start");
        assert_eq!(Source::Hook.as_str(), "hook");
        assert_eq!(SessionState::Orphaned.as_str(), "orphaned");
    }

    #[test]
    fn unknown_strings_fall_back_instead_of_failing() {
        assert_eq!(SessionMode::from_db("nonsense"), SessionMode::Interactive);
        assert_eq!(LaunchedBy::from_db(""), LaunchedBy::User);
        assert_eq!(TaskStatus::from_db("weird"), TaskStatus::Backlog);
    }

    #[test]
    fn a_session_serializes_with_its_string_forms() {
        let s = Session {
            id: "s-1".into(),
            repo: "demo".into(),
            repo_root: "c:\\repos\\demo".into(),
            cwd: "c:\\repos\\demo".into(),
            worktree: None,
            branch: Some("main".into()),
            mode: SessionMode::Interactive,
            launched_by: LaunchedBy::User,
            started_at: crate::clock::parse("2026-09-16T12:00:00Z").unwrap(),
            last_seen: crate::clock::parse("2026-09-16T12:00:00Z").unwrap(),
            ended_at: None,
        };
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("\"mode\":\"interactive\""), "{text}");
        assert!(text.contains("\"launched_by\":\"user\""), "{text}");
        assert!(text.contains("2026-09-16T12:00:00Z"), "{text}");
    }

    #[test]
    fn the_board_event_kinds_have_their_database_strings() {
        assert_eq!(EventKind::TaskCreated.as_str(), "task.created");
        assert_eq!(EventKind::TaskClaimed.as_str(), "task.claimed");
        assert_eq!(EventKind::TaskArchived.as_str(), "task.archived");
        assert_eq!(EventKind::TaskUnarchived.as_str(), "task.unarchived");
        assert_eq!(EventKind::ChecklistDone.as_str(), "checklist.done");
        assert_eq!(EventKind::ChecklistUndone.as_str(), "checklist.undone");
        assert_eq!(EventKind::Handoff.as_str(), "handoff");
        assert_eq!(
            EventKind::GuardrailMainTreeWrite.as_str(),
            "guardrail.main_tree_write"
        );
        assert_eq!(
            EventKind::from_db("checklist.done"),
            EventKind::ChecklistDone
        );
        // The kinds group 1 wrote keep their strings.
        assert_eq!(EventKind::SessionStart.as_str(), "session.start");
        assert_eq!(EventKind::TaskStatus.as_str(), "task.status");
    }

    #[test]
    fn the_transition_table_is_the_one_the_spec_lists() {
        use TaskStatus::*;
        assert_eq!(allowed_transitions(Backlog), &[Ready]);
        assert_eq!(allowed_transitions(Ready), &[InProgress, Backlog]);
        assert_eq!(
            allowed_transitions(InProgress),
            &[Blocked, Review, Done, Ready, Backlog]
        );
        assert_eq!(allowed_transitions(Blocked), &[InProgress, Ready, Backlog]);
        assert_eq!(
            allowed_transitions(Review),
            &[InProgress, Done, Ready, Backlog]
        );
        assert_eq!(allowed_transitions(Done), &[Backlog]);
        // Every state can be put back in the queue except the two that are already there.
        for from in [InProgress, Blocked, Review] {
            assert!(allowed_transitions(from).contains(&Ready), "{from:?}");
        }
        assert!(!allowed_transitions(Ready).contains(&Done));
    }

    #[test]
    fn a_task_reads_back_from_its_row_with_its_tags() {
        let conn = crate::db::open_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (id TEXT, title TEXT, body TEXT, repo TEXT, repo_root TEXT, \
             status TEXT, priority INTEGER, parent_id TEXT, tags TEXT, claimed_by TEXT, \
             created_at TEXT, updated_at TEXT, archived_at TEXT);
             INSERT INTO tasks VALUES ('T-0001','a title','body','demo','c:\\repos\\demo',\
             'in_progress',2,NULL,'[\"one\",\"two\"]','s-1','2026-09-16T12:00:00Z',\
             '2026-09-16T12:05:00Z',NULL);",
        )
        .unwrap();
        let task: Task = conn
            .query_row("SELECT * FROM tasks", [], Task::from_row)
            .unwrap();
        assert_eq!(task.id, "T-0001");
        assert_eq!(task.status, TaskStatus::InProgress);
        assert_eq!(task.priority, 2);
        assert_eq!(task.tags, vec!["one".to_string(), "two".to_string()]);
        assert_eq!(task.claimed_by.as_deref(), Some("s-1"));
        assert!(task.archived_at.is_none());
        let text = serde_json::to_string(&task).unwrap();
        assert!(text.contains("\"status\":\"in_progress\""), "{text}");
    }

    #[test]
    fn broken_tags_do_not_fail_a_read() {
        let conn = crate::db::open_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (id TEXT, title TEXT, body TEXT, repo TEXT, repo_root TEXT, \
             status TEXT, priority INTEGER, parent_id TEXT, tags TEXT, claimed_by TEXT, \
             created_at TEXT, updated_at TEXT, archived_at TEXT);
             INSERT INTO tasks VALUES ('T-0002','t','','demo','root','weird',3,NULL,'not json',\
             NULL,'nonsense','2026-09-16T12:00:00Z',NULL);",
        )
        .unwrap();
        let task: Task = conn
            .query_row("SELECT * FROM tasks", [], Task::from_row)
            .unwrap();
        assert!(task.tags.is_empty());
        assert_eq!(task.status, TaskStatus::Backlog); // lenient, like every other enum
        assert_eq!(crate::clock::iso(task.created_at), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn a_checklist_item_reads_back_from_its_row() {
        let conn = crate::db::open_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE checklist_items (id INTEGER, task_id TEXT, position INTEGER, text TEXT, \
             done INTEGER, done_by_session TEXT, done_at TEXT);
             INSERT INTO checklist_items VALUES (7,'T-0001',3,'test it',1,'s-1',\
             '2026-09-16T12:00:00Z');",
        )
        .unwrap();
        let item: ChecklistItem = conn
            .query_row("SELECT * FROM checklist_items", [], |r| {
                ChecklistItem::from_row(r)
            })
            .unwrap();
        assert_eq!(item.id, 7);
        assert_eq!(item.position, 3);
        assert!(item.done);
        assert_eq!(item.done_by_session.as_deref(), Some("s-1"));
        assert!(item.done_at.is_some());
    }
}
