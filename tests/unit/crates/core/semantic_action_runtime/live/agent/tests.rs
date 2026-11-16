// Tests for the corresponding production module.
use std::collections::BTreeMap;

use model_core::event::{
    DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload, ProcessPayload,
};
use model_core::ids::{CollectorName, EventId};
use semantic_action::{SemanticActionCompleteness, SemanticActionStatus};

use super::*;

#[test]
fn agent_identity_carries_the_executable_from_the_same_trace_process() {
    let trace_id = TraceId::new(7);
    let process = ProcessIdentity::new(11);
    let mut projector = AgentProjector::new(false, 8);

    let exec = process_exec_event(trace_id, process.clone(), "/root/.cargo/bin/xiaoo");
    projector.observe_process_exec(&exec);

    let identities = projector.observe_llm_request(&llm_request(trace_id, process));

    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].kind, SemanticActionKind::AgentIdentity);
    assert_eq!(identities[0].trace_id, trace_id);
    assert_eq!(
        identities[0]
            .attributes
            .get(attrs::process::EXECUTABLE)
            .map(String::as_str),
        Some("/root/.cargo/bin/xiaoo")
    );
    assert_eq!(
        identities[0]
            .attributes
            .get(attrs::agent::TYPE)
            .map(String::as_str),
        Some("xiaoo")
    );
    assert_eq!(
        identities[0]
            .attributes
            .get(attrs::agent::TYPE_SOURCE)
            .map(String::as_str),
        Some("known_product")
    );
}

#[test]
fn agent_identity_does_not_borrow_an_executable_from_another_trace() {
    let process = ProcessIdentity::new(11);
    let mut projector = AgentProjector::new(false, 8);

    let exec = process_exec_event(TraceId::new(7), process.clone(), "/usr/bin/codex");
    projector.observe_process_exec(&exec);

    let identities = projector.observe_llm_request(&llm_request(TraceId::new(8), process));

    assert_eq!(identities.len(), 1);
    assert!(
        !identities[0]
            .attributes
            .contains_key(attrs::process::EXECUTABLE)
    );
    assert!(!identities[0].attributes.contains_key(attrs::agent::TYPE));
    assert!(
        !identities[0]
            .attributes
            .contains_key(attrs::agent::TYPE_SOURCE)
    );
}

fn process_exec_event(
    trace_id: TraceId,
    process: ProcessIdentity,
    executable: &str,
) -> DomainEvent {
    DomainEvent::new(
        EventEnvelope {
            event_id: EventId::new(1),
            trace_id,
            observed_at: SystemTime::UNIX_EPOCH,
            process,
            collector: CollectorName::new(E_BPF_COLLECTOR),
            kind: EventKind::Process,
            flags: EventFlags::clean(),
        },
        EventPayload::Process(ProcessPayload {
            operation: "exec".to_string(),
            parent: None,
            executable: Some(executable.to_string()),
            metadata: BTreeMap::new(),
        }),
    )
}

fn llm_request(trace_id: TraceId, process: ProcessIdentity) -> SemanticAction {
    SemanticAction {
        action_id: format!("llm-request-{}", trace_id.get()),
        trace_id,
        kind: SemanticActionKind::LlmRequest,
        title: "LLM request".to_string(),
        start_time: SystemTime::UNIX_EPOCH,
        end_time: Some(SystemTime::UNIX_EPOCH),
        process,
        status: SemanticActionStatus::Success,
        completeness: SemanticActionCompleteness::Complete,
        attributes: BTreeMap::new(),
        evidence: Vec::new(),
    }
}
