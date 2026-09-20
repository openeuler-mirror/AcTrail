CREATE TABLE IF NOT EXISTS semantic_action_state (
    action_key INTEGER PRIMARY KEY,
    end_time INTEGER,
    status_code INTEGER NOT NULL,
    completeness_code INTEGER NOT NULL,
    finalization_reason INTEGER,
    failure_title TEXT,
    failure_body_format TEXT,
    failure_http_status INTEGER,
    failure_http_reason TEXT,
    command_invocation_kind INTEGER,
    command_tool_name TEXT,
    tool_result_binding INTEGER
);
CREATE TABLE IF NOT EXISTS semantic_action_evidence (
    evidence_key INTEGER PRIMARY KEY,
    action_key INTEGER NOT NULL,
    kind_code INTEGER NOT NULL,
    evidence_id INTEGER NOT NULL,
    role TEXT NOT NULL,
    UNIQUE (action_key, kind_code, evidence_id, role)
);
CREATE TABLE IF NOT EXISTS semantic_action_state_revisions (
    trace_id INTEGER PRIMARY KEY,
    revision INTEGER NOT NULL
);
CREATE TRIGGER IF NOT EXISTS verify_semantic_action_identity
BEFORE INSERT ON semantic_actions
WHEN EXISTS (
    SELECT 1 FROM semantic_actions old WHERE old.action_key = NEW.action_key
      AND (old.trace_id != NEW.trace_id OR old.kind_code != NEW.kind_code
           OR old.process_id != NEW.process_id)
)
BEGIN
    SELECT RAISE(ABORT, 'semantic action identity collision');
END;
