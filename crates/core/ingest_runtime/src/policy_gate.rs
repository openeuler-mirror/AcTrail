//! Policy-before-persistence gate for ingest processing.

use std::collections::BTreeMap;

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use policy_evaluate_contract::decision::PolicyDecision;
use policy_evaluate_contract::evaluate::{PolicyEvaluator, PolicyInput};

pub fn apply_policy<P: PolicyEvaluator>(
    evaluator: &P,
    trace_id: TraceId,
    event: &DomainEvent,
) -> PolicyDecision {
    let input = PolicyInput {
        trace_id,
        process: event.envelope.process.clone(),
        event_kind: format!("{:?}", event.envelope.kind),
        fields: payload_fields(&event.payload),
        bytes: payload_bytes(&event.payload),
    };
    evaluator.evaluate(&input)
}

fn payload_fields(payload: &EventPayload) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    match payload {
        EventPayload::Process(payload) => {
            fields.insert("operation".to_string(), payload.operation.clone());
            if let Some(executable) = &payload.executable {
                fields.insert("executable".to_string(), executable.clone());
            }
        }
        EventPayload::File(payload) => {
            fields.insert("operation".to_string(), payload.operation.clone());
            if let Some(path) = &payload.path {
                fields.insert("path".to_string(), path.clone());
            }
        }
        EventPayload::Net(payload) => {
            fields.insert("transport".to_string(), payload.transport.clone());
            if let Some(remote) = &payload.remote {
                fields.insert("remote".to_string(), remote.clone());
            }
        }
        EventPayload::Ipc(payload) => {
            fields.insert("channel".to_string(), payload.channel.clone());
        }
        EventPayload::Stdio(payload) => {
            fields.insert("stream".to_string(), payload.stream.clone());
        }
        EventPayload::Application(payload) => {
            fields.insert("protocol".to_string(), payload.protocol.clone());
            fields.insert("operation".to_string(), payload.operation.clone());
            fields.insert("summary".to_string(), payload.summary.clone());
        }
        EventPayload::Resource(payload) => {
            fields.insert("scope".to_string(), payload.scope.clone());
            fields.insert("subject".to_string(), payload.subject.clone());
        }
        EventPayload::Control(payload) => {
            fields.insert("action".to_string(), payload.action.clone());
        }
        EventPayload::Loss(payload) => {
            fields.insert("reason".to_string(), payload.reason.clone());
        }
        EventPayload::Label(payload) => {
            fields.insert("provider".to_string(), payload.provider.clone());
        }
        EventPayload::Enforcement(payload) => {
            fields.insert("backend".to_string(), payload.backend.clone());
            fields.insert("operation".to_string(), payload.operation.clone());
            fields.insert("decision".to_string(), payload.decision.clone());
            fields.insert("result".to_string(), payload.result.clone());
            if let Some(path) = &payload.path {
                fields.insert("path".to_string(), path.clone());
            }
            if let Some(rule_id) = &payload.rule_id {
                fields.insert("rule_id".to_string(), rule_id.clone());
            }
        }
    }
    fields
}

fn payload_bytes(payload: &EventPayload) -> Vec<u8> {
    match payload {
        EventPayload::Stdio(payload) => payload.data.clone(),
        _ => Vec::new(),
    }
}
