use model_core::event::EventPayload;
use model_core::process::ProcessIdentity;
use semantic_action::{SemanticActionKind, SemanticActionLinkRole, SemanticEvidenceKind};

use super::test_support::*;

#[test]
fn command_invocation_links_child_command_invocation() {
    let mut runtime = runtime();
    let parent = ProcessIdentity::new(WRAPPER_GENERATION);
    let child = ProcessIdentity::new(AGENT_GENERATION);

    let parent_output = runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        parent.clone(),
        None,
        "/usr/bin/gh",
    ));
    let parent_command = parent_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("parent exec should project a command.invocation");

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
    assert!(child_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::CommandContainsCommandInvocation
            || link.child_action_id != child_command.action_id
    }));

    let fork_output = runtime.observe_event(&fork_event(AGENT_FORK_EVENT_ID, child, parent));
    let link = fork_output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
                && link.child_action_id == child_command.action_id
        })
        .expect("child command should be linked under the parent command");

    assert_eq!(link.parent_action_id, parent_command.action_id);
    assert!(link.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::Event && evidence.id == AGENT_FORK_EVENT_ID.get()
    }));
}

#[test]
fn command_invocation_links_pending_child_command_invocation() {
    let mut runtime = runtime();
    let parent = ProcessIdentity::new(WRAPPER_GENERATION);
    let child = ProcessIdentity::new(AGENT_GENERATION);

    runtime.observe_event(&fork_event(
        AGENT_FORK_EVENT_ID,
        child.clone(),
        parent.clone(),
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
    assert!(child_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::CommandContainsCommandInvocation
            || link.child_action_id != child_command.action_id
    }));

    let parent_output = runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        parent,
        None,
        "/usr/bin/gh",
    ));
    let parent_command = parent_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("parent exec should project a command.invocation");
    let link = parent_output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
                && link.child_action_id == child_command.action_id
        })
        .expect("pending child command should link when parent command appears");

    assert_eq!(link.parent_action_id, parent_command.action_id);
}

#[test]
fn command_invocation_links_when_fork_parent_arrives_after_exec() {
    let mut runtime = runtime();
    let parent = ProcessIdentity::new(WRAPPER_GENERATION);
    let child = ProcessIdentity::new(AGENT_GENERATION);

    let parent_output = runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        parent.clone(),
        None,
        "/usr/bin/gh",
    ));
    let parent_command = parent_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("parent exec should project a command.invocation");

    let mut child_exec = exec_event(AGENT_EXEC_EVENT_ID, child.clone(), None, "/usr/bin/git");
    if let EventPayload::Process(payload) = &mut child_exec.payload {
        payload
            .metadata
            .insert("ppid".to_string(), parent.get().to_string());
    }
    let child_exec_output = runtime.observe_event(&child_exec);
    let child_command = child_exec_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("child exec should project a command.invocation");
    assert!(child_exec_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::CommandContainsCommandInvocation
            || link.child_action_id != child_command.action_id
    }));

    let fork_output = runtime.observe_event(&fork_event(
        AGENT_FORK_EVENT_ID,
        child.clone(),
        parent.clone(),
    ));
    let link = fork_output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
                && link.child_action_id == child_command.action_id
        })
        .expect("fork parent identity should relink the child command");

    assert_eq!(link.parent_action_id, parent_command.action_id);
    assert!(link.evidence.iter().any(|evidence| {
        evidence.kind == SemanticEvidenceKind::Event && evidence.id == AGENT_FORK_EVENT_ID.get()
    }));

    let duplicate_exec_output = runtime.observe_event(&exec_event(
        AGENT_DUPLICATE_EXEC_EVENT_ID,
        child,
        None,
        "/usr/bin/git",
    ));
    let refreshed_child_command = duplicate_exec_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("duplicate exec should refresh the command.invocation");
    assert_eq!(
        refreshed_child_command
            .attributes
            .get("process.parent.id")
            .cloned(),
        Some(parent.get().to_string())
    );
    assert!(duplicate_exec_output.links.iter().all(|link| {
        link.role != SemanticActionLinkRole::CommandContainsCommandInvocation
            || link.child_action_id != child_command.action_id
    }));
}

#[test]
fn command_invocation_links_when_fork_parent_arrives_before_exec() {
    let mut runtime = runtime();
    let parent = ProcessIdentity::new(WRAPPER_GENERATION);
    let child = ProcessIdentity::new(AGENT_GENERATION);

    let parent_output = runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        parent.clone(),
        None,
        "/usr/bin/gh",
    ));
    let parent_command = parent_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("parent exec should project a command.invocation");

    runtime.observe_event(&fork_event(AGENT_FORK_EVENT_ID, child.clone(), parent));
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
    let link = child_output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
                && link.child_action_id == child_command.action_id
        })
        .expect("cached fork parent identity should link the child command");

    assert_eq!(link.parent_action_id, parent_command.action_id);
}

#[test]
fn command_invocation_labels_child_agent_and_contains_llm_call() {
    let mut runtime = runtime();
    let parent = ProcessIdentity::new(AGENT_GENERATION);
    let child = ProcessIdentity::new(WRAPPER_GENERATION);

    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        parent.clone(),
        None,
        "/root/.cargo/bin/opencode",
    ));
    let parent_update = runtime.observe_payload_segment(&llm_payload_segment(parent.clone()));
    let parent_agent = parent_update
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::ProcessExec)
        .expect("LLM request should mark the parent process as an observed agent");

    let child_exec_output = runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        child.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    ));
    let child_command = child_exec_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("child exec should project a command.invocation");
    let fork_output = runtime.observe_event(&fork_event(
        AGENT_FORK_EVENT_ID,
        child.clone(),
        parent.clone(),
    ));
    let parent_agent_link = fork_output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::AgentPerformedAction
                && link.child_action_id == child_command.action_id
        })
        .expect("child command should link under the observed parent agent");
    assert_eq!(parent_agent_link.parent_action_id, parent_agent.action_id);

    let output = runtime.observe_payload_segment(&llm_payload_segment(child));
    let command = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("child LLM request should label the child command invocation");
    assert_eq!(
        command.attributes.get("process.parent.id").cloned(),
        Some(parent.get().to_string())
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
    let llm_call = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("child LLM request should project a call aggregate");
    let command_link = output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::CommandContainsLlmCall
                && link.child_action_id == llm_call.action_id
        })
        .expect("llm.call should link under the child command");
    assert_eq!(command_link.parent_action_id, child_command.action_id);
}
