//! Cluster-center API rendering for actrailweb.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;
use storage_core::SemanticActionChildPageQuery;
use storage_factory::StorageConfig;

use crate::json;

#[path = "cluster/index.rs"]
mod index;
#[path = "cluster/representation.rs"]
mod representation;
use index::ClusterIndex;
pub(crate) use index::ClusterTracePageQuery;
#[path = "cluster/timeline.rs"]
mod timeline;
use representation::*;
pub(crate) use timeline::{timeline_page_json, trace_metadata_json};

const SQLITE_SNAPSHOT_FILE: &str = "trace.sqlite";

#[derive(Clone, Debug)]
struct ClusterTraceRow {
    trace_uid: String,
    cluster_id: String,
    node_ip: String,
    node_id: String,
    node_name: Option<String>,
    local_trace_id: String,
    display_name: Option<String>,
    profile_name: Option<String>,
    lifecycle_state: String,
    health: String,
    created_unix_ms: Option<String>,
    started_unix_ms: Option<String>,
    completed_unix_ms: Option<String>,
    imported_at: String,
    bundle_path: String,
    graph_json_path: String,
    process_count: u64,
    event_count: u64,
    payload_segment_count: u64,
    retained_payload_bytes: u64,
    semantic_action_count: u64,
    semantic_link_count: u64,
    diagnostic_count: u64,
    manifest_json: String,
}

pub fn validate_cluster_root(root: &Path) -> Result<(), String> {
    let index = root.join("index.sqlite");
    if !index.exists() {
        return Err(format!("cluster index does not exist: {}", index.display()));
    }
    open_index(root)?
        .prepare("SELECT trace_uid FROM imported_traces LIMIT 0")
        .map_err(|error| format!("read cluster index schema: {error}"))?;
    Ok(())
}

pub fn traces_json(root: &Path, query: &ClusterTracePageQuery) -> Result<String, String> {
    ClusterIndex::open(root)?.page(query)
}

pub fn trace_json(root: &Path, trace_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return row.project_identity(super::trace_json(&storage, local_trace_id)?);
    }
    let graph = read_graph(&row)?;
    trace_detail_json(&row, &graph)
}

pub fn trace_summary_json(root: &Path, trace_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return row.project_identity(super::trace_summary_json(&storage, local_trace_id)?);
    }
    Ok(format!(
        "{{\"trace\":{},\"counts\":{}}}",
        trace_row_json(&row)?,
        counts_json(&row)
    ))
}

pub fn trace_time_attribution_json(root: &Path, trace_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? else {
        return Err(format!(
            "cluster trace {} has no trace.sqlite semantic snapshot; time attribution cannot be computed reliably",
            row.trace_uid
        ));
    };
    super::trace_time_attribution_json(&storage, local_trace_id)
}

pub fn trace_events_json(root: &Path, trace_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::trace_events_json(&storage, local_trace_id);
    }
    let graph = read_graph(&row)?;
    Ok(format!(
        "{{\"events\":[{}]}}",
        graph_events(&graph).join(",")
    ))
}

pub fn trace_payloads_json(root: &Path, trace_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::trace_payloads_json(&storage, local_trace_id);
    }
    let graph = read_graph(&row)?;
    Ok(format!(
        "{{\"payloads\":[{}]}}",
        graph_payloads(&graph).join(",")
    ))
}

pub fn trace_timeline_json(root: &Path, trace_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::trace_timeline_json(&storage, local_trace_id);
    }
    let graph = read_graph(&row)?;
    let mut items = Vec::new();
    items.extend(graph_events(&graph));
    items.extend(graph_payloads(&graph));
    Ok(format!("{{\"timeline\":[{}]}}", items.join(",")))
}

pub fn trace_processes_json(root: &Path, trace_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::trace_processes_json(&storage, local_trace_id);
    }
    let graph = read_graph(&row)?;
    let processes = graph_processes(&graph);
    let process_tree = graph_process_tree(&graph);
    Ok(format!(
        "{{\"processes\":[{}],\"process_tree\":[{}]}}",
        processes.join(","),
        process_tree.join(",")
    ))
}

pub fn trace_diagnostics_json(root: &Path, trace_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::trace_diagnostics_json(&storage, local_trace_id);
    }
    let graph = read_graph(&row)?;
    Ok(format!(
        "{{\"diagnostics\":[{}]}}",
        graph_diagnostics(&graph).join(",")
    ))
}

pub fn action_tree_root_json(root: &Path, trace_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::action_tree_root_json(&storage, local_trace_id);
    }
    Ok(format!(
        "{{\"summary\":{},\"root\":{{\"observed_agent\":{},\"has_children\":false}}}}",
        action_summary_json(&row),
        observed_agent_json(&row)
    ))
}

pub fn action_tree_children_json(
    root: &Path,
    trace_id: &str,
    parent_id: &str,
    page: SemanticActionChildPageQuery,
) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::action_tree_children_json(&storage, local_trace_id, parent_id, page);
    }
    Ok("{\"actions\":[],\"links\":[],\"child_state\":[],\"next_offset\":null}".to_string())
}

pub fn action_tree_llm_nav_json(
    root: &Path,
    trace_id: &str,
    mode: super::LlmNavMode,
    after_action_id: Option<&str>,
) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::action_tree_llm_nav_json(&storage, local_trace_id, mode, after_action_id);
    }
    Ok(format!(
        "{{\"mode\":{},\"found\":false,\"path\":[]}}",
        json::string(mode.as_str())
    ))
}

pub fn commands_json(root: &Path, trace_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::commands_json(&storage, local_trace_id);
    }
    Ok(action_tree_empty_json(&row))
}

pub fn action_detail_json(root: &Path, trace_id: &str, action_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::action_detail_json(&storage, local_trace_id, action_id);
    }
    let mut output = String::from("{");
    json::field(&mut output, "id", &json::string(action_id));
    output.push(',');
    json::field(&mut output, "kind", &json::string("cluster.action"));
    output.push(',');
    json::field(&mut output, "title", &json::string(action_id));
    output.push(',');
    json::field(&mut output, "attributes", "{}");
    output.push('}');
    Ok(output)
}

pub fn file_path_set_json(
    root: &Path,
    trace_id: &str,
    action_id: &str,
    page: SemanticActionChildPageQuery,
) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::action_file_path_set_json(&storage, local_trace_id, action_id, page);
    }
    Ok(format!(
        "{{\"path_set\":null,\"offset\":{},\"limit\":{},\"total\":0,\"next_offset\":null,\"has_more\":false,\"paths\":[]}}",
        json::number(page.offset),
        json::number(page.limit)
    ))
}

pub fn llm_request_content_json(
    root: &Path,
    trace_id: &str,
    action_id: &str,
    max_bytes: usize,
) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::action_llm_request_content_json(
            &storage,
            local_trace_id,
            action_id,
            max_bytes,
        );
    }
    Ok("{\"content\":null}".to_string())
}

pub fn llm_request_lineage_json(
    root: &Path,
    trace_id: &str,
    action_id: &str,
) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::action_llm_request_lineage_json(&storage, local_trace_id, action_id);
    }
    Ok("{\"lineage\":null,\"forks\":[]}".to_string())
}

pub fn llm_trajectory_graph_json(root: &Path, trace_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return row.project_identity(super::llm_trajectory_graph_json(&storage, local_trace_id)?);
    }
    Ok(format!(
        "{{\"trace_id\":{},\"partial\":true,\"nodes\":[],\"edges\":[],\"stats\":{{\"node_count\":0,\"trajectory_count\":0,\"append_count\":0,\"fork_count\":0,\"duplicate_count\":0,\"strongly_linked_node_ratio\":0.0,\"duplicate_node_ratio\":0.0}},\"capabilities\":{{\"strict_prefix_edges\":false,\"related_edges\":false,\"compaction_detection\":false}}}}",
        json::string(trace_id),
    ))
}

pub fn llm_request_trajectory_json(
    root: &Path,
    trace_id: &str,
    trajectory_id: &str,
) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::llm_request_trajectory_json(&storage, local_trace_id, trajectory_id);
    }
    Ok(format!(
        "{{\"trajectory_id\":{},\"nodes\":[]}}",
        json::string(trajectory_id)
    ))
}

pub fn llm_request_content_node_json(
    root: &Path,
    trace_id: &str,
    action_id: &str,
    query: storage_core::ContentNodeQuery,
) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::action_llm_request_content_node_json(
            &storage,
            local_trace_id,
            action_id,
            query,
        );
    }
    Ok("{\"content\":null}".to_string())
}

pub fn payload_json(root: &Path, trace_id: &str, segment_id: u64) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::payload_json(&storage, local_trace_id, segment_id);
    }
    let graph = read_graph(&row)?;
    let wanted = format!("payload:{segment_id}");
    graph
        .nodes()
        .iter()
        .find(|node| node.string("id").as_deref() == Some(wanted.as_str()))
        .map(payload_detail_json)
        .ok_or_else(|| format!("payload {segment_id} not found"))
}

fn open_index(root: &Path) -> Result<Connection, String> {
    let path = root.join("index.sqlite");
    Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("open cluster index {}: {error}", path.display()))
}

fn row_by_uid(root: &Path, uid: &str) -> Result<ClusterTraceRow, String> {
    ClusterIndex::open(root)?.get(uid)
}

pub(crate) fn with_snapshot<T>(
    root: &Path,
    uid: &str,
    read: impl FnOnce(&StorageConfig, u64) -> Result<T, String>,
) -> Result<T, String> {
    let row = row_by_uid(root, uid)?;
    let (storage, local_id) = sqlite_storage_for_row(&row)?
        .ok_or_else(|| format!("cluster trace {uid} requires a trace.sqlite snapshot"))?;
    read(&storage, local_id)
}

fn read_graph(row: &ClusterTraceRow) -> Result<GraphJson, String> {
    let path = PathBuf::from(&row.graph_json_path);
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("read cluster graph {}: {error}", path.display()))?;
    let value = serde_json::from_str(&raw)
        .map_err(|error| format!("parse cluster graph {}: {error}", path.display()))?;
    Ok(GraphJson { value })
}

fn sqlite_storage_for_row(row: &ClusterTraceRow) -> Result<Option<(StorageConfig, u64)>, String> {
    let path = sqlite_snapshot_path(row);
    if !path.exists() {
        return Ok(None);
    }
    let trace_id = parse_local_trace_id(&row.local_trace_id)?;
    Ok(Some((StorageConfig::sqlite(path, 5_000), trace_id)))
}

fn sqlite_snapshot_path(row: &ClusterTraceRow) -> PathBuf {
    PathBuf::from(&row.graph_json_path).with_file_name(SQLITE_SNAPSHOT_FILE)
}

fn parse_local_trace_id(raw: &str) -> Result<u64, String> {
    raw.strip_prefix("trace-")
        .unwrap_or(raw)
        .parse::<u64>()
        .map_err(|error| format!("parse local trace id {raw}: {error}"))
}
