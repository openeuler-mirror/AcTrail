use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticActionKind, SemanticActionLinkRole, SemanticActionStatus, SemanticEvidenceKind,
};

use super::test_support::*;

#[test]
fn process_exec_alone_does_not_mark_agent_identity() {
    let mut runtime = runtime();
    let process = ProcessIdentity::new(ROOT_PID, ROOT_START_TICKS, ROOT_GENERATION);
    let output = runtime.observe_event(&exec_event(
        ROOT_EXEC_EVENT_ID,
        process.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    ));

    assert_eq!(output.actions.len(), 2);
    assert_eq!(output.actions[0].kind, SemanticActionKind::ProcessExec);
    assert_eq!(
        output.actions[1].kind,
        SemanticActionKind::CommandInvocation
    );
    assert_eq!(output.links.len(), 1);
    assert_eq!(
        output.links[0].role,
        SemanticActionLinkRole::CommandContainsProcessExec
    );
    let process_action = &output.actions[0];
    assert!(
        process_action
            .attributes
            .get("agent.identity.status")
            .is_none()
    );
    assert!(
        output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::AgentInvocation)
    );

    runtime.observe_event(&exit_event(
        ROOT_EXIT_EVENT_ID,
        process.clone(),
        SUCCESS_EXIT_CODE,
    ));
    let late = runtime.observe_event(&exec_event(
        ROOT_LATE_EXEC_EVENT_ID,
        process,
        None,
        "/usr/bin/ls",
    ));
    for kind in [
        SemanticActionKind::ProcessExec,
        SemanticActionKind::CommandInvocation,
    ] {
        let action = late
            .actions
            .iter()
            .find(|action| action.kind == kind)
            .unwrap();
        assert_eq!(action.status, SemanticActionStatus::Success);
        assert!(action.end_time.is_some());
        assert!(
            action
                .evidence
                .iter()
                .any(|evidence| evidence.id == ROOT_EXIT_EVENT_ID.get())
        );
    }
}

#[test]
fn llm_request_marks_child_agent_and_upgrades_only_direct_edge() {
    let mut runtime = runtime();
    let root = ProcessIdentity::new(ROOT_PID, ROOT_START_TICKS, ROOT_GENERATION);
    let wrapper = ProcessIdentity::new(WRAPPER_PID, WRAPPER_START_TICKS, WRAPPER_GENERATION);
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    runtime.observe_event(&exec_event(
        ROOT_EXEC_EVENT_ID,
        root.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    ));
    runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        wrapper.clone(),
        None,
        "/usr/bin/bash",
    ));
    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        agent.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    ));
    runtime.observe_event(&fork_event(AGENT_FORK_EVENT_ID, agent.clone(), wrapper));

    let output = runtime.observe_payload_segment(&llm_payload_segment(agent));
    let process_exec = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::ProcessExec)
        .expect("LLM evidence should update the child process.exec action");
    assert_eq!(
        process_exec
            .attributes
            .get("agent.identity.status")
            .map(String::as_str),
        Some("observed")
    );

    assert!(
        output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::AgentInvocation)
    );
    let command = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("LLM evidence should label the child command.invocation as agent");
    assert_eq!(
        command.attributes.get("process.parent.pid").cloned(),
        Some(WRAPPER_PID.to_string())
    );
    assert_eq!(
        command.attributes.get("agent.child.pid").cloned(),
        Some(AGENT_PID.to_string())
    );
    assert_eq!(
        command
            .attributes
            .get("invocation.kind")
            .map(String::as_str),
        Some("agent")
    );
    assert_eq!(
        command
            .attributes
            .get("agent.invocation.trigger")
            .map(String::as_str),
        Some("child_llm_request")
    );
    let llm_call = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("LLM request should project an llm.call aggregate");
    assert!(output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::CommandContainsLlmCall
            && link.parent_action_id == command.action_id
            && link.child_action_id == llm_call.action_id
    }));
}

#[test]
fn llm_request_labels_late_exec_command_as_agent() {
    let mut runtime = runtime();
    let parent = ProcessIdentity::new(WRAPPER_PID, WRAPPER_START_TICKS, WRAPPER_GENERATION);
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        parent.clone(),
        None,
        "/usr/bin/bash",
    ));
    runtime.observe_event(&fork_event(
        AGENT_FORK_EVENT_ID,
        agent.clone(),
        parent.clone(),
    ));
    runtime.observe_payload_segment(&llm_payload_segment(agent.clone()));

    let output = runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        agent,
        None,
        "/root/.cargo/bin/xiaoo",
    ));
    let process_exec = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::ProcessExec)
        .expect("late exec should be marked with prior LLM evidence");
    assert_eq!(
        process_exec
            .attributes
            .get("agent.identity.status")
            .map(String::as_str),
        Some("observed")
    );
    let command = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("late exec should project a command.invocation");
    assert_eq!(
        command.attributes.get("process.parent.pid").cloned(),
        Some(parent.pid.to_string())
    );
    assert_eq!(
        command
            .attributes
            .get("invocation.kind")
            .map(String::as_str),
        Some("agent")
    );
    assert!(
        output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::AgentInvocation)
    );

    let conflicting_parent = ProcessIdentity::new(ROOT_PID, ROOT_START_TICKS, ROOT_GENERATION);
    let conflict_output = runtime.observe_event(&fork_event(
        AGENT_SECOND_FORK_EVENT_ID,
        command.process.clone(),
        conflicting_parent,
    ));
    let conflict_command = conflict_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("late fork conflict should refresh the command invocation");
    assert_eq!(
        conflict_command
            .attributes
            .get("process.parent.identity_state")
            .map(String::as_str),
        Some("conflict")
    );
    assert_eq!(
        conflict_command
            .attributes
            .get("invocation.kind")
            .map(String::as_str),
        Some("agent")
    );
}

#[test]
fn llm_request_links_to_http_message_on_same_payload_segment() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let llm_output = runtime.observe_payload_segment(&llm_payload_segment(agent.clone()));
    let llm_request = llm_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmRequest)
        .expect("payload should project an llm.request action");

    let http_output = runtime.observe_event(&http_request_event(HTTP_REQUEST_EVENT_ID, agent));
    let http_message = http_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::HttpMessage)
        .expect("application event should project an http.message action");
    let link = http_output
        .links
        .iter()
        .find(|link| link.role == SemanticActionLinkRole::LlmRequestHttpMessage)
        .expect("http.message should be linked under llm.request");

    assert_eq!(link.parent_action_id, llm_request.action_id);
    assert_eq!(link.child_action_id, http_message.action_id);
    assert!(link.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::PayloadSegment
            && evidence.id == PAYLOAD_SEGMENT_ID.get()
    }));
}

#[test]
fn agent_performed_action_links_child_command_invocation() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let child = ProcessIdentity::new(WRAPPER_PID, WRAPPER_START_TICKS, WRAPPER_GENERATION);

    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        agent.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    ));
    let agent_update = runtime.observe_payload_segment(&llm_payload_segment(agent.clone()));
    let agent_process = agent_update
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::ProcessExec)
        .expect("LLM request should mark the process as an observed agent");
    let llm_call = agent_update
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("LLM payload should project a call action");
    let call_link = agent_update
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::AgentPerformedAction
                && link.child_action_id == llm_call.action_id
        })
        .expect("LLM call should be linked under the observed agent");
    assert_eq!(
        call_link
            .attributes
            .get(AGENT_ACTION_SEQUENCE_ATTR)
            .map(String::as_str),
        Some(FIRST_AGENT_ACTION_SEQUENCE)
    );
    let child_exec_output = runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        child.clone(),
        None,
        "/usr/bin/bash",
    ));
    let command = child_exec_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("child exec should project a command.invocation");
    assert!(child_exec_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::AgentPerformedAction
            || link.child_action_id != command.action_id
    }));

    let output = runtime.observe_event(&fork_event(
        AGENT_FORK_EVENT_ID,
        child.clone(),
        agent.clone(),
    ));
    let refreshed_command = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("fork should refresh the child command.invocation");
    let link = output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::AgentPerformedAction
                && link.child_action_id == refreshed_command.action_id
        })
        .expect("child command should be linked under the observed agent");

    assert_eq!(link.parent_action_id, agent_process.action_id);
    assert_eq!(
        refreshed_command
            .attributes
            .get("process.parent.pid")
            .cloned(),
        Some(AGENT_PID.to_string())
    );
    assert_eq!(
        link.attributes
            .get(AGENT_ACTION_SEQUENCE_ATTR)
            .map(String::as_str),
        Some(SECOND_AGENT_ACTION_SEQUENCE)
    );

    let conflicting_parent = ProcessIdentity::new(ROOT_PID, ROOT_START_TICKS, ROOT_GENERATION);
    let conflict_output = runtime.observe_event(&fork_event(
        AGENT_SECOND_FORK_EVENT_ID,
        child,
        conflicting_parent,
    ));
    let invalidated = conflict_output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::AgentPerformedAction
                && link.child_action_id == refreshed_command.action_id
        })
        .expect("late fork conflict should invalidate the agent child command link");
    assert_eq!(
        invalidated
            .attributes
            .get("actrail.link.valid")
            .map(String::as_str),
        Some("false")
    );
}

#[test]
fn agent_performed_action_links_process_fork_attempt() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        agent.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    ));
    runtime.observe_payload_segment(&llm_payload_segment(agent.clone()));
    let output = runtime.observe_event(&fork_attempt_event(FORK_ATTEMPT_EVENT_ID, agent));
    let fork = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::ProcessForkAttempt)
        .expect("fork attempt event should project process.fork_attempt");
    let link = output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::AgentPerformedAction
                && link.child_action_id == fork.action_id
        })
        .expect("fork attempt should link under the observed agent");

    assert_eq!(link.evidence, fork.evidence);
}
