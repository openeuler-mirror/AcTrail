//! Network resource projection for graph exports.

use std::collections::{BTreeMap, BTreeSet};

use graph_contract::document::{GraphEdge, GraphNode, GraphNodeKind};
use graph_contract::relationships::GraphRelationship;
use model_core::event::{DomainEvent, EventPayload, NetPayload};
use model_core::process::ProcessMembership;

use crate::nodes::process_node_id;

pub fn network_resource_nodes(events: &[DomainEvent]) -> Vec<GraphNode> {
    let mut nodes = BTreeMap::<String, GraphNode>::new();
    for event in events {
        let EventPayload::Net(payload) = &event.payload else {
            continue;
        };
        for endpoint in network_endpoints(payload) {
            nodes.entry(endpoint.id.clone()).or_insert_with(|| {
                let mut attributes = BTreeMap::new();
                attributes.insert("resource_type".to_string(), "network_endpoint".to_string());
                attributes.insert("transport".to_string(), endpoint.transport.clone());
                attributes.insert("endpoint".to_string(), endpoint.value.clone());
                GraphNode {
                    id: endpoint.id,
                    kind: GraphNodeKind::Resource,
                    title: endpoint.value,
                    attributes,
                }
            });
        }
    }
    nodes.into_values().collect()
}

pub fn network_resource_edges(
    events: &[DomainEvent],
    memberships: &[ProcessMembership],
) -> Vec<GraphEdge> {
    let known_processes = memberships
        .iter()
        .map(|membership| (membership.identity.clone(), process_node_id(membership)))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    let mut edges = Vec::new();

    for event in events {
        let EventPayload::Net(payload) = &event.payload else {
            continue;
        };
        let Some(process_id) = known_processes.get(&event.envelope.process) else {
            continue;
        };
        for endpoint in network_endpoints(payload) {
            let edge_key = format!(
                "{}:{}:{}",
                event.envelope.event_id.get(),
                process_id,
                endpoint.id
            );
            if !seen.insert(edge_key) {
                continue;
            }
            edges.push(GraphEdge {
                from: process_id.clone(),
                to: endpoint.id,
                relationship: GraphRelationship::ProcessOpenedChannel,
                attributes: network_edge_attributes(event, payload, endpoint.role),
            });
        }
    }

    edges
}

fn network_edge_attributes(
    event: &DomainEvent,
    payload: &NetPayload,
    endpoint_role: &'static str,
) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    attributes.insert(
        "event_id".to_string(),
        event.envelope.event_id.get().to_string(),
    );
    attributes.insert("endpoint_role".to_string(), endpoint_role.to_string());
    insert_if_present(
        &mut attributes,
        "operation",
        payload.metadata.get("operation"),
    );
    insert_if_present(
        &mut attributes,
        "direction",
        payload.metadata.get("direction"),
    );
    insert_if_present(&mut attributes, "fd", payload.metadata.get("fd"));
    if let Some(size) = payload.size {
        attributes.insert("size".to_string(), size.to_string());
    }
    if let Some(result) = payload.result {
        attributes.insert("result".to_string(), result.to_string());
    }
    attributes
}

fn network_endpoints(payload: &NetPayload) -> Vec<NetworkEndpoint> {
    let mut endpoints = Vec::new();
    if let Some(local) = &payload.local {
        endpoints.push(NetworkEndpoint::new("local", &payload.transport, local));
    }
    if let Some(remote) = &payload.remote {
        endpoints.push(NetworkEndpoint::new("remote", &payload.transport, remote));
    }
    endpoints
}

fn insert_if_present(
    attributes: &mut BTreeMap<String, String>,
    key: &'static str,
    value: Option<&String>,
) {
    if let Some(value) = value {
        attributes.insert(key.to_string(), value.clone());
    }
}

struct NetworkEndpoint {
    id: String,
    role: &'static str,
    transport: String,
    value: String,
}

impl NetworkEndpoint {
    fn new(role: &'static str, transport: &str, value: &str) -> Self {
        Self {
            id: endpoint_node_id(transport, value),
            role,
            transport: transport.to_string(),
            value: value.to_string(),
        }
    }
}

fn endpoint_node_id(transport: &str, endpoint: &str) -> String {
    format!("network:endpoint:{transport}:{endpoint}")
}
