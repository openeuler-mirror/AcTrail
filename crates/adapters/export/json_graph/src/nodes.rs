//! Node projection for traces, processes, resources, and events.

use std::collections::BTreeMap;

use graph_contract::document::{GraphNode, GraphNodeKind};
use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::payload::PayloadSegment;
use model_core::process::ProcessMembership;
use model_core::trace::TraceRecord;

use crate::event_attributes::{
    event_attributes, event_title, insert_process_identity, insert_time,
};

pub fn trace_node(trace: &TraceRecord) -> GraphNode {
    let mut attributes = BTreeMap::new();
    attributes.insert(
        "state".to_string(),
        trace.lifecycle_state.as_display_str().to_string(),
    );
    attributes.insert("health".to_string(), format!("{:?}", trace.health));
    attributes.insert("profile".to_string(), trace.profile_name.to_string());

    GraphNode {
        id: format!("trace:{}", trace.trace_id.get()),
        kind: GraphNodeKind::Trace,
        title: trace.display_name.to_string(),
        attributes,
    }
}

pub fn process_node(membership: &ProcessMembership) -> GraphNode {
    let mut attributes = BTreeMap::new();
    insert_process_identity(&mut attributes, "", &membership.identity);
    attributes.insert(
        "capture_enabled".to_string(),
        membership.capture_enabled.to_string(),
    );
    attributes.insert(
        "propagation_enabled".to_string(),
        membership.propagation_enabled.to_string(),
    );
    attributes.insert("state".to_string(), format!("{:?}", membership.state));
    if let Some(parent) = &membership.inherited_from {
        insert_process_identity(&mut attributes, "inherited_from_", parent);
    }
    if let Some(status) = &membership.exit_status {
        if let Some(code) = status.code {
            attributes.insert("exit_code".to_string(), code.to_string());
        }
        insert_time(&mut attributes, "exit_observed_at", status.observed_at);
    }

    GraphNode {
        id: process_node_id(membership),
        kind: GraphNodeKind::Process,
        title: format!("pid {}", membership.identity.pid),
        attributes,
    }
}

pub fn event_node(event: &DomainEvent) -> GraphNode {
    GraphNode {
        id: format!("event:{}", event.envelope.event_id.get()),
        kind: GraphNodeKind::Event,
        title: event_title(event),
        attributes: event_attributes(event),
    }
}

pub fn payload_node(
    segment: &PayloadSegment,
    include_bytes: bool,
    include_text: bool,
) -> GraphNode {
    let mut attributes = BTreeMap::new();
    insert_process_identity(&mut attributes, "", &segment.process);
    attributes.insert("trace_id".to_string(), segment.trace_id.get().to_string());
    attributes.insert("direction".to_string(), format!("{:?}", segment.direction));
    attributes.insert(
        "source_boundary".to_string(),
        format!("{:?}", segment.source_boundary),
    );
    attributes.insert("stream_key".to_string(), segment.stream_key.to_string());
    attributes.insert("sequence".to_string(), segment.sequence.to_string());
    attributes.insert(
        "original_size".to_string(),
        segment.original_size.to_string(),
    );
    attributes.insert(
        "captured_size".to_string(),
        segment.captured_size.to_string(),
    );
    attributes.insert("operation_id".to_string(), segment.operation_id.to_string());
    attributes.insert(
        "operation_offset".to_string(),
        segment.operation_offset.to_string(),
    );
    attributes.insert(
        "operation_original_size".to_string(),
        segment.operation_original_size.to_string(),
    );
    attributes.insert(
        "operation_captured_size".to_string(),
        segment.operation_captured_size.to_string(),
    );
    attributes.insert(
        "operation_completion_state".to_string(),
        segment.operation_completion_state.as_str().to_string(),
    );
    attributes.insert(
        "truncation".to_string(),
        format!("{:?}", segment.truncation),
    );
    attributes.insert("redaction".to_string(), format!("{:?}", segment.redaction));
    attributes.insert("library".to_string(), segment.library.clone());
    attributes.insert("symbol".to_string(), segment.symbol.clone());
    if let Some(protocol_hint) = &segment.protocol_hint {
        attributes.insert("protocol_hint".to_string(), protocol_hint.clone());
    }
    if include_bytes {
        attributes.insert("bytes_base64".to_string(), base64_encode(&segment.bytes));
    }
    if include_text {
        attributes.insert(
            "text".to_string(),
            String::from_utf8_lossy(&segment.bytes).into_owned(),
        );
    }

    GraphNode {
        id: format!("payload:{}", segment.segment_id.get()),
        kind: GraphNodeKind::Payload,
        title: format!("Payload {}", segment.segment_id),
        attributes,
    }
}

pub fn diagnostic_node(diagnostic: &DiagnosticRecord) -> GraphNode {
    let mut attributes = BTreeMap::new();
    attributes.insert("kind".to_string(), format!("{:?}", diagnostic.kind));
    attributes.insert("severity".to_string(), format!("{:?}", diagnostic.severity));

    GraphNode {
        id: format!("diag:{}", diagnostic.diagnostic_id.get()),
        kind: GraphNodeKind::Diagnostic,
        title: diagnostic.message.clone(),
        attributes,
    }
}

pub fn process_node_id(membership: &ProcessMembership) -> String {
    format!(
        "process:{}:{}",
        membership.identity.pid, membership.identity.generation
    )
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        let combined = ((first as u32) << 16) | ((second as u32) << 8) | third as u32;
        output.push(ALPHABET[((combined >> 18) & 0x3f) as usize] as char);
        output.push(ALPHABET[((combined >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            output.push(ALPHABET[((combined >> 6) & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(ALPHABET[(combined & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}
