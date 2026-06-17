use model_core::event::EventPayload;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticActionKind, SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionStatus,
};

use super::test_support::*;

#[test]
fn llm_request_does_not_create_agent_invocation_from_pid_only_parent() {
    let mut runtime = runtime();
    let parent = ProcessIdentity::new(WRAPPER_PID, WRAPPER_START_TICKS, WRAPPER_GENERATION);
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        parent.clone(),
        None,
        "/usr/bin/bash",
    ));
    let mut child_exec = exec_event(
        AGENT_EXEC_EVENT_ID,
        agent.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    );
    if let EventPayload::Process(payload) = &mut child_exec.payload {
        payload
            .metadata
            .insert("ppid".to_string(), parent.pid.to_string());
    }
    runtime.observe_event(&child_exec);
    let output = runtime.observe_payload_segment(&llm_payload_segment(agent));
    assert!(
        output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::AgentInvocation)
    );
}

#[test]
fn llm_request_does_not_create_agent_invocation_from_conflicting_parent_edges() {
    let mut runtime = runtime();
    let first_parent = ProcessIdentity::new(WRAPPER_PID, WRAPPER_START_TICKS, WRAPPER_GENERATION);
    let second_parent = ProcessIdentity::new(ROOT_PID, ROOT_START_TICKS, ROOT_GENERATION);
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        first_parent.clone(),
        None,
        "/usr/bin/bash",
    ));
    runtime.observe_event(&exec_event(
        ROOT_EXEC_EVENT_ID,
        second_parent.clone(),
        None,
        "/usr/bin/bash",
    ));
    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        agent.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    ));
    runtime.observe_event(&fork_event(
        AGENT_SECOND_FORK_EVENT_ID,
        agent.clone(),
        first_parent,
    ));
    runtime.observe_event(&fork_event(
        AGENT_FORK_EVENT_ID,
        agent.clone(),
        second_parent,
    ));

    let output = runtime.observe_payload_segment(&llm_payload_segment(agent));
    assert!(
        output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::AgentInvocation)
    );
}

#[test]
fn command_invocation_does_not_link_child_command_with_pid_only_parent() {
    let mut runtime = runtime();
    let parent = ProcessIdentity::new(WRAPPER_PID, WRAPPER_START_TICKS, WRAPPER_GENERATION);
    let child = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        parent.clone(),
        None,
        "/usr/bin/gh",
    ));
    let mut child_event = exec_event(AGENT_EXEC_EVENT_ID, child, None, "/usr/bin/git");
    if let EventPayload::Process(payload) = &mut child_event.payload {
        payload
            .metadata
            .insert("ppid".to_string(), parent.pid.to_string());
    }
    let child_output = runtime.observe_event(&child_event);
    let child_command = child_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("child exec should project a command.invocation");

    assert!(child_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::CommandContainsCommandInvocation
            || link.child_action_id != child_command.action_id
    }));
}

#[test]
fn agent_performed_action_does_not_link_pid_only_child_command_invocation() {
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

    let mut child_event = exec_event(WRAPPER_EXEC_EVENT_ID, child, None, "/usr/bin/bash");
    if let EventPayload::Process(payload) = &mut child_event.payload {
        payload
            .metadata
            .insert("ppid".to_string(), agent.pid.to_string());
    }
    let output = runtime.observe_event(&child_event);
    let command = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("child exec should project a command.invocation");

    assert!(output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::AgentPerformedAction
            || link.parent_action_id != agent_process.action_id
            || link.child_action_id != command.action_id
    }));
}

#[test]
fn command_invocation_does_not_link_reused_parent_pid() {
    let mut runtime = runtime();
    let old_parent = ProcessIdentity::new(WRAPPER_PID, WRAPPER_START_TICKS, WRAPPER_GENERATION);
    let reused_parent = ProcessIdentity::new(WRAPPER_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let child = ProcessIdentity::new(ROOT_PID, ROOT_START_TICKS, ROOT_GENERATION);

    runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        old_parent,
        None,
        "/usr/bin/gh",
    ));
    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        child.clone(),
        None,
        "/usr/bin/git",
    ));
    runtime.observe_event(&fork_event(
        AGENT_FORK_EVENT_ID,
        child.clone(),
        reused_parent,
    ));
    let child_output = runtime.observe_event(&exec_event(
        AGENT_DUPLICATE_EXEC_EVENT_ID,
        child,
        None,
        "/usr/bin/git",
    ));
    let child_command = child_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("child exec should project a command.invocation");

    assert!(child_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::CommandContainsCommandInvocation
            || link.child_action_id != child_command.action_id
    }));
}

#[test]
fn command_invocation_parent_identity_conflict_invalidates_emitted_child_link() {
    let mut runtime = runtime();
    let old_parent = ProcessIdentity::new(WRAPPER_PID, WRAPPER_START_TICKS, WRAPPER_GENERATION);
    let conflicting_parent = ProcessIdentity::new(ROOT_PID, ROOT_START_TICKS, ROOT_GENERATION);
    let child = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let parent_output = runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        old_parent.clone(),
        None,
        "/usr/bin/gh",
    ));
    let parent_command = parent_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("parent exec should project a command.invocation");
    runtime.observe_event(&fork_event(
        AGENT_FORK_EVENT_ID,
        child.clone(),
        old_parent.clone(),
    ));
    let child_output = runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        child.clone(),
        None,
        "/usr/bin/git",
    ));
    let child_command = child_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("child exec should project a command.invocation");
    let emitted_link = child_output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
                && link.child_action_id == child_command.action_id
        })
        .expect("fork-confirmed child command should link under the parent command");
    assert_eq!(emitted_link.parent_action_id, parent_command.action_id);

    let conflict_output = runtime.observe_event(&fork_event(
        AGENT_SECOND_FORK_EVENT_ID,
        child,
        conflicting_parent,
    ));
    let conflict_command = conflict_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("fork conflict should refresh the command.invocation");
    assert_eq!(
        conflict_command
            .attributes
            .get("process.parent.identity_state")
            .map(String::as_str),
        Some("conflict")
    );
    assert!(
        conflict_command
            .attributes
            .get("process.parent.pid")
            .is_none()
    );
    let invalidated = conflict_output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
                && link.parent_action_id == parent_command.action_id
                && link.child_action_id == child_command.action_id
        })
        .expect("late fork conflict should invalidate the emitted child-command link");
    assert_eq!(
        invalidated.confidence,
        SemanticActionLinkConfidence::Derived
    );
    assert_eq!(invalidated.valid, false);
}

#[test]
fn command_invocation_pre_exec_parent_conflict_is_not_overwritten() {
    let mut runtime = runtime();
    let first_parent = ProcessIdentity::new(WRAPPER_PID, WRAPPER_START_TICKS, WRAPPER_GENERATION);
    let second_parent = ProcessIdentity::new(ROOT_PID, ROOT_START_TICKS, ROOT_GENERATION);
    let child = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        first_parent.clone(),
        None,
        "/usr/bin/gh",
    ));
    runtime.observe_event(&exec_event(
        ROOT_EXEC_EVENT_ID,
        second_parent.clone(),
        None,
        "/usr/bin/bash",
    ));
    runtime.observe_event(&fork_event(
        AGENT_FORK_EVENT_ID,
        child.clone(),
        first_parent,
    ));
    runtime.observe_event(&fork_event(
        AGENT_SECOND_FORK_EVENT_ID,
        child.clone(),
        second_parent,
    ));

    let child_output = runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        child,
        None,
        "/usr/bin/git",
    ));
    let child_command = child_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("child exec should project a command.invocation");
    assert_eq!(
        child_command
            .attributes
            .get("process.parent.identity_state")
            .map(String::as_str),
        Some("conflict")
    );
    assert!(child_command.attributes.get("process.parent.pid").is_none());
    assert!(child_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::CommandContainsCommandInvocation
            || link.child_action_id != child_command.action_id
    }));
}

#[test]
fn command_invocation_state_is_trace_scoped() {
    let mut runtime = runtime();
    let process = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        process.clone(),
        None,
        "/usr/bin/git",
    ));
    runtime.observe_event(&exit_event(
        ROOT_EXIT_EVENT_ID,
        process.clone(),
        SUCCESS_EXIT_CODE,
    ));

    let mut other_exec = exec_event(AGENT_DUPLICATE_EXEC_EVENT_ID, process, None, "/usr/bin/git");
    other_exec.envelope.trace_id = OTHER_TRACE_ID;
    let other_output = runtime.observe_event(&other_exec);
    let other_command = other_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("other trace exec should project a command.invocation");

    assert_eq!(other_command.trace_id, OTHER_TRACE_ID);
    assert_eq!(other_command.status, SemanticActionStatus::InProgress);
    assert!(other_command.end_time.is_none());
}
