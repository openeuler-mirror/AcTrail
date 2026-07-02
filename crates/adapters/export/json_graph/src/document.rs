//! Graph-document projection owned by the JSON export adapter.

use graph_contract::completeness::GraphCompleteness;
use graph_contract::document::GraphDocument;
use model_core::trace::{TraceHealth, TraceLifecycleState};
use storage_core::SnapshotView;

use crate::edges::{diagnostic_edges, event_edges, payload_edges, process_edges};
use crate::network::{network_resource_edges, network_resource_nodes};
use crate::nodes::{diagnostic_node, event_node, payload_node, process_node, trace_node};

pub fn build_graph_document(
    schema_version: String,
    snapshot: SnapshotView,
    include_payload_bytes: bool,
    include_payload_text: bool,
) -> GraphDocument {
    let completeness = match (snapshot.trace.lifecycle_state, snapshot.trace.health) {
        (TraceLifecycleState::Failed, _) | (_, TraceHealth::Degraded) => {
            GraphCompleteness::Degraded
        }
        (lifecycle_state, TraceHealth::Clean) if lifecycle_state.is_terminal() => {
            GraphCompleteness::Complete
        }
        _ => GraphCompleteness::Snapshot,
    };

    let mut nodes = Vec::new();
    nodes.push(trace_node(&snapshot.trace));
    nodes.extend(snapshot.memberships.iter().map(process_node));
    nodes.extend(network_resource_nodes(&snapshot.events));
    nodes.extend(snapshot.events.iter().map(event_node));
    nodes.extend(
        snapshot
            .payload_segments
            .iter()
            .map(|segment| payload_node(segment, include_payload_bytes, include_payload_text)),
    );
    nodes.extend(snapshot.diagnostics.iter().map(diagnostic_node));

    let mut edges = Vec::new();
    edges.extend(process_edges(&snapshot.trace, &snapshot.memberships));
    edges.extend(event_edges(&snapshot.events, &snapshot.memberships));
    edges.extend(payload_edges(
        &snapshot.payload_segments,
        &snapshot.memberships,
    ));
    edges.extend(network_resource_edges(
        &snapshot.events,
        &snapshot.memberships,
    ));
    edges.extend(diagnostic_edges(&snapshot.trace, &snapshot.diagnostics));

    GraphDocument {
        schema_version,
        trace_id: snapshot.trace.trace_id,
        completeness,
        nodes,
        edges,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::SystemTime;

    use graph_contract::document::GraphNodeKind;
    use graph_contract::relationships::GraphRelationship;
    use model_core::event::{
        DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload, NetPayload,
    };
    use model_core::ids::{CollectorName, EventId, ProfileName, TraceId, TraceName};
    use model_core::process::{ProcessIdentity, ProcessMembership};
    use model_core::trace::TraceRecord;
    use storage_core::SnapshotView;

    use super::build_graph_document;

    #[test]
    fn net_event_export_preserves_payload_and_endpoint_resource() {
        let trace_id = TraceId::new(1);
        let process = ProcessIdentity::new(100, 200, 1);
        let trace = TraceRecord::new(
            trace_id,
            process.clone(),
            TraceName::new("demo"),
            ProfileName::new("default"),
            SystemTime::UNIX_EPOCH,
        );
        let event = DomainEvent::new(
            EventEnvelope {
                event_id: EventId::new(7),
                trace_id,
                observed_at: SystemTime::UNIX_EPOCH,
                process: process.clone(),
                collector: CollectorName::new("ebpf"),
                kind: EventKind::Net,
                flags: EventFlags::clean(),
            },
            EventPayload::Net(NetPayload {
                transport: "tcp".to_string(),
                local: Some("127.0.0.1:40000".to_string()),
                remote: Some("127.0.0.1:50000".to_string()),
                size: Some(21),
                result: Some(21),
                metadata: BTreeMap::from([
                    ("operation".to_string(), "send".to_string()),
                    ("direction".to_string(), "outbound".to_string()),
                    ("fd".to_string(), "5".to_string()),
                ]),
            }),
        );
        let snapshot = SnapshotView {
            trace,
            memberships: vec![ProcessMembership::root(
                trace_id,
                process,
                SystemTime::UNIX_EPOCH,
            )],
            events: vec![event],
            payload_segments: Vec::new(),
            diagnostics: Vec::new(),
        };

        let document = build_graph_document("v1".to_string(), snapshot, false, false);
        let event_node = document
            .nodes
            .iter()
            .find(|node| node.id == "event:7")
            .expect("event node must exist");
        assert_eq!(event_node.title, "Net send 127.0.0.1:50000");
        assert_eq!(
            event_node.attributes.get("operation").map(String::as_str),
            Some("send")
        );
        assert_eq!(
            event_node.attributes.get("remote").map(String::as_str),
            Some("127.0.0.1:50000")
        );
        assert_eq!(
            event_node.attributes.get("size").map(String::as_str),
            Some("21")
        );
        assert!(document.nodes.iter().any(|node| {
            node.kind == GraphNodeKind::Resource
                && node.attributes.get("endpoint").map(String::as_str) == Some("127.0.0.1:50000")
        }));
        assert!(document.edges.iter().any(|edge| {
            edge.relationship == GraphRelationship::ProcessOpenedChannel
                && edge.attributes.get("operation").map(String::as_str) == Some("send")
                && edge.attributes.get("endpoint_role").map(String::as_str) == Some("remote")
        }));
    }
}
