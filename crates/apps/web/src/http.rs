//! Small HTTP boundary for the read-only web UI.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::args::WebConfig;
use crate::{render, view};

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
    validate_storage(&config.storage_path)?;
    let listener = TcpListener::bind(config.listen_addr)
        .map_err(|error| format!("bind {} failed: {error}", config.listen_addr))?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    println!(
        "actrailweb listening on http://{address} storage={}",
        config.storage_path.display()
    );
    println!("actrailweb is running; press Ctrl-C to stop");
    serve_listener(
        listener,
        config.storage_path,
        config.request_read_timeout,
        RequestBudget::Forever,
    )
}

pub fn serve_listener(
    listener: TcpListener,
    storage_path: PathBuf,
    request_read_timeout: Option<Duration>,
    budget: RequestBudget,
) -> Result<(), String> {
    match budget {
        RequestBudget::Forever => loop {
            let (stream, _) = listener.accept().map_err(|error| error.to_string())?;
            detach_connection(stream, storage_path.clone(), request_read_timeout);
        },
        RequestBudget::Count(count) => {
            let mut handles = Vec::new();
            for _ in 0..count {
                let (stream, _) = listener.accept().map_err(|error| error.to_string())?;
                handles.push(spawn_connection(
                    stream,
                    storage_path.clone(),
                    request_read_timeout,
                ));
            }
            join_connections(handles)
        }
    }
}

fn detach_connection(
    stream: TcpStream,
    storage_path: PathBuf,
    request_read_timeout: Option<Duration>,
) {
    drop(thread::spawn(move || {
        if let Err(error) = serve_stream(stream, &storage_path, request_read_timeout) {
            eprintln!("actrailweb request failed: {error}");
        }
    }));
}

fn validate_storage(storage_path: &Path) -> Result<(), String> {
    if !storage_path.exists() {
        return Err(format!(
            "storage path does not exist: {}",
            storage_path.display()
        ));
    }
    sqlite_storage::SqliteStorage::open_read_only(storage_path)
        .map(|_| ())
        .map_err(|error| format!("open storage read-only failed: {error}"))
}

fn spawn_connection(
    stream: TcpStream,
    storage_path: PathBuf,
    request_read_timeout: Option<Duration>,
) -> JoinHandle<Result<(), String>> {
    thread::spawn(move || serve_stream(stream, &storage_path, request_read_timeout))
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
    storage_path: &Path,
    request_read_timeout: Option<Duration>,
) -> Result<(), String> {
    stream
        .set_read_timeout(request_read_timeout)
        .map_err(|error| format!("set request read timeout failed: {error}"))?;
    let request = read_request(&mut stream)?;
    let response = match route(&request, storage_path) {
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
    loop {
        let mut header = String::new();
        reader
            .read_line(&mut header)
            .map_err(|error| error.to_string())?;
        if header == "\r\n" || header == "\n" || header.is_empty() {
            break;
        }
        if header
            .to_ascii_lowercase()
            .starts_with("accept-encoding:")
            && header.to_ascii_lowercase().contains("gzip")
        {
            accepts_gzip = true;
        }
    }
    Ok(Request {
        method: parts[0].to_string(),
        path: parts[1].to_string(),
        accepts_gzip,
    })
}

fn route(request: &Request, storage_path: &Path) -> Result<Response, String> {
    if request.method != "GET" {
        return Ok(Response::text(
            STATUS_METHOD_NOT_ALLOWED,
            "only GET is supported",
        ));
    }
    let path = request
        .path
        .split('?')
        .next()
        .unwrap_or(request.path.as_str());
    match path {
        "/" => Ok(Response::html(render::html())),
        "/assets/app.css" => Ok(Response::css(render::css())),
        "/assets/app.js" => Ok(Response::javascript(render::javascript())),
        "/api/traces" => view::traces_json(storage_path).map(Response::json),
        "/health" => Ok(Response::text(STATUS_OK, "ok")),
        _ => route_trace_api(path, storage_path),
    }
}

fn route_trace_api(path: &str, storage_path: &Path) -> Result<Response, String> {
    let parts = path
        .strip_prefix("/api/traces/")
        .map(|suffix| suffix.split('/').collect::<Vec<_>>());
    let Some(parts) = parts else {
        return Ok(Response::text(STATUS_NOT_FOUND, "not found"));
    };
    match parts.as_slice() {
        [trace_id] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_json(storage_path, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "summary"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_summary_json(storage_path, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "events"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_events_json(storage_path, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "payloads"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_payloads_json(storage_path, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "timeline"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_timeline_json(storage_path, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "processes"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_processes_json(storage_path, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "diagnostics"] => parse_u64(trace_id)
            .and_then(|trace_id| view::trace_diagnostics_json(storage_path, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree"] => parse_u64(trace_id)
            .and_then(|trace_id| view::action_tree_json(storage_path, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree", "root"] => parse_u64(trace_id)
            .and_then(|trace_id| view::action_tree_root_json(storage_path, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "action-tree", "children", parent_id] => {
            let trace_id = parse_u64(trace_id);
            let parent_id = percent_decode(parent_id);
            match (trace_id, parent_id) {
                (Ok(trace_id), Ok(parent_id)) => {
                    view::action_tree_children_json(storage_path, trace_id, &parent_id)
                        .map(Response::json)
                        .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error)))
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        [trace_id, "commands"] => parse_u64(trace_id)
            .and_then(|trace_id| view::commands_json(storage_path, trace_id))
            .map(Response::json)
            .or_else(|error| Ok(Response::text(STATUS_BAD_REQUEST, error))),
        [trace_id, "payloads", segment_id] => {
            let trace_id = parse_u64(trace_id);
            let segment_id = parse_u64(segment_id);
            match (trace_id, segment_id) {
                (Ok(trace_id), Ok(segment_id)) => {
                    view::payload_json(storage_path, trace_id, segment_id).map(Response::json)
                }
                (Err(error), _) | (_, Err(error)) => Ok(Response::text(STATUS_BAD_REQUEST, error)),
            }
        }
        _ => Ok(Response::text(STATUS_NOT_FOUND, "not found")),
    }
}

fn parse_u64(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|error| format!("invalid numeric path segment {value}: {error}"))
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            output.push(bytes[index]);
            index += 1;
            continue;
        }
        let Some(hex) = bytes.get(index + 1..index + 3) else {
            return Err(format!("invalid percent escape in {value}"));
        };
        let text = std::str::from_utf8(hex)
            .map_err(|error| format!("invalid percent escape in {value}: {error}"))?;
        let decoded = u8::from_str_radix(text, 16)
            .map_err(|error| format!("invalid percent escape %{text}: {error}"))?;
        output.push(decoded);
        index += 3;
    }
    String::from_utf8(output).map_err(|error| format!("invalid path utf-8: {error}"))
}

struct Request {
    method: String,
    path: String,
    accepts_gzip: bool,
}

struct Response {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
    content_encoding: Option<&'static str>,
}

impl Response {
    fn html(body: String) -> Self {
        Self::text_bytes(STATUS_OK, "text/html; charset=utf-8", body.into_bytes())
    }

    fn css(body: String) -> Self {
        Self::text_bytes(STATUS_OK, "text/css; charset=utf-8", body.into_bytes())
    }

    fn javascript(body: String) -> Self {
        Self::text_bytes(
            STATUS_OK,
            "application/javascript; charset=utf-8",
            body.into_bytes(),
        )
    }

    fn json(body: String) -> Self {
        Self::text_bytes(STATUS_OK, "application/json; charset=utf-8", body.into_bytes())
    }

    fn text(status: &'static str, body: impl Into<String>) -> Self {
        Self::text_bytes(status, "text/plain; charset=utf-8", body.into().into_bytes())
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
        if !enabled || self.body.len() < 1024 {
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
