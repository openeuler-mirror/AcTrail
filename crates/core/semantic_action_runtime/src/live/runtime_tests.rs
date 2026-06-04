use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::AgentInvocationConfig;
use model_core::event::{
    DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload, ProcessPayload,
};
use model_core::ids::{CollectorName, EventId, TraceId};
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadRedactionState,
    PayloadSegment, PayloadSegmentId, PayloadSourceBoundary, PayloadStreamKey,
    PayloadTruncationState,
};
use model_core::process::ProcessIdentity;
use semantic_action::{SemanticActionKind, SemanticActionLinkRole};

use super::LiveSemanticActionRuntime;

const TRACE_ID: TraceId = TraceId::new(42);
const ROOT_PID: u32 = 1000;
const WRAPPER_PID: u32 = 1001;
const AGENT_PID: u32 = 1002;
const ROOT_GENERATION: u64 = 7000;
const WRAPPER_GENERATION: u64 = 7001;
const AGENT_GENERATION: u64 = 7002;
const ROOT_START_TICKS: u64 = 5000;
const WRAPPER_START_TICKS: u64 = 5001;
const AGENT_START_TICKS: u64 = 5002;
const ROOT_EXEC_EVENT_ID: EventId = EventId::new(10);
const WRAPPER_EXEC_EVENT_ID: EventId = EventId::new(11);
const AGENT_EXEC_EVENT_ID: EventId = EventId::new(12);
const PAYLOAD_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(30);
const PAYLOAD_OPERATION_ID: u64 = 31;
const PAYLOAD_SEQUENCE: u64 = 0;
const PAYLOAD_OFFSET: u64 = 0;

#[test]
fn command_match_is_only_agent_candidate_hint() {
    let mut runtime = runtime();
    let process = ProcessIdentity::new(ROOT_PID, ROOT_START_TICKS, ROOT_GENERATION);
    let output = runtime.observe_event(&exec_event(
        ROOT_EXEC_EVENT_ID,
        process,
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
    assert_eq!(
        output.actions[0]
            .attributes
            .get("agent.identity.candidate_command_match")
            .map(String::as_str),
        Some("true")
    );
    assert!(
        output.actions[0]
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
        Some(root),
        "/usr/bin/bash",
    ));
    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        agent.clone(),
        Some(wrapper),
        "/root/.cargo/bin/xiaoo",
    ));

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

    let invocations = output
        .actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::AgentInvocation)
        .collect::<Vec<_>>();
    assert_eq!(invocations.len(), 1);
    assert!(output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::AgentInvocationExec
            && link.parent_action_id == invocations[0].action_id
    }));
    assert!(output.links.iter().any(|link| {
        link.role == SemanticActionLinkRole::AgentInvocationChildLlmRequest
            && link.parent_action_id == invocations[0].action_id
    }));
    let invocation = invocations[0];
    assert_eq!(
        invocation.attributes.get("agent.parent.pid").cloned(),
        Some(WRAPPER_PID.to_string())
    );
    assert_eq!(
        invocation.attributes.get("agent.child.pid").cloned(),
        Some(AGENT_PID.to_string())
    );
    assert_eq!(
        invocation
            .attributes
            .get("agent.invocation.trigger")
            .map(String::as_str),
        Some("child_llm_request")
    );
}

fn runtime() -> LiveSemanticActionRuntime {
    LiveSemanticActionRuntime::new(AgentInvocationConfig {
        enabled: true,
        commands: vec!["xiaoo".to_string()],
    })
}

fn exec_event(
    event_id: EventId,
    process: ProcessIdentity,
    parent: Option<ProcessIdentity>,
    executable: &str,
) -> DomainEvent {
    let mut metadata = BTreeMap::new();
    if let Some(parent) = &parent {
        metadata.insert("ppid".to_string(), parent.pid.to_string());
    }
    metadata.insert("command_line".to_string(), executable.to_string());
    DomainEvent::new(
        EventEnvelope {
            event_id,
            trace_id: TRACE_ID,
            observed_at: observed_at(),
            process,
            collector: CollectorName::new("test"),
            kind: EventKind::Process,
            flags: EventFlags::clean(),
        },
        EventPayload::Process(ProcessPayload {
            operation: "exec".to_string(),
            parent,
            executable: Some(executable.to_string()),
            metadata,
        }),
    )
}

fn llm_payload_segment(process: ProcessIdentity) -> PayloadSegment {
    let body = r#"{"model":"deepseek-chat","messages":[{"role":"user","content":"hello"}]}"#;
    let bytes = format!(
        "POST /chat/completions HTTP/1.1\r\nHost: api.local\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes();
    let size = bytes.len() as u64;
    PayloadSegment {
        segment_id: PAYLOAD_SEGMENT_ID,
        trace_id: TRACE_ID,
        observed_at: observed_at(),
        process,
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction: PayloadDirection::Outbound,
        stream_key: PayloadStreamKey::new("stream-1"),
        sequence: PAYLOAD_SEQUENCE,
        original_size: size,
        captured_size: size,
        operation_id: PAYLOAD_OPERATION_ID,
        operation_offset: PAYLOAD_OFFSET,
        operation_original_size: size,
        operation_captured_size: size,
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: PayloadTruncationState::Complete,
        redaction: PayloadRedactionState::Unredacted,
        library: "rustls".to_string(),
        symbol: "buffer_plaintext".to_string(),
        protocol_hint: Some("http/1.x".to_string()),
        bytes,
    }
}

fn observed_at() -> SystemTime {
    SystemTime::UNIX_EPOCH
}
