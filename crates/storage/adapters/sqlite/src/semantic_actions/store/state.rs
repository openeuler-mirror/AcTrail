//! Direct assignments to producer-owned lifecycle and classification facts.

use std::collections::BTreeMap;

use model_core::ids::TraceId;
use rusqlite::{Connection, OptionalExtension, Row, params};
use semantic_action::{
    SemanticAction, SemanticActionChange, SemanticActionFinalizationReason, SemanticActionKind,
    SemanticActionStatus, SemanticActionStoreError, SemanticActionUpdate, SemanticCommandKind,
    SemanticEvidence, SemanticToolResultBinding, attr_keys as attrs,
};

use crate::records::encode_time;
use crate::semantic_actions::codebook::sqlite::{
    action_completeness_code, action_kind_code, action_status_code, decode_status,
    evidence_kind_code,
};

macro_rules! assignment_sql {
    ($assignment:literal) => {
        concat!(
            $assignment,
            " WHERE action_key = (SELECT action.action_key FROM semantic_actions action
            JOIN semantic_action_ids ids ON ids.action_key = action.action_key
            WHERE ids.action_id = ?1 AND ids.trace_id = ?2 AND action.trace_id = ?2
              AND action.kind_code = ?3 AND action.process_id = ?4) RETURNING action_key"
        )
    };
}

pub(super) struct InitialActionState {
    finalization_reason: Option<i16>,
    tool_binding: Option<i16>,
    command_kind: Option<i16>,
    command_tool_name: Option<String>,
}

pub(super) struct ActionStateWriter<'a> {
    connection: &'a Connection,
}

impl<'a> ActionStateWriter<'a> {
    pub(super) fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub(super) fn take_initial_binding(
        action: &mut SemanticAction,
    ) -> Result<InitialActionState, SemanticActionStoreError> {
        let tool_binding = if action.kind == SemanticActionKind::LlmToolResult {
            action
                .attributes
                .remove(attrs::llm_tool_result::BINDING_STATE)
                .map(|value| match value.as_str() {
                    "missing_id" => Ok(1),
                    "unmatched" => Ok(2),
                    "bound" => Ok(3),
                    "ambiguous" => Ok(4),
                    _ => Err(SemanticActionStoreError::new(
                        "tool_result_binding",
                        "invalid binding state",
                    )),
                })
                .transpose()?
        } else {
            None
        };
        let (command_kind, command_tool_name) =
            if action.kind == SemanticActionKind::CommandInvocation {
                (
                    action.attributes.remove(attrs::invocation::KIND),
                    action.attributes.remove(attrs::command::TOOL_NAME),
                )
            } else {
                (None, None)
            };
        let command_kind = command_kind
            .map(|kind| match kind.as_str() {
                "agent" => Ok(1),
                "mcp" => Ok(2),
                "command" => Ok(3),
                _ => Err(SemanticActionStoreError::new(
                    "command_invocation_kind",
                    "invalid invocation kind",
                )),
            })
            .transpose()?;
        if action.kind == SemanticActionKind::McpToolCall {
            action.attributes.remove(attrs::mcp::EXECUTION_STATUS);
        }
        let finalization_reason = action
            .attributes
            .remove(attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE)
            .filter(|value| value == "true")
            .map(|_| 1);
        Ok(InitialActionState {
            finalization_reason,
            tool_binding,
            command_kind,
            command_tool_name,
        })
    }

    pub(super) fn insert(
        &self,
        key: i64,
        action: &SemanticAction,
        initial: InitialActionState,
    ) -> Result<(), SemanticActionStoreError> {
        self.connection.prepare_cached(
            "INSERT INTO semantic_action_state(action_key, end_time, status_code, completeness_code,
                tool_result_binding, command_invocation_kind, command_tool_name, finalization_reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
            .and_then(|mut statement| statement.execute(params![key, action.end_time.map(encode_time),
                action_status_code(action.status), action_completeness_code(action.completeness),
                initial.tool_binding, initial.command_kind, initial.command_tool_name, initial.finalization_reason]))
            .map(|_| ()).map_err(|e| Self::error("insert_semantic_action_state", e))
    }

    pub(super) fn update(
        &self,
        update: SemanticActionUpdate,
    ) -> Result<(), SemanticActionStoreError> {
        let kind = action_kind_code(update.kind);
        let key = match &update.change {
            SemanticActionChange::Lifecycle {
                end_time,
                status,
                completeness,
                finalization_reason,
            } => {
                let reason = finalization_reason.map(|reason| match reason {
                    SemanticActionFinalizationReason::TraceClosed => 1,
                    SemanticActionFinalizationReason::CapacityEvicted => 2,
                });
                self.assigned(
                    assignment_sql!(
                        "UPDATE semantic_action_state SET end_time=?5,
                    status_code=?6, completeness_code=?7, finalization_reason=?8"
                    ),
                    params![
                        update.action_id,
                        update.trace_id.get(),
                        kind,
                        update.process.get(),
                        end_time.map(encode_time),
                        action_status_code(*status),
                        action_completeness_code(*completeness),
                        reason
                    ],
                )?
            }
            SemanticActionChange::LlmResponseFailure {
                title,
                end_time,
                body_format,
                http_status_code,
                http_reason,
            } => {
                if update.kind != SemanticActionKind::LlmResponse {
                    return Err(Self::error(
                        "update_semantic_action_kind",
                        "response failure requires LLM response",
                    ));
                }
                self.assigned(assignment_sql!("UPDATE semantic_action_state SET end_time=?5, status_code=?6,
                    failure_title=?7, failure_body_format=?8, failure_http_status=?9, failure_http_reason=?10"),
                    params![update.action_id, update.trace_id.get(), kind, update.process.get(),
                        end_time.map(encode_time), action_status_code(SemanticActionStatus::Error),
                        title, body_format, http_status_code, http_reason])?
            }
            SemanticActionChange::CommandClassification { kind: command_kind } => {
                if update.kind != SemanticActionKind::CommandInvocation {
                    return Err(Self::error(
                        "update_semantic_action_kind",
                        "classification requires command",
                    ));
                }
                let code = match command_kind {
                    SemanticCommandKind::Agent => 1,
                    SemanticCommandKind::Mcp => 2,
                    SemanticCommandKind::Command => 3,
                };
                self.assigned(
                    assignment_sql!("UPDATE semantic_action_state SET command_invocation_kind=?5"),
                    params![
                        update.action_id,
                        update.trace_id.get(),
                        kind,
                        update.process.get(),
                        code
                    ],
                )?
            }
            SemanticActionChange::CommandToolName { tool_name } => {
                if update.kind != SemanticActionKind::CommandInvocation {
                    return Err(Self::error(
                        "update_semantic_action_kind",
                        "tool name requires command",
                    ));
                }
                self.assigned(
                    assignment_sql!("UPDATE semantic_action_state SET command_tool_name=?5"),
                    params![
                        update.action_id,
                        update.trace_id.get(),
                        kind,
                        update.process.get(),
                        tool_name
                    ],
                )?
            }
            SemanticActionChange::ToolResultBinding { state } => {
                if update.kind != SemanticActionKind::LlmToolResult {
                    return Err(Self::error(
                        "update_semantic_action_kind",
                        "binding requires tool result",
                    ));
                }
                let code = match state {
                    SemanticToolResultBinding::MissingId => 1,
                    SemanticToolResultBinding::Unmatched => 2,
                    SemanticToolResultBinding::Bound => 3,
                    SemanticToolResultBinding::Ambiguous => 4,
                };
                self.assigned(
                    assignment_sql!("UPDATE semantic_action_state SET tool_result_binding=?5"),
                    params![
                        update.action_id,
                        update.trace_id.get(),
                        kind,
                        update.process.get(),
                        code
                    ],
                )?
            }
        };
        self.append_evidence(key, &update.evidence)?;
        self.revise(update.trace_id)
    }

    fn assigned(
        &self,
        sql: &str,
        values: impl rusqlite::Params,
    ) -> Result<i64, SemanticActionStoreError> {
        self.connection
            .prepare_cached(sql)
            .and_then(|mut statement| statement.query_row(values, |row| row.get(0)))
            .optional()
            .map_err(|e| Self::error("update_semantic_action_state", e))?
            .ok_or_else(|| {
                Self::error(
                    "update_semantic_action_identity",
                    "action identity is missing or conflicts",
                )
            })
    }

    pub(super) fn append_evidence(
        &self,
        key: i64,
        evidence: &[SemanticEvidence],
    ) -> Result<bool, SemanticActionStoreError> {
        if evidence.is_empty() {
            return Ok(false);
        }
        let mut statement = self.connection.prepare_cached(
            "INSERT INTO semantic_action_evidence(action_key, kind_code, evidence_id, role)
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT(action_key, kind_code, evidence_id, role) DO NOTHING")
            .map_err(|e| Self::error("prepare_semantic_action_evidence", e))?;
        let mut changed = false;
        for item in evidence {
            changed |= statement
                .execute(params![
                    key,
                    evidence_kind_code(item.kind),
                    item.id,
                    item.role
                ])
                .map_err(|e| Self::error("insert_semantic_action_evidence", e))?
                != 0;
        }
        Ok(changed)
    }

    pub(super) fn revise(&self, trace: TraceId) -> Result<(), SemanticActionStoreError> {
        self.connection
            .prepare_cached(
                "INSERT INTO semantic_action_state_revisions(trace_id, revision)
            VALUES (?1, 1) ON CONFLICT(trace_id) DO UPDATE SET revision=revision+1",
            )
            .and_then(|mut statement| statement.execute([trace.get()]))
            .map(|_| ())
            .map_err(|e| Self::error("revise_semantic_action_state", e))
    }

    pub(super) fn hydrate_attributes(
        row: &Row<'_>,
        attributes: &mut BTreeMap<String, String>,
    ) -> Result<(), rusqlite::Error> {
        if let Some(code) = row.get::<_, Option<i16>>("command_invocation_kind")? {
            let value = match code {
                1 => "agent",
                2 => "mcp",
                3 => "command",
                _ => return Err(rusqlite::Error::InvalidQuery),
            };
            attributes.insert(attrs::invocation::KIND.into(), value.into());
        }
        if let Some(value) = row.get::<_, Option<String>>("command_tool_name")? {
            attributes.insert(attrs::command::TOOL_NAME.into(), value);
        }
        if let Some(code) = row.get::<_, Option<i16>>("tool_result_binding")? {
            let value = match code {
                1 => "missing_id",
                2 => "unmatched",
                3 => "bound",
                4 => "ambiguous",
                _ => return Err(rusqlite::Error::InvalidQuery),
            };
            attributes.insert(attrs::llm_tool_result::BINDING_STATE.into(), value.into());
        }
        if let Some(format) = row.get::<_, Option<String>>("failure_body_format")? {
            attributes.insert(attrs::llm_response::BODY_FORMAT.into(), format);
            for (column, key) in [
                ("failure_http_status", attrs::http_response::STATUS_CODE),
                ("failure_http_reason", attrs::http_response::REASON),
            ] {
                let value = if column == "failure_http_status" {
                    row.get::<_, Option<u16>>(column)?.map(|n| n.to_string())
                } else {
                    row.get::<_, Option<String>>(column)?
                };
                if let Some(value) = value {
                    attributes.insert(key.into(), value);
                } else {
                    attributes.remove(key);
                }
            }
        }
        if row.get::<_, i16>("kind_code")? == action_kind_code(SemanticActionKind::McpToolCall) {
            attributes.insert(
                attrs::mcp::EXECUTION_STATUS.into(),
                decode_status(row.get("status_code")?)?.as_str().into(),
            );
        }
        if let Some(code) = row.get::<_, Option<i16>>("finalization_reason")? {
            match code {
                1 => {
                    attributes.insert(
                        attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.into(),
                        "true".into(),
                    );
                }
                2 => {}
                _ => return Err(rusqlite::Error::InvalidQuery),
            }
        }
        Ok(())
    }

    fn error(stage: &str, error: impl std::fmt::Display) -> SemanticActionStoreError {
        SemanticActionStoreError::new(stage, error.to_string())
    }
}
