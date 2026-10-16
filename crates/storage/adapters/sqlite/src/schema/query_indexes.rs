use rusqlite::Connection;

use super::require_schema_object;

pub(super) struct QueryIndexes;

impl QueryIndexes {
    pub(super) fn prepare(connection: &Connection) -> Result<(), rusqlite::Error> {
        connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_events_trace_id ON events(trace_id);
         CREATE INDEX IF NOT EXISTS idx_alerts_trace_definition_id ON alerts(trace_id, alert_definition_id, alert_id);
         CREATE INDEX IF NOT EXISTS idx_alerts_trace_definition_episode ON alerts(trace_id, alert_definition_id, json_extract(payload_json, '$.episode_key'), alert_id);
         CREATE INDEX IF NOT EXISTS idx_traces_created_id ON traces(created_at DESC, trace_id DESC);
         CREATE INDEX IF NOT EXISTS idx_traces_name_created_id ON traces(display_name, created_at DESC, trace_id DESC);
         CREATE INDEX IF NOT EXISTS idx_payload_segments_trace_id ON payload_segments(trace_id);
         CREATE INDEX IF NOT EXISTS idx_tls_flow_diagnostics_trace_id ON tls_flow_diagnostics(trace_id);
         CREATE INDEX IF NOT EXISTS idx_semantic_actions_trace_start ON semantic_actions(trace_id, start_time);
         CREATE INDEX IF NOT EXISTS idx_semantic_actions_trace_kind_start ON semantic_actions(trace_id, kind_code, start_time, action_key);
         CREATE INDEX IF NOT EXISTS idx_semantic_actions_timeline_process ON semantic_actions(trace_id, process_id, start_time, action_key);
         CREATE INDEX IF NOT EXISTS idx_semantic_actions_timeline_process_kind ON semantic_actions(trace_id, process_id, kind_code, start_time, action_key);
         CREATE INDEX IF NOT EXISTS idx_semantic_action_links_timeline_parent_role ON semantic_action_links(trace_id, parent_action_key, role_code, child_action_key);
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

    pub(super) fn validate(connection: &Connection) -> Result<(), rusqlite::Error> {
        for index in [
            "idx_alerts_trace_definition_id",
            "idx_alerts_trace_definition_episode",
            "idx_traces_created_id",
            "idx_traces_name_created_id",
            "idx_semantic_action_links_trace_child_role",
            "idx_semantic_actions_trace_start",
            "idx_semantic_actions_trace_kind_start",
            "idx_semantic_actions_timeline_process",
            "idx_semantic_actions_timeline_process_kind",
            "idx_semantic_action_links_timeline_parent_role",
        ] {
            require_schema_object(connection, "index", index).map_err(|_| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_SCHEMA),
                Some(format!("required query index {index} is missing; open this database with the current AcTrail writer before starting a read-only viewer; recreate exported snapshots from the prepared database")),
            )
        })?;
        }
        Ok(())
    }
}
