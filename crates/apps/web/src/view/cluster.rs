//! Cluster-center API rendering for actrailweb.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;
use storage_core::SemanticActionChildPageQuery;
use storage_factory::StorageConfig;

use crate::json;

const SQLITE_SNAPSHOT_FILE: &str = "trace.sqlite";

#[derive(Clone, Debug)]
struct ClusterTraceRow {
    ui_id: u64,
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
    open_index(root).map(|_| ())
}

pub fn traces_json(root: &Path) -> Result<String, String> {
    let rows = list_rows(root)?;
    let traces = rows.iter().map(trace_row_json).collect::<Vec<_>>();
    Ok(format!("{{\"traces\":[{}]}}", traces.join(",")))
}

pub fn trace_json(root: &Path, trace_id: u64) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::trace_json(&storage, local_trace_id);
    }
    let graph = read_graph(&row)?;
    Ok(trace_detail_json(&row, &graph))
}

pub fn trace_summary_json(root: &Path, trace_id: u64) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::trace_summary_json(&storage, local_trace_id);
    }
    Ok(format!(
        "{{\"trace\":{},\"counts\":{}}}",
        trace_row_json(&row),
        counts_json(&row)
    ))
}

pub fn trace_events_json(root: &Path, trace_id: u64) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::trace_events_json(&storage, local_trace_id);
    }
    let graph = read_graph(&row)?;
    Ok(format!(
        "{{\"events\":[{}]}}",
        graph_events(&graph).join(",")
    ))
}

pub fn trace_payloads_json(root: &Path, trace_id: u64) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::trace_payloads_json(&storage, local_trace_id);
    }
    let graph = read_graph(&row)?;
    Ok(format!(
        "{{\"payloads\":[{}]}}",
        graph_payloads(&graph).join(",")
    ))
}

pub fn trace_timeline_json(root: &Path, trace_id: u64) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::trace_timeline_json(&storage, local_trace_id);
    }
    let graph = read_graph(&row)?;
    let mut items = Vec::new();
    items.extend(graph_events(&graph));
    items.extend(graph_payloads(&graph));
    Ok(format!("{{\"timeline\":[{}]}}", items.join(",")))
}

pub fn trace_processes_json(root: &Path, trace_id: u64) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
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

pub fn trace_diagnostics_json(root: &Path, trace_id: u64) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::trace_diagnostics_json(&storage, local_trace_id);
    }
    let graph = read_graph(&row)?;
    Ok(format!(
        "{{\"diagnostics\":[{}]}}",
        graph_diagnostics(&graph).join(",")
    ))
}

pub fn action_tree_json(root: &Path, trace_id: u64) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::action_tree_json(&storage, local_trace_id);
    }
    Ok(action_tree_empty_json(&row))
}

pub fn action_tree_root_json(root: &Path, trace_id: u64) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
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
    trace_id: u64,
    parent_id: &str,
    page: SemanticActionChildPageQuery,
) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::action_tree_children_json(&storage, local_trace_id, parent_id, page);
    }
    Ok("{\"actions\":[],\"links\":[],\"child_state\":[],\"next_offset\":null}".to_string())
}

pub fn commands_json(root: &Path, trace_id: u64) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
    if let Some((storage, local_trace_id)) = sqlite_storage_for_row(&row)? {
        return super::commands_json(&storage, local_trace_id);
    }
    Ok(action_tree_empty_json(&row))
}

pub fn action_detail_json(root: &Path, trace_id: u64, action_id: &str) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
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
    trace_id: u64,
    action_id: &str,
    page: SemanticActionChildPageQuery,
) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
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
    trace_id: u64,
    action_id: &str,
    max_bytes: usize,
) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
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

pub fn payload_json(root: &Path, trace_id: u64, segment_id: u64) -> Result<String, String> {
    let row = row_by_ui_id(root, trace_id)?;
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
    Connection::open(&path)
        .map_err(|error| format!("open cluster index {}: {error}", path.display()))
}

fn list_rows(root: &Path) -> Result<Vec<ClusterTraceRow>, String> {
    let connection = open_index(root)?;
    let mut statement = connection
        .prepare(
            "select trace_uid, cluster_id, node_ip, node_id, node_name, local_trace_id, display_name, profile_name, lifecycle_state, health, created_unix_ms, started_unix_ms, completed_unix_ms, imported_at, bundle_path, graph_json_path, process_count, event_count, payload_segment_count, retained_payload_bytes, semantic_action_count, semantic_link_count, diagnostic_count, manifest_json from imported_traces order by imported_at desc",
        )
        .map_err(|error| format!("prepare cluster trace list: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(ClusterTraceRow {
                ui_id: 0,
                trace_uid: row.get(0)?,
                cluster_id: row.get(1)?,
                node_ip: row.get(2)?,
                node_id: row.get(3)?,
                node_name: row.get(4)?,
                local_trace_id: row.get(5)?,
                display_name: row.get(6)?,
                profile_name: row.get(7)?,
                lifecycle_state: row.get(8)?,
                health: row.get(9)?,
                created_unix_ms: row.get(10)?,
                started_unix_ms: row.get(11)?,
                completed_unix_ms: row.get(12)?,
                imported_at: row.get(13)?,
                bundle_path: row.get(14)?,
                graph_json_path: row.get(15)?,
                process_count: row.get::<_, i64>(16)?.max(0) as u64,
                event_count: row.get::<_, i64>(17)?.max(0) as u64,
                payload_segment_count: row.get::<_, i64>(18)?.max(0) as u64,
                retained_payload_bytes: row.get::<_, i64>(19)?.max(0) as u64,
                semantic_action_count: row.get::<_, i64>(20)?.max(0) as u64,
                semantic_link_count: row.get::<_, i64>(21)?.max(0) as u64,
                diagnostic_count: row.get::<_, i64>(22)?.max(0) as u64,
                manifest_json: row.get(23)?,
            })
        })
        .map_err(|error| format!("query cluster traces: {error}"))?;
    let mut output = Vec::new();
    for (index, row) in rows.enumerate() {
        let mut row = row.map_err(|error| format!("read cluster trace row: {error}"))?;
        row.ui_id = (index as u64) + 1;
        output.push(row);
    }
    Ok(output)
}

fn row_by_ui_id(root: &Path, ui_id: u64) -> Result<ClusterTraceRow, String> {
    if ui_id == 0 {
        return Err("cluster trace id must be positive".to_string());
    }
    list_rows(root)?
        .into_iter()
        .find(|row| row.ui_id == ui_id)
        .ok_or_else(|| format!("cluster trace {ui_id} not found"))
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

fn trace_detail_json(row: &ClusterTraceRow, graph: &GraphJson) -> String {
    format!(
        "{{\"trace\":{},\"counts\":{},\"events\":[{}],\"processes\":[{}],\"process_tree\":[{}],\"payloads\":[{}],\"timeline\":[{}],\"diagnostics\":[{}]}}",
        trace_row_json(row),
        counts_json(row),
        graph_events(graph).join(","),
        graph_processes(graph).join(","),
        graph_process_tree(graph).join(","),
        graph_payloads(graph).join(","),
        {
            let mut items = graph_events(graph);
            items.extend(graph_payloads(graph));
            items.join(",")
        },
        graph_diagnostics(graph).join(",")
    )
}

fn trace_row_json(row: &ClusterTraceRow) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "id", &json::number(row.ui_id));
    output.push(',');
    json::field(&mut output, "display_id", &json::string(&row.trace_uid));
    output.push(',');
    json::field(
        &mut output,
        "name",
        &json::string(row.display_name.as_deref().unwrap_or(&row.trace_uid)),
    );
    output.push(',');
    json::field(
        &mut output,
        "profile",
        &json::string(row.profile_name.as_deref().unwrap_or("cluster")),
    );
    output.push(',');
    json::field(&mut output, "root_process_id", &json::number(1));
    output.push(',');
    json::field(&mut output, "root_pid", "null");
    output.push(',');
    json::field(&mut output, "container_id", "null");
    output.push(',');
    json::field(
        &mut output,
        "state",
        &json::string(&display_state(&row.lifecycle_state)),
    );
    output.push(',');
    json::field(
        &mut output,
        "health",
        &json::string(&display_health(&row.health)),
    );
    output.push(',');
    json::field(
        &mut output,
        "created_at",
        &optional_ms_json(row.created_unix_ms.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "created_at_unix_nanos",
        &optional_ms_nanos_json(row.created_unix_ms.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "started_at",
        &optional_ms_json(row.started_unix_ms.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "started_at_unix_nanos",
        &optional_ms_nanos_json(row.started_unix_ms.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "completed_at",
        &optional_ms_json(row.completed_unix_ms.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "completed_at_unix_nanos",
        &optional_ms_nanos_json(row.completed_unix_ms.as_deref()),
    );
    output.push(',');
    json::field(&mut output, "exited_at", "null");
    output.push(',');
    json::field(&mut output, "exited_at_unix_nanos", "null");
    output.push(',');
    json::field(&mut output, "failed_at", "null");
    output.push(',');
    json::field(&mut output, "failed_at_unix_nanos", "null");
    output.push(',');
    json::field(
        &mut output,
        "tags",
        &json::string_array([
            format!("cluster:{}", row.cluster_id),
            format!("node:{}", row.node_id),
            format!("node_ip:{}", row.node_ip),
        ]),
    );
    output.push(',');
    json::field(&mut output, "trace_uid", &json::string(&row.trace_uid));
    output.push(',');
    json::field(&mut output, "cluster_id", &json::string(&row.cluster_id));
    output.push(',');
    json::field(&mut output, "node_ip", &json::string(&row.node_ip));
    output.push(',');
    json::field(&mut output, "node_id", &json::string(&row.node_id));
    output.push(',');
    json::field(
        &mut output,
        "node_name",
        &json::optional_string(row.node_name.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "local_trace_id",
        &json::string(&row.local_trace_id),
    );
    output.push(',');
    json::field(&mut output, "imported_at", &json::string(&row.imported_at));
    output.push(',');
    json::field(&mut output, "bundle_path", &json::string(&row.bundle_path));
    output.push(',');
    json::field(
        &mut output,
        "graph_json_path",
        &json::string(&row.graph_json_path),
    );
    output.push(',');
    json::field(&mut output, "manifest", &row.manifest_json);
    output.push('}');
    output
}

fn counts_json(row: &ClusterTraceRow) -> String {
    format!(
        "{{\"events\":{},\"process\":{},\"net\":0,\"file\":0,\"ipc\":0,\"stdio\":0,\"application\":0,\"resource\":0,\"control\":0,\"loss\":0,\"label\":0,\"enforcement\":0,\"payloads\":{},\"retained_payload_bytes\":{},\"diagnostics\":{},\"actions\":{},\"links\":{}}}",
        row.event_count,
        row.process_count,
        row.payload_segment_count,
        row.retained_payload_bytes,
        row.diagnostic_count,
        row.semantic_action_count,
        row.semantic_link_count
    )
}

fn action_summary_json(row: &ClusterTraceRow) -> String {
    format!(
        "{{\"actions\":{},\"links\":{},\"roots\":0}}",
        row.semantic_action_count, row.semantic_link_count
    )
}

fn action_tree_empty_json(row: &ClusterTraceRow) -> String {
    format!(
        "{{\"actions\":[],\"links\":[],\"roots\":[],\"summary\":{}}}",
        action_summary_json(row)
    )
}

fn observed_agent_json(row: &ClusterTraceRow) -> String {
    let mut attrs = BTreeMap::new();
    attrs.insert("trace_uid".to_string(), row.trace_uid.clone());
    attrs.insert("cluster_id".to_string(), row.cluster_id.clone());
    attrs.insert("node_ip".to_string(), row.node_ip.clone());
    attrs.insert("node_id".to_string(), row.node_id.clone());
    if let Some(name) = &row.node_name {
        attrs.insert("node_name".to_string(), name.clone());
    }
    let mut output = String::from("{");
    json::field(
        &mut output,
        "title",
        &json::string(row.display_name.as_deref().unwrap_or(&row.trace_uid)),
    );
    output.push(',');
    json::field(&mut output, "attributes", &json::map(&attrs));
    output.push('}');
    output
}

#[derive(Clone, Debug)]
struct GraphJson {
    value: Value,
}

impl GraphJson {
    fn nodes(&self) -> &[Value] {
        self.value
            .get("nodes")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

trait JsonNodeExt {
    fn string(&self, key: &str) -> Option<String>;
    fn attrs(&self) -> BTreeMap<String, String>;
}

impl JsonNodeExt for Value {
    fn string(&self, key: &str) -> Option<String> {
        self.get(key).and_then(Value::as_str).map(ToOwned::to_owned)
    }

    fn attrs(&self) -> BTreeMap<String, String> {
        self.get("attributes")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|value| (key.clone(), value.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn graph_processes(graph: &GraphJson) -> Vec<String> {
    graph
        .nodes()
        .iter()
        .filter(|node| node.string("kind").as_deref() == Some("Process"))
        .enumerate()
        .map(|(index, node)| {
            let attrs = node.attrs();
            let process_id = parse_process_node_id(node).unwrap_or((index as u64) + 1);
            let parent = attrs
                .get("inherited_from_process_id")
                .and_then(|value| value.parse::<u64>().ok());
            let mut output = String::from("{");
            json::field(&mut output, "process_id", &json::number(process_id));
            output.push(',');
            json::field(
                &mut output,
                "identity",
                &format!("{{\"process_id\":{process_id}}}"),
            );
            output.push(',');
            json::field(
                &mut output,
                "parent_process_id",
                &json::optional_number(parent),
            );
            output.push(',');
            json::field(&mut output, "observed_at", "null");
            output.push(',');
            json::field(&mut output, "observed_at_unix_nanos", "null");
            output.push(',');
            json::field(
                &mut output,
                "state",
                &json::string(attrs.get("state").map(String::as_str).unwrap_or("Observed")),
            );
            output.push(',');
            json::field(
                &mut output,
                "exit_code",
                &json::optional_number(
                    attrs
                        .get("exit_code")
                        .and_then(|value| value.parse::<i64>().ok()),
                ),
            );
            output.push(',');
            json::field(&mut output, "exit_observed_at", "null");
            output.push(',');
            json::field(&mut output, "exit_observed_at_unix_nanos", "null");
            output.push(',');
            json::field(&mut output, "exit_observation_source", "null");
            output.push(',');
            json::field(
                &mut output,
                "title",
                &json::string(&node.string("title").unwrap_or_default()),
            );
            output.push(',');
            json::field(&mut output, "metadata", &json::map(&attrs));
            output.push('}');
            output
        })
        .collect()
}

fn graph_process_tree(graph: &GraphJson) -> Vec<String> {
    graph
        .nodes()
        .iter()
        .filter(|node| node.string("kind").as_deref() == Some("Process"))
        .enumerate()
        .map(|(index, node)| {
            let attrs = node.attrs();
            let process_id = parse_process_node_id(node).unwrap_or((index as u64) + 1);
            let parent = attrs
                .get("inherited_from_process_id")
                .and_then(|value| value.parse::<u64>().ok());
            let mut output = String::from("{");
            json::field(&mut output, "process_id", &json::number(process_id));
            output.push(',');
            json::field(&mut output, "pid", "null");
            output.push(',');
            json::field(
                &mut output,
                "parent_process_id",
                &json::optional_number(parent),
            );
            output.push(',');
            json::field(&mut output, "parent_pid", "null");
            output.push(',');
            json::field(
                &mut output,
                "depth",
                &json::number(if parent.is_some() { 1 } else { 0 }),
            );
            output.push(',');
            json::field(&mut output, "child_count", &json::number(0));
            output.push(',');
            json::field(
                &mut output,
                "label",
                &json::string(
                    &node
                        .string("title")
                        .unwrap_or_else(|| format!("process-{process_id}")),
                ),
            );
            output.push('}');
            output
        })
        .collect()
}

fn graph_events(graph: &GraphJson) -> Vec<String> {
    graph
        .nodes()
        .iter()
        .filter(|node| node.string("kind").as_deref() == Some("Event"))
        .enumerate()
        .map(|(index, node)| generic_node_event_json(index, node, "event"))
        .collect()
}

fn graph_payloads(graph: &GraphJson) -> Vec<String> {
    graph
        .nodes()
        .iter()
        .filter(|node| node.string("kind").as_deref() == Some("Payload"))
        .enumerate()
        .map(|(index, node)| payload_row_json(index, node))
        .collect()
}

fn graph_diagnostics(graph: &GraphJson) -> Vec<String> {
    graph
        .nodes()
        .iter()
        .filter(|node| node.string("kind").as_deref() == Some("Diagnostic"))
        .enumerate()
        .map(|(index, node)| {
            let attrs = node.attrs();
            let id = parse_suffix_id(node, "diagnostic:").unwrap_or((index as u64) + 1);
            let mut output = String::from("{");
            json::field(&mut output, "id", &json::number(id));
            output.push(',');
            json::field(
                &mut output,
                "severity",
                &json::string(attrs.get("severity").map(String::as_str).unwrap_or("Info")),
            );
            output.push(',');
            json::field(
                &mut output,
                "kind",
                &json::string(
                    attrs
                        .get("kind")
                        .map(String::as_str)
                        .unwrap_or("ClusterGraph"),
                ),
            );
            output.push(',');
            json::field(
                &mut output,
                "message",
                &json::string(&node.string("title").unwrap_or_default()),
            );
            output.push(',');
            json::field(&mut output, "metadata", &json::map(&attrs));
            output.push('}');
            output
        })
        .collect()
}

fn generic_node_event_json(index: usize, node: &Value, domain: &str) -> String {
    let attrs = node.attrs();
    let id = parse_suffix_id(node, "event:").unwrap_or((index as u64) + 1);
    let process_id = attrs
        .get("process_id")
        .or_else(|| attrs.get("process_process_id"))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1);
    let operation = attrs
        .get("operation")
        .or_else(|| attrs.get("action"))
        .or_else(|| attrs.get("scope"))
        .cloned()
        .unwrap_or_else(|| node.string("title").unwrap_or_default());
    let mut output = String::from("{");
    json::field(&mut output, "id", &json::number(id));
    output.push(',');
    json::field(
        &mut output,
        "display_id",
        &json::string(&format!("event-{id}")),
    );
    output.push(',');
    json::field(&mut output, "domain", &json::string(domain));
    output.push(',');
    json::field(&mut output, "process_id", &json::number(process_id));
    output.push(',');
    json::field(
        &mut output,
        "observed_at",
        &optional_ms_json(attrs.get("observed_at_unix_ms").map(String::as_str)),
    );
    output.push(',');
    json::field(
        &mut output,
        "observed_at_unix_nanos",
        &optional_ms_nanos_json(attrs.get("observed_at_unix_ms").map(String::as_str)),
    );
    output.push(',');
    json::field(&mut output, "operation", &json::string(&operation));
    output.push(',');
    json::field(
        &mut output,
        "summary",
        &json::string(&node.string("title").unwrap_or_default()),
    );
    output.push(',');
    json::field(&mut output, "metadata", &json::map(&attrs));
    output.push('}');
    output
}

fn payload_row_json(index: usize, node: &Value) -> String {
    let attrs = node.attrs();
    let id = parse_suffix_id(node, "payload:").unwrap_or((index as u64) + 1);
    let process_id = attrs
        .get("process_id")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1);
    let mut output = String::from("{");
    json::field(&mut output, "id", &json::number(id));
    output.push(',');
    json::field(&mut output, "segment_id", &json::number(id));
    output.push(',');
    json::field(
        &mut output,
        "display_id",
        &json::string(&format!("payload-{id}")),
    );
    output.push(',');
    json::field(&mut output, "process_id", &json::number(process_id));
    output.push(',');
    json::field(&mut output, "observed_at", "null");
    output.push(',');
    json::field(&mut output, "observed_at_unix_nanos", "null");
    output.push(',');
    json::field(
        &mut output,
        "direction",
        &json::string(
            attrs
                .get("direction")
                .map(String::as_str)
                .unwrap_or("Unknown"),
        ),
    );
    output.push(',');
    json::field(
        &mut output,
        "source",
        &json::string(
            attrs
                .get("source_boundary")
                .map(String::as_str)
                .unwrap_or("ClusterGraph"),
        ),
    );
    output.push(',');
    json::field(
        &mut output,
        "captured_size",
        &json::number(
            attrs
                .get("captured_size")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
        ),
    );
    output.push(',');
    json::field(
        &mut output,
        "original_size",
        &json::number(
            attrs
                .get("original_size")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
        ),
    );
    output.push(',');
    json::field(&mut output, "metadata", &json::map(&attrs));
    output.push('}');
    output
}

fn payload_detail_json(node: &Value) -> String {
    let attrs = node.attrs();
    let mut output = payload_row_json(0, node);
    output.pop();
    output.push(',');
    json::field(
        &mut output,
        "text",
        &json::optional_string(attrs.get("text").map(String::as_str)),
    );
    output.push(',');
    json::field(
        &mut output,
        "bytes_base64",
        &json::optional_string(attrs.get("bytes_base64").map(String::as_str)),
    );
    output.push('}');
    output
}

fn parse_process_node_id(node: &Value) -> Option<u64> {
    node.attrs()
        .get("process_id")
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| parse_suffix_id(node, "process:"))
}

fn parse_suffix_id(node: &Value, prefix: &str) -> Option<u64> {
    node.string("id")?.strip_prefix(prefix)?.parse::<u64>().ok()
}

fn optional_ms_json(raw: Option<&str>) -> String {
    match raw.and_then(|value| value.parse::<u64>().ok()) {
        Some(value) => json::number(value),
        None => "null".to_string(),
    }
}

fn optional_ms_nanos_json(raw: Option<&str>) -> String {
    match raw.and_then(|value| value.parse::<u128>().ok()) {
        Some(value) => json::string(&(value * 1_000_000).to_string()),
        None => "null".to_string(),
    }
}

fn display_state(raw: &str) -> String {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Unknown".to_string(),
    }
}

fn display_health(raw: &str) -> String {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Unknown".to_string(),
    }
}
