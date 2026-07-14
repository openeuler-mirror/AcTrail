//! Small HTTP boundary for the read-only web UI.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::args::WebConfig;
use crate::{render, view};
use config_core::daemon::OperatorConfig;
use storage_core::StorageOpenMode;
use storage_factory::{StorageConfig, open_storage_backend};

#[path = "http/query.rs"]
mod query;
use query::{
    parse_action_tree_page, parse_llm_activity_query, parse_llm_export_query,
    parse_llm_request_content_query, parse_llm_rows_query, parse_token_usage_stats_query,
    parse_u64, percent_decode, required_query_param,
};

const STATUS_OK: &str = "200 OK";
const STATUS_BAD_REQUEST: &str = "400 Bad Request";
const STATUS_NOT_FOUND: &str = "404 Not Found";
const STATUS_METHOD_NOT_ALLOWED: &str = "405 Method Not Allowed";
const STATUS_INTERNAL_ERROR: &str = "500 Internal Server Error";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestBudget {
    Forever,
    Count(usize),
}

pub fn run_server(config: WebConfig) -> Result<(), String> {
    if let Some(cluster_root) = &config.cluster_root {
        view::cluster::validate_cluster_root(cluster_root)?;
    } else {
        validate_storage(&config.storage)?;
    }
    let listener = TcpListener::bind(config.listen_addr)
        .map_err(|error| format!("bind {} failed: {error}", config.listen_addr))?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    if let Some(cluster_root) = &config.cluster_root {
        println!(
            "actrailweb listening on http://{address} cluster_root={}",
            cluster_root.display()
        );
    } else {
        println!(
            "actrailweb listening on http://{address} storage={}",
            config.storage.path().display()
        );
    }
    println!("actrailweb is running; press Ctrl-C to stop");
    serve_listener_with_config(listener, config, RequestBudget::Forever)
}

pub fn serve_listener(
    listener: TcpListener,
    storage: StorageConfig,
    request_read_timeout: Option<Duration>,
    budget: RequestBudget,
) -> Result<(), String> {
    let context = WebContext {
        storage,
        cluster_root: None,
        request_read_timeout,
        operator_config_path: None,
        operator_config: None,
    };
    serve_listener_context(listener, context, budget)
}

fn serve_listener_with_config(
    listener: TcpListener,
    config: WebConfig,
    budget: RequestBudget,
) -> Result<(), String> {
    let context = WebContext {
        storage: config.storage,
        cluster_root: config.cluster_root,
        request_read_timeout: config.request_read_timeout,
        operator_config_path: config.operator_config_path,
        operator_config: config.operator_config,
    };
    serve_listener_context(listener, context, budget)
}

fn serve_listener_context(
    listener: TcpListener,
    context: WebContext,
    budget: RequestBudget,
) -> Result<(), String> {
    match budget {
        RequestBudget::Forever => loop {
            let (stream, _) = listener.accept().map_err(|error| error.to_string())?;
            detach_connection(stream, context.clone());
        },
        RequestBudget::Count(count) => {
            let mut handles = Vec::new();
            for _ in 0..count {
                let (stream, _) = listener.accept().map_err(|error| error.to_string())?;
                handles.push(spawn_connection(stream, context.clone()));
            }
            join_connections(handles)
        }
    }
}

#[derive(Clone)]
struct WebContext {
    storage: StorageConfig,
    cluster_root: Option<std::path::PathBuf>,
    request_read_timeout: Option<Duration>,
    operator_config_path: Option<std::path::PathBuf>,
    operator_config: Option<OperatorConfig>,
}

fn detach_connection(stream: TcpStream, context: WebContext) {
    drop(thread::spawn(move || {
        if let Err(error) = serve_stream(stream, &context) {
            eprintln!("actrailweb request failed: {error}");
        }
    }));
}

fn validate_storage(storage: &StorageConfig) -> Result<(), String> {
    if !storage.path().exists() {
        return Err(format!(
            "storage path does not exist: {}",
            storage.path().display()
        ));
    }
    open_storage_backend(storage, StorageOpenMode::ReadOnly)
        .map(|_| ())
        .map_err(|error| {
            format!(
                "open storage read-only failed: {}: {}",
                error.stage, error.message
            )
        })
}

fn spawn_connection(stream: TcpStream, context: WebContext) -> JoinHandle<Result<(), String>> {
    thread::spawn(move || serve_stream(stream, &context))
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

fn serve_stream(mut stream: TcpStream, context: &WebContext) -> Result<(), String> {
    stream
        .set_read_timeout(context.request_read_timeout)
        .map_err(|error| format!("set request read timeout failed: {error}"))?;
    let request = read_request(&mut stream)?;
    let response = match route(&request, context) {
        Ok(response) => response,
        Err(error) => Response::text(STATUS_INTERNAL_ERROR, error),
    };
    let response = response.with_optional_gzip(request.accepts_gzip);
    write_response(&mut stream, &response).map_err(|error| error.to_string())
}

fn write_response(stream: &mut TcpStream, response: &Response) -> std::io::Result<()> {
    let mut headers = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        response.content_type,
        response.body.len(),
    );
    if let Some(encoding) = response.content_encoding {
        headers.push_str(&format!("Content-Encoding: {encoding}\r\n"));
    }
    headers.push_str("\r\n");
    stream.write_all(headers.as_bytes())?;
    stream.write_all(&response.body)
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
    let mut accepts_gzip = false;
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        reader
            .read_line(&mut header)
            .map_err(|error| error.to_string())?;
        if header == "\r\n" || header == "\n" || header.is_empty() {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if lower.starts_with("accept-encoding:") && lower.contains("gzip") {
            accepts_gzip = true;
        }
        if lower.starts_with("content-length:") {
            let (_, raw) = header
                .split_once(':')
                .ok_or_else(|| format!("invalid Content-Length header {header:?}"))?;
            content_length = raw
                .trim()
                .parse::<usize>()
                .map_err(|error| format!("invalid Content-Length header: {error}"))?;
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).map_err(|error| {
            format!("read HTTP request body failed for {content_length} bytes: {error}")
        })?;
    }
    Ok(Request {
        method: parts[0].to_string(),
        path: parts[1].to_string(),
        accepts_gzip,
        body,
    })
}

fn route(request: &Request, context: &WebContext) -> Result<Response, String> {
    let (path, query) = split_request_target(&request.path);
    if path == "/api/cache/clear" {
        if request.method != "POST" {
            return Ok(Response::text(
                STATUS_METHOD_NOT_ALLOWED,
                "POST required for /api/cache/clear",
            ));
        }
        return Ok(Response::json(view::clear_cache_json()?));
    }
    if path == "/api/plugins/runtime/unload" {
        if request.method != "POST" {
            return Ok(Response::text(
                STATUS_METHOD_NOT_ALLOWED,
                "POST required for /api/plugins/runtime/unload",
            ));
        }
        return required_query_param(query, "instance_id")
            .and_then(|instance_id| {
                view::runtime_plugin_unload_json(
                    context.operator_config_path.as_deref(),
                    context.operator_config.as_ref(),
                    &instance_id,
                )
            })
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)));
    }
    if path == "/api/stats/llm-requests/explore" {
        if request.method != "POST" {
            return Ok(Response::text(
                STATUS_METHOD_NOT_ALLOWED,
                "POST required for /api/stats/llm-requests/explore",
            ));
        }
        return String::from_utf8(request.body.clone())
            .map_err(|error| format!("invalid UTF-8 explore request body: {error}"))
            .and_then(|body| view::parse_llm_explore_query(&body))
            .and_then(|query| view::llm_explore_json(&context.storage, query))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)));
    }
    if request.method != "GET" {
        return Ok(Response::text(
            STATUS_METHOD_NOT_ALLOWED,
            "only GET is supported",
        ));
    }
    if path == "/" || path.starts_with("/assets/") {
        return Ok(render::asset(path)
            .map(Response::static_asset)
            .unwrap_or_else(|| Response::text(STATUS_NOT_FOUND, "not found")));
    }
    match path {
        "/api/traces" => match &context.cluster_root {
            Some(cluster_root) => view::cluster::traces_json(cluster_root).map(Response::json),
            None => view::traces_json(&context.storage).map(Response::json),
        },
        "/api/config/current" => view::current_config_json(
            context.operator_config_path.as_deref(),
            context.operator_config.as_ref(),
        )
        .map(Response::json),
        "/api/plugins/enabled" => view::plugin_enablement_json(
            context.operator_config_path.as_deref(),
            context.operator_config.as_ref(),
        )
        .map(Response::json),
        "/api/plugins/runtime" => view::runtime_plugin_status_json(
            context.operator_config_path.as_deref(),
            context.operator_config.as_ref(),
        )
        .map(Response::json),
        "/api/stats/token-usage" => match parse_token_usage_stats_query(query) {
            Ok(query) => view::token_usage_stats_json(&context.storage, query).map(Response::json),
            Err(error) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
        },
        "/api/stats/llm-requests/activity" => match parse_llm_activity_query(query) {
            Ok(query) => view::llm_activity_json(&context.storage, query).map(Response::json),
            Err(error) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
        },
        "/api/stats/llm-requests/rows" => match parse_llm_rows_query(query) {
            Ok(query) => view::llm_request_rows_json(&context.storage, query).map(Response::json),
            Err(error) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
        },
        "/api/stats/llm-requests/export.csv" => match parse_llm_export_query(query) {
            Ok(query) => view::llm_export_csv(&context.storage, query).map(Response::csv),
            Err(error) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
        },
        "/health" => Ok(Response::text(STATUS_OK, "ok")),
        _ => match &context.cluster_root {
            Some(cluster_root) => route_cluster_trace_api(path, query, cluster_root),
            None => route_trace_api(path, query, &context.storage),
        },
    }
}

fn route_cluster_trace_api(
    path: &str,
    query: &str,
    cluster_root: &std::path::Path,
) -> Result<Response, String> {
    let parts = path
        .strip_prefix("/api/traces/")
        .map(|suffix| suffix.split('/').collect::<Vec<_>>());
    let Some(parts) = parts else {
        return Ok(Response::text(STATUS_NOT_FOUND, "not found"));
    };
    match parts.as_slice() {
        [trace_id] => parse_u64(trace_id)
            .and_then(|trace_id| view::cluster::trace_json(cluster_root, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "summary"] => parse_u64(trace_id)
            .and_then(|trace_id| view::cluster::trace_summary_json(cluster_root, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "events"] => parse_u64(trace_id)
            .and_then(|trace_id| view::cluster::trace_events_json(cluster_root, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "payloads"] => parse_u64(trace_id)
            .and_then(|trace_id| view::cluster::trace_payloads_json(cluster_root, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "timeline"] => parse_u64(trace_id)
            .and_then(|trace_id| view::cluster::trace_timeline_json(cluster_root, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "processes"] => parse_u64(trace_id)
            .and_then(|trace_id| view::cluster::trace_processes_json(cluster_root, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "diagnostics"] => parse_u64(trace_id)
            .and_then(|trace_id| view::cluster::trace_diagnostics_json(cluster_root, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree"] => parse_u64(trace_id)
            .and_then(|trace_id| view::cluster::action_tree_json(cluster_root, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree", "root"] => parse_u64(trace_id)
            .and_then(|trace_id| view::cluster::action_tree_root_json(cluster_root, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree", "children", parent_id] => {
            let trace_id = parse_u64(trace_id);
            let parent_id = percent_decode(parent_id);
            let page = parse_action_tree_page(query);
            match (trace_id, parent_id, page) {
                (Ok(trace_id), Ok(parent_id), Ok(page)) => {
                    view::cluster::action_tree_children_json(
                        cluster_root,
                        trace_id,
                        &parent_id,
                        page,
                    )
                    .map(Response::json)
                    .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "commands"] => parse_u64(trace_id)
            .and_then(|trace_id| view::cluster::commands_json(cluster_root, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "actions", action_id] => {
            let trace_id = parse_u64(trace_id);
            let action_id = percent_decode(action_id);
            match (trace_id, action_id) {
                (Ok(trace_id), Ok(action_id)) => {
                    view::cluster::action_detail_json(cluster_root, trace_id, &action_id)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        [trace_id, "actions", action_id, "file-path-set"] => {
            let trace_id = parse_u64(trace_id);
            let action_id = percent_decode(action_id);
            let page = parse_action_tree_page(query);
            match (trace_id, action_id, page) {
                (Ok(trace_id), Ok(action_id), Ok(page)) => {
                    view::cluster::file_path_set_json(cluster_root, trace_id, &action_id, page)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "actions", action_id, "content", "llm-request"] => {
            let trace_id = parse_u64(trace_id);
            let action_id = percent_decode(action_id);
            let max_bytes = parse_llm_request_content_query(query);
            match (trace_id, action_id, max_bytes) {
                (Ok(trace_id), Ok(action_id), Ok(max_bytes)) => {
                    view::cluster::llm_request_content_json(
                        cluster_root,
                        trace_id,
                        &action_id,
                        max_bytes,
                    )
                    .map(Response::json)
                    .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "payloads", segment_id] => {
            let trace_id = parse_u64(trace_id);
            let segment_id = parse_u64(segment_id);
            match (trace_id, segment_id) {
                (Ok(trace_id), Ok(segment_id)) => {
                    view::cluster::payload_json(cluster_root, trace_id, segment_id)
                        .map(Response::json)
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        _ => Ok(Response::text(STATUS_NOT_FOUND, "not found")),
    }
}

fn route_trace_api(path: &str, query: &str, storage: &StorageConfig) -> Result<Response, String> {
    let parts = path
        .strip_prefix("/api/traces/")
        .map(|suffix| suffix.split('/').collect::<Vec<_>>());
    let Some(parts) = parts else {
        return Ok(Response::text(STATUS_NOT_FOUND, "not found"));
    };
    match parts.as_slice() {
        [trace_id] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "summary"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_summary_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "events"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_events_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "payloads"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_payloads_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "timeline"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_timeline_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "processes"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_processes_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "diagnostics"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_diagnostics_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree"] => parse_u64(trace_id)
            .and_then(|trace_id| view::action_tree_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree", "root"] => parse_u64(trace_id)
            .and_then(|trace_id| view::action_tree_root_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree", "children", parent_id] => {
            let trace_id = parse_u64(trace_id);
            let parent_id = percent_decode(parent_id);
            let page = parse_action_tree_page(query);
            match (trace_id, parent_id, page) {
                (Ok(trace_id), Ok(parent_id), Ok(page)) => {
                    view::action_tree_children_json(storage, trace_id, &parent_id, page)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "actions", action_id] => {
            let trace_id = parse_u64(trace_id);
            let action_id = percent_decode(action_id);
            match (trace_id, action_id) {
                (Ok(trace_id), Ok(action_id)) => {
                    view::action_detail_json(storage, trace_id, &action_id)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        [trace_id, "actions", action_id, "file-path-set"] => {
            let trace_id = parse_u64(trace_id);
            let action_id = percent_decode(action_id);
            let page = parse_action_tree_page(query);
            match (trace_id, action_id, page) {
                (Ok(trace_id), Ok(action_id), Ok(page)) => {
                    view::action_file_path_set_json(storage, trace_id, &action_id, page)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "actions", action_id, "content", "llm-request"] => {
            let trace_id = parse_u64(trace_id);
            let action_id = percent_decode(action_id);
            let max_bytes = parse_llm_request_content_query(query);
            match (trace_id, action_id, max_bytes) {
                (Ok(trace_id), Ok(action_id), Ok(max_bytes)) => {
                    view::action_llm_request_content_json(storage, trace_id, &action_id, max_bytes)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    Ok(Response::text(STATUS_BAD_REQUEST, error))
                }
            }
        }
        [trace_id, "commands"] => parse_u64(trace_id)
            .and_then(|trace_id| view::commands_json(storage, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "payloads", segment_id] => {
            let trace_id = parse_u64(trace_id);
            let segment_id = parse_u64(segment_id);
            match (trace_id, segment_id) {
                (Ok(trace_id), Ok(segment_id)) => {
                    view::payload_json(storage, trace_id, segment_id).map(Response::json)
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        _ => Ok(Response::text(STATUS_NOT_FOUND, "not found")),
    }
}

fn split_request_target(target: &str) -> (&str, &str) {
    target
        .split_once('?')
        .map(|(path, query)| (path, query))
        .unwrap_or((target, ""))
}

struct Request {
    method: String,
    path: String,
    accepts_gzip: bool,
    body: Vec<u8>,
}

struct Response {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
    content_encoding: Option<&'static str>,
}

impl Response {
    fn static_asset(asset: render::StaticAsset) -> Self {
        Self::text_bytes(STATUS_OK, asset.content_type, asset.body.to_vec())
    }

    fn json(body: String) -> Self {
        Self::text_bytes(
            STATUS_OK,
            "application/json; charset=utf-8",
            body.into_bytes(),
        )
    }

    fn csv(body: String) -> Self {
        Self::text_bytes(STATUS_OK, "text/csv; charset=utf-8", body.into_bytes())
    }

    fn text(status: &'static str, body: impl Into<String>) -> Self {
        Self::text_bytes(
            status,
            "text/plain; charset=utf-8",
            body.into().into_bytes(),
        )
    }

    fn text_bytes(status: &'static str, content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type,
            body,
            content_encoding: None,
        }
    }

    fn with_optional_gzip(mut self, enabled: bool) -> Self {
        if !enabled {
            return self;
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        if encoder.write_all(&self.body).is_err() {
            return self;
        }
        let compressed = match encoder.finish() {
            Ok(body) => body,
            Err(_) => return self,
        };
        if compressed.len() >= self.body.len() {
            return self;
        }
        self.body = compressed;
        self.content_encoding = Some("gzip");
        self
    }
}
