//! SQLite storage for semantic actions.

use model_core::ids::TraceId;
use model_core::process::{NamespaceIdentity, ProcessIdentity};
use rusqlite::{Row, params};
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionReadStore,
    SemanticActionStatus, SemanticActionStoreError, SemanticActionWriteStore, SemanticEvidence,
    SemanticEvidenceKind,
};

use crate::SqliteStorage;
use crate::records::{decode_map, decode_time, encode_map, encode_time};

impl SemanticActionWriteStore for SqliteStorage {
    fn upsert_semantic_action(
        &mut self,
        action: SemanticAction,
    ) -> Result<(), SemanticActionStoreError> {
        let connection = self.connection().borrow_mut();
        connection
            .execute(
                "INSERT OR REPLACE INTO semantic_actions (
                    action_id, trace_id, kind, title, start_time, end_time, process_pid,
                    process_task_id, process_start_ticks, process_pid_namespace,
                    process_generation, status, completeness, confidence_millis, attributes
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    &action.action_id,
                    action.trace_id.get(),
                    action.kind.as_str(),
                    &action.title,
                    encode_time(action.start_time),
                    action.end_time.map(encode_time),
                    action.process.pid,
                    action.process.task_id,
                    action.process.start_time_ticks,
                    action
                        .process
                        .pid_namespace
                        .as_ref()
                        .map(|value| value.as_str().to_string()),
                    action.process.generation,
                    action.status.as_str(),
                    action.completeness.as_str(),
                    action.confidence_millis,
                    encode_map(&action.attributes),
                ],
            )
            .map_err(|error| {
                SemanticActionStoreError::new("upsert_semantic_action", error.to_string())
            })?;
        connection
            .execute(
                "DELETE FROM semantic_action_evidence WHERE action_id = ?1",
                params![&action.action_id],
            )
            .map_err(|error| {
                SemanticActionStoreError::new("replace_semantic_action_evidence", error.to_string())
            })?;
        for (index, evidence) in action.evidence.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO semantic_action_evidence (
                        action_id, evidence_order, kind, evidence_id, role
                    ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        &action.action_id,
                        index,
                        evidence.kind.as_str(),
                        evidence.id,
                        &evidence.role,
                    ],
                )
                .map_err(|error| {
                    SemanticActionStoreError::new(
                        "insert_semantic_action_evidence",
                        error.to_string(),
                    )
                })?;
        }
        Ok(())
    }
}

impl SemanticActionReadStore for SqliteStorage {
    fn list_semantic_actions(
        &self,
        trace_id: TraceId,
    ) -> Result<Vec<SemanticAction>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "list_semantic_actions",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        let mut statement = connection
            .prepare(
                "SELECT * FROM semantic_actions
                 WHERE trace_id = ?1
                 ORDER BY start_time ASC, action_id ASC",
            )
            .map_err(|error| {
                SemanticActionStoreError::new("prepare_semantic_actions", error.to_string())
            })?;
        let rows = statement
            .query_map(params![trace_id.get()], action_from_row)
            .map_err(|error| {
                SemanticActionStoreError::new("query_semantic_actions", error.to_string())
            })?;
        let mut actions = Vec::new();
        for row in rows {
            let mut action = row.map_err(|error| {
                SemanticActionStoreError::new("map_semantic_action", error.to_string())
            })?;
            action.evidence = read_evidence(&connection, &action.action_id)?;
            actions.push(action);
        }
        Ok(actions)
    }
}

fn read_evidence(
    connection: &rusqlite::Connection,
    action_id: &str,
) -> Result<Vec<SemanticEvidence>, SemanticActionStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT kind, evidence_id, role FROM semantic_action_evidence
             WHERE action_id = ?1
             ORDER BY evidence_order ASC",
        )
        .map_err(|error| {
            SemanticActionStoreError::new("prepare_semantic_action_evidence", error.to_string())
        })?;
    let rows = statement
        .query_map(params![action_id], evidence_from_row)
        .map_err(|error| {
            SemanticActionStoreError::new("query_semantic_action_evidence", error.to_string())
        })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
        SemanticActionStoreError::new("map_semantic_action_evidence", error.to_string())
    })
}

fn action_from_row(row: &Row<'_>) -> Result<SemanticAction, rusqlite::Error> {
    Ok(SemanticAction {
        action_id: row.get("action_id")?,
        trace_id: TraceId::new(row.get("trace_id")?),
        kind: decode_kind(row.get::<_, String>("kind")?)?,
        title: row.get("title")?,
        start_time: decode_time(row.get("start_time")?),
        end_time: row.get::<_, Option<i64>>("end_time")?.map(decode_time),
        process: ProcessIdentity {
            pid: row.get("process_pid")?,
            task_id: row.get("process_task_id")?,
            start_time_ticks: row.get("process_start_ticks")?,
            pid_namespace: row
                .get::<_, Option<String>>("process_pid_namespace")?
                .map(NamespaceIdentity::new),
            generation: row.get("process_generation")?,
        },
        status: decode_status(row.get::<_, String>("status")?)?,
        completeness: decode_completeness(row.get::<_, String>("completeness")?)?,
        confidence_millis: row.get("confidence_millis")?,
        attributes: decode_map(&row.get::<_, String>("attributes")?),
        evidence: Vec::new(),
    })
}

fn evidence_from_row(row: &Row<'_>) -> Result<SemanticEvidence, rusqlite::Error> {
    Ok(SemanticEvidence {
        kind: decode_evidence_kind(row.get::<_, String>("kind")?)?,
        id: row.get("evidence_id")?,
        role: row.get("role")?,
    })
}

fn decode_kind(value: String) -> Result<SemanticActionKind, rusqlite::Error> {
    SemanticActionKind::parse(&value).ok_or(rusqlite::Error::InvalidQuery)
}

fn decode_status(value: String) -> Result<SemanticActionStatus, rusqlite::Error> {
    SemanticActionStatus::parse(&value).ok_or(rusqlite::Error::InvalidQuery)
}

fn decode_completeness(value: String) -> Result<SemanticActionCompleteness, rusqlite::Error> {
    SemanticActionCompleteness::parse(&value).ok_or(rusqlite::Error::InvalidQuery)
}

fn decode_evidence_kind(value: String) -> Result<SemanticEvidenceKind, rusqlite::Error> {
    SemanticEvidenceKind::parse(&value).ok_or(rusqlite::Error::InvalidQuery)
}
