use std::collections::BTreeMap;

use model_core::ids::TraceId;
use rusqlite::{Connection, OptionalExtension, Row, params};
use semantic_action::{
    LlmRequestLineage, LlmRequestLineageWrite, LlmTrajectoryStartReason, LlmTrajectoryTransition,
    SemanticActionKind, SemanticActionStoreError,
};

use super::action_ids::require_action_key;
use super::codebook::sqlite::action_kind_code;

const TRANSITION_ROOT: i16 = 1;
const TRANSITION_APPEND: i16 = 2;
const TRANSITION_FORK_ROOT: i16 = 3;
const TRANSITION_DUPLICATE_ROOT: i16 = 4;

const START_UNSPECIFIED: i16 = 0;
const START_CONTEXT_REWRITE: i16 = 1;
const START_RUNTIME_RESET: i16 = 2;
const START_CAPACITY_EVICTION: i16 = 3;
const START_UNSUPPORTED_MULTIMODAL: i16 = 4;
const START_HISTORY_LIMIT: i16 = 5;
const START_CLASSIFIER_FAILURE: i16 = 6;

pub(super) struct LlmRequestLineageStore;

impl LlmRequestLineageStore {
    pub(super) fn upsert_batch(
        connection: &mut Connection,
        lineages: &[LlmRequestLineageWrite],
    ) -> Result<(), SemanticActionStoreError> {
        if lineages.is_empty() {
            return Ok(());
        }
        let mut by_action = BTreeMap::new();
        for lineage in lineages {
            match by_action.insert(lineage.action_id.as_str(), lineage) {
                Some(existing) if existing != lineage => {
                    return Err(store_error(
                        "validate_llm_request_lineage_batch",
                        format!(
                            "conflicting lineage records for action {}",
                            lineage.action_id
                        ),
                    ));
                }
                _ => {}
            }
            Self::validate_action(connection, lineage.trace_id, &lineage.action_id)?;
            Self::validate_action(connection, lineage.trace_id, &lineage.trajectory_id)?;
            if let Some(parent) = lineage.parent_action_id.as_deref() {
                Self::validate_action(connection, lineage.trace_id, parent)?;
            }
            if let Some(source) = lineage.forked_from_action_id.as_deref() {
                Self::validate_action(connection, lineage.trace_id, source)?;
            }
            Self::validate_shape(lineage)?;
        }
        for lineage in lineages {
            Self::validate_relationships(connection, &by_action, lineage)?;
        }

        let savepoint = connection.savepoint().map_err(|error| {
            SemanticActionStoreError::new("begin_llm_request_lineage", error.to_string())
        })?;
        for lineage in lineages {
            Self::insert_one(&savepoint, lineage)?;
        }
        savepoint.commit().map_err(|error| {
            SemanticActionStoreError::new("commit_llm_request_lineage", error.to_string())
        })
    }

    pub(super) fn by_action(
        connection: &Connection,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<Option<LlmRequestLineage>, SemanticActionStoreError> {
        Self::query_one(
            connection,
            "lineage.trace_id = ?1 AND action_ids.action_id = ?2",
            params![trace_id.get(), action_id],
            "read_llm_request_lineage",
        )
    }

    pub(super) fn by_trace(
        connection: &Connection,
        trace_id: TraceId,
    ) -> Result<Vec<LlmRequestLineage>, SemanticActionStoreError> {
        Self::query_many(
            connection,
            "lineage.trace_id = ?1",
            params![trace_id.get()],
            "read_llm_request_lineages",
        )
    }

    pub(super) fn by_trajectory(
        connection: &Connection,
        trace_id: TraceId,
        trajectory_id: &str,
    ) -> Result<Vec<LlmRequestLineage>, SemanticActionStoreError> {
        Self::query_many(
            connection,
            "lineage.trace_id = ?1 AND trajectory_ids.action_id = ?2",
            params![trace_id.get(), trajectory_id],
            "read_llm_request_trajectory",
        )
    }

    pub(super) fn forks_from(
        connection: &Connection,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<Vec<LlmRequestLineage>, SemanticActionStoreError> {
        Self::query_many(
            connection,
            "lineage.trace_id = ?1 AND fork_ids.action_id = ?2",
            params![trace_id.get(), action_id],
            "read_llm_request_forks",
        )
    }

    fn validate_action(
        connection: &Connection,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<i64, SemanticActionStoreError> {
        let key = require_action_key(connection, action_id)?;
        let valid = connection
            .query_row(
                "SELECT 1 FROM semantic_actions
                 WHERE action_key = ?1 AND trace_id = ?2 AND kind_code = ?3",
                params![
                    key,
                    trace_id.get(),
                    action_kind_code(SemanticActionKind::LlmRequest)
                ],
                |_| Ok(()),
            )
            .optional()
            .map_err(|source| store_error("validate_llm_request_lineage_action", source))?
            .is_some();
        if valid {
            Ok(key)
        } else {
            Err(store_error(
                "validate_llm_request_lineage_action",
                format!("action {action_id} is not an llm.request in the lineage trace"),
            ))
        }
    }

    fn validate_shape(lineage: &LlmRequestLineageWrite) -> Result<(), SemanticActionStoreError> {
        let root = lineage.trajectory_id == lineage.action_id;
        let valid = lineage.inference_version > 0
            && match lineage.transition {
                LlmTrajectoryTransition::Root | LlmTrajectoryTransition::DuplicateRoot => {
                    root && lineage.trajectory_position == 0
                        && lineage.parent_action_id.is_none()
                        && lineage.forked_from_action_id.is_none()
                }
                LlmTrajectoryTransition::ForkRoot => {
                    root && lineage.trajectory_position == 0
                        && lineage.parent_action_id.is_none()
                        && lineage.forked_from_action_id.is_some()
                }
                LlmTrajectoryTransition::Append => {
                    lineage.trajectory_position > 0
                        && lineage.parent_action_id.is_some()
                        && lineage.forked_from_action_id.is_none()
                        && lineage.start_reason == LlmTrajectoryStartReason::Unspecified
                }
            };
        if valid {
            Ok(())
        } else {
            Err(store_error(
                "validate_llm_request_lineage_shape",
                format!(
                    "invalid {:?} lineage shape for {}",
                    lineage.transition, lineage.action_id
                ),
            ))
        }
    }

    fn validate_relationships(
        connection: &Connection,
        batch: &BTreeMap<&str, &LlmRequestLineageWrite>,
        lineage: &LlmRequestLineageWrite,
    ) -> Result<(), SemanticActionStoreError> {
        if let Some(parent_id) = lineage.parent_action_id.as_deref() {
            let parent =
                Self::lineage_for_validation(connection, batch, lineage.trace_id, parent_id)?;
            if parent.trajectory_id != lineage.trajectory_id
                || parent.trajectory_position.checked_add(1) != Some(lineage.trajectory_position)
            {
                return Err(store_error(
                    "validate_llm_request_lineage_parent",
                    format!(
                        "parent {parent_id} does not precede {} in the same trajectory",
                        lineage.action_id
                    ),
                ));
            }
        }
        if let Some(source_id) = lineage.forked_from_action_id.as_deref() {
            let source =
                Self::lineage_for_validation(connection, batch, lineage.trace_id, source_id)?;
            if source.trajectory_id == lineage.trajectory_id {
                return Err(store_error(
                    "validate_llm_request_lineage_fork",
                    "fork source must belong to a different trajectory",
                ));
            }
        }
        if lineage.trajectory_position > 0 {
            let root = Self::lineage_for_validation(
                connection,
                batch,
                lineage.trace_id,
                &lineage.trajectory_id,
            )?;
            if root.action_id != root.trajectory_id || root.trajectory_position != 0 {
                return Err(store_error(
                    "validate_llm_request_lineage_root",
                    "trajectory id does not resolve to its root request",
                ));
            }
        }
        Ok(())
    }

    fn lineage_for_validation(
        connection: &Connection,
        batch: &BTreeMap<&str, &LlmRequestLineageWrite>,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<LlmRequestLineage, SemanticActionStoreError> {
        if let Some(lineage) = batch.get(action_id) {
            return Ok(lineage_from_write(lineage));
        }
        Self::by_action(connection, trace_id, action_id)?.ok_or_else(|| {
            store_error(
                "validate_llm_request_lineage_reference",
                format!("referenced lineage {action_id} does not exist"),
            )
        })
    }

    fn insert_one(
        connection: &Connection,
        lineage: &LlmRequestLineageWrite,
    ) -> Result<(), SemanticActionStoreError> {
        let action_key = require_action_key(connection, &lineage.action_id)?;
        let trajectory_key = require_action_key(connection, &lineage.trajectory_id)?;
        let parent_key = lineage
            .parent_action_id
            .as_deref()
            .map(|id| require_action_key(connection, id))
            .transpose()?;
        let fork_key = lineage
            .forked_from_action_id
            .as_deref()
            .map(|id| require_action_key(connection, id))
            .transpose()?;
        connection
            .execute(
                "INSERT OR IGNORE INTO llm_request_lineage (
                    action_key, trace_id, trajectory_root_action_key, parent_action_key,
                    forked_from_action_key, trajectory_position, transition_code,
                    start_reason_code, inference_version
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    action_key,
                    lineage.trace_id.get(),
                    trajectory_key,
                    parent_key,
                    fork_key,
                    lineage.trajectory_position,
                    transition_code(lineage.transition),
                    start_reason_code(lineage.start_reason),
                    lineage.inference_version,
                ],
            )
            .map_err(|source| store_error("insert_llm_request_lineage", source))?;
        let stored = Self::by_action(connection, lineage.trace_id, &lineage.action_id)?;
        if stored.as_ref() == Some(&lineage_from_write(lineage)) {
            Ok(())
        } else {
            Err(store_error(
                "conflicting_llm_request_lineage",
                format!(
                    "action {} already has a different lineage",
                    lineage.action_id
                ),
            ))
        }
    }

    fn query_one<P: rusqlite::Params>(
        connection: &Connection,
        predicate: &str,
        params: P,
        stage: &'static str,
    ) -> Result<Option<LlmRequestLineage>, SemanticActionStoreError> {
        connection
            .query_row(&select_sql(predicate), params, map_row)
            .optional()
            .map_err(|error| SemanticActionStoreError::new(stage, error.to_string()))
    }

    fn query_many<P: rusqlite::Params>(
        connection: &Connection,
        predicate: &str,
        params: P,
        stage: &'static str,
    ) -> Result<Vec<LlmRequestLineage>, SemanticActionStoreError> {
        let mut statement = connection
            .prepare(&format!(
                "{} ORDER BY lineage.trajectory_position ASC, action_ids.action_id ASC",
                select_sql(predicate)
            ))
            .map_err(|error| SemanticActionStoreError::new(stage, error.to_string()))?;
        let rows = statement
            .query_map(params, map_row)
            .map_err(|error| SemanticActionStoreError::new(stage, error.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| SemanticActionStoreError::new(stage, error.to_string()))
    }
}

fn lineage_from_write(value: &LlmRequestLineageWrite) -> LlmRequestLineage {
    LlmRequestLineage {
        trace_id: value.trace_id,
        action_id: value.action_id.clone(),
        trajectory_id: value.trajectory_id.clone(),
        parent_action_id: value.parent_action_id.clone(),
        forked_from_action_id: value.forked_from_action_id.clone(),
        trajectory_position: value.trajectory_position,
        transition: value.transition,
        start_reason: value.start_reason,
        inference_version: value.inference_version,
    }
}

fn select_sql(predicate: &str) -> String {
    format!(
        "SELECT lineage.trace_id,
                action_ids.action_id,
                trajectory_ids.action_id AS trajectory_id,
                parent_ids.action_id AS parent_action_id,
                fork_ids.action_id AS forked_from_action_id,
                lineage.trajectory_position,
                lineage.transition_code,
                lineage.start_reason_code,
                lineage.inference_version
         FROM llm_request_lineage lineage
         JOIN semantic_action_ids action_ids ON action_ids.action_key = lineage.action_key
         JOIN semantic_action_ids trajectory_ids
           ON trajectory_ids.action_key = lineage.trajectory_root_action_key
         LEFT JOIN semantic_action_ids parent_ids ON parent_ids.action_key = lineage.parent_action_key
         LEFT JOIN semantic_action_ids fork_ids ON fork_ids.action_key = lineage.forked_from_action_key
         WHERE {predicate}"
    )
}

fn map_row(row: &Row<'_>) -> rusqlite::Result<LlmRequestLineage> {
    let trace_id = row.get::<_, u64>("trace_id")?;
    let position = row.get::<_, u64>("trajectory_position")?;
    let version = row.get::<_, u64>("inference_version")?;
    Ok(LlmRequestLineage {
        trace_id: TraceId::new(trace_id),
        action_id: row.get("action_id")?,
        trajectory_id: row.get("trajectory_id")?,
        parent_action_id: row.get("parent_action_id")?,
        forked_from_action_id: row.get("forked_from_action_id")?,
        trajectory_position: u32::try_from(position).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        transition: transition_from_code(row.get("transition_code")?)?,
        start_reason: start_reason_from_code(row.get("start_reason_code")?)?,
        inference_version: u32::try_from(version).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
    })
}

fn transition_code(value: LlmTrajectoryTransition) -> i16 {
    match value {
        LlmTrajectoryTransition::Root => TRANSITION_ROOT,
        LlmTrajectoryTransition::Append => TRANSITION_APPEND,
        LlmTrajectoryTransition::ForkRoot => TRANSITION_FORK_ROOT,
        LlmTrajectoryTransition::DuplicateRoot => TRANSITION_DUPLICATE_ROOT,
    }
}

fn transition_from_code(code: i16) -> rusqlite::Result<LlmTrajectoryTransition> {
    match code {
        TRANSITION_ROOT => Ok(LlmTrajectoryTransition::Root),
        TRANSITION_APPEND => Ok(LlmTrajectoryTransition::Append),
        TRANSITION_FORK_ROOT => Ok(LlmTrajectoryTransition::ForkRoot),
        TRANSITION_DUPLICATE_ROOT => Ok(LlmTrajectoryTransition::DuplicateRoot),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn start_reason_code(value: LlmTrajectoryStartReason) -> i16 {
    match value {
        LlmTrajectoryStartReason::Unspecified => START_UNSPECIFIED,
        LlmTrajectoryStartReason::ContextRewriteOrCompression => START_CONTEXT_REWRITE,
        LlmTrajectoryStartReason::RuntimeReset => START_RUNTIME_RESET,
        LlmTrajectoryStartReason::CapacityEviction => START_CAPACITY_EVICTION,
        LlmTrajectoryStartReason::UnsupportedMultimodal => START_UNSUPPORTED_MULTIMODAL,
        LlmTrajectoryStartReason::HistoryLimit => START_HISTORY_LIMIT,
        LlmTrajectoryStartReason::ClassifierFailure => START_CLASSIFIER_FAILURE,
    }
}

fn start_reason_from_code(code: i16) -> rusqlite::Result<LlmTrajectoryStartReason> {
    match code {
        START_UNSPECIFIED => Ok(LlmTrajectoryStartReason::Unspecified),
        START_CONTEXT_REWRITE => Ok(LlmTrajectoryStartReason::ContextRewriteOrCompression),
        START_RUNTIME_RESET => Ok(LlmTrajectoryStartReason::RuntimeReset),
        START_CAPACITY_EVICTION => Ok(LlmTrajectoryStartReason::CapacityEviction),
        START_UNSUPPORTED_MULTIMODAL => Ok(LlmTrajectoryStartReason::UnsupportedMultimodal),
        START_HISTORY_LIMIT => Ok(LlmTrajectoryStartReason::HistoryLimit),
        START_CLASSIFIER_FAILURE => Ok(LlmTrajectoryStartReason::ClassifierFailure),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn store_error(stage: &'static str, message: impl ToString) -> SemanticActionStoreError {
    SemanticActionStoreError::new(stage, message.to_string())
}
