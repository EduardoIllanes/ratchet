-- 0002 (T-0016): a board write can be attributed to a subagent, not only its bare session.
--
-- `events.agent_id` names the subagent that made the write, alongside the existing `session_id`
-- column; NULL means the bare session, exactly as every event before this migration. Kept as its
-- own nullable column, not folded into `session_id` as a composed string, so every query already
-- written against `session_id` (usage's attribute.rs, the briefing, the done gate) keeps working
-- unmodified. `agent_type` has no column of its own: it rides in the JSON `payload` of an
-- attributed event, which is the only place anything reads it back for display.
ALTER TABLE events ADD COLUMN agent_id TEXT;
CREATE INDEX IF NOT EXISTS ix_events_agent ON events(agent_id);

-- One open row per in-flight `Bash`/`PowerShell` tool call, from its pre-tool hook to its
-- post-tool hook (or a sweep). A board write resolves the identity of the subagent that ran it,
-- if any, by matching this session's own open rows against its own task id and subcommand word.
CREATE TABLE IF NOT EXISTS pending_calls (
    tool_use_id TEXT PRIMARY KEY,
    session_id  TEXT NOT NULL,
    agent_id    TEXT,
    agent_type  TEXT,
    command     TEXT NOT NULL,
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_pending_calls_session ON pending_calls(session_id);
