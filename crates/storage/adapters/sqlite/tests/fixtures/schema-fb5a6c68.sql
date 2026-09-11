-- Frozen verbatim DDL from fb5a6c68: schema.rs and alerts/schema.rs.
-- Keep independent of current schema constants so migration tests cover real upgrades.

CREATE TABLE IF NOT EXISTS process_id_sequence (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    next_process_id INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS processes (
    process_id INTEGER PRIMARY KEY,
    host_pid INTEGER,
    host_task_id INTEGER,
    host_start_ticks INTEGER,
    host_start_boottime_ns INTEGER,
    resolution_state TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS process_namespace_aliases (
    process_id INTEGER NOT NULL,
    pid_namespace TEXT NOT NULL,
    namespace_pid INTEGER NOT NULL,
    namespace_start_ticks INTEGER NOT NULL,
    PRIMARY KEY (process_id, pid_namespace, namespace_pid, namespace_start_ticks),
    UNIQUE (pid_namespace, namespace_pid, namespace_start_ticks)
);

INSERT OR IGNORE INTO process_id_sequence (singleton, next_process_id) VALUES (1, 1);

CREATE TABLE IF NOT EXISTS traces (
    trace_id INTEGER PRIMARY KEY,
    otel_trace_id BLOB NOT NULL UNIQUE CHECK (length(otel_trace_id) = 16),
    alert_token BLOB NOT NULL,
    root_process_id INTEGER NOT NULL,
    root_container_id TEXT,
    root_working_directory TEXT,
    display_name TEXT NOT NULL,
    profile_name TEXT NOT NULL,
    tags TEXT NOT NULL,
    lifecycle_state TEXT NOT NULL,
    health TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    completed_at INTEGER,
    exited_at INTEGER,
    failed_at INTEGER
);

CREATE TRIGGER IF NOT EXISTS reject_otel_trace_id_rotation
BEFORE UPDATE OF otel_trace_id ON traces
WHEN OLD.otel_trace_id != NEW.otel_trace_id
BEGIN
    SELECT RAISE(ABORT, 'OTLP trace identity cannot change');
END;

CREATE TABLE IF NOT EXISTS memberships (
    trace_id INTEGER NOT NULL,
    process_id INTEGER NOT NULL,
    inherited_from_process_id INTEGER,
    observed_at INTEGER,
    capture_enabled INTEGER NOT NULL,
    propagation_enabled INTEGER NOT NULL,
    membership_state TEXT NOT NULL,
    exit_code INTEGER,
    exit_observed_at INTEGER,
    exit_observation_source TEXT,
    PRIMARY KEY (trace_id, process_id)
);

CREATE TABLE IF NOT EXISTS events (
    event_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    observed_at INTEGER NOT NULL,
    process_id INTEGER NOT NULL,
    collector TEXT NOT NULL,
    kind TEXT NOT NULL,
    bootstrap_observed INTEGER NOT NULL,
    metadata_partial INTEGER NOT NULL,
    policy_modified INTEGER NOT NULL,
    payload_variant TEXT NOT NULL,
    payload BLOB NOT NULL,
    payload_code INTEGER NOT NULL,
    payload_blocks TEXT NOT NULL,
    policy_verdict TEXT NOT NULL,
    policy_note TEXT,
    policy_redactions TEXT NOT NULL,
    policy_truncations TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS event_payload_blocks (
    block_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    kind INTEGER NOT NULL,
    encoded_bytes BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS payload_segments (
    segment_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    observed_at INTEGER NOT NULL,
    process_id INTEGER NOT NULL,
    source_boundary TEXT NOT NULL,
    content_state TEXT NOT NULL,
    direction TEXT NOT NULL,
    stream_key TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    original_size INTEGER NOT NULL,
    captured_size INTEGER NOT NULL,
    operation_id INTEGER NOT NULL DEFAULT 0,
    operation_offset INTEGER NOT NULL DEFAULT 0,
    operation_original_size INTEGER NOT NULL DEFAULT 0,
    operation_captured_size INTEGER NOT NULL DEFAULT 0,
    operation_completion_state TEXT NOT NULL DEFAULT 'unknown',
    truncation_state TEXT NOT NULL,
    redaction_state TEXT NOT NULL,
    library TEXT NOT NULL,
    symbol TEXT NOT NULL,
    protocol_hint TEXT,
    bytes BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS semantic_action_ids (
    action_key INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    action_id TEXT NOT NULL UNIQUE,
    action_id_hash BLOB NOT NULL,
    UNIQUE (trace_id, action_id_hash, action_id)
);

CREATE TABLE IF NOT EXISTS agent_identities (
    trace_id INTEGER NOT NULL,
    process_id INTEGER NOT NULL,
    identity_action_key INTEGER NOT NULL,
    PRIMARY KEY (trace_id, process_id)
);

CREATE TABLE IF NOT EXISTS semantic_actions (
    action_key INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    kind_code INTEGER NOT NULL,
    title TEXT NOT NULL,
    start_time INTEGER NOT NULL,
    end_time INTEGER,
    process_id INTEGER NOT NULL,
    status_code INTEGER NOT NULL,
    completeness_code INTEGER NOT NULL,
    action_valid_code INTEGER NOT NULL DEFAULT 1,
    process_parent_conflict INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS semantic_action_evidence (
    action_key INTEGER NOT NULL,
    evidence_order INTEGER NOT NULL,
    kind_code INTEGER NOT NULL,
    evidence_id INTEGER NOT NULL,
    role TEXT NOT NULL,
    PRIMARY KEY (action_key, evidence_order)
);

CREATE TABLE IF NOT EXISTS semantic_action_links (
    trace_id INTEGER NOT NULL,
    parent_action_key INTEGER NOT NULL,
    child_action_key INTEGER NOT NULL,
    role_code INTEGER NOT NULL,
    confidence_code INTEGER NOT NULL,
    valid INTEGER NOT NULL DEFAULT 1,
    link_valid_code INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (trace_id, parent_action_key, child_action_key, role_code)
);

CREATE TABLE IF NOT EXISTS semantic_action_link_evidence (
    trace_id INTEGER NOT NULL,
    parent_action_key INTEGER NOT NULL,
    child_action_key INTEGER NOT NULL,
    role_code INTEGER NOT NULL,
    evidence_order INTEGER NOT NULL,
    kind_code INTEGER NOT NULL,
    evidence_id INTEGER NOT NULL,
    evidence_role TEXT NOT NULL,
    PRIMARY KEY (trace_id, parent_action_key, child_action_key, role_code, evidence_order)
);

CREATE TABLE IF NOT EXISTS semantic_action_cold_fields (
    owner_key INTEGER NOT NULL,
    field_code INTEGER NOT NULL,
    encoding_code INTEGER NOT NULL,
    uncompressed_bytes INTEGER NOT NULL,
    payload BLOB NOT NULL,
    PRIMARY KEY (owner_key, field_code)
);

CREATE TABLE IF NOT EXISTS semantic_action_link_cold_fields (
    trace_id INTEGER NOT NULL,
    parent_action_key INTEGER NOT NULL,
    child_action_key INTEGER NOT NULL,
    role_code INTEGER NOT NULL,
    field_code INTEGER NOT NULL,
    encoding_code INTEGER NOT NULL,
    uncompressed_bytes INTEGER NOT NULL,
    payload BLOB NOT NULL,
    PRIMARY KEY (trace_id, parent_action_key, child_action_key, role_code, field_code)
);

CREATE TABLE IF NOT EXISTS file_observation_paths (
    trace_id INTEGER NOT NULL,
    action_key INTEGER NOT NULL,
    path_order INTEGER NOT NULL,
    path TEXT NOT NULL,
    PRIMARY KEY (trace_id, action_key, path)
);

CREATE TABLE IF NOT EXISTS file_paths (
    path_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    path_hash TEXT NOT NULL,
    path_text TEXT NOT NULL,
    UNIQUE (trace_id, path_hash, path_text)
);

CREATE TABLE IF NOT EXISTS file_path_sets (
    trace_id INTEGER NOT NULL,
    path_set_id TEXT NOT NULL,
    path_set_hash TEXT NOT NULL,
    state TEXT NOT NULL,
    unique_path_count INTEGER NOT NULL,
    stored_path_count INTEGER NOT NULL,
    chunking_scheme TEXT NOT NULL,
    PRIMARY KEY (trace_id, path_set_id)
);

CREATE TABLE IF NOT EXISTS file_path_set_action_refs (
    trace_id INTEGER NOT NULL,
    action_key INTEGER NOT NULL,
    path_set_id TEXT NOT NULL,
    PRIMARY KEY (trace_id, action_key)
);

CREATE TABLE IF NOT EXISTS file_path_set_chunks (
    trace_id INTEGER NOT NULL,
    chunk_id TEXT NOT NULL,
    chunk_hash TEXT NOT NULL,
    item_count INTEGER NOT NULL,
    encoded_sorted_path_ids TEXT NOT NULL,
    chunking_scheme TEXT NOT NULL,
    PRIMARY KEY (trace_id, chunk_id),
    UNIQUE (trace_id, chunking_scheme, chunk_hash, encoded_sorted_path_ids)
);

CREATE TABLE IF NOT EXISTS file_path_set_chunk_refs (
    trace_id INTEGER NOT NULL,
    path_set_id TEXT NOT NULL,
    chunk_order INTEGER NOT NULL,
    chunk_id TEXT NOT NULL,
    PRIMARY KEY (trace_id, path_set_id, chunk_order)
);

CREATE TABLE IF NOT EXISTS llm_request_manifests (
    manifest_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    action_key INTEGER NOT NULL,
    format_version INTEGER NOT NULL,
    canonical_body_hash BLOB NOT NULL,
    canonical_body_bytes INTEGER NOT NULL,
    skeleton_json TEXT NOT NULL,
    UNIQUE (trace_id, action_key)
);

CREATE TABLE IF NOT EXISTS llm_request_blocks (
    block_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    block_hash BLOB NOT NULL,
    uncompressed_bytes INTEGER NOT NULL,
    encoded_bytes BLOB NOT NULL,
    UNIQUE (trace_id, block_hash)
);

CREATE TABLE IF NOT EXISTS llm_request_block_refs (
    manifest_id INTEGER NOT NULL,
    ordinal INTEGER NOT NULL,
    block_id INTEGER NOT NULL,
    PRIMARY KEY (manifest_id, ordinal)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS llm_request_lineage (
    action_key INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    trajectory_root_action_key INTEGER NOT NULL,
    parent_action_key INTEGER,
    forked_from_action_key INTEGER,
    trajectory_position INTEGER NOT NULL,
    transition_code INTEGER NOT NULL,
    start_reason_code INTEGER NOT NULL,
    inference_version INTEGER NOT NULL,
    UNIQUE (trace_id, trajectory_root_action_key, trajectory_position),
    UNIQUE (parent_action_key)
);

CREATE TABLE IF NOT EXISTS mcp_jsonrpc_messages (
    message_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    format_version INTEGER NOT NULL,
    canonical_json_hash BLOB NOT NULL,
    canonical_json_bytes INTEGER NOT NULL,
    canonical_json BLOB NOT NULL,
    UNIQUE (trace_id, format_version, canonical_json_hash)
);

CREATE TABLE IF NOT EXISTS mcp_jsonrpc_action_refs (
    trace_id INTEGER NOT NULL,
    action_key INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    PRIMARY KEY (trace_id, action_key)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS tls_flow_diagnostics (
    id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    stream_key TEXT NOT NULL,
    direction INTEGER NOT NULL,
    reason_code INTEGER NOT NULL,
    observed_size INTEGER NOT NULL,
    emitted_size INTEGER NOT NULL,
    emitted_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS llm_pipeline_diagnostics (
    id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    process_id INTEGER NOT NULL,
    stream_key TEXT,
    stage INTEGER NOT NULL,
    code INTEGER NOT NULL,
    severity INTEGER NOT NULL,
    observed_at INTEGER NOT NULL,
    discarded_bytes INTEGER,
    discarded_entries INTEGER
);

CREATE INDEX IF NOT EXISTS idx_llm_pipeline_diagnostics_trace_time
ON llm_pipeline_diagnostics (trace_id, observed_at, id);

CREATE TABLE IF NOT EXISTS diagnostics (
    diagnostic_id INTEGER PRIMARY KEY,
    trace_id INTEGER,
    process_id INTEGER,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    emitted_at INTEGER NOT NULL,
    message TEXT NOT NULL,
    metadata TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tombstones (
    trace_id INTEGER PRIMARY KEY,
    lifecycle_state TEXT NOT NULL,
    health TEXT NOT NULL,
    cleaned_at INTEGER NOT NULL,
    cleanup_reason TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memberships_trace_parent ON memberships (
    trace_id,
    inherited_from_process_id
);

CREATE INDEX IF NOT EXISTS idx_semantic_actions_trace_process_kind ON semantic_actions (
    trace_id,
    process_id,
    kind_code
);

CREATE INDEX IF NOT EXISTS idx_processes_host_pid ON processes (host_pid);
CREATE INDEX IF NOT EXISTS idx_process_alias_namespace_pid
    ON process_namespace_aliases (pid_namespace, namespace_pid);

CREATE INDEX IF NOT EXISTS idx_semantic_action_links_trace_child_role ON semantic_action_links (
    trace_id,
    child_action_key,
    role_code
);

CREATE INDEX IF NOT EXISTS idx_file_observation_paths_action_order ON file_observation_paths (
    trace_id,
    action_key,
    path_order
);

CREATE INDEX IF NOT EXISTS idx_file_paths_trace_text ON file_paths (
    trace_id,
    path_text
);

CREATE INDEX IF NOT EXISTS idx_file_path_set_refs_path_set ON file_path_set_chunk_refs (
    trace_id,
    path_set_id,
    chunk_order
);

CREATE INDEX IF NOT EXISTS idx_file_path_set_action_refs_path_set ON file_path_set_action_refs (
    trace_id,
    path_set_id
);

CREATE INDEX IF NOT EXISTS idx_mcp_jsonrpc_action_refs_message ON mcp_jsonrpc_action_refs (
    trace_id,
    message_id
);

CREATE INDEX IF NOT EXISTS idx_llm_request_lineage_parent ON llm_request_lineage (
    trace_id,
    parent_action_key
);

CREATE INDEX IF NOT EXISTS idx_llm_request_lineage_fork ON llm_request_lineage (
    trace_id,
    forked_from_action_key
);


CREATE TABLE IF NOT EXISTS alert_definitions (
    alert_definition_id INTEGER PRIMARY KEY,
    producer_plugin_id TEXT NOT NULL,
    definition_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    severity_code INTEGER NOT NULL,
    payload_schema_id TEXT NOT NULL,
    UNIQUE (producer_plugin_id, definition_key)
);

CREATE TABLE IF NOT EXISTS trace_alert_authorizations (
    trace_id INTEGER PRIMARY KEY,
    alert_token BLOB NOT NULL CHECK (length(alert_token) = 32)
);

CREATE TRIGGER IF NOT EXISTS reject_trace_alert_token_rotation
BEFORE INSERT ON traces
WHEN EXISTS (
    SELECT 1 FROM trace_alert_authorizations
    WHERE trace_id = NEW.trace_id AND alert_token != NEW.alert_token
)
BEGIN
    SELECT RAISE(ABORT, 'trace alert token cannot change');
END;

CREATE TRIGGER IF NOT EXISTS persist_trace_alert_authorization
AFTER INSERT ON traces
BEGIN
    INSERT OR IGNORE INTO trace_alert_authorizations (trace_id, alert_token)
    VALUES (NEW.trace_id, NEW.alert_token);
END;

CREATE TABLE IF NOT EXISTS alerts (
    alert_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    alert_definition_id INTEGER NOT NULL REFERENCES alert_definitions(alert_definition_id),
    created_at INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS alert_deduplication_keys (
    trace_id INTEGER NOT NULL,
    alert_definition_id INTEGER NOT NULL REFERENCES alert_definitions(alert_definition_id),
    deduplication_key TEXT NOT NULL,
    PRIMARY KEY (trace_id, alert_definition_id, deduplication_key)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_alerts_latest
ON alerts(created_at DESC, alert_id DESC);

CREATE INDEX IF NOT EXISTS idx_alerts_trace_latest
ON alerts(trace_id, created_at DESC, alert_id DESC);

PRAGMA user_version = 26;
