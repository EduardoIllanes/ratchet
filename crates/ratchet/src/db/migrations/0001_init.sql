-- 0001: the whole v1 schema. Tasks are mutable; events are append-only.
-- The runner owns the transaction: this script declares none of its own.
--
-- `repo` is the display name from the marker (or the directory name); `repo_root` is the
-- absolute, lower-cased main root of the checkout, and it is what every lookup scopes by --
-- ratchet has no central registry of repos, so two checkouts may share a name.

CREATE TABLE IF NOT EXISTS task_seq (
    id INTEGER PRIMARY KEY AUTOINCREMENT
);

CREATE TABLE IF NOT EXISTS tasks (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    body        TEXT NOT NULL DEFAULT '',
    repo        TEXT NOT NULL,
    repo_root   TEXT NOT NULL,
    status      TEXT NOT NULL,
    priority    INTEGER NOT NULL DEFAULT 3,
    parent_id   TEXT REFERENCES tasks(id),
    tags        TEXT NOT NULL DEFAULT '[]',
    -- session id, deliberately without a foreign key: a task can be claimed by a session the
    -- registry has not seen yet; `claim` validates it instead.
    claimed_by  TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    archived_at TEXT
);
CREATE INDEX IF NOT EXISTS ix_tasks_repo_status ON tasks(repo_root, status);
CREATE INDEX IF NOT EXISTS ix_tasks_claimed_by ON tasks(claimed_by);

CREATE TABLE IF NOT EXISTS checklist_items (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id         TEXT NOT NULL REFERENCES tasks(id),
    position        INTEGER NOT NULL,
    text            TEXT NOT NULL,
    done            INTEGER NOT NULL DEFAULT 0,
    done_by_session TEXT,
    done_at         TEXT,
    UNIQUE (task_id, position)
);

CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY,
    repo        TEXT NOT NULL,
    repo_root   TEXT NOT NULL,
    cwd         TEXT NOT NULL,
    worktree    TEXT,
    branch      TEXT,
    mode        TEXT NOT NULL,
    launched_by TEXT NOT NULL,
    started_at  TEXT NOT NULL,
    last_seen   TEXT NOT NULL,
    ended_at    TEXT
);
CREATE INDEX IF NOT EXISTS ix_sessions_repo ON sessions(repo_root);
CREATE INDEX IF NOT EXISTS ix_sessions_last_seen ON sessions(last_seen);

CREATE TABLE IF NOT EXISTS events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    ts         TEXT NOT NULL,
    session_id TEXT,
    task_id    TEXT,
    kind       TEXT NOT NULL,
    payload    TEXT NOT NULL DEFAULT '{}',
    source     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_events_task ON events(task_id, id);
CREATE INDEX IF NOT EXISTS ix_events_session ON events(session_id, id);
