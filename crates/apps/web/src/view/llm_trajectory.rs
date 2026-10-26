//! Trace-level LLM trajectory graph read model.

use std::collections::{BTreeMap, BTreeSet};
use std::time::UNIX_EPOCH;

use model_core::ids::TraceId;
use semantic_action::{LlmTrajectoryStartReason, SemanticAction, attr_keys as attrs};
use serde::Serialize;
use storage_core::StorageBackend;

#[derive(Serialize)]
struct GraphResponse {
    trace_id: u64,
    partial: bool,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    stats: GraphStats,
    capabilities: GraphCapabilities,
}

#[derive(Serialize)]
struct GraphNode {
    id: String,
    trajectory_id: String,
    trajectory_position: u32,
    transition: &'static str,
    start_reason: &'static str,
    inference_version: u32,
    start_time: u64,
    start_time_unix_nanos: String,
    model: Option<String>,
    classifier_id: Option<String>,
    block_count: Option<u64>,
    user_message_count: Option<u64>,
    tool_result_count: Option<u64>,
    process: GraphProcess,
    status: &'static str,
    completeness: &'static str,
    compaction_boundary: bool,
}

#[derive(Serialize)]
struct GraphProcess {
    process_id: u64,
}

#[derive(Serialize)]
struct GraphEdge {
    source: String,
    target: String,
    kind: &'static str,
    confidence: &'static str,
}

#[derive(Serialize)]
struct GraphStats {
    node_count: usize,
    trajectory_count: usize,
    append_count: usize,
    fork_count: usize,
    duplicate_count: usize,
    strongly_linked_node_ratio: f64,
    duplicate_node_ratio: f64,
}

#[derive(Serialize)]
struct GraphCapabilities {
    strict_prefix_edges: bool,
    related_edges: bool,
    compaction_detection: bool,
}

pub(super) fn graph_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    response_trace_id: u64,
) -> Result<String, String> {
    let lineages = storage
        .llm_request_lineages(trace_id)
        .map_err(|error| format!("read LLM request lineages failed: {error:?}"))?;
    let actions = storage
        .semantic_actions_matching_kinds_lite(trace_id, &["llm.request"])
        .map_err(|error| format!("read LLM request actions failed: {error:?}"))?;
    let actions = actions
        .into_iter()
        .map(|action| (action.action_id.clone(), action))
        .collect::<BTreeMap<_, _>>();

    let mut partial = false;
    let mut nodes = Vec::with_capacity(lineages.len());
    let mut append_count = 0usize;
    let mut fork_count = 0usize;
    let mut duplicate_count = 0usize;
    let mut trajectories = BTreeSet::new();

    for lineage in &lineages {
        let Some(action) = actions.get(&lineage.action_id) else {
            partial = true;
            continue;
        };
        trajectories.insert(lineage.trajectory_id.clone());
        match lineage.transition.as_str() {
            "append" => append_count += 1,
            "fork_root" => fork_count += 1,
            "duplicate_root" => duplicate_count += 1,
            _ => {}
        }
        nodes.push(node_from(action, lineage));
    }

    nodes.sort_by(|left, right| {
        left.start_time_unix_nanos
            .len()
            .cmp(&right.start_time_unix_nanos.len())
            .then_with(|| left.start_time_unix_nanos.cmp(&right.start_time_unix_nanos))
            .then_with(|| left.id.cmp(&right.id))
    });
    let node_ids = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut edges = Vec::new();
    for lineage in &lineages {
        let relation = lineage
            .parent_action_id
            .as_deref()
            .map(|source| (source, "append"))
            .or_else(|| {
                lineage
                    .forked_from_action_id
                    .as_deref()
                    .map(|source| (source, "fork"))
            });
        let Some((source, kind)) = relation else {
            continue;
        };
        if !node_ids.contains(source) || !node_ids.contains(lineage.action_id.as_str()) {
            partial = true;
            continue;
        }
        edges.push(GraphEdge {
            source: source.to_string(),
            target: lineage.action_id.clone(),
            kind,
            confidence: "derived",
        });
    }
    edges.sort_by(|left, right| {
        left.target
            .cmp(&right.target)
            .then_with(|| left.source.cmp(&right.source))
    });

    let node_count = nodes.len();
    let strongly_linked_node_ratio = ratio(append_count + fork_count, node_count);
    let duplicate_node_ratio = ratio(duplicate_count, node_count);
    let response = GraphResponse {
        trace_id: response_trace_id,
        partial,
        nodes,
        edges,
        stats: GraphStats {
            node_count,
            trajectory_count: trajectories.len(),
            append_count,
            fork_count,
            duplicate_count,
            strongly_linked_node_ratio,
            duplicate_node_ratio,
        },
        capabilities: GraphCapabilities {
            strict_prefix_edges: true,
            related_edges: false,
            compaction_detection: false,
        },
    };
    serde_json::to_string(&response)
        .map_err(|error| format!("serialize LLM trajectory graph failed: {error}"))
}

fn node_from(action: &SemanticAction, lineage: &semantic_action::LlmRequestLineage) -> GraphNode {
    let start = action
        .start_time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    GraphNode {
        id: action.action_id.clone(),
        trajectory_id: lineage.trajectory_id.clone(),
        trajectory_position: lineage.trajectory_position,
        transition: lineage.transition.as_str(),
        start_reason: lineage.start_reason.as_str(),
        inference_version: lineage.inference_version,
        start_time: u64::try_from(start.as_millis()).unwrap_or(u64::MAX),
        start_time_unix_nanos: start.as_nanos().to_string(),
        model: attribute(action, attrs::llm_request::MODEL),
        classifier_id: attribute(action, attrs::llm_request::CLASSIFIER_ID),
        block_count: count_attribute(action, attrs::llm_request::BLOCK_COUNT),
        user_message_count: count_attribute(action, attrs::llm_request::USER_MESSAGE_COUNT),
        tool_result_count: count_attribute(action, attrs::llm_request::TOOL_RESULT_COUNT),
        process: GraphProcess {
            process_id: action.process.get(),
        },
        status: action.status.as_str(),
        completeness: action.completeness.as_str(),
        compaction_boundary: lineage.start_reason
            == LlmTrajectoryStartReason::ContextRewriteOrCompression,
    }
}

fn attribute(action: &SemanticAction, key: &str) -> Option<String> {
    action.attributes.get(key).cloned()
}

fn count_attribute(action: &SemanticAction, key: &str) -> Option<u64> {
    action.attributes.get(key)?.parse().ok()
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}
