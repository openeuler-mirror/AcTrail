use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceDocument {
    pub schema_version: String,
    pub trace_id: String,
    pub completeness: String,
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    #[serde(alias = "from")]
    pub source: String,
    #[serde(alias = "to")]
    pub target: String,
    #[serde(default, alias = "relationship")]
    pub kind: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct EventInfo {
    pub id: String,
    pub domain: String,
    pub pid: u32,
    pub operation: String,
    pub summary: String,
    pub observed_at: Option<u64>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ResourceInfo {
    pub id: String,
    pub pid: u32,
    pub scope: String,
    pub subject: String,
    pub cpu_percent_millis: Option<u64>,
    pub rss_kb: Option<u64>,
    pub virtual_memory_kb: Option<u64>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct DiagnosticInfo {
    pub id: String,
    pub severity: String,
    pub kind: String,
    pub message: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ProcessInfo {
    pub pid: u32,
    pub generation: u64,
    pub state: String,
    pub parent_pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub exit_time: Option<u64>,
    pub exit_time_millis: Option<u64>,
    pub start_time_ticks: u64,
    pub start_unix_seconds: Option<u64>,
    pub start_unix_millis: Option<u64>,
    pub capture_enabled: bool,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct NetworkInfo {
    pub source_pid: u32,
    pub target_pid: Option<u32>,
    pub protocol: String,
    pub local_address: String,
    pub remote_address: String,
    pub state: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub pid: u32,
    pub path: String,
    pub operation: String,
    pub fd: Option<u32>,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct LatencyAnalysis {
    pub process_lifetimes: Vec<ProcessLifetime>,
    pub events: Vec<EventInfo>,
    pub resources: Vec<ResourceInfo>,
    pub diagnostics: Vec<DiagnosticInfo>,
    pub network_connections: Vec<NetworkConnection>,
    pub file_operations: Vec<FileOperation>,
    pub timeline_events: Vec<TimelineEvent>,
    pub summary: LatencySummary,
}

#[derive(Debug, Clone)]
pub struct ProcessLifetime {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub start_ticks: u64,
    pub end_ticks: Option<u64>,
    pub start_unix_seconds: Option<u64>,
    pub start_unix_millis: Option<u64>,
    pub end_unix_seconds: Option<u64>,
    pub end_unix_millis: Option<u64>,
    pub duration_ms: Option<f64>,
    pub state: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct NetworkConnection {
    pub source_pid: u32,
    pub target_pid: Option<u32>,
    pub protocol: String,
    pub local_address: String,
    pub remote_address: String,
    pub duration_ms: Option<f64>,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct FileOperation {
    pub pid: u32,
    pub path: String,
    pub operation: String,
    pub timestamp: Option<u64>,
    pub fd: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct TimelineEvent {
    pub timestamp_ms: f64,
    pub event_type: String,
    pub pid: u32,
    pub description: String,
    pub duration_ms: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct LatencySummary {
    pub total_processes: usize,
    pub active_processes: usize,
    pub exited_processes: usize,
    pub avg_process_duration_ms: Option<f64>,
    pub max_process_duration_ms: Option<f64>,
    pub min_process_duration_ms: Option<f64>,
    pub total_network_connections: usize,
    pub total_file_operations: usize,
    pub trace_duration_ms: Option<f64>,
}

pub fn load_trace(path: &Path) -> Result<TraceDocument, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("failed to read trace file: {e}"))?;
    
    parse_trace(&content)
}

pub fn parse_trace(content: &str) -> Result<TraceDocument, String> {
    let document: TraceDocument = serde_json::from_str(content)
        .map_err(|e| format!("failed to parse trace JSON: {e}"))?;
    
    Ok(document)
}

pub fn analyze_trace(document: &TraceDocument) -> LatencyAnalysis {
    let processes = extract_processes(document);
    let events = extract_events(document);
    let resources = extract_resources(document);
    let diagnostics = extract_diagnostics(document);
    let network = extract_network(document);
    let files = extract_files(document);
    
    let process_lifetimes = calculate_process_lifetimes(&processes);
    let network_connections = calculate_network_connections(&network);
    let file_operations = calculate_file_operations(&files);
    
    let mut timeline_events = Vec::new();
    timeline_events.extend(process_lifetimes.iter().map(|p| TimelineEvent {
        timestamp_ms: p.start_ticks as f64 / 1_000_000.0,
        event_type: "process_start".to_string(),
        pid: p.pid,
        description: format!("Process {} started", p.pid),
        duration_ms: p.duration_ms,
    }));
    
    let summary = calculate_summary(&process_lifetimes, &network_connections, &file_operations);
    
    LatencyAnalysis {
        process_lifetimes,
        events,
        resources,
        diagnostics,
        network_connections,
        file_operations,
        timeline_events,
        summary,
    }
}

fn extract_processes(document: &TraceDocument) -> Vec<ProcessInfo> {
    document.nodes.iter()
        .filter(|n| n.kind == "Process")
        .filter_map(|node| {
            let pid = node.attributes.get("pid")?.parse().ok()?;
            let generation = node.attributes.get("generation")?.parse().ok()?;
            let start_time_ticks = node.attributes.get("start_time_ticks")?.parse().ok()?;
            let state = node.attributes.get("state").cloned().unwrap_or_default();
            let exit_code = node.attributes.get("exit_code").and_then(|s| s.parse().ok());
            let exit_time = node.attributes.get("exit_observed_at_unix_seconds")
                .and_then(|s| s.parse().ok());
            let exit_time_millis = node.attributes.get("exit_observed_at_unix_millis")
                .and_then(|s| s.parse().ok());
            let parent_pid = node.attributes.get("inherited_from_pid")
                .and_then(|s| s.parse().ok());
            let capture_enabled = node.attributes.get("capture_enabled")
                .map(|s| s == "true")
                .unwrap_or(false);
            let start_unix_seconds = node.attributes.get("start_unix_seconds")
                .or_else(|| node.attributes.get("start_observed_at_unix_seconds"))
                .and_then(|s| s.parse().ok());
            let start_unix_millis = node.attributes.get("start_unix_millis")
                .or_else(|| node.attributes.get("start_observed_at_unix_millis"))
                .and_then(|s| s.parse().ok())
                .or_else(|| start_unix_seconds.map(|s| s * 1000));
            
            Some(ProcessInfo {
                pid,
                generation,
                state,
                parent_pid,
                exit_code,
                exit_time,
                exit_time_millis,
                start_time_ticks,
                start_unix_seconds,
                start_unix_millis,
                capture_enabled,
                attributes: node.attributes.clone(),
            })
        })
        .collect()
}

fn extract_events(document: &TraceDocument) -> Vec<EventInfo> {
    document.nodes.iter()
        .filter(|n| n.kind == "Event")
        .filter_map(|node| {
            let id = node.id.clone();
            let domain = node.attributes.get("kind").cloned().unwrap_or_default();
            let pid = node.attributes.get("process_pid")?.parse().ok()?;
            let operation = node.attributes.get("operation").cloned()
                .or_else(|| node.attributes.get("metadata.operation").cloned())
                .unwrap_or_default();
            let summary = node.attributes.get("summary").cloned()
                .or_else(|| node.attributes.get("title").cloned())
                .unwrap_or_else(|| node.title.clone());
            let observed_at = node.attributes.get("observed_at_unix_nanos")
                .and_then(|s| s.parse().ok());
            
            let mut metadata = BTreeMap::new();
            for (key, value) in &node.attributes {
                if key.starts_with("metadata.") {
                    let clean_key = key.trim_start_matches("metadata.");
                    metadata.insert(clean_key.to_string(), value.clone());
                } else if !matches!(key.as_str(), "kind" | "process_pid" | "operation" | "summary" | "title" | "observed_at_unix_nanos" | "observed_at_unix_seconds") {
                    metadata.insert(key.clone(), value.clone());
                }
            }
            
            Some(EventInfo {
                id,
                domain,
                pid,
                operation,
                summary,
                observed_at,
                metadata,
            })
        })
        .collect()
}

fn extract_resources(document: &TraceDocument) -> Vec<ResourceInfo> {
    document.nodes.iter()
        .filter(|n| n.kind == "Resource")
        .filter_map(|node| {
            let id = node.id.clone();
            let resource_type = node.attributes.get("resource_type").cloned().unwrap_or_default();
            let endpoint = node.attributes.get("endpoint").cloned().unwrap_or_default();
            let _transport = node.attributes.get("transport").cloned().unwrap_or_default();
            
            let mut metadata = node.attributes.clone();
            metadata.remove("resource_type");
            metadata.remove("endpoint");
            metadata.remove("transport");
            
            Some(ResourceInfo {
                id,
                pid: 0,
                scope: resource_type,
                subject: endpoint,
                cpu_percent_millis: None,
                rss_kb: None,
                virtual_memory_kb: None,
                metadata,
            })
        })
        .collect()
}

fn extract_diagnostics(document: &TraceDocument) -> Vec<DiagnosticInfo> {
    document.nodes.iter()
        .filter(|n| n.kind == "Diagnostic")
        .filter_map(|node| {
            let id = node.id.clone();
            let severity = node.attributes.get("severity").cloned().unwrap_or_default();
            let kind = node.attributes.get("kind").cloned().unwrap_or_default();
            let message = node.attributes.get("message").cloned()
                .or_else(|| Some(node.title.clone()))
                .unwrap_or_default();
            
            let mut metadata = BTreeMap::new();
            for (key, value) in &node.attributes {
                if !matches!(key.as_str(), "severity" | "kind" | "message") {
                    metadata.insert(key.clone(), value.clone());
                }
            }
            
            Some(DiagnosticInfo {
                id,
                severity,
                kind,
                message,
                metadata,
            })
        })
        .collect()
}

fn extract_network(document: &TraceDocument) -> Vec<NetworkInfo> {
    document.edges.iter()
        .filter(|e| e.kind.contains("Net") || e.kind.contains("net"))
        .filter_map(|edge| {
            let source_pid = parse_pid_from_id(&edge.source)?;
            let target_pid = parse_pid_from_id(&edge.target);
            let protocol = edge.attributes.get("protocol").cloned()
                .or_else(|| edge.attributes.get("l4_protocol").cloned())
                .unwrap_or_default();
            let local_address = edge.attributes.get("local_address").cloned()
                .or_else(|| edge.attributes.get("local").cloned())
                .unwrap_or_default();
            let remote_address = edge.attributes.get("remote_address").cloned()
                .or_else(|| edge.attributes.get("remote").cloned())
                .unwrap_or_default();
            let state = edge.attributes.get("state").cloned().unwrap_or_default();
            
            Some(NetworkInfo {
                source_pid,
                target_pid,
                protocol,
                local_address,
                remote_address,
                state,
                attributes: edge.attributes.clone(),
            })
        })
        .collect()
}

fn extract_files(document: &TraceDocument) -> Vec<FileInfo> {
    document.nodes.iter()
        .filter(|n| n.kind.contains("File") || n.kind.contains("file"))
        .filter_map(|node| {
            let pid = node.attributes.get("pid")?.parse().ok()?;
            let path = node.attributes.get("path").cloned().unwrap_or_default();
            let operation = node.attributes.get("operation").cloned().unwrap_or_default();
            let fd = node.attributes.get("fd").and_then(|s| s.parse().ok());
            
            Some(FileInfo {
                pid,
                path,
                operation,
                fd,
                attributes: node.attributes.clone(),
            })
        })
        .collect()
}

fn calculate_process_lifetimes(processes: &[ProcessInfo]) -> Vec<ProcessLifetime> {
    // Try to estimate boot_time from root process
    let boot_time_estimate = processes.iter()
        .find(|p| p.start_unix_seconds.is_some() && p.start_time_ticks < 100_000_000)
        .map(|p| {
            // boot_time = start_unix - (ticks / CLK_TCK)
            p.start_unix_seconds.unwrap() - (p.start_time_ticks / 100)
        });
    
    processes.iter().map(|p| {
        let start_unix = p.start_unix_seconds.or_else(|| {
            boot_time_estimate.map(|boot| boot + (p.start_time_ticks / 100))
        });
        let end_unix = p.exit_time;
        
        // Use millis precision if available
        let start_unix_millis = p.start_unix_millis.or_else(|| {
            start_unix.map(|s| s * 1000)
        });
        let end_unix_millis = p.exit_time_millis.or_else(|| {
            end_unix.map(|s| s * 1000)
        });
        
        // Calculate duration with millis precision
        let duration_ms = match (start_unix_millis, end_unix_millis) {
            (Some(start), Some(end)) if end > start => {
                Some((end - start) as f64)
            }
            _ => None,
        };
        
        ProcessLifetime {
            pid: p.pid,
            parent_pid: p.parent_pid,
            start_ticks: p.start_time_ticks,
            end_ticks: p.exit_time,
            start_unix_seconds: start_unix,
            start_unix_millis,
            end_unix_seconds: end_unix,
            end_unix_millis,
            duration_ms,
            state: p.state.clone(),
            exit_code: p.exit_code,
        }
    }).collect()
}

fn calculate_network_connections(network: &[NetworkInfo]) -> Vec<NetworkConnection> {
    network.iter().map(|n| {
        let duration_ms = n.attributes.get("duration_ms")
            .and_then(|s| s.parse().ok());
        
        NetworkConnection {
            source_pid: n.source_pid,
            target_pid: n.target_pid,
            protocol: n.protocol.clone(),
            local_address: n.local_address.clone(),
            remote_address: n.remote_address.clone(),
            duration_ms,
            state: n.state.clone(),
        }
    }).collect()
}

fn calculate_file_operations(files: &[FileInfo]) -> Vec<FileOperation> {
    files.iter().map(|f| {
        let timestamp = f.attributes.get("timestamp")
            .and_then(|s| s.parse().ok());
        
        FileOperation {
            pid: f.pid,
            path: f.path.clone(),
            operation: f.operation.clone(),
            timestamp,
            fd: f.fd,
        }
    }).collect()
}

fn calculate_summary(
    process_lifetimes: &[ProcessLifetime],
    network_connections: &[NetworkConnection],
    file_operations: &[FileOperation],
) -> LatencySummary {
    let total = process_lifetimes.len();
    let active = process_lifetimes.iter().filter(|p| p.state == "Active").count();
    let exited = process_lifetimes.iter().filter(|p| p.state == "Exited").count();
    
    let durations: Vec<f64> = process_lifetimes.iter()
        .filter_map(|p| p.duration_ms)
        .collect();
    
    let avg_duration = if !durations.is_empty() {
        Some(durations.iter().sum::<f64>() / durations.len() as f64)
    } else {
        None
    };
    
    let max_duration = durations.iter().cloned().fold(None, |max, x| {
        Some(max.map_or(x, |m: f64| m.max(x)))
    });
    
    let min_duration = durations.iter().cloned().fold(None, |min, x| {
        Some(min.map_or(x, |m: f64| m.min(x)))
    });
    
    let trace_duration = if !durations.is_empty() {
        let min_start = process_lifetimes.iter()
            .map(|p| p.start_ticks)
            .min()
            .unwrap_or(0) as f64 / 1_000_000.0;
        
        let max_end = process_lifetimes.iter()
            .filter_map(|p| p.duration_ms)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);
        
        if max_end > min_start {
            Some(max_end - min_start)
        } else {
            Some(max_end)
        }
    } else {
        None
    };
    
    LatencySummary {
        total_processes: total,
        active_processes: active,
        exited_processes: exited,
        avg_process_duration_ms: avg_duration,
        max_process_duration_ms: max_duration,
        min_process_duration_ms: min_duration,
        total_network_connections: network_connections.len(),
        total_file_operations: file_operations.len(),
        trace_duration_ms: trace_duration,
    }
}

fn parse_pid_from_id(id: &str) -> Option<u32> {
    if let Some(pid_str) = id.strip_prefix("process:") {
        pid_str.split(':').next()?.parse().ok()
    } else {
        id.parse().ok()
    }
}
