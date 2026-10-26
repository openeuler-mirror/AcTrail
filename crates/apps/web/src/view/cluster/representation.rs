//! Cluster trace and graph JSON representation.

use super::{BTreeMap, ClusterTraceRow, Value, json};

pub(super) fn trace_detail_json(
    row: &ClusterTraceRow,
    graph: &GraphJson,
) -> Result<String, String> {
    Ok(format!(
        "{{\"trace\":{},\"counts\":{},\"events\":[{}],\"processes\":[{}],\"process_tree\":[{}],\"payloads\":[{}],\"timeline\":[{}],\"diagnostics\":[{}]}}",
        trace_row_json(row)?,
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
    ))
}

pub(super) fn trace_row_json(row: &ClusterTraceRow) -> Result<String, String> {
    let mut output = String::from("{");
    json::field(&mut output, "id", &json::string(&row.trace_uid));
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
        "local_id",
        &json::number(super::parse_local_trace_id(&row.local_trace_id)?),
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
    Ok(output)
}

pub(super) fn counts_json(row: &ClusterTraceRow) -> String {
    format!(
        "{{\"events\":{},\"process\":{},\"net\":0,\"file\":0,\"ipc\":0,\"stdio\":0,\"application\":0,\"control\":0,\"loss\":0,\"label\":0,\"enforcement\":0,\"payloads\":{},\"retained_payload_bytes\":{},\"diagnostics\":{},\"actions\":{},\"links\":{}}}",
        row.event_count,
        row.process_count,
        row.payload_segment_count,
        row.retained_payload_bytes,
        row.diagnostic_count,
        row.semantic_action_count,
        row.semantic_link_count
    )
}

pub(super) fn action_summary_json(row: &ClusterTraceRow) -> String {
    format!(
        "{{\"actions\":{},\"links\":{},\"roots\":0}}",
        row.semantic_action_count, row.semantic_link_count
    )
}

pub(super) fn action_tree_empty_json(row: &ClusterTraceRow) -> String {
    format!(
        "{{\"actions\":[],\"links\":[],\"roots\":[],\"summary\":{}}}",
        action_summary_json(row)
    )
}

pub(super) fn observed_agent_json(row: &ClusterTraceRow) -> String {
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
pub(super) struct GraphJson {
    pub(super) value: Value,
}

impl GraphJson {
    pub(super) fn nodes(&self) -> &[Value] {
        self.value
            .get("nodes")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

pub(super) trait JsonNodeExt {
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

pub(super) fn graph_processes(graph: &GraphJson) -> Vec<String> {
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

pub(super) fn graph_process_tree(graph: &GraphJson) -> Vec<String> {
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

pub(super) fn graph_events(graph: &GraphJson) -> Vec<String> {
    graph
        .nodes()
        .iter()
        .filter(|node| node.string("kind").as_deref() == Some("Event"))
        .enumerate()
        .map(|(index, node)| generic_node_event_json(index, node, "event"))
        .collect()
}

pub(super) fn graph_payloads(graph: &GraphJson) -> Vec<String> {
    graph
        .nodes()
        .iter()
        .filter(|node| node.string("kind").as_deref() == Some("Payload"))
        .enumerate()
        .map(|(index, node)| payload_row_json(index, node))
        .collect()
}

pub(super) fn graph_diagnostics(graph: &GraphJson) -> Vec<String> {
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

pub(super) fn generic_node_event_json(index: usize, node: &Value, domain: &str) -> String {
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

pub(super) fn payload_row_json(index: usize, node: &Value) -> String {
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

pub(super) fn payload_detail_json(node: &Value) -> String {
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

pub(super) fn parse_process_node_id(node: &Value) -> Option<u64> {
    node.attrs()
        .get("process_id")
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| parse_suffix_id(node, "process:"))
}

pub(super) fn parse_suffix_id(node: &Value, prefix: &str) -> Option<u64> {
    node.string("id")?.strip_prefix(prefix)?.parse::<u64>().ok()
}

pub(super) fn optional_ms_json(raw: Option<&str>) -> String {
    match raw.and_then(|value| value.parse::<u64>().ok()) {
        Some(value) => json::number(value),
        None => "null".to_string(),
    }
}

pub(super) fn optional_ms_nanos_json(raw: Option<&str>) -> String {
    match raw.and_then(|value| value.parse::<u128>().ok()) {
        Some(value) => json::string(&(value * 1_000_000).to_string()),
        None => "null".to_string(),
    }
}

pub(super) fn display_state(raw: &str) -> String {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Unknown".to_string(),
    }
}

pub(super) fn display_health(raw: &str) -> String {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Unknown".to_string(),
    }
}

impl ClusterTraceRow {
    pub(super) fn project_identity(&self, raw: String) -> Result<String, String> {
        let mut value: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        if let Some(trace) = value.get_mut("trace") {
            let metadata: Value =
                serde_json::from_str(&trace_row_json(self)?).map_err(|e| e.to_string())?;
            let target = trace.as_object_mut().ok_or("invalid trace response")?;
            for key in [
                "id",
                "display_id",
                "trace_uid",
                "cluster_id",
                "node_id",
                "node_ip",
                "node_name",
                "local_id",
                "created_at",
                "created_at_unix_nanos",
            ] {
                if let Some(field) = metadata.get(key) {
                    target.insert(key.into(), field.clone());
                }
            }
        }
        if let Some(id) = value.get_mut("trace_id") {
            *id = Value::String(self.trace_uid.clone());
        }
        serde_json::to_string(&value).map_err(|e| e.to_string())
    }
}
