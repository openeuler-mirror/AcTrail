//! Edge projection from export snapshots to graph relationships.

use std::collections::BTreeMap;

use graph_contract::document::GraphEdge;
use graph_contract::relationships::GraphRelationship;
use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::payload::PayloadSegment;
use model_core::process::ProcessMembership;
use model_core::trace::TraceRecord;

use crate::nodes::process_node_id;

pub fn process_edges(trace: &TraceRecord, memberships: &[ProcessMembership]) -> Vec<GraphEdge> {
    memberships
        .iter()
        .map(|membership| {
            let relationship = if membership.identity == trace.root_process_identity {
                GraphRelationship::RootOwns
            } else {
                GraphRelationship::ProcessSpawned
            };
            let from = membership
                .inherited_from
                .as_ref()
                .map(|identity| format!("process:{}", identity.get()))
                .unwrap_or_else(|| format!("trace:{}", trace.trace_id.get()));

            GraphEdge {
                from,
                to: process_node_id(membership),
                relationship,
                attributes: BTreeMap::new(),
            }
        })
        .collect()
}

pub fn event_edges(events: &[DomainEvent], memberships: &[ProcessMembership]) -> Vec<GraphEdge> {
    let known_processes = memberships
        .iter()
        .map(|membership| (membership.identity.clone(), process_node_id(membership)))
        .collect::<std::collections::BTreeMap<_, _>>();

    events
        .iter()
        .filter_map(|event| {
            known_processes
                .get(&event.envelope.process)
                .map(|from| GraphEdge {
                    from: from.clone(),
                    to: format!("event:{}", event.envelope.event_id.get()),
                    relationship: GraphRelationship::ProcessObserved,
                    attributes: BTreeMap::new(),
                })
        })
        .collect()
}

pub fn diagnostic_edges(trace: &TraceRecord, diagnostics: &[DiagnosticRecord]) -> Vec<GraphEdge> {
    diagnostics
        .iter()
        .map(|diagnostic| GraphEdge {
            from: format!("trace:{}", trace.trace_id.get()),
            to: format!("diag:{}", diagnostic.diagnostic_id.get()),
            relationship: GraphRelationship::TraceHasDiagnostic,
            attributes: BTreeMap::new(),
        })
        .collect()
}

pub fn payload_edges(
    payloads: &[PayloadSegment],
    memberships: &[ProcessMembership],
) -> Vec<GraphEdge> {
    let known_processes = memberships
        .iter()
        .map(|membership| (membership.identity.clone(), process_node_id(membership)))
        .collect::<std::collections::BTreeMap<_, _>>();

    payloads
        .iter()
        .filter_map(|segment| {
            known_processes.get(&segment.process).map(|from| GraphEdge {
                from: from.clone(),
                to: format!("payload:{}", segment.segment_id.get()),
                relationship: GraphRelationship::ProcessEmittedPayload,
                attributes: BTreeMap::from([
                    ("direction".to_string(), format!("{:?}", segment.direction)),
                    (
                        "source_boundary".to_string(),
                        format!("{:?}", segment.source_boundary),
                    ),
                ]),
            })
        })
        .collect()
}
