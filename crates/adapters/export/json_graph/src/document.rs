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
