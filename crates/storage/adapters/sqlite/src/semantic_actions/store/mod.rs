//! SQLite storage for semantic actions.

use model_core::ids::TraceId;
use rusqlite::params;
use semantic_action::{
    FileObservationPath, FilePathSetPathPage, FilePathSetWrite, LlmRequestContentPage,
    LlmRequestContentWrite, LlmRequestLineage, LlmRequestLineageWrite, McpJsonRpcContentPage,
    McpJsonRpcContentWrite, SemanticAction, SemanticActionKind, SemanticActionLink,
    SemanticActionPage, SemanticActionReadStore, SemanticActionStoreError, SemanticActionUpdate,
    SemanticActionWriteStore, attr_keys as attrs,
};

use crate::SqliteStorage;
use crate::records::PathInterner;
use crate::semantic_actions::action_ids::{create_action_id, intern_action_id};
use crate::semantic_actions::codebook::sqlite::{link_origin_code, link_role_code};
use crate::semantic_actions::cold_fields::upsert_link_attributes;
use crate::semantic_actions::evidence;
use crate::semantic_actions::path_sets::intern_path;

mod hydration;
mod rows;
mod state;
mod write;

pub(super) use hydration::ActionReadHydrator;
pub(super) use rows::{action_from_row, action_link_from_row, read_action_by_id_shared};
use state::ActionStateWriter;
use write::{write_action_row, write_agent_identity};

pub(super) const ACTION_SELECT_COLUMNS: &str = "ids.action_id AS action_id,
    action.trace_id, action.kind_code, COALESCE(action_state.failure_title, action.title) AS stored_title, action.file_path_id,
    (SELECT path.path_text FROM file_paths path
      WHERE path.trace_id = action.trace_id AND path.path_id = action.file_path_id) AS file_path_text,
    action.start_time, action_state.end_time,
    action.process_id, action_state.status_code,
    action_state.completeness_code,
    action_state.finalization_reason, action_state.failure_body_format,
    action_state.failure_http_status, action_state.failure_http_reason,
    action_state.command_invocation_kind, action_state.command_tool_name, action_state.tool_result_binding,
    action_attrs.encoding_code AS attributes_encoding_code,
    action_attrs.uncompressed_bytes AS attributes_uncompressed_bytes,
    action_attrs.payload AS attributes_payload";

pub(super) const LINK_SELECT_COLUMNS: &str = "link.trace_id,
    parent_ids.action_id AS parent_action_id, child_ids.action_id AS child_action_id,
    link.role_code, link.origin_code, link.valid, link.evidence_blob,
    link_attrs.encoding_code AS attributes_encoding_code,
    link_attrs.uncompressed_bytes AS attributes_uncompressed_bytes,
    link_attrs.payload AS attributes_payload";

pub(super) fn action_cold_field_join() -> &'static str {
    "JOIN semantic_action_state action_state ON action_state.action_key = action.action_key
     LEFT JOIN semantic_action_cold_fields action_attrs
       ON action_attrs.owner_key = action.action_key"
}

pub(super) fn link_cold_field_join() -> &'static str {
    "LEFT JOIN semantic_action_link_cold_fields link_attrs
       ON link_attrs.trace_id = link.trace_id
      AND link_attrs.parent_action_key = link.parent_action_key
      AND link_attrs.child_action_key = link.child_action_key
      AND link_attrs.role_code = link.role_code"
}

impl SemanticActionWriteStore for SqliteStorage {
    fn insert_semantic_action(
        &mut self,
        mut action: SemanticAction,
    ) -> Result<(), SemanticActionStoreError> {
        let mut connection = self.connection().borrow_mut();
        let key = create_action_id(&connection, action.trace_id.get(), &action.action_id)?;
        let path_interner = self.event_path_dictionary().clone();
        let mut paths = path_interner.borrow_mut();
        let stored_path = StoredActionPath::prepare(&connection, &mut paths, &mut action)?;
        let binding = ActionStateWriter::take_initial_binding(&mut action)?;
        ActionReadHydrator::remove_relationship_attributes(&mut action);
        let inserted = write_action_row(
            &mut connection,
            key,
            &action,
            stored_path.path_id,
            stored_path.store_title.then_some(action.title.as_str()),
            &self.cold_field_encoder,
        )?;
        let writer = ActionStateWriter::new(&connection);
        if inserted {
            writer.insert(key, &action, binding)?;
            if action.kind == SemanticActionKind::AgentIdentity {
                write_agent_identity(
                    &mut connection,
                    action.trace_id.get(),
                    action.process.get(),
                    key,
                )?;
            }
        }
        let writer = ActionStateWriter::new(&connection);
        if writer.append_evidence(key, &action.evidence)? && !inserted {
            writer.revise(action.trace_id)?;
        }
        Ok(())
    }

    fn update_semantic_action(
        &mut self,
        update: SemanticActionUpdate,
    ) -> Result<(), SemanticActionStoreError> {
        ActionStateWriter::new(&self.connection().borrow()).update(update)
    }

    fn upsert_semantic_action_link(
        &mut self,
        link: SemanticActionLink,
    ) -> Result<(), SemanticActionStoreError> {
        let mut connection = self.connection().borrow_mut();
        let parent_action_key =
            intern_action_id(&mut connection, link.trace_id.get(), &link.parent_action_id)?;
        let child_action_key =
            intern_action_id(&mut connection, link.trace_id.get(), &link.child_action_id)?;
        let role_code = link_role_code(link.role);
        let evidence_blob = evidence::encode(&link.evidence).map_err(|error| {
            SemanticActionStoreError::new("encode_semantic_action_link_evidence", error.to_string())
        })?;
        connection
            .prepare_cached(
                "INSERT OR REPLACE INTO semantic_action_links (
                    trace_id, parent_action_key, child_action_key, role_code,
                    origin_code, valid, evidence_blob
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .and_then(|mut statement| {
                statement.execute(params![
                    link.trace_id.get(),
                    parent_action_key,
                    child_action_key,
                    role_code,
                    link_origin_code(link.origin),
                    link.valid,
                    evidence_blob,
                ])
            })
            .map_err(|error| {
                SemanticActionStoreError::new("upsert_semantic_action_link", error.to_string())
            })?;
        upsert_link_attributes(
            &mut connection,
            link.trace_id.get(),
            parent_action_key,
            child_action_key,
            role_code,
            &link.attributes,
            &self.cold_field_encoder,
        )
        .map_err(|error| {
            SemanticActionStoreError::new(
                "upsert_semantic_action_link_attributes",
                error.to_string(),
            )
        })?;
        Ok(())
    }

    fn upsert_file_observation_paths(
        &mut self,
        paths: &[FileObservationPath],
    ) -> Result<(), SemanticActionStoreError> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut connection = self.connection().borrow_mut();
        let path_interner = self.event_path_dictionary().clone();
        let mut path_interner = path_interner.borrow_mut();
        let mut interned = Vec::with_capacity(paths.len());
        for path in paths {
            let action_key =
                intern_action_id(&mut connection, path.trace_id.get(), &path.action_id)?;
            let path_id = intern_path(
                &connection,
                &mut path_interner,
                path.trace_id.get(),
                &path.path,
            )?;
            interned.push((path, action_key, path_id));
        }
        let mut statement = connection
            .prepare(
                "INSERT OR IGNORE INTO file_observation_paths (
                    trace_id, action_key, path_order, path_id
                ) VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|error| {
                SemanticActionStoreError::new("prepare_file_observation_paths", error.to_string())
            })?;
        for (path, action_key, path_id) in interned {
            statement
                .execute(params![
                    path.trace_id.get(),
                    action_key,
                    path.path_order,
                    path_id,
                ])
                .map_err(|error| {
                    SemanticActionStoreError::new(
                        "upsert_file_observation_paths",
                        error.to_string(),
                    )
                })?;
        }
        Ok(())
    }

    fn upsert_file_path_sets(
        &mut self,
        path_sets: &[FilePathSetWrite],
    ) -> Result<(), SemanticActionStoreError> {
        if path_sets.is_empty() {
            return Ok(());
        }
        let connection = self.connection().borrow_mut();
        let path_interner = self.event_path_dictionary().clone();
        crate::semantic_actions::path_sets::upsert_file_path_sets(
            &connection,
            &mut path_interner.borrow_mut(),
            path_sets,
        )
    }

    fn upsert_llm_request_contents(
        &mut self,
        contents: &[LlmRequestContentWrite],
    ) -> Result<(), SemanticActionStoreError> {
        if contents.is_empty() {
            return Ok(());
        }
        let connection = self.connection().borrow_mut();
        crate::semantic_actions::llm_request_content::upsert_llm_request_contents(
            &connection,
            contents,
        )
    }

    fn upsert_llm_request_lineages(
        &mut self,
        lineages: &[LlmRequestLineageWrite],
    ) -> Result<(), SemanticActionStoreError> {
        let mut connection = self.connection().borrow_mut();
        crate::semantic_actions::llm_request_lineage::LlmRequestLineageStore::upsert_batch(
            &mut connection,
            lineages,
        )
    }

    fn upsert_mcp_jsonrpc_contents(
        &mut self,
        contents: &[McpJsonRpcContentWrite],
    ) -> Result<(), SemanticActionStoreError> {
        if contents.is_empty() {
            return Ok(());
        }
        let connection = self.connection().borrow_mut();
        crate::semantic_actions::mcp_jsonrpc_content::upsert_mcp_jsonrpc_contents(
            &connection,
            contents,
        )
    }
}

impl SemanticActionReadStore for SqliteStorage {
    fn llm_request_lineage(
        &self,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<Option<LlmRequestLineage>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "read_llm_request_lineage",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        crate::semantic_actions::llm_request_lineage::LlmRequestLineageStore::by_action(
            &connection,
            trace_id,
            action_id,
        )
    }

    fn llm_request_lineages(
        &self,
        trace_id: TraceId,
    ) -> Result<Vec<LlmRequestLineage>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "read_llm_request_lineages",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        crate::semantic_actions::llm_request_lineage::LlmRequestLineageStore::by_trace(
            &connection,
            trace_id,
        )
    }

    fn llm_request_trajectory(
        &self,
        trace_id: TraceId,
        trajectory_id: &str,
    ) -> Result<Vec<LlmRequestLineage>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "read_llm_request_trajectory",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        crate::semantic_actions::llm_request_lineage::LlmRequestLineageStore::by_trajectory(
            &connection,
            trace_id,
            trajectory_id,
        )
    }

    fn llm_request_forks(
        &self,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<Vec<LlmRequestLineage>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "read_llm_request_forks",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        crate::semantic_actions::llm_request_lineage::LlmRequestLineageStore::forks_from(
            &connection,
            trace_id,
            action_id,
        )
    }

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
        let action_cold_join = action_cold_field_join();
        let mut statement = connection
            .prepare(&format!(
                "SELECT {ACTION_SELECT_COLUMNS}
                     FROM semantic_actions action
                     JOIN semantic_action_ids ids
                       ON ids.action_key = action.action_key
                     {action_cold_join}
                     WHERE action.trace_id = ?1
                     ORDER BY action.start_time ASC, ids.action_id ASC"
            ))
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
            let action = row.map_err(|error| {
                SemanticActionStoreError::new("map_semantic_action", error.to_string())
            })?;
            actions.push(action);
        }
        ActionReadHydrator::hydrate(&connection, actions.iter_mut(), true)?;
        Ok(actions)
    }

    fn semantic_actions_page(
        &self,
        trace_id: TraceId,
        offset: usize,
        limit: usize,
    ) -> Result<SemanticActionPage, SemanticActionStoreError> {
        if limit == 0 {
            return Err(SemanticActionStoreError::new(
                "semantic_actions_page",
                "limit must be greater than zero",
            ));
        }
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "semantic_actions_page",
                "trace has been purged",
            ));
        }
        let fetch_limit = limit.checked_add(1).ok_or_else(|| {
            SemanticActionStoreError::new("semantic_actions_page", "limit overflow")
        })?;
        let connection = self.connection().borrow();
        let action_cold_join = action_cold_field_join();
        let mut statement = connection
            .prepare(&format!(
                "SELECT {ACTION_SELECT_COLUMNS}
                 FROM semantic_actions action
                 JOIN semantic_action_ids ids
                   ON ids.action_key = action.action_key
                 {action_cold_join}
                 WHERE action.trace_id = ?1
                 ORDER BY action.start_time ASC, ids.action_id ASC
                 LIMIT ?2 OFFSET ?3"
            ))
            .map_err(|error| {
                SemanticActionStoreError::new("prepare_semantic_actions_page", error.to_string())
            })?;
        let rows = statement
            .query_map(
                params![trace_id.get(), fetch_limit, offset],
                action_from_row,
            )
            .map_err(|error| {
                SemanticActionStoreError::new("query_semantic_actions_page", error.to_string())
            })?;
        let mut actions = rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            SemanticActionStoreError::new("map_semantic_actions_page", error.to_string())
        })?;
        let has_more = actions.len() > limit;
        actions.truncate(limit);
        ActionReadHydrator::hydrate(&connection, actions.iter_mut(), true)?;
        let next_offset = if has_more {
            Some(offset.checked_add(limit).ok_or_else(|| {
                SemanticActionStoreError::new("semantic_actions_page", "offset overflow")
            })?)
        } else {
            None
        };
        Ok(SemanticActionPage {
            actions,
            next_offset,
        })
    }

    fn list_file_observation_paths(
        &self,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<Vec<FileObservationPath>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "list_file_observation_paths",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        let mut statement = connection
            .prepare(
                "SELECT path.trace_id, ids.action_id, path.path_order, dictionary.path_text
                 FROM file_observation_paths path
                 JOIN semantic_action_ids ids ON ids.action_key = path.action_key
                 JOIN file_paths dictionary
                   ON dictionary.trace_id = path.trace_id AND dictionary.path_id = path.path_id
                 WHERE path.trace_id = ?1 AND ids.action_id = ?2
                 ORDER BY path.path_order ASC, dictionary.path_text ASC",
            )
            .map_err(|error| {
                SemanticActionStoreError::new(
                    "prepare_file_observation_path_list",
                    error.to_string(),
                )
            })?;
        let rows = statement
            .query_map(params![trace_id.get(), action_id], |row| {
                Ok(FileObservationPath {
                    trace_id: TraceId::new(row.get(0)?),
                    action_id: row.get(1)?,
                    path_order: row.get(2)?,
                    path: row.get(3)?,
                })
            })
            .map_err(|error| {
                SemanticActionStoreError::new("query_file_observation_path_list", error.to_string())
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            SemanticActionStoreError::new("map_file_observation_path_list", error.to_string())
        })
    }

    fn list_semantic_action_links(
        &self,
        trace_id: TraceId,
    ) -> Result<Vec<SemanticActionLink>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "list_semantic_action_links",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        let link_cold_join = link_cold_field_join();
        let mut statement = connection
            .prepare(&format!(
                "SELECT {LINK_SELECT_COLUMNS}
                     FROM semantic_action_links link
                     JOIN semantic_action_ids parent_ids
                       ON parent_ids.action_key = link.parent_action_key
                     JOIN semantic_action_ids child_ids
                       ON child_ids.action_key = link.child_action_key
                     {link_cold_join}
                     WHERE link.trace_id = ?1
                     ORDER BY parent_ids.action_id ASC, child_ids.action_id ASC, link.role_code ASC"
            ))
            .map_err(|error| {
                SemanticActionStoreError::new("prepare_semantic_action_links", error.to_string())
            })?;
        let rows = statement
            .query_map(params![trace_id.get()], action_link_from_row)
            .map_err(|error| {
                SemanticActionStoreError::new("query_semantic_action_links", error.to_string())
            })?;
        let mut links = Vec::new();
        for row in rows {
            let link = row.map_err(|error| {
                SemanticActionStoreError::new("map_semantic_action_link", error.to_string())
            })?;
            links.push(link);
        }
        Ok(links)
    }

    fn file_path_set_paths_page(
        &self,
        trace_id: TraceId,
        action_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Option<FilePathSetPathPage>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "file_path_set_paths_page",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        crate::semantic_actions::path_sets::file_path_set_paths_page(
            &connection,
            trace_id,
            action_id,
            offset,
            limit,
        )
    }

    fn llm_request_content_page(
        &self,
        trace_id: TraceId,
        action_id: &str,
        max_bytes: usize,
    ) -> Result<Option<LlmRequestContentPage>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "llm_request_content_page",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        crate::semantic_actions::llm_request_content::llm_request_content_page(
            &connection,
            trace_id,
            action_id,
            max_bytes,
        )
    }

    fn mcp_jsonrpc_content_page(
        &self,
        trace_id: TraceId,
        action_id: &str,
        max_bytes: usize,
    ) -> Result<Option<McpJsonRpcContentPage>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "mcp_jsonrpc_content_page",
                "trace has been purged",
            ));
        }
        let connection = self.connection().borrow();
        crate::semantic_actions::mcp_jsonrpc_content::mcp_jsonrpc_content_page(
            &connection,
            trace_id,
            action_id,
            max_bytes,
        )
    }
}

struct StoredActionPath {
    path_id: Option<u64>,
    store_title: bool,
}

impl StoredActionPath {
    fn prepare(
        connection: &rusqlite::Connection,
        paths: &mut PathInterner,
        action: &mut SemanticAction,
    ) -> Result<Self, SemanticActionStoreError> {
        if !matches!(
            action.kind,
            SemanticActionKind::FileModify
                | SemanticActionKind::FileRead
                | SemanticActionKind::FileWrite
        ) {
            return Ok(Self {
                path_id: None,
                store_title: true,
            });
        }
        let Some(path) = action.attributes.remove(attrs::file::PATH) else {
            return Ok(Self {
                path_id: None,
                store_title: true,
            });
        };
        let path_id = intern_path(connection, paths, action.trace_id.get(), &path)?;
        Ok(Self {
            path_id: Some(path_id),
            store_title: action.title != path,
        })
    }
}
