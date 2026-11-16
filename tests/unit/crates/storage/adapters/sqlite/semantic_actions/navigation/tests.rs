// Tests for the corresponding production module.
use semantic_action::{
    SemanticActionCompleteness, SemanticActionKind, SemanticActionLinkOrigin,
    SemanticActionLinkRole, SemanticActionStatus,
};

use super::*;
use crate::semantic_actions::codebook::sqlite::{
    action_completeness_code, action_kind_code, action_status_code, link_origin_code,
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
        .semantic_action_display_path_to_kind(
            trace_id,
            DISPLAY_ROLES,
            DISPLAY_ROLES,
            "llm.call",
            None,
        )
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
                  origin_code, valid)
                 VALUES (?1, 1, ?2, ?3, ?4, 1)",
                params![
                    TRACE_ID,
                    child_key,
                    link_role_code(role),
                    link_origin_code(SemanticActionLinkOrigin::Observed)
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
