//! Indexed navigation through the semantic-action display tree.

use std::collections::BTreeSet;

use model_core::ids::TraceId;
use rusqlite::{Connection, OptionalExtension, params};
use semantic_action::{SemanticActionKind, SemanticActionStoreError};
use storage_core::SemanticActionDisplayPathEntry;

use crate::SqliteStorage;
use crate::semantic_actions::codebook::sqlite::{
    action_kind_code_from_str, decode_kind, link_role_code_from_str,
};

#[derive(Clone, Debug)]
struct ActionPosition {
    action_key: i64,
    action_id: String,
    start_time: i64,
    kind: SemanticActionKind,
}

impl SqliteStorage {
    pub fn semantic_action_display_path_to_kind(
        &self,
        trace_id: TraceId,
        display_parent_roles: &[&str],
        target_kind: &str,
        after_action_id: Option<&str>,
    ) -> Result<Option<Vec<SemanticActionDisplayPathEntry>>, SemanticActionStoreError> {
        if self.is_purged(trace_id) {
            return Err(SemanticActionStoreError::new(
                "semantic_action_display_path_to_kind",
                "trace has been purged",
            ));
        }
        if display_parent_roles.is_empty() {
            return Ok(None);
        }
        let connection = self.connection().borrow();
        let target = navigation_target(
            &connection,
            trace_id,
            target_kind,
            after_action_id.filter(|value| !value.is_empty()),
        )?;
        let Some(mut current) = target else {
            return Ok(None);
        };

        let mut reverse_path = Vec::new();
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current.action_key) {
                return Err(SemanticActionStoreError::new(
                    "semantic_action_display_path_to_kind",
                    "cycle in semantic action display parents",
                ));
            }
            let parent = selected_parent(
                &connection,
                trace_id,
                current.action_key,
                display_parent_roles,
            )?;
            let offset = sibling_offset(
                &connection,
                trace_id,
                parent.as_ref().map(|value| value.action_key),
                &current,
                display_parent_roles,
            )?;
            reverse_path.push(SemanticActionDisplayPathEntry {
                parent_action_id: parent.as_ref().map(|value| value.action_id.clone()),
                action_id: current.action_id,
                offset,
                kind: current.kind,
            });
            let Some(parent) = parent else {
                break;
            };
            current = parent;
        }
        reverse_path.reverse();
        Ok(Some(reverse_path))
    }
}

fn navigation_target(
    connection: &Connection,
    trace_id: TraceId,
    target_kind: &str,
    after_action_id: Option<&str>,
) -> Result<Option<ActionPosition>, SemanticActionStoreError> {
    let kind_code = i64::from(action_kind_code_from_str(target_kind)?);
    let anchor = match after_action_id {
        Some(action_id) => action_position_by_id(connection, trace_id, action_id)?,
        None => None,
    };
    let (after_start, after_id) = anchor
        .as_ref()
        .map(|value| (Some(value.start_time), Some(value.action_id.as_str())))
        .unwrap_or((None, None));
    let mut statement = connection
        .prepare(
            "SELECT action.action_key, ids.action_id, action.start_time, action.kind_code
             FROM semantic_actions action
             JOIN semantic_action_ids ids ON ids.action_key = action.action_key
             WHERE action.trace_id = ?1
               AND action.kind_code = ?2
               AND action.action_valid_code = 1
               AND (?3 IS NULL OR action.start_time > ?3
                    OR (action.start_time = ?3 AND ids.action_id > ?4))
             ORDER BY action.start_time ASC, ids.action_id ASC
             LIMIT 1",
        )
        .map_err(|error| {
            SemanticActionStoreError::new(
                "prepare_semantic_action_navigation_target",
                error.to_string(),
            )
        })?;
    statement
        .query_row(
            params![trace_id.get(), kind_code, after_start, after_id],
            action_position_from_row,
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new(
                "query_semantic_action_navigation_target",
                error.to_string(),
            )
        })
}

fn action_position_by_id(
    connection: &Connection,
    trace_id: TraceId,
    action_id: &str,
) -> Result<Option<ActionPosition>, SemanticActionStoreError> {
    connection
        .query_row(
            "SELECT action.action_key, ids.action_id, action.start_time, action.kind_code
             FROM semantic_actions action
             JOIN semantic_action_ids ids ON ids.action_key = action.action_key
             WHERE action.trace_id = ?1 AND ids.action_id = ?2
             LIMIT 1",
            params![trace_id.get(), action_id],
            action_position_from_row,
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new(
                "query_semantic_action_navigation_anchor",
                error.to_string(),
            )
        })
}

fn selected_parent(
    connection: &Connection,
    trace_id: TraceId,
    child_action_key: i64,
    roles: &[&str],
) -> Result<Option<ActionPosition>, SemanticActionStoreError> {
    let role_codes = role_codes(roles)?;
    let query = format!(
        "SELECT parent.action_key, parent_ids.action_id, parent.start_time, parent.kind_code
         FROM semantic_action_links link
         JOIN semantic_actions parent ON parent.action_key = link.parent_action_key
         JOIN semantic_action_ids parent_ids ON parent_ids.action_key = parent.action_key
         WHERE link.trace_id = ?1
           AND link.child_action_key = ?2
           AND link.link_valid_code = 1
           AND parent.action_valid_code = 1
           AND link.role_code IN ({})
         ORDER BY {} ASC, parent.start_time ASC, parent_ids.action_id ASC
         LIMIT 1",
        sql_integers(&role_codes),
        role_rank_sql("link.role_code", &role_codes),
    );
    connection
        .query_row(
            &query,
            params![trace_id.get(), child_action_key],
            action_position_from_row,
        )
        .optional()
        .map_err(|error| {
            SemanticActionStoreError::new(
                "query_semantic_action_navigation_parent",
                error.to_string(),
            )
        })
}

fn sibling_offset(
    connection: &Connection,
    trace_id: TraceId,
    parent_action_key: Option<i64>,
    child: &ActionPosition,
    roles: &[&str],
) -> Result<usize, SemanticActionStoreError> {
    let role_codes = role_codes(roles)?;
    let count: i64 = if let Some(parent_action_key) = parent_action_key {
        let query = format!(
            "SELECT COUNT(DISTINCT candidate.action_key)
             FROM semantic_action_links link
             JOIN semantic_actions candidate ON candidate.action_key = link.child_action_key
             JOIN semantic_action_ids candidate_ids ON candidate_ids.action_key = candidate.action_key
             WHERE link.trace_id = ?1
               AND link.parent_action_key = ?2
               AND link.link_valid_code = 1
               AND candidate.action_valid_code = 1
               AND link.role_code IN ({})
               AND (candidate.start_time < ?3
                    OR (candidate.start_time = ?3 AND candidate_ids.action_id < ?4))",
            sql_integers(&role_codes),
        );
        connection.query_row(
            &query,
            params![
                trace_id.get(),
                parent_action_key,
                child.start_time,
                child.action_id
            ],
            |row| row.get(0),
        )
    } else {
        let query = format!(
            "SELECT COUNT(*)
             FROM semantic_actions candidate
             JOIN semantic_action_ids candidate_ids ON candidate_ids.action_key = candidate.action_key
             WHERE candidate.trace_id = ?1
               AND candidate.action_valid_code = 1
               AND (candidate.start_time < ?2
                    OR (candidate.start_time = ?2 AND candidate_ids.action_id < ?3))
               AND NOT EXISTS (
                 SELECT 1 FROM semantic_action_links incoming
                 JOIN semantic_actions parent ON parent.action_key = incoming.parent_action_key
                 WHERE incoming.trace_id = candidate.trace_id
                   AND incoming.child_action_key = candidate.action_key
                   AND incoming.link_valid_code = 1
                   AND parent.action_valid_code = 1
                   AND incoming.role_code IN ({})
               )",
            sql_integers(&role_codes),
        );
        connection.query_row(
            &query,
            params![trace_id.get(), child.start_time, child.action_id],
            |row| row.get(0),
        )
    }
    .map_err(|error| {
        SemanticActionStoreError::new(
            "query_semantic_action_navigation_offset",
            error.to_string(),
        )
    })?;
    usize::try_from(count).map_err(|error| {
        SemanticActionStoreError::new("map_semantic_action_navigation_offset", error.to_string())
    })
}

fn action_position_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActionPosition> {
    Ok(ActionPosition {
        action_key: row.get(0)?,
        action_id: row.get(1)?,
        start_time: row.get(2)?,
        kind: decode_kind(row.get(3)?)?,
    })
}

fn role_codes(roles: &[&str]) -> Result<Vec<i16>, SemanticActionStoreError> {
    roles
        .iter()
        .map(|role| link_role_code_from_str(role))
        .collect()
}

fn role_rank_sql(column: &str, role_codes: &[i16]) -> String {
    let cases = role_codes
        .iter()
        .enumerate()
        .map(|(rank, code)| format!("WHEN {code} THEN {rank}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("CASE {column} {cases} ELSE {} END", role_codes.len())
}

fn sql_integers(values: &[i16]) -> String {
    values
        .iter()
        .map(i16::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use semantic_action::{
        SemanticActionCompleteness, SemanticActionKind, SemanticActionLinkConfidence,
        SemanticActionLinkRole, SemanticActionStatus,
    };

    use super::*;
    use crate::semantic_actions::codebook::sqlite::{
        action_completeness_code, action_kind_code, action_status_code, link_confidence_code,
        link_role_code,
    };

    const TRACE_ID: u64 = 41;
    const DISPLAY_ROLES: &[&str] = &["command.contains_file_access", "command.contains_llm_call"];

    #[test]
    fn finds_first_and_next_llm_without_loading_the_trace() {
        let storage = SqliteStorage::open_in_memory().expect("open storage");
        seed_navigation_tree(&storage);
        let trace_id = TraceId::new(TRACE_ID);

        let llm_links = storage
            .semantic_action_links_matching_roles(
                trace_id,
                &[SemanticActionLinkRole::CommandContainsLlmCall.as_str()],
            )
            .expect("read selected links");
        assert_eq!(llm_links.len(), 2);
        assert!(
            llm_links
                .iter()
                .all(|link| { link.role == SemanticActionLinkRole::CommandContainsLlmCall })
        );

        let first = storage
            .semantic_action_display_path_to_kind(
                trace_id,
                DISPLAY_ROLES,
                SemanticActionKind::LlmCall.as_str(),
                None,
            )
            .expect("find first")
            .expect("first path");
        assert_eq!(
            path_shape(&first),
            vec![
                (None, "command", 0, SemanticActionKind::CommandInvocation),
                (Some("command"), "llm-a", 1, SemanticActionKind::LlmCall),
            ]
        );

        let next = storage
            .semantic_action_display_path_to_kind(
                trace_id,
                DISPLAY_ROLES,
                SemanticActionKind::LlmCall.as_str(),
                Some("llm-a"),
            )
            .expect("find next")
            .expect("next path");
        assert_eq!(
            path_shape(&next),
            vec![
                (None, "command", 0, SemanticActionKind::CommandInvocation),
                (Some("command"), "llm-b", 2, SemanticActionKind::LlmCall),
            ]
        );

        let missing_anchor = storage
            .semantic_action_display_path_to_kind(
                trace_id,
                DISPLAY_ROLES,
                SemanticActionKind::LlmCall.as_str(),
                Some("missing"),
            )
            .expect("fallback from missing anchor")
            .expect("fallback path");
        assert_eq!(
            missing_anchor.last().map(|entry| entry.action_id.as_str()),
            Some("llm-a")
        );

        let exhausted = storage
            .semantic_action_display_path_to_kind(
                trace_id,
                DISPLAY_ROLES,
                SemanticActionKind::LlmCall.as_str(),
                Some("llm-b"),
            )
            .expect("find after last");
        assert!(exhausted.is_none());
    }

    #[test]
    fn llm_target_query_uses_trace_kind_start_index() {
        let storage = SqliteStorage::open_in_memory().expect("open storage");
        let connection = storage.connection().borrow();
        let kind_code = action_kind_code(SemanticActionKind::LlmCall);
        let mut statement = connection
            .prepare(
                "EXPLAIN QUERY PLAN
                 SELECT action.action_key
                 FROM semantic_actions action
                 WHERE action.trace_id = ?1 AND action.kind_code = ?2
                 ORDER BY action.start_time ASC, action.action_key ASC
                 LIMIT 1",
            )
            .expect("prepare query plan");
        let details = statement
            .query_map(params![TRACE_ID, kind_code], |row| row.get::<_, String>(3))
            .expect("query plan")
            .collect::<Result<Vec<_>, _>>()
            .expect("map query plan");
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("idx_semantic_actions_trace_kind_start")),
            "query plan did not use trace/kind/start index: {details:?}"
        );
    }

    #[test]
    fn finds_llm_among_fifty_thousand_noise_actions_without_materializing_noise() {
        const NOISE_COUNT: i64 = 50_000;
        let storage = SqliteStorage::open_in_memory().expect("open storage");
        {
            let connection = storage.connection().borrow();
            connection
                .execute(
                    "WITH RECURSIVE sequence(value) AS (
                       SELECT 1
                       UNION ALL
                       SELECT value + 1 FROM sequence WHERE value < ?1
                     )
                     INSERT INTO semantic_action_ids
                       (action_key, trace_id, action_id, action_id_hash)
                     SELECT value, ?2, 'noise-' || value, CAST(value AS BLOB)
                     FROM sequence",
                    params![NOISE_COUNT, TRACE_ID],
                )
                .expect("insert noise ids");
            connection
                .execute(
                    "WITH RECURSIVE sequence(value) AS (
                       SELECT 1
                       UNION ALL
                       SELECT value + 1 FROM sequence WHERE value < ?1
                     )
                     INSERT INTO semantic_actions
                       (action_key, trace_id, kind_code, title, start_time, process_id,
                        status_code, completeness_code, action_valid_code, process_parent_conflict)
                     SELECT value, ?2, ?3, 'noise', value, 1, ?4, ?5, 1, 0
                     FROM sequence",
                    params![
                        NOISE_COUNT,
                        TRACE_ID,
                        action_kind_code(SemanticActionKind::FileRead),
                        action_status_code(SemanticActionStatus::Success),
                        action_completeness_code(SemanticActionCompleteness::Complete)
                    ],
                )
                .expect("insert noise actions");
            let target_key = NOISE_COUNT + 1;
            connection
                .execute(
                    "INSERT INTO semantic_action_ids
                     (action_key, trace_id, action_id, action_id_hash)
                     VALUES (?1, ?2, 'target-llm', ?3)",
                    params![target_key, TRACE_ID, vec![0xff_u8]],
                )
                .expect("insert target id");
            connection
                .execute(
                    "INSERT INTO semantic_actions
                     (action_key, trace_id, kind_code, title, start_time, process_id,
                      status_code, completeness_code, action_valid_code, process_parent_conflict)
                     VALUES (?1, ?2, ?3, 'target', ?1, 1, ?4, ?5, 1, 0)",
                    params![
                        target_key,
                        TRACE_ID,
                        action_kind_code(SemanticActionKind::LlmCall),
                        action_status_code(SemanticActionStatus::Success),
                        action_completeness_code(SemanticActionCompleteness::Complete)
                    ],
                )
                .expect("insert target action");
        }

        let trace_id = TraceId::new(TRACE_ID);
        let selected = storage
            .semantic_actions_matching_kinds_lite(trace_id, &["llm.call"])
            .expect("select LLM actions");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].action_id, "target-llm");

        let path = storage
            .semantic_action_display_path_to_kind(trace_id, DISPLAY_ROLES, "llm.call", None)
            .expect("navigate")
            .expect("target path");
        assert_eq!(path.len(), 1);
        assert_eq!(path[0].action_id, "target-llm");
        assert_eq!(path[0].offset, NOISE_COUNT as usize);
    }

    fn seed_navigation_tree(storage: &SqliteStorage) {
        let connection = storage.connection().borrow();
        let actions = [
            (
                1_i64,
                "command",
                10_i64,
                SemanticActionKind::CommandInvocation,
            ),
            (2, "file", 15, SemanticActionKind::FileRead),
            (3, "llm-a", 20, SemanticActionKind::LlmCall),
            (4, "llm-b", 30, SemanticActionKind::LlmCall),
        ];
        for (key, action_id, start_time, kind) in actions {
            connection
                .execute(
                    "INSERT INTO semantic_action_ids
                     (action_key, trace_id, action_id, action_id_hash)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![key, TRACE_ID, action_id, vec![key as u8]],
                )
                .expect("insert action id");
            connection
                .execute(
                    "INSERT INTO semantic_actions
                     (action_key, trace_id, kind_code, title, start_time, process_id,
                      status_code, completeness_code, action_valid_code, process_parent_conflict)
                     VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, 1, 0)",
                    params![
                        key,
                        TRACE_ID,
                        action_kind_code(kind),
                        action_id,
                        start_time,
                        action_status_code(SemanticActionStatus::Success),
                        action_completeness_code(SemanticActionCompleteness::Complete)
                    ],
                )
                .expect("insert action");
        }
        for (child_key, role) in [
            (2_i64, SemanticActionLinkRole::CommandContainsFileAccess),
            (3, SemanticActionLinkRole::CommandContainsLlmCall),
            (4, SemanticActionLinkRole::CommandContainsLlmCall),
        ] {
            connection
                .execute(
                    "INSERT INTO semantic_action_links
                     (trace_id, parent_action_key, child_action_key, role_code,
                      confidence_code, valid, link_valid_code)
                     VALUES (?1, 1, ?2, ?3, ?4, 1, 1)",
                    params![
                        TRACE_ID,
                        child_key,
                        link_role_code(role),
                        link_confidence_code(SemanticActionLinkConfidence::Observed)
                    ],
                )
                .expect("insert link");
        }
    }

    fn path_shape(
        path: &[SemanticActionDisplayPathEntry],
    ) -> Vec<(Option<&str>, &str, usize, SemanticActionKind)> {
        path.iter()
            .map(|entry| {
                (
                    entry.parent_action_id.as_deref(),
                    entry.action_id.as_str(),
                    entry.offset,
                    entry.kind,
                )
            })
            .collect()
    }
}
