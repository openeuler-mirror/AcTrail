//! SQLite storage for semantic actions.

use std::collections::BTreeMap;

use model_core::ids::TraceId;
use rusqlite::{OptionalExtension, params};
use semantic_action::{
    FileObservationPath, FilePathSetPathPage, FilePathSetWrite, LlmRequestContentPage,
    LlmRequestContentWrite, SemanticAction, SemanticActionLink, SemanticActionReadStore,
    SemanticActionStoreError, SemanticActionWriteStore, SemanticEvidence,
};

use crate::SqliteStorage;
use crate::records::encode_map;
use crate::semantic_actions::action_ids::{intern_action_id, require_action_key};
use crate::semantic_actions::codebook::sqlite::{
    LinkEvidenceKey, evidence_kind_code, link_confidence_code, link_role_code,
};
use crate::semantic_actions::cold_fields::upsert_link_attributes;
use crate::semantic_actions::storage_meta::current;
use crate::semantic_actions::upsert_merge::merge_action;

mod rows;
mod write;

use rows::action_link_from_row;
pub(super) use rows::{action_from_row, evidence_from_row};
use write::{action_row_matches, link_valid_code, replace_action_evidence, write_action_row};

pub(super) const ACTION_SELECT_COLUMNS: &str = "ids.action_id AS action_id,
    action.trace_id, action.kind_code, action.title, action.start_time, action.end_time,
    action.process_id, action.status_code,
    action.completeness_code, action.confidence_millis, action.attributes AS legacy_attributes,
    action_attrs.encoding_code AS attributes_encoding_code,
    action_attrs.uncompressed_bytes AS attributes_uncompressed_bytes,
    action_attrs.value_hash AS attributes_value_hash,
    action_attrs.payload AS attributes_payload";

pub(super) const LINK_SELECT_COLUMNS: &str = "link.trace_id,
    parent_ids.action_id AS parent_action_id, child_ids.action_id AS child_action_id,
    link.role_code, link.confidence_code, link.valid, link.attributes AS legacy_attributes,
    link_attrs.encoding_code AS attributes_encoding_code,
    link_attrs.uncompressed_bytes AS attributes_uncompressed_bytes,
    link_attrs.value_hash AS attributes_value_hash,
    link_attrs.payload AS attributes_payload";

pub(super) fn action_cold_field_join() -> String {
    format!(
        "LEFT JOIN semantic_action_cold_fields action_attrs
           ON action_attrs.owner_key = action.action_key
          AND action_attrs.field_code = {}",
        current().cold_fields.action_attributes
    )
}

pub(super) fn link_cold_field_join() -> String {
    format!(
        "LEFT JOIN semantic_action_link_cold_fields link_attrs
           ON link_attrs.trace_id = link.trace_id
          AND link_attrs.parent_action_key = link.parent_action_key
          AND link_attrs.child_action_key = link.child_action_key
          AND link_attrs.role_code = link.role_code
          AND link_attrs.field_code = {}",
        current().cold_fields.link_attributes
    )
}

impl SemanticActionWriteStore for SqliteStorage {
    fn upsert_semantic_action(
        &mut self,
        mut action: SemanticAction,
    ) -> Result<(), SemanticActionStoreError> {
        let connection = self.connection().borrow_mut();
        let existing = read_action_by_id(&connection, &action.action_id)?;
        if let Some(existing) = existing.as_ref() {
            action = merge_action(existing.clone(), action)?;
        }
        let row_changed = existing
            .as_ref()
            .is_none_or(|existing| !action_row_matches(existing, &action));
        let evidence_changed = existing
            .as_ref()
            .is_none_or(|existing| existing.evidence != action.evidence);
        if !row_changed && !evidence_changed {
            return Ok(());
        }
        if row_changed {
            let action_key =
                intern_action_id(&connection, action.trace_id.get(), &action.action_id)?;
            write_action_row(&connection, action_key, &action)?;
        }
        if evidence_changed {
            replace_action_evidence(&connection, &action)?;
        }
        Ok(())
    }

    fn upsert_semantic_action_link(
        &mut self,
        link: SemanticActionLink,
    ) -> Result<(), SemanticActionStoreError> {
        let connection = self.connection().borrow_mut();
        let parent_action_key =
            intern_action_id(&connection, link.trace_id.get(), &link.parent_action_id)?;
        let child_action_key =
            intern_action_id(&connection, link.trace_id.get(), &link.child_action_id)?;
        let role_code = link_role_code(link.role);
        let attributes = encode_map(&link.attributes);
        connection
            .execute(
                "INSERT OR REPLACE INTO semantic_action_links (
                    trace_id, parent_action_key, child_action_key, role_code,
                    confidence_code, valid, link_valid_code, attributes
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    link.trace_id.get(),
                    parent_action_key,
                    child_action_key,
                    role_code,
                    link_confidence_code(link.confidence),
                    link.valid,
                    link_valid_code(&link),
                    "",
                ],
            )
            .map_err(|error| {
                SemanticActionStoreError::new("upsert_semantic_action_link", error.to_string())
            })?;
        connection
            .execute(
                "DELETE FROM semantic_action_link_evidence
                 WHERE trace_id = ?1
                 AND parent_action_key = ?2
                 AND child_action_key = ?3
                 AND role_code = ?4",
                params![
                    link.trace_id.get(),
                    parent_action_key,
                    child_action_key,
                    role_code,
                ],
            )
            .map_err(|error| {
                SemanticActionStoreError::new(
                    "replace_semantic_action_link_evidence",
                    error.to_string(),
                )
            })?;
        for (index, evidence) in link.evidence.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO semantic_action_link_evidence (
                        trace_id, parent_action_key, child_action_key, role_code, evidence_order,
                        kind_code, evidence_id, evidence_role
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        link.trace_id.get(),
                        parent_action_key,
                        child_action_key,
                        role_code,
                        index,
                        evidence_kind_code(evidence.kind),
                        evidence.id,
                        &evidence.role,
                    ],
                )
                .map_err(|error| {
                    SemanticActionStoreError::new(
                        "insert_semantic_action_link_evidence",
                        error.to_string(),
                    )
                })?;
        }
        upsert_link_attributes(
            &connection,
            link.trace_id.get(),
            parent_action_key,
            child_action_key,
            role_code,
            &attributes,
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
        let connection = self.connection().borrow_mut();
        let mut statement = connection
            .prepare(
                "INSERT OR IGNORE INTO file_observation_paths (
                    trace_id, action_key, path_order, path
                ) VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|error| {
                SemanticActionStoreError::new("prepare_file_observation_paths", error.to_string())
            })?;
        for path in paths {
            let action_key = intern_action_id(&connection, path.trace_id.get(), &path.action_id)?;
            statement
                .execute(params![
                    path.trace_id.get(),
                    action_key,
                    path.path_order,
                    &path.path,
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
        crate::semantic_actions::path_sets::upsert_file_path_sets(&connection, path_sets)
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
        let mut evidence = read_evidence_for_trace(&connection, trace_id)?;
        for action in &mut actions {
            action.evidence = evidence.remove(&action.action_id).unwrap_or_default();
        }
        Ok(actions)
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
        let mut evidence = read_link_evidence_for_trace(&connection, trace_id)?;
        for link in &mut links {
            link.evidence = evidence
                .remove(&LinkEvidenceKey::from_link(link))
                .unwrap_or_default();
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
}

pub(super) fn read_evidence(
    connection: &rusqlite::Connection,
    action_id: &str,
) -> Result<Vec<SemanticEvidence>, SemanticActionStoreError> {
    let action_key = require_action_key(connection, action_id)?;
    let mut statement = connection
        .prepare(
            "SELECT kind_code, evidence_id, role FROM semantic_action_evidence
             WHERE action_key = ?1
             ORDER BY evidence_order ASC",
        )
        .map_err(|error| {
            SemanticActionStoreError::new("prepare_semantic_action_evidence", error.to_string())
        })?;
    let rows = statement
        .query_map(params![action_key], evidence_from_row)
        .map_err(|error| {
            SemanticActionStoreError::new("query_semantic_action_evidence", error.to_string())
        })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
        SemanticActionStoreError::new("map_semantic_action_evidence", error.to_string())
    })
}

fn read_evidence_for_trace(
    connection: &rusqlite::Connection,
    trace_id: TraceId,
) -> Result<BTreeMap<String, Vec<SemanticEvidence>>, SemanticActionStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT ids.action_id, evidence.kind_code, evidence.evidence_id, evidence.role
             FROM semantic_action_evidence evidence
             JOIN semantic_actions action
               ON action.action_key = evidence.action_key
             JOIN semantic_action_ids ids
               ON ids.action_key = evidence.action_key
             WHERE action.trace_id = ?1
             ORDER BY ids.action_id ASC, evidence.evidence_order ASC",
        )
        .map_err(|error| {
            SemanticActionStoreError::new(
                "prepare_semantic_action_evidence_trace",
                error.to_string(),
            )
        })?;
    let rows = statement
        .query_map(params![trace_id.get()], |row| {
            Ok((row.get::<_, String>("action_id")?, evidence_from_row(row)?))
        })
        .map_err(|error| {
            SemanticActionStoreError::new("query_semantic_action_evidence_trace", error.to_string())
        })?;
    let mut evidence = BTreeMap::<String, Vec<SemanticEvidence>>::new();
    for row in rows {
        let (action_id, item) = row.map_err(|error| {
            SemanticActionStoreError::new("map_semantic_action_evidence_trace", error.to_string())
        })?;
        evidence.entry(action_id).or_default().push(item);
    }
    Ok(evidence)
}

pub(super) fn read_action_by_id(
    connection: &rusqlite::Connection,
    action_id: &str,
) -> Result<Option<SemanticAction>, SemanticActionStoreError> {
    let action_cold_join = action_cold_field_join();
    let mut action = connection
        .query_row(
            &format!(
                "SELECT {ACTION_SELECT_COLUMNS}
                 FROM semantic_actions action
                 JOIN semantic_action_ids ids
                   ON ids.action_key = action.action_key
                 {action_cold_join}
                 WHERE ids.action_id = ?1"
            ),
            params![action_id],
            action_from_row,
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new("read_existing_semantic_action", error.to_string())
        })?;
    if let Some(action) = &mut action {
        action.evidence = read_evidence(connection, &action.action_id)?;
    }
    Ok(action)
}

pub(super) fn read_link_evidence(
    connection: &rusqlite::Connection,
    link: &SemanticActionLink,
) -> Result<Vec<SemanticEvidence>, SemanticActionStoreError> {
    let parent_action_key = require_action_key(connection, &link.parent_action_id)?;
    let child_action_key = require_action_key(connection, &link.child_action_id)?;
    let mut statement = connection
        .prepare(
            "SELECT kind_code, evidence_id, evidence_role FROM semantic_action_link_evidence
             WHERE trace_id = ?1
             AND parent_action_key = ?2
             AND child_action_key = ?3
             AND role_code = ?4
             ORDER BY evidence_order ASC",
        )
        .map_err(|error| {
            SemanticActionStoreError::new(
                "prepare_semantic_action_link_evidence",
                error.to_string(),
            )
        })?;
    let rows = statement
        .query_map(
            params![
                link.trace_id.get(),
                parent_action_key,
                child_action_key,
                link_role_code(link.role),
            ],
            evidence_from_row,
        )
        .map_err(|error| {
            SemanticActionStoreError::new("query_semantic_action_link_evidence", error.to_string())
        })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
        SemanticActionStoreError::new("map_semantic_action_link_evidence", error.to_string())
    })
}

fn read_link_evidence_for_trace(
    connection: &rusqlite::Connection,
    trace_id: TraceId,
) -> Result<BTreeMap<LinkEvidenceKey, Vec<SemanticEvidence>>, SemanticActionStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT evidence.trace_id,
                    parent_ids.action_id AS parent_action_id,
                    child_ids.action_id AS child_action_id,
                    evidence.role_code AS link_role_code,
                    kind_code, evidence_id, evidence_role
             FROM semantic_action_link_evidence evidence
             JOIN semantic_action_ids parent_ids
               ON parent_ids.action_key = evidence.parent_action_key
             JOIN semantic_action_ids child_ids
               ON child_ids.action_key = evidence.child_action_key
             WHERE evidence.trace_id = ?1
             ORDER BY parent_ids.action_id ASC, child_ids.action_id ASC, link_role_code ASC, evidence.evidence_order ASC",
        )
        .map_err(|error| {
            SemanticActionStoreError::new(
                "prepare_semantic_action_link_evidence_trace",
                error.to_string(),
            )
        })?;
    let rows = statement
        .query_map(params![trace_id.get()], |row| {
            Ok((LinkEvidenceKey::from_row(row)?, evidence_from_row(row)?))
        })
        .map_err(|error| {
            SemanticActionStoreError::new(
                "query_semantic_action_link_evidence_trace",
                error.to_string(),
            )
        })?;
    let mut evidence = BTreeMap::<LinkEvidenceKey, Vec<SemanticEvidence>>::new();
    for row in rows {
        let (key, item) = row.map_err(|error| {
            SemanticActionStoreError::new(
                "map_semantic_action_link_evidence_trace",
                error.to_string(),
            )
        })?;
        evidence.entry(key).or_default().push(item);
    }
    Ok(evidence)
}
