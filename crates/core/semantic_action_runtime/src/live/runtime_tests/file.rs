use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticActionCompleteness, SemanticActionKind, SemanticActionLinkRole, SemanticActionStatus,
};

use super::test_support::*;

#[test]
fn startup_file_read_is_projected_and_linked_when_agent_is_observed_later() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        agent.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    ));
    assert!(
        runtime
            .observe_event(&file_event(
                FILE_OPEN_EVENT_ID,
                agent.clone(),
                "open",
                TEST_FILE_FD as i32,
                None,
            ))
            .actions
            .is_empty()
    );
    let read_output = runtime.observe_event(&file_event(
        FILE_READ_EVENT_ID,
        agent.clone(),
        "read",
        TEST_FILE_READ_BYTES as i32,
        Some(TEST_FILE_READ_BYTES),
    ));
    let read_action = read_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileRead)
        .expect("read event should project a file.read action");
    assert_eq!(read_action.status, SemanticActionStatus::Success);
    assert_eq!(
        read_action.completeness,
        SemanticActionCompleteness::Partial
    );
    assert_eq!(
        read_action.attributes.get("file.bytes_read").cloned(),
        Some(TEST_FILE_READ_BYTES.to_string())
    );
    assert!(
        read_output
            .links
            .iter()
            .all(|link| link.role != SemanticActionLinkRole::AgentPerformedAction)
    );

    let close_output = runtime.observe_event(&file_event(
        FILE_CLOSE_EVENT_ID,
        agent.clone(),
        "close",
        0,
        None,
    ));
    let complete_read = close_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileRead)
        .expect("close should complete the file.read action");
    assert_eq!(
        complete_read.completeness,
        SemanticActionCompleteness::Complete
    );
    assert!(complete_read.evidence.iter().any(|evidence| {
        evidence.id == FILE_OPEN_EVENT_ID.get() && evidence.role == "file.open"
    }));
    assert!(complete_read.evidence.iter().any(|evidence| {
        evidence.id == FILE_READ_EVENT_ID.get() && evidence.role == "file.read"
    }));
    assert!(complete_read.evidence.iter().any(|evidence| {
        evidence.id == FILE_CLOSE_EVENT_ID.get() && evidence.role == "file.close"
    }));

    let agent_update = runtime.observe_payload_segment(&llm_payload_segment(agent));
    let llm_request = agent_update
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("LLM payload should project a request action");
    let file_read_link = agent_update
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::AgentPerformedAction
                && link.child_action_id == complete_read.action_id
        })
        .expect("previous startup file.read should link under the observed agent");
    let llm_request_link = agent_update
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::AgentPerformedAction
                && link.child_action_id == llm_request.action_id
        })
        .expect("current LLM request should link under the observed agent");

    assert_eq!(
        file_read_link
            .attributes
            .get(AGENT_ACTION_SEQUENCE_ATTR)
            .map(String::as_str),
        Some(FIRST_AGENT_ACTION_SEQUENCE)
    );
    assert_eq!(
        llm_request_link
            .attributes
            .get(AGENT_ACTION_SEQUENCE_ATTR)
            .map(String::as_str),
        Some(SECOND_AGENT_ACTION_SEQUENCE)
    );
}

#[test]
fn command_process_file_read_links_under_command_invocation() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let command_process =
        ProcessIdentity::new(WRAPPER_PID, WRAPPER_START_TICKS, WRAPPER_GENERATION);

    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        agent.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    ));
    runtime.observe_payload_segment(&llm_payload_segment(agent.clone()));
    runtime.observe_event(&fork_event(
        AGENT_FORK_EVENT_ID,
        command_process.clone(),
        agent,
    ));
    let command_output = runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        command_process.clone(),
        None,
        "/usr/bin/bash",
    ));
    let command = command_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("child exec should project command.invocation");

    runtime.observe_event(&file_event(
        FILE_OPEN_EVENT_ID,
        command_process.clone(),
        "open",
        TEST_FILE_FD as i32,
        None,
    ));
    let read_output = runtime.observe_event(&file_event(
        FILE_READ_EVENT_ID,
        command_process,
        "read",
        TEST_FILE_READ_BYTES as i32,
        Some(TEST_FILE_READ_BYTES),
    ));
    let file_read = read_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileRead)
        .expect("command process read should project file.read");
    let link = read_output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::CommandContainsFileAccess
                && link.child_action_id == file_read.action_id
        })
        .expect("file.read should link under the same-process command invocation");

    assert_eq!(link.parent_action_id, command.action_id);
}
