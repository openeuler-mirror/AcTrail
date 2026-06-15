use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionStatus,
    SemanticActionWriteStore,
};

use crate::SqliteStorage;
use crate::semantic_actions::SemanticActionChildPageQuery;

const DISPLAY_PARENT_ROLES: &[&str] = &[
    "command.contains_process_exec",
    "command.contains_file_access",
    "command.contains_llm_call",
    "command.contains_command_invocation",
];
const ROOT_LINK_ROLES: &[&str] = &["agent.performed_action"];

#[test]
fn display_root_children_are_paged_without_full_projection_semantics_leaking() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(1);
    let agent_process = ProcessIdentity::new(100, 1, 1);
    let command_process = ProcessIdentity::new(101, 2, 1);

    write_action(
        &mut storage,
        action(
            trace_id,
            "agent",
            SemanticActionKind::ProcessExec,
            agent_process,
            1,
        ),
    );
    let mut command = action(
        trace_id,
        "command",
        SemanticActionKind::CommandInvocation,
        command_process.clone(),
        2,
    );
    command.end_time = None;
    write_action(&mut storage, command);
    write_action(
        &mut storage,
        action(
            trace_id,
            "exec",
            SemanticActionKind::ProcessExec,
            command_process.clone(),
            3,
        ),
    );
    write_action(
        &mut storage,
        action(
            trace_id,
            "fallback-http",
            SemanticActionKind::HttpMessage,
            command_process.clone(),
            4,
        ),
    );
    let mut invalid = action(
        trace_id,
        "invalid-root",
        SemanticActionKind::HttpMessage,
        command_process,
        5,
    );
    invalid
        .attributes
        .insert("actrail.action.valid".to_string(), "false".to_string());
    write_action(&mut storage, invalid);
    write_link(
        &mut storage,
        link(
            trace_id,
            "agent",
            "command",
            SemanticActionLinkRole::AgentPerformedAction,
        ),
    );
    write_link(
        &mut storage,
        link(
            trace_id,
            "command",
            "exec",
            SemanticActionLinkRole::CommandContainsProcessExec,
        ),
    );

    assert_eq!(
        storage
            .semantic_action_display_root_child_count(trace_id, DISPLAY_PARENT_ROLES)
            .expect("count display root children"),
        2
    );

    let first_page = storage
        .semantic_action_display_root_children_page(
            trace_id,
            DISPLAY_PARENT_ROLES,
            ROOT_LINK_ROLES,
            SemanticActionChildPageQuery {
                offset: 0,
                limit: 1,
            },
        )
        .expect("read first root page");
    assert_eq!(first_page.total_count, 2);
    assert_eq!(action_ids(&first_page.rows), vec!["agent"]);
    assert_eq!(first_page.rows[0].child_count, 0);
    assert!(first_page.rows[0].root_link.is_none());

    let second_page = storage
        .semantic_action_display_root_children_page(
            trace_id,
            DISPLAY_PARENT_ROLES,
            ROOT_LINK_ROLES,
            SemanticActionChildPageQuery {
                offset: 1,
                limit: 1,
            },
        )
        .expect("read second root page");
    assert_eq!(second_page.total_count, 2);
    assert_eq!(action_ids(&second_page.rows), vec!["command"]);
    assert_eq!(second_page.rows[0].child_count, 2);
    assert_eq!(
        second_page.rows[0].root_link.as_ref().map(|link| link.role),
        Some(SemanticActionLinkRole::AgentPerformedAction)
    );
}

#[test]
fn invalid_display_parent_link_does_not_remove_action_from_display_root() {
    let mut storage = SqliteStorage::open_in_memory().expect("open in-memory sqlite storage");
    let trace_id = TraceId::new(2);
    let process = ProcessIdentity::new(200, 1, 1);
    write_action(
        &mut storage,
        action(
            trace_id,
            "command",
            SemanticActionKind::CommandInvocation,
            process.clone(),
            1,
        ),
    );
    write_action(
        &mut storage,
        action(
            trace_id,
            "exec",
            SemanticActionKind::ProcessExec,
            process,
            3,
        ),
    );
    let mut invalid_link = link(
        trace_id,
        "command",
        "exec",
        SemanticActionLinkRole::CommandContainsProcessExec,
    );
    invalid_link
        .attributes
        .insert("actrail.link.valid".to_string(), "false".to_string());
    write_link(&mut storage, invalid_link);

    let page = storage
        .semantic_action_display_root_children_page(
            trace_id,
            DISPLAY_PARENT_ROLES,
            ROOT_LINK_ROLES,
            SemanticActionChildPageQuery {
                offset: 0,
                limit: 10,
            },
        )
        .expect("read root children");

    assert_eq!(action_ids(&page.rows), vec!["command", "exec"]);
}

fn write_action(storage: &mut SqliteStorage, action: SemanticAction) {
    storage
        .upsert_semantic_action(action)
        .expect("write semantic action");
}

fn write_link(storage: &mut SqliteStorage, link: SemanticActionLink) {
    storage
        .upsert_semantic_action_link(link)
        .expect("write semantic action link");
}

fn action(
    trace_id: TraceId,
    action_id: &str,
    kind: SemanticActionKind,
    process: ProcessIdentity,
    start_millis: u64,
) -> SemanticAction {
    SemanticAction {
        action_id: action_id.to_string(),
        trace_id,
        kind,
        title: action_id.to_string(),
        start_time: UNIX_EPOCH + Duration::from_millis(start_millis),
        end_time: Some(UNIX_EPOCH + Duration::from_millis(start_millis + 1)),
        process,
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        confidence_millis: None,
        attributes: BTreeMap::new(),
        evidence: Vec::new(),
    }
}

fn link(
    trace_id: TraceId,
    parent_action_id: &str,
    child_action_id: &str,
    role: SemanticActionLinkRole,
) -> SemanticActionLink {
    SemanticActionLink {
        trace_id,
        parent_action_id: parent_action_id.to_string(),
        child_action_id: child_action_id.to_string(),
        role,
        confidence: SemanticActionLinkConfidence::Observed,
        evidence: Vec::new(),
        attributes: BTreeMap::new(),
    }
}

fn action_ids(rows: &[crate::semantic_actions::SemanticActionDisplayRootChildRow]) -> Vec<&str> {
    rows.iter()
        .map(|row| row.action.action_id.as_str())
        .collect()
}
