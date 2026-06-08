use crate::trace_data::{LatencyAnalysis, ProcessLifetime, NetworkConnection, FileOperation};
use serde::Serialize;
use std::collections::BTreeMap;

pub fn get_process_hierarchy(analysis: &LatencyAnalysis) -> Vec<ProcessTreeNode> {
    let process_map: BTreeMap<u32, &ProcessLifetime> = analysis.process_lifetimes.iter()
        .map(|p| (p.pid, p))
        .collect();
    
    let mut children_map: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for process in &analysis.process_lifetimes {
        if let Some(parent_pid) = process.parent_pid {
            children_map.entry(parent_pid).or_default().push(process.pid);
        }
    }
    
    let root_processes = analysis.process_lifetimes.iter()
        .filter(|p| p.parent_pid.is_none())
        .collect::<Vec<_>>();
    
    root_processes.iter().map(|p| {
        build_tree_node(p, &children_map, &process_map, 0)
    }).collect()
}

fn build_tree_node<'a>(
    process: &'a ProcessLifetime,
    children_map: &BTreeMap<u32, Vec<u32>>,
    process_map: &BTreeMap<u32, &'a ProcessLifetime>,
    depth: usize,
) -> ProcessTreeNode {
    let children = children_map.get(&process.pid)
        .map(|child_pids| {
            child_pids.iter()
                .filter_map(|pid| process_map.get(pid))
                .map(|child| build_tree_node(child, children_map, process_map, depth + 1))
                .collect()
        })
        .unwrap_or_default();
    
    ProcessTreeNode {
        pid: process.pid,
        parent_pid: process.parent_pid,
        depth,
        duration_ms: process.duration_ms,
        start_unix_seconds: process.start_unix_seconds,
        start_unix_millis: process.start_unix_millis,
        end_unix_seconds: process.end_unix_seconds,
        end_unix_millis: process.end_unix_millis,
        state: process.state.clone(),
        exit_code: process.exit_code,
        children,
    }
}

#[derive(Serialize)]
pub struct ProcessTreeNode {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub depth: usize,
    pub duration_ms: Option<f64>,
    pub start_unix_seconds: Option<u64>,
    pub start_unix_millis: Option<u64>,
    pub end_unix_seconds: Option<u64>,
    pub end_unix_millis: Option<u64>,
    pub state: String,
    pub exit_code: Option<i32>,
    pub children: Vec<ProcessTreeNode>,
}

pub fn get_slowest_processes(analysis: &LatencyAnalysis, limit: usize) -> Vec<&ProcessLifetime> {
    let mut sorted: Vec<_> = analysis.process_lifetimes.iter()
        .filter(|p| p.duration_ms.is_some())
        .collect();
    
    sorted.sort_by(|a, b| {
        b.duration_ms.partial_cmp(&a.duration_ms).unwrap_or(std::cmp::Ordering::Equal)
    });
    
    sorted.into_iter().take(limit).collect()
}

#[allow(dead_code)]
pub fn get_longest_network_connections(analysis: &LatencyAnalysis, limit: usize) -> Vec<&NetworkConnection> {
    let mut sorted: Vec<_> = analysis.network_connections.iter()
        .filter(|c| c.duration_ms.is_some())
        .collect();
    
    sorted.sort_by(|a, b| {
        b.duration_ms.partial_cmp(&a.duration_ms).unwrap_or(std::cmp::Ordering::Equal)
    });
    
    sorted.into_iter().take(limit).collect()
}

#[allow(dead_code)]
pub fn group_file_operations_by_pid(analysis: &LatencyAnalysis) -> BTreeMap<u32, Vec<&FileOperation>> {
    let mut groups: BTreeMap<u32, Vec<&FileOperation>> = BTreeMap::new();
    
    for op in &analysis.file_operations {
        groups.entry(op.pid).or_default().push(op);
    }
    
    groups
}

#[allow(dead_code)]
pub fn format_duration_ms(ms: Option<f64>) -> String {
    match ms {
        Some(value) => {
            if value >= 1000.0 {
                format!("{:.2}s", value / 1000.0)
            } else if value >= 1.0 {
                format!("{:.2}ms", value)
            } else {
                format!("{:.2}μs", value * 1000.0)
            }
        }
        None => "N/A".to_string(),
    }
}

#[allow(dead_code)]
pub fn format_ticks(ticks: u64) -> String {
    if ticks >= 1_000_000_000 {
        format!("{:.2}s", ticks as f64 / 1_000_000_000.0)
    } else if ticks >= 1_000_000 {
        format!("{:.2}ms", ticks as f64 / 1_000_000.0)
    } else {
        format!("{ticks} ticks")
    }
}
