use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::args::WebConfig;
use crate::trace_data;
use crate::analysis;
use crate::render;
use crate::sqlite_read;
use serde_json::json;

const STATUS_OK: &str = "200 OK";
const STATUS_NOT_FOUND: &str = "404 Not Found";
const STATUS_METHOD_NOT_ALLOWED: &str = "405 Method Not Allowed";
const STATUS_INTERNAL_ERROR: &str = "500 Internal Server Error";

#[derive(Clone)]
pub enum DataSource {
    TraceFile(PathBuf),
    Sqlite(PathBuf),
}

pub enum RequestBudget {
    Forever,
    Count(usize),
}

pub fn run_server(config: WebConfig) -> Result<(), String> {
    let data_source = if let Some(ref trace_path) = config.trace_path {
        validate_trace(trace_path)?;
        println!("actrailweb listening on http://{} trace={}", config.listen_addr, trace_path.display());
        DataSource::TraceFile(trace_path.clone())
    } else if let Some(ref storage_path) = config.storage_path {
        validate_storage(storage_path)?;
        println!("actrailweb listening on http://{} storage={}", config.listen_addr, storage_path.display());
        DataSource::Sqlite(storage_path.clone())
    } else {
        return Err("no data source provided".to_string());
    };
    
    let listener = TcpListener::bind(config.listen_addr)
        .map_err(|error| format!("bind {} failed: {error}", config.listen_addr))?;
    println!("actrailweb is running; press Ctrl-C to stop");
    serve_listener(
        listener,
        data_source,
        config.request_read_timeout,
        RequestBudget::Forever,
    )
}

pub fn serve_listener(
    listener: TcpListener,
    data_source: DataSource,
    request_read_timeout: Option<Duration>,
    budget: RequestBudget,
) -> Result<(), String> {
    match budget {
        RequestBudget::Forever => loop {
            let (stream, _) = listener.accept().map_err(|error| error.to_string())?;
            detach_connection(stream, data_source.clone(), request_read_timeout);
        },
        RequestBudget::Count(count) => {
            let mut handles = Vec::new();
            for _ in 0..count {
                let (stream, _) = listener.accept().map_err(|error| error.to_string())?;
                handles.push(spawn_connection(
                    stream,
                    data_source.clone(),
                    request_read_timeout,
                ));
            }
            join_connections(handles)
        }
    }
}

fn detach_connection(
    stream: TcpStream,
    data_source: DataSource,
    request_read_timeout: Option<Duration>,
) {
    drop(thread::spawn(move || {
        if let Err(error) = serve_stream(stream, &data_source, request_read_timeout) {
            eprintln!("actrailweb request failed: {error}");
        }
    }));
}

fn validate_trace(trace_path: &Path) -> Result<(), String> {
    if !trace_path.exists() {
        return Err(format!("trace file does not exist: {}", trace_path.display()));
    }
    trace_data::load_trace(trace_path).map(|_| ())
}

fn validate_storage(storage_path: &Path) -> Result<(), String> {
    if !storage_path.exists() {
        return Err(format!("storage file does not exist: {}", storage_path.display()));
    }
    Ok(())
}

fn spawn_connection(
    stream: TcpStream,
    data_source: DataSource,
    request_read_timeout: Option<Duration>,
) -> JoinHandle<Result<(), String>> {
    thread::spawn(move || serve_stream(stream, &data_source, request_read_timeout))
}

fn join_connections(handles: Vec<JoinHandle<Result<(), String>>>) -> Result<(), String> {
    for handle in handles {
        let result = handle
            .join()
            .map_err(|_| "actrailweb request worker panicked".to_string())?;
        result?;
    }
    Ok(())
}

fn serve_stream(
    mut stream: TcpStream,
    data_source: &DataSource,
    request_read_timeout: Option<Duration>,
) -> Result<(), String> {
    stream
        .set_read_timeout(request_read_timeout)
        .map_err(|error| format!("set request read timeout failed: {error}"))?;
    let response = match read_request(&mut stream).and_then(|request| route(&request, data_source))
    {
        Ok(response) => response,
        Err(error) => Response::text(STATUS_INTERNAL_ERROR, error),
    };
    stream
        .write_all(response.serialize().as_bytes())
        .map_err(|error| error.to_string())
}

fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|error| error.to_string())?;
    if request_line.trim().is_empty() {
        return Err("empty HTTP request".to_string());
    }
    let parts = request_line.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(format!("invalid HTTP request line {request_line:?}"));
    }
    loop {
        let mut header = String::new();
        reader
            .read_line(&mut header)
            .map_err(|error| error.to_string())?;
        if header == "\r\n" || header == "\n" || header.is_empty() {
            break;
        }
    }
    Ok(Request {
        method: parts[0].to_string(),
        path: parts[1].to_string(),
    })
}

fn route(request: &Request, data_source: &DataSource) -> Result<Response, String> {
    if request.method != "GET" {
        return Ok(Response::text(
            STATUS_METHOD_NOT_ALLOWED,
            "only GET is supported",
        ));
    }
    // Parse path and query string
    let (path, query) = parse_path_and_query(&request.path);
    
    // Handle static assets
    match path {
        "/" => Ok(Response::html(render::html())),
        "/assets/app.css" => Ok(Response::css(render::css())),
        "/assets/app.js" => Ok(Response::javascript(render::javascript())),
        "/health" => Ok(Response::text(STATUS_OK, "ok")),
        _ => route_api(path, query, data_source),
    }
}

fn route_api(path: &str, query: Option<&str>, data_source: &DataSource) -> Result<Response, String> {
    const STATUS_BAD_REQUEST: &str = "400 Bad Request";
    
    // /api/traces - list all traces
    if path == "/api/traces" {
        return traces_json(data_source).map(Response::json);
    }
    
    // /api/trace?id=<trace_id> - my style (supports both modes)
    if path == "/api/trace" {
        let trace_id = query
            .and_then(|q| parse_query_param(q, "id"))
            .and_then(|v: &str| v.parse::<u64>().ok())
            .unwrap_or(1);
        return trace_json(data_source, trace_id).map(Response::json);
    }
    
    // /api/traces/<trace_id>... - upstream style (SQLite only)
    if let Some(suffix) = path.strip_prefix("/api/traces/") {
        let parts: Vec<&str> = suffix.split('/').collect();
        
        // Only SQLite mode supports these routes
        match data_source {
            DataSource::TraceFile(_) => return Ok(Response::text(STATUS_NOT_FOUND, "not found - use /api/trace?id=<id> for trace file mode")),
            DataSource::Sqlite(storage_path) => {
                match parts.as_slice() {
                    [trace_id_str] => {
                        let trace_id = parse_u64(trace_id_str)?;
                        return crate::view::trace_json(storage_path, trace_id).map(Response::json)
                            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)));
                    },
                    [trace_id_str, "action-tree"] => {
                        let trace_id = parse_u64(trace_id_str)?;
                        return crate::view::action_tree_json(storage_path, trace_id).map(Response::json)
                            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)));
                    },
                    [trace_id_str, "payloads", segment_id_str] => {
                        let trace_id = parse_u64(trace_id_str)?;
                        let segment_id = parse_u64(segment_id_str)?;
                        return crate::view::payload_json(storage_path, trace_id, segment_id).map(Response::json)
                            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)));
                    },
                    _ => return Ok(Response::text(STATUS_NOT_FOUND, "not found")),
                }
            }
        }
    }
    
    Ok(Response::text(STATUS_NOT_FOUND, "not found"))
}

fn parse_u64(value: &str) -> Result<u64, String> {
    value.parse::<u64>()
        .map_err(|error| format!("invalid numeric path segment {value}: {error}"))
}

fn parse_path_and_query(path: &str) -> (&str, Option<&str>) {
    let mut parts = path.splitn(2, '?');
    let path_only = parts.next().unwrap_or(path);
    let query = parts.next();
    (path_only, query)
}

fn parse_query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next() == Some(key) {
            return kv.next();
        }
    }
    None
}

fn traces_json(data_source: &DataSource) -> Result<String, String> {
    match data_source {
        DataSource::TraceFile(_) => Ok("{\"traces\":[]}".to_string()),
        DataSource::Sqlite(storage_path) => crate::view::traces_json(storage_path),
    }
}

fn trace_json(data_source: &DataSource, trace_id: u64) -> Result<String, String> {
    match data_source {
        DataSource::TraceFile(trace_path) => trace_json_from_file(trace_path),
        DataSource::Sqlite(storage_path) => sqlite_read::trace_json(storage_path, trace_id),
    }
}

fn trace_json_from_file(trace_path: &Path) -> Result<String, String> {
    let document = trace_data::load_trace(trace_path)?;
    let analysis = trace_data::analyze_trace(&document);
    
    let process_tree = analysis::get_process_hierarchy(&analysis);
    let slowest = analysis::get_slowest_processes(&analysis, 10);
    
    let process_tree_json: Vec<serde_json::Value> = process_tree.iter().map(|node| {
        json!({
            "pid": node.pid,
            "parent_pid": node.parent_pid,
            "depth": node.depth,
            "duration_ms": node.duration_ms,
            "start_unix_seconds": node.start_unix_seconds,
            "start_unix_millis": node.start_unix_millis,
            "end_unix_seconds": node.end_unix_seconds,
            "end_unix_millis": node.end_unix_millis,
            "state": node.state,
            "exit_code": node.exit_code,
            "children": serde_json::to_value(&node.children).unwrap_or_default()
        })
    }).collect();
    
    let slowest_json: Vec<serde_json::Value> = slowest.iter().map(|p| {
        json!({
            "pid": p.pid,
            "parent_pid": p.parent_pid,
            "start_ticks": p.start_ticks,
            "duration_ms": p.duration_ms,
            "state": p.state,
            "exit_code": p.exit_code
        })
    }).collect();
    
    let process_lifetimes_json: Vec<serde_json::Value> = analysis.process_lifetimes.iter().map(|p| {
        json!({
            "pid": p.pid,
            "parent_pid": p.parent_pid,
            "start_ticks": p.start_ticks,
            "end_ticks": p.end_ticks,
            "start_unix_seconds": p.start_unix_seconds,
            "start_unix_millis": p.start_unix_millis,
            "end_unix_seconds": p.end_unix_seconds,
            "end_unix_millis": p.end_unix_millis,
            "duration_ms": p.duration_ms,
            "state": p.state,
            "exit_code": p.exit_code
        })
    }).collect();
    
    let network_json: Vec<serde_json::Value> = analysis.network_connections.iter().map(|n| {
        json!({
            "source_pid": n.source_pid,
            "target_pid": n.target_pid,
            "protocol": n.protocol,
            "local_address": n.local_address,
            "remote_address": n.remote_address,
            "duration_ms": n.duration_ms,
            "state": n.state
        })
    }).collect();
    
    let files_json: Vec<serde_json::Value> = analysis.file_operations.iter().map(|f| {
        json!({
            "pid": f.pid,
            "path": f.path,
            "operation": f.operation,
            "fd": f.fd,
            "timestamp": f.timestamp
        })
    }).collect();
    
    let timeline_json: Vec<serde_json::Value> = analysis.timeline_events.iter().map(|e| {
        json!({
            "timestamp_ms": e.timestamp_ms,
            "event_type": e.event_type,
            "pid": e.pid,
            "description": e.description,
            "duration_ms": e.duration_ms
        })
    }).collect();
    
    let summary = &analysis.summary;
    let summary_json = json!({
        "total_processes": summary.total_processes,
        "active_processes": summary.active_processes,
        "exited_processes": summary.exited_processes,
        "avg_process_duration_ms": summary.avg_process_duration_ms,
        "max_process_duration_ms": summary.max_process_duration_ms,
        "min_process_duration_ms": summary.min_process_duration_ms,
        "total_network_connections": summary.total_network_connections,
        "total_file_operations": summary.total_file_operations,
        "trace_duration_ms": summary.trace_duration_ms
    });
    
    let events_json: Vec<serde_json::Value> = analysis.events.iter().map(|e| {
        json!({
            "id": e.id,
            "display_id": e.id,
            "domain": e.domain,
            "pid": e.pid,
            "operation": e.operation,
            "summary": e.summary,
            "observed_at": e.observed_at,
            "metadata": e.metadata
        })
    }).collect();
    
    let resources_json: Vec<serde_json::Value> = analysis.resources.iter().map(|r| {
        json!({
            "id": r.id,
            "pid": r.pid,
            "scope": r.scope,
            "subject": r.subject,
            "cpu_percent_millis": r.cpu_percent_millis,
            "rss_kb": r.rss_kb,
            "virtual_memory_kb": r.virtual_memory_kb,
            "metadata": r.metadata
        })
    }).collect();
    
    let diagnostics_json: Vec<serde_json::Value> = analysis.diagnostics.iter().map(|d| {
        json!({
            "id": d.id,
            "display_id": d.id,
            "severity": d.severity,
            "kind": d.kind,
            "message": d.message,
            "metadata": d.metadata
        })
    }).collect();
    
    let output = json!({
        "trace_id": document.trace_id,
        "schema_version": document.schema_version,
        "completeness": document.completeness,
        "nodes": document.nodes,
        "edges": document.edges,
        "analysis": {
            "process_lifetimes": process_lifetimes_json,
            "events": events_json,
            "resources": resources_json,
            "diagnostics": diagnostics_json,
            "network_connections": network_json,
            "file_operations": files_json,
            "timeline_events": timeline_json,
            "process_tree": process_tree_json,
            "slowest_processes": slowest_json,
            "summary": summary_json
        }
    });
    
    serde_json::to_string(&output).map_err(|e| format!("failed to serialize JSON: {e}"))
}

struct Request {
    method: String,
    path: String,
}

struct Response {
    status: String,
    content_type: String,
    body: String,
}

impl Response {
    fn text(status: &str, body: impl Into<String>) -> Self {
        Self {
            status: status.to_string(),
            content_type: "text/plain; charset=utf-8".to_string(),
            body: body.into(),
        }
    }

    fn html(body: impl Into<String>) -> Self {
        Self {
            status: STATUS_OK.to_string(),
            content_type: "text/html; charset=utf-8".to_string(),
            body: body.into(),
        }
    }

    fn css(body: impl Into<String>) -> Self {
        Self {
            status: STATUS_OK.to_string(),
            content_type: "text/css; charset=utf-8".to_string(),
            body: body.into(),
        }
    }

    fn javascript(body: impl Into<String>) -> Self {
        Self {
            status: STATUS_OK.to_string(),
            content_type: "application/javascript; charset=utf-8".to_string(),
            body: body.into(),
        }
    }

    fn json(body: impl Into<String>) -> Self {
        Self {
            status: STATUS_OK.to_string(),
            content_type: "application/json; charset=utf-8".to_string(),
            body: body.into(),
        }
    }

    fn serialize(&self) -> String {
        format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.status,
            self.content_type,
            self.body.len(),
            self.body
        )
    }
}