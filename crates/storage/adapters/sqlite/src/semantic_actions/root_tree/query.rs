use model_core::ids::TraceId;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use semantic_action::{SemanticActionKind, SemanticActionLinkRole, SemanticActionStoreError};

use super::{sql_placeholders, trace_id_value, usize_value};
use crate::semantic_actions::codebook::sqlite::{action_kind_code, link_role_code};
use crate::semantic_actions::tree_metadata::{
    display_parent_link_absence_predicate, display_parent_link_value_count,
    push_display_parent_link_values,
};

/// One eligibility predicate and ordering for root pages and root navigation.
pub(in crate::semantic_actions) struct DisplayRootQuery<'a> {
    trace_id: TraceId,
    roles: &'a [&'a str],
}

impl<'a> DisplayRootQuery<'a> {
    pub(in crate::semantic_actions) fn new(trace_id: TraceId, roles: &'a [&'a str]) -> Self {
        Self { trace_id, roles }
    }

    pub(super) fn count(&self, connection: &Connection) -> Result<usize, SemanticActionStoreError> {
        let query = format!(
            "SELECT COUNT(*) FROM semantic_actions action WHERE {}",
            self.predicate()
        );
        let count = connection
            .query_row(&query, params_from_iter(self.values()?), |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| {
                SemanticActionStoreError::new(
                    "count_semantic_action_root_children",
                    error.to_string(),
                )
            })?;
        usize::try_from(count).map_err(|error| {
            SemanticActionStoreError::new("count_semantic_action_root_children", error.to_string())
        })
    }

    pub(super) fn action_ids(
        &self,
        connection: &Connection,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<String>, SemanticActionStoreError> {
        let query = format!(
            "SELECT ids.action_id FROM semantic_actions action
            JOIN semantic_action_ids ids ON ids.action_key = action.action_key
            WHERE {} ORDER BY action.start_time ASC, ids.action_id ASC LIMIT ? OFFSET ?",
            self.predicate()
        );
        let mut values = self.values()?;
        values.push(usize_value(limit, "semantic_action_root_child_limit")?);
        values.push(usize_value(offset, "semantic_action_root_child_offset")?);
        let mut statement = connection.prepare(&query).map_err(|error| {
            SemanticActionStoreError::new(
                "prepare_semantic_action_root_children",
                error.to_string(),
            )
        })?;
        let rows = statement
            .query_map(params_from_iter(values), |row| row.get::<_, String>(0))
            .map_err(|error| {
                SemanticActionStoreError::new(
                    "query_semantic_action_root_children",
                    error.to_string(),
                )
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            SemanticActionStoreError::new("map_semantic_action_root_child", error.to_string())
        })
    }

    pub(in crate::semantic_actions) fn offset_before(
        &self,
        connection: &Connection,
        start_time: i64,
        action_id: &str,
    ) -> Result<usize, SemanticActionStoreError> {
        let query = format!(
            "SELECT COUNT(*) FROM semantic_actions action
            JOIN semantic_action_ids ids ON ids.action_key = action.action_key
            WHERE {} AND (action.start_time < ? OR (action.start_time = ? AND ids.action_id < ?))",
            self.predicate()
        );
        let mut values = self.values()?;
        values.extend([
            Value::Integer(start_time),
            Value::Integer(start_time),
            Value::Text(action_id.to_string()),
        ]);
        let count = connection
            .query_row(&query, params_from_iter(values), |row| row.get::<_, i64>(0))
            .map_err(|error| {
                SemanticActionStoreError::new(
                    "query_semantic_action_root_offset",
                    error.to_string(),
                )
            })?;
        usize::try_from(count).map_err(|error| {
            SemanticActionStoreError::new("map_semantic_action_root_offset", error.to_string())
        })
    }

    pub(super) fn offset_of(
        &self,
        connection: &Connection,
        action_id: &str,
    ) -> Result<Option<usize>, SemanticActionStoreError> {
        let query = format!(
            "SELECT action.start_time FROM semantic_actions action
            JOIN semantic_action_ids ids ON ids.action_key = action.action_key
            WHERE {} AND ids.action_id = ?",
            self.predicate()
        );
        let mut values = self.values()?;
        values.push(Value::Text(action_id.to_string()));
        let start = connection
            .query_row(&query, params_from_iter(values), |row| row.get::<_, i64>(0))
            .optional()
            .map_err(|error| {
                SemanticActionStoreError::new(
                    "query_semantic_action_root_anchor",
                    error.to_string(),
                )
            })?;
        start
            .map(|start| self.offset_before(connection, start, action_id))
            .transpose()
    }

    fn predicate(&self) -> String {
        let parent_filter = display_parent_link_absence_predicate(self.roles, "action");
        format!(
            "action.trace_id = ?
         AND action.action_valid_code = 1
         {}
         {}
         AND (
           action.kind_code = ?
           OR NOT EXISTS (
             SELECT 1
             FROM semantic_actions command
             JOIN semantic_action_state command_state
               ON command_state.action_key = command.action_key
             WHERE command.trace_id = action.trace_id
               AND command.kind_code = ?
               AND command.action_key != action.action_key
               AND command.process_id = action.process_id
               AND command.start_time <= action.start_time
               AND (command_state.end_time IS NULL OR action.start_time <= command_state.end_time)
               AND command.action_valid_code = 1
           )
         )",
            parent_filter,
            self.compact_parent_predicate(),
        )
    }

    /// Compact replaces its implementation in the existing parent's page.
    /// Exclude that compact from root eligibility before counting or paging.
    fn compact_parent_predicate(&self) -> String {
        let roles = self.implementation_parent_roles();
        if roles.is_empty() {
            return String::new();
        }
        format!(
            "AND (action.kind_code != ? OR NOT EXISTS (
               SELECT 1
               FROM semantic_action_links compact_link
               JOIN semantic_actions implementation
                 ON implementation.action_key = compact_link.child_action_key
               JOIN semantic_action_links parent_link
                 ON parent_link.trace_id = implementation.trace_id
                AND parent_link.child_action_key = implementation.action_key
               JOIN semantic_actions display_parent
                 ON display_parent.action_key = parent_link.parent_action_key
               WHERE compact_link.trace_id = action.trace_id
                 AND compact_link.parent_action_key = action.action_key
                 AND compact_link.role_code = ?
                 AND compact_link.valid = 1
                 AND implementation.trace_id = action.trace_id
                 AND implementation.kind_code = ?
                 AND implementation.process_id = action.process_id
                 AND implementation.action_valid_code = 1
                 AND parent_link.role_code IN ({})
                 AND parent_link.valid = 1
                 AND display_parent.trace_id = action.trace_id
                 AND display_parent.action_valid_code = 1
                 AND display_parent.action_key != action.action_key
                 AND NOT (
                   (parent_link.role_code = ? OR parent_link.role_code = ? OR parent_link.role_code = ?)
                   AND implementation.process_parent_conflict = 1
                 )
             ))",
            sql_placeholders(roles.len()),
        )
    }

    fn implementation_parent_roles(&self) -> Vec<&'a str> {
        let compact_role =
            SemanticActionLinkRole::AgentContextCompactImplementationLlmCall.as_str();
        if !self.roles.contains(&compact_role) {
            return Vec::new();
        }
        self.roles
            .iter()
            .copied()
            .filter(|role| *role != compact_role)
            .collect()
    }

    fn values(&self) -> Result<Vec<Value>, SemanticActionStoreError> {
        let compact_roles = self.implementation_parent_roles();
        let compact_value_count = if compact_roles.is_empty() {
            0
        } else {
            3 + display_parent_link_value_count(&compact_roles)
        };
        let mut values = Vec::with_capacity(
            3 + display_parent_link_value_count(self.roles) + compact_value_count,
        );
        values.push(trace_id_value(self.trace_id)?);
        push_display_parent_link_values(&mut values, self.roles)?;
        if !compact_roles.is_empty() {
            values.extend([
                Value::Integer(i64::from(action_kind_code(
                    SemanticActionKind::AgentContextCompact,
                ))),
                Value::Integer(i64::from(link_role_code(
                    SemanticActionLinkRole::AgentContextCompactImplementationLlmCall,
                ))),
                Value::Integer(i64::from(action_kind_code(SemanticActionKind::LlmCall))),
            ]);
            push_display_parent_link_values(&mut values, &compact_roles)?;
        }
        values.push(Value::Integer(i64::from(action_kind_code(
            SemanticActionKind::CommandInvocation,
        ))));
        values.push(Value::Integer(i64::from(action_kind_code(
            SemanticActionKind::CommandInvocation,
        ))));
        Ok(values)
    }
}
