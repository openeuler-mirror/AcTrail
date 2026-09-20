//! Schema boundaries for traces, events, diagnostics, and tombstones.

use rusqlite::Connection;

use crate::semantic_actions::{codebook, storage_meta};

const SQLITE_SCHEMA_VERSION_CURRENT: i32 = storage_meta::CURRENT_SCHEMA_VERSION;
const CREATE_TABLES_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS process_id_sequence (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    next_process_id INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS event_id_high_water (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    last_event_id INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS event_id_claim_words (
    word_id INTEGER PRIMARY KEY,
    claimed_bits INTEGER NOT NULL CHECK (claimed_bits >= 0)
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
INSERT OR IGNORE INTO event_id_high_water (singleton, last_event_id) VALUES (1, 0);

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

CREATE TABLE IF NOT EXISTS trace_resource_scopes (
    trace_id INTEGER PRIMARY KEY,
    nonce TEXT NOT NULL,
    relative_path TEXT NOT NULL UNIQUE,
    accounting_method TEXT NOT NULL,
    lifecycle_state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    final_event_id INTEGER,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
    event_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    observed_at INTEGER NOT NULL,
    process_id INTEGER NOT NULL,
    event_meta INTEGER NOT NULL,
    kind_code INTEGER NOT NULL,
    payload_path_id INTEGER,
    payload_id INTEGER,
    payload_inline BLOB,
    CHECK ((payload_id IS NOT NULL) != (payload_inline IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS event_payload_blocks (
    event_id INTEGER NOT NULL,
    block_order INTEGER NOT NULL,
    kind INTEGER NOT NULL,
    encoded_bytes BLOB NOT NULL,
    PRIMARY KEY (event_id, block_order)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS event_payload_dictionary (
    payload_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    payload BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS event_record_blocks (
    block_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    first_event_id INTEGER NOT NULL UNIQUE,
    max_event_id INTEGER NOT NULL,
    min_observed_at INTEGER NOT NULL,
    max_observed_at INTEGER NOT NULL,
    event_count INTEGER NOT NULL CHECK (event_count > 0),
    kind_counts BLOB NOT NULL,
    codec_version INTEGER NOT NULL,
    uncompressed_bytes INTEGER NOT NULL,
    encoded_bytes BLOB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_event_record_blocks_trace_time
ON event_record_blocks (trace_id, min_observed_at, first_event_id);

CREATE TABLE IF NOT EXISTS event_record_pending (
    event_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    observed_at INTEGER NOT NULL,
    kind_code INTEGER NOT NULL,
    encoded_frame BLOB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_event_record_pending_trace
ON event_record_pending (trace_id, event_id);

CREATE TABLE IF NOT EXISTS event_record_pending_state (
    trace_id INTEGER PRIMARY KEY,
    event_count INTEGER NOT NULL CHECK (event_count > 0),
    framed_bytes INTEGER NOT NULL CHECK (framed_bytes > 0)
);

CREATE TABLE IF NOT EXISTS event_policy_details (
    event_id INTEGER PRIMARY KEY,
    note TEXT,
    redactions TEXT NOT NULL,
    truncations TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS payload_segments (
    segment_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    observed_at INTEGER NOT NULL,
    process_id INTEGER NOT NULL,
    segment_meta INTEGER NOT NULL
        CONSTRAINT payload_segment_meta_valid CHECK (
            typeof(segment_meta) = 'integer'
            AND segment_meta BETWEEN 0 AND 1023
            AND (segment_meta & 12) != 12
            AND (segment_meta & 192) != 192
            AND (segment_meta & 768) != 768
        ),
    stream_key TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    original_size INTEGER NOT NULL,
    captured_size INTEGER NOT NULL,
    operation_id INTEGER NOT NULL DEFAULT 0,
    operation_offset INTEGER NOT NULL DEFAULT 0,
    operation_original_size INTEGER NOT NULL DEFAULT 0,
    operation_captured_size INTEGER NOT NULL DEFAULT 0,
    library TEXT NOT NULL,
    symbol TEXT NOT NULL,
    protocol_hint TEXT,
    storage_omission INTEGER NOT NULL DEFAULT 0 CHECK (storage_omission IN (0, 1)),
    bytes BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS semantic_action_ids (
    action_key INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    action_id TEXT NOT NULL UNIQUE
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
    title TEXT,
    file_path_id INTEGER,
    start_time INTEGER NOT NULL,
    process_id INTEGER NOT NULL,
    action_valid_code INTEGER NOT NULL DEFAULT 1,
    process_parent_conflict INTEGER NOT NULL DEFAULT 0,
    CHECK (title IS NOT NULL OR file_path_id IS NOT NULL)
);

CREATE TABLE IF NOT EXISTS semantic_action_links (
    trace_id INTEGER NOT NULL,
    parent_action_key INTEGER NOT NULL,
    child_action_key INTEGER NOT NULL,
    role_code INTEGER NOT NULL,
    origin_code INTEGER NOT NULL,
    valid INTEGER NOT NULL DEFAULT 1,
    evidence_blob BLOB NOT NULL,
    PRIMARY KEY (trace_id, parent_action_key, child_action_key, role_code)
);

CREATE TABLE IF NOT EXISTS semantic_action_cold_fields (
    owner_key INTEGER PRIMARY KEY,
    encoding_code INTEGER NOT NULL,
    uncompressed_bytes INTEGER NOT NULL,
    payload BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS semantic_action_link_cold_fields (
    trace_id INTEGER NOT NULL,
    parent_action_key INTEGER NOT NULL,
    child_action_key INTEGER NOT NULL,
    role_code INTEGER NOT NULL,
    encoding_code INTEGER NOT NULL,
    uncompressed_bytes INTEGER NOT NULL,
    payload BLOB NOT NULL,
    PRIMARY KEY (trace_id, parent_action_key, child_action_key, role_code)
);

CREATE TABLE IF NOT EXISTS file_observation_paths (
    trace_id INTEGER NOT NULL,
    action_key INTEGER NOT NULL,
    path_order INTEGER NOT NULL,
    path_id INTEGER NOT NULL,
    PRIMARY KEY (trace_id, action_key, path_id)
);

CREATE TABLE IF NOT EXISTS file_paths (
    path_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    path_text TEXT NOT NULL,
    UNIQUE (trace_id, path_text)
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

CREATE INDEX IF NOT EXISTS idx_event_payload_dictionary_trace
ON event_payload_dictionary (trace_id);

CREATE INDEX IF NOT EXISTS idx_semantic_actions_trace_process_kind ON semantic_actions (
    trace_id,
    process_id,
    kind_code
);

CREATE INDEX IF NOT EXISTS idx_trace_resource_scopes_lifecycle
    ON trace_resource_scopes (lifecycle_state, updated_at);
CREATE INDEX IF NOT EXISTS idx_semantic_action_ids_trace ON semantic_action_ids (trace_id);

CREATE INDEX IF NOT EXISTS idx_semantic_action_links_trace_child_role ON semantic_action_links (
    trace_id,
    child_action_key,
    role_code
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_http_response_unique_request_owner
    ON semantic_action_links (trace_id, child_action_key, role_code)
    WHERE role_code = 527;

CREATE INDEX IF NOT EXISTS idx_file_observation_paths_action_order ON file_observation_paths (
    trace_id,
    action_key,
    path_order
);

CREATE INDEX IF NOT EXISTS idx_llm_request_lineage_fork ON llm_request_lineage (
    trace_id,
    forked_from_action_key
);

"#;

pub(crate) const CREATE_EXTERNAL_CGROUP_BINDINGS_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS trace_external_cgroup_bindings (
    trace_id             INTEGER PRIMARY KEY,
    runtime              TEXT NOT NULL CHECK (
        runtime IN ('docker', 'containerd', 'kubernetes-unknown-cri', 'podman', 'crio')
    ),
    container_id         TEXT NOT NULL CHECK (length(container_id) = 64),
    relative_path        TEXT NOT NULL CHECK (length(relative_path) > 0),
    cgroup_device_be     BLOB NOT NULL CHECK (length(cgroup_device_be) = 8),
    cgroup_inode_be      BLOB NOT NULL CHECK (length(cgroup_inode_be) = 8),
    host_boot_id         BLOB NOT NULL CHECK (length(host_boot_id) = 16),
    lifecycle_state      TEXT NOT NULL CHECK (
        lifecycle_state IN ('active', 'stale', 'closed')
    ),
    stale_reason         TEXT CHECK (
        stale_reason IS NULL OR stale_reason IN (
            'host_boot_changed', 'boundary_missing', 'boundary_identity_changed',
            'container_identity_mismatch', 'root_moved',
            'required_counter_unavailable', 'permission_denied', 'repeated_read_failure'
        )
    ),
    consecutive_failures INTEGER NOT NULL DEFAULT 0 CHECK (
        consecutive_failures BETWEEN 0 AND 4294967295
    ),
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    last_good_at         INTEGER,
    closed_at            INTEGER,
    final_event_id       INTEGER,
    CHECK (
        (lifecycle_state = 'active' AND stale_reason IS NULL
         AND closed_at IS NULL AND final_event_id IS NULL)
        OR
        (lifecycle_state = 'stale' AND stale_reason IS NOT NULL
         AND closed_at IS NULL AND final_event_id IS NULL)
        OR
        (lifecycle_state = 'closed'
         AND closed_at IS NOT NULL AND final_event_id IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_trace_external_cgroup_bindings_lifecycle
    ON trace_external_cgroup_bindings (lifecycle_state, updated_at);
"#;

pub fn initialize(connection: &Connection) -> Result<(), rusqlite::Error> {
    let version = user_version(connection)?;
    validate_writable_schema_state(connection, version)?;
    connection.execute_batch(CREATE_TABLES_SQL)?;
    connection.execute_batch(CREATE_EXTERNAL_CGROUP_BINDINGS_SQL)?;
    connection.execute_batch(include_str!("semantic_actions/store/schema.sql"))?;
    connection.execute_batch(crate::alerts::schema::CREATE_SQL)?;
    codebook::for_schema_version(SQLITE_SCHEMA_VERSION_CURRENT)
        .and_then(|codebook| codebook.validate())
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    validate_current_schema(connection)?;
    connection.pragma_update(None, "user_version", SQLITE_SCHEMA_VERSION_CURRENT)?;
    migrate_query_indexes(connection)
}

#[cfg(test)]
mod baseline_upgrade_tests {
    use super::*;

    fn create_dev_baseline_without_external_bindings(
        connection: &Connection,
        with_host_scopes: bool,
    ) {
        connection.execute_batch(CREATE_TABLES_SQL).unwrap();
        connection
            .execute_batch(include_str!("semantic_actions/store/schema.sql"))
            .unwrap();
        connection
            .execute_batch(crate::alerts::schema::CREATE_SQL)
            .unwrap();
        if !with_host_scopes {
            connection
                .execute_batch("DROP TABLE trace_resource_scopes")
                .unwrap();
        }
        connection
            .pragma_update(None, "user_version", SQLITE_SCHEMA_VERSION_CURRENT)
            .unwrap();
    }

    #[test]
    fn archived_database_opens_read_only_without_creating_resource_registry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.sqlite");
        let connection = Connection::open(&path).unwrap();
        create_dev_baseline_without_external_bindings(&connection, false);
        drop(connection);
        let before = std::fs::read(&path).unwrap();
        let storage = crate::SqliteStorage::open_read_only(&path).unwrap();
        assert!(
            storage_core::StorageBackend::list_events(&storage, model_core::ids::TraceId::new(1))
                .unwrap()
                .is_empty()
        );
        drop(storage);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let connection =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(
            user_version(&connection).unwrap(),
            SQLITE_SCHEMA_VERSION_CURRENT
        );
        assert!(!column_exists(&connection, "trace_resource_scopes", "trace_id").unwrap());
        assert!(!column_exists(&connection, "trace_external_cgroup_bindings", "trace_id").unwrap());
    }

    #[test]
    fn dev_and_host_databases_gain_external_bindings() {
        for with_host_scopes in [false, true] {
            let connection = Connection::open_in_memory().unwrap();
            create_dev_baseline_without_external_bindings(&connection, with_host_scopes);
            initialize(&connection).unwrap();
            assert_eq!(
                user_version(&connection).unwrap(),
                SQLITE_SCHEMA_VERSION_CURRENT
            );
            require_schema_object(&connection, "table", "trace_resource_scopes").unwrap();
            require_schema_object(&connection, "table", "trace_external_cgroup_bindings").unwrap();
            validate_read_schema(&connection).unwrap();
            initialize(&connection).unwrap();
        }
    }
}

fn migrate_query_indexes(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_events_trace_id ON events(trace_id);
         CREATE INDEX IF NOT EXISTS idx_payload_segments_trace_id ON payload_segments(trace_id);
         CREATE INDEX IF NOT EXISTS idx_tls_flow_diagnostics_trace_id ON tls_flow_diagnostics(trace_id);
         CREATE INDEX IF NOT EXISTS idx_semantic_actions_trace_start ON semantic_actions(trace_id, start_time);
         CREATE INDEX IF NOT EXISTS idx_semantic_actions_trace_kind_start ON semantic_actions(trace_id, kind_code, start_time, action_key);
         CREATE INDEX IF NOT EXISTS idx_semantic_action_links_trace_valid_role ON semantic_action_links(trace_id, valid, role_code);
         DROP INDEX IF EXISTS idx_memberships_trace_parent;
         DROP INDEX IF EXISTS idx_processes_host_pid;
         DROP INDEX IF EXISTS idx_process_alias_namespace_pid;
         DROP INDEX IF EXISTS idx_file_path_set_refs_path_set;
         DROP INDEX IF EXISTS idx_file_path_set_action_refs_path_set;
         DROP INDEX IF EXISTS idx_mcp_jsonrpc_action_refs_message;
         DROP INDEX IF EXISTS idx_llm_request_lineage_parent;
         DROP INDEX IF EXISTS idx_semantic_action_links_trace_valid_parent;
         DROP INDEX IF EXISTS idx_semantic_action_links_trace_valid_child;",
    )
}

pub fn validate_read_schema(connection: &Connection) -> Result<(), rusqlite::Error> {
    if user_version(connection)? != SQLITE_SCHEMA_VERSION_CURRENT {
        return Err(rusqlite::Error::InvalidQuery);
    }
    codebook::for_schema_version(SQLITE_SCHEMA_VERSION_CURRENT)
        .and_then(|codebook| codebook.validate())
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    // Resource registries are writer-owned lifecycle state. Read-only clients can
    // open databases created by dev before the additive registries were introduced.
    validate_schema(connection, false, false)?;
    Ok(())
}

fn validate_writable_schema_state(
    connection: &Connection,
    version: i32,
) -> Result<(), rusqlite::Error> {
    if version == SQLITE_SCHEMA_VERSION_CURRENT {
        return Ok(());
    }
    if version == 0 && user_table_count(connection)? == 0 {
        return Ok(());
    }
    Err(rusqlite::Error::InvalidQuery)
}

fn validate_current_schema(connection: &Connection) -> Result<(), rusqlite::Error> {
    validate_schema(connection, true, true)
}

fn validate_schema(
    connection: &Connection,
    require_resource_scopes: bool,
    require_external_bindings: bool,
) -> Result<(), rusqlite::Error> {
    crate::alerts::schema::validate(connection)?;
    require_schema_object(connection, "table", "tls_flow_diagnostics")?;
    require_column(connection, "tls_flow_diagnostics", "trace_id")?;
    require_column(connection, "tls_flow_diagnostics", "stream_key")?;
    require_column(connection, "tls_flow_diagnostics", "direction")?;
    require_column(connection, "tls_flow_diagnostics", "reason_code")?;
    require_column(connection, "tls_flow_diagnostics", "observed_size")?;
    require_column(connection, "tls_flow_diagnostics", "emitted_size")?;
    require_column(connection, "tls_flow_diagnostics", "emitted_at")?;
    require_integer_column(connection, "llm_pipeline_diagnostics", "stage")?;
    require_integer_column(connection, "llm_pipeline_diagnostics", "code")?;
    require_integer_column(connection, "llm_pipeline_diagnostics", "severity")?;
    require_column(connection, "processes", "process_id")?;
    require_column(connection, "event_id_high_water", "last_event_id")?;
    require_column(connection, "event_id_claim_words", "word_id")?;
    require_column(connection, "event_id_claim_words", "claimed_bits")?;
    require_column(connection, "process_namespace_aliases", "process_id")?;
    require_column(connection, "traces", "alert_token")?;
    require_column(connection, "traces", "otel_trace_id")?;
    require_schema_object(connection, "trigger", "reject_otel_trace_id_rotation")?;
    require_column(connection, "traces", "root_process_id")?;
    require_column(connection, "traces", "root_working_directory")?;
    require_column(connection, "memberships", "process_id")?;
    if require_resource_scopes {
        require_column(connection, "trace_resource_scopes", "trace_id")?;
        require_column(connection, "trace_resource_scopes", "nonce")?;
        require_column(connection, "trace_resource_scopes", "relative_path")?;
        require_column(connection, "trace_resource_scopes", "accounting_method")?;
        require_column(connection, "trace_resource_scopes", "lifecycle_state")?;
        require_column(connection, "trace_resource_scopes", "created_at")?;
        require_column(connection, "trace_resource_scopes", "final_event_id")?;
        require_column(connection, "trace_resource_scopes", "updated_at")?;
    }
    require_column(connection, "events", "process_id")?;
    require_integer_column(connection, "events", "event_meta")?;
    require_integer_column(connection, "events", "kind_code")?;
    require_column(connection, "events", "payload_path_id")?;
    require_column(connection, "events", "payload_id")?;
    require_column(connection, "events", "payload_inline")?;
    require_column(connection, "event_payload_blocks", "event_id")?;
    require_integer_column(connection, "event_payload_blocks", "block_order")?;
    require_column(connection, "event_payload_blocks", "kind")?;
    require_column(connection, "event_payload_blocks", "encoded_bytes")?;
    require_column(connection, "event_payload_dictionary", "payload_id")?;
    require_column(connection, "event_payload_dictionary", "trace_id")?;
    require_column(connection, "event_payload_dictionary", "payload")?;
    require_column(connection, "event_record_blocks", "trace_id")?;
    require_column(connection, "event_record_blocks", "first_event_id")?;
    require_column(connection, "event_record_blocks", "max_event_id")?;
    require_column(connection, "event_record_blocks", "min_observed_at")?;
    require_column(connection, "event_record_blocks", "max_observed_at")?;
    require_column(connection, "event_record_blocks", "event_count")?;
    require_column(connection, "event_record_blocks", "kind_counts")?;
    require_column(connection, "event_record_blocks", "codec_version")?;
    require_column(connection, "event_record_blocks", "uncompressed_bytes")?;
    require_column(connection, "event_record_blocks", "encoded_bytes")?;
    require_column(connection, "event_record_pending", "event_id")?;
    require_column(connection, "event_record_pending", "trace_id")?;
    require_column(connection, "event_record_pending", "observed_at")?;
    require_column(connection, "event_record_pending", "kind_code")?;
    require_column(connection, "event_record_pending", "encoded_frame")?;
    require_column(connection, "event_record_pending_state", "trace_id")?;
    require_column(connection, "event_record_pending_state", "event_count")?;
    require_column(connection, "event_record_pending_state", "framed_bytes")?;
    require_column(connection, "event_policy_details", "event_id")?;
    require_column(connection, "event_policy_details", "note")?;
    require_column(connection, "event_policy_details", "redactions")?;
    require_column(connection, "event_policy_details", "truncations")?;
    require_column(connection, "payload_segments", "process_id")?;
    require_integer_column(connection, "payload_segments", "segment_meta")?;
    require_integer_column(connection, "payload_segments", "storage_omission")?;
    require_column(connection, "traces", "exited_at")?;
    require_column(connection, "semantic_action_ids", "action_key")?;
    require_column(connection, "semantic_action_ids", "action_id")?;
    require_column(connection, "semantic_actions", "action_key")?;
    require_column(connection, "semantic_actions", "kind_code")?;
    require_column(connection, "semantic_actions", "title")?;
    require_column(connection, "semantic_actions", "file_path_id")?;
    for column in [
        "action_key",
        "end_time",
        "status_code",
        "completeness_code",
        "finalization_reason",
        "failure_http_status",
        "command_invocation_kind",
        "tool_result_binding",
    ] {
        require_integer_column(connection, "semantic_action_state", column)?;
    }
    for column in [
        "failure_title",
        "failure_body_format",
        "failure_http_reason",
        "command_tool_name",
    ] {
        require_column(connection, "semantic_action_state", column)?;
    }
    for column in ["evidence_key", "action_key", "kind_code", "evidence_id"] {
        require_integer_column(connection, "semantic_action_evidence", column)?;
    }
    require_column(connection, "semantic_action_evidence", "role")?;
    for column in ["trace_id", "revision"] {
        require_integer_column(connection, "semantic_action_state_revisions", column)?;
    }
    require_column(connection, "semantic_actions", "action_valid_code")?;
    require_column(connection, "semantic_actions", "process_parent_conflict")?;
    require_column(connection, "agent_identities", "trace_id")?;
    require_column(connection, "agent_identities", "process_id")?;
    require_column(connection, "agent_identities", "identity_action_key")?;
    require_column(connection, "semantic_action_links", "parent_action_key")?;
    require_column(connection, "semantic_action_links", "child_action_key")?;
    require_column(connection, "semantic_action_links", "role_code")?;
    require_column(connection, "semantic_action_links", "origin_code")?;
    require_column(connection, "semantic_action_links", "valid")?;
    require_column(connection, "semantic_action_links", "evidence_blob")?;
    require_column(connection, "file_observation_paths", "path_id")?;
    require_column(connection, "file_paths", "trace_id")?;
    require_column(connection, "file_paths", "path_text")?;
    require_column(connection, "file_observation_paths", "action_key")?;
    require_column(connection, "file_path_sets", "path_set_hash")?;
    require_column(connection, "file_path_set_action_refs", "action_key")?;
    require_column(connection, "llm_request_manifests", "manifest_id")?;
    require_column(connection, "llm_request_manifests", "action_key")?;
    require_column(connection, "llm_request_blocks", "block_id")?;
    require_column(connection, "llm_request_block_refs", "manifest_id")?;
    require_column(connection, "llm_request_lineage", "action_key")?;
    require_column(
        connection,
        "llm_request_lineage",
        "trajectory_root_action_key",
    )?;
    require_column(connection, "llm_request_lineage", "parent_action_key")?;
    require_column(connection, "llm_request_lineage", "forked_from_action_key")?;
    require_column(connection, "llm_request_lineage", "trajectory_position")?;
    require_column(connection, "llm_request_lineage", "transition_code")?;
    require_column(connection, "llm_request_lineage", "start_reason_code")?;
    require_column(connection, "llm_request_lineage", "inference_version")?;
    require_column(connection, "mcp_jsonrpc_messages", "message_id")?;
    require_column(connection, "mcp_jsonrpc_messages", "canonical_json_hash")?;
    require_column(connection, "mcp_jsonrpc_messages", "canonical_json_bytes")?;
    require_column(connection, "mcp_jsonrpc_messages", "canonical_json")?;
    require_column(connection, "mcp_jsonrpc_action_refs", "action_key")?;
    require_column(connection, "mcp_jsonrpc_action_refs", "message_id")?;
    require_column(connection, "semantic_action_cold_fields", "payload")?;
    require_column(connection, "semantic_action_link_cold_fields", "payload")?;
    if require_external_bindings {
        require_schema_object(connection, "table", "trace_external_cgroup_bindings")?;
        for column in [
            "trace_id",
            "runtime",
            "container_id",
            "relative_path",
            "cgroup_device_be",
            "cgroup_inode_be",
            "host_boot_id",
            "lifecycle_state",
            "stale_reason",
            "consecutive_failures",
            "created_at",
            "updated_at",
            "last_good_at",
            "closed_at",
            "final_event_id",
        ] {
            require_column(connection, "trace_external_cgroup_bindings", column)?;
        }
        require_schema_object(
            connection,
            "index",
            "idx_trace_external_cgroup_bindings_lifecycle",
        )?;
    }
    Ok(())
}

fn user_version(connection: &Connection) -> Result<i32, rusqlite::Error> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}

fn user_table_count(connection: &Connection) -> Result<i64, rusqlite::Error> {
    connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )
}

pub(crate) fn require_column(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<(), rusqlite::Error> {
    if column_exists(connection, table, column)? {
        return Ok(());
    }
    Err(rusqlite::Error::InvalidQuery)
}

fn require_integer_column(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<(), rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    })?;
    for row in rows {
        let (name, column_type) = row?;
        if name == column && column_type.eq_ignore_ascii_case("INTEGER") {
            return Ok(());
        }
    }
    Err(rusqlite::Error::InvalidQuery)
}

fn require_schema_object(
    connection: &Connection,
    object_type: &str,
    name: &str,
) -> Result<(), rusqlite::Error> {
    let count = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
        [object_type, name],
        |row| row.get::<_, i64>(0),
    )?;
    if count == 1 {
        return Ok(());
    }
    Err(rusqlite::Error::InvalidQuery)
}

fn column_exists(
    connection: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}
