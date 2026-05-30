//! Graph document contracts consumed by export and view flows.

use std::collections::BTreeMap;

use model_core::ids::TraceId;

use crate::completeness::GraphCompleteness;
use crate::relationships::GraphRelationship;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphNodeKind {
    Trace,
    Process,
    Payload,
    Resource,
    Event,
    Diagnostic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub kind: GraphNodeKind,
    pub title: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub relationship: GraphRelationship,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphDocument {
    pub schema_version: String,
    pub trace_id: TraceId,
    pub completeness: GraphCompleteness,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}
