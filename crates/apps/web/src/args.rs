use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

pub struct WebConfig {
    pub trace_path: Option<PathBuf>,
    pub storage_path: Option<PathBuf>,
    pub listen_addr: SocketAddr,
    pub request_read_timeout: Option<Duration>,
}

pub const HELP_TEXT: &str = "\
AcTrail Web UI - Enhanced trace viewer with latency analysis

Usage:
  actrailweb --trace <PATH> [--addr <ADDR>] [--port <PORT>] [--request-read-timeout-ms <MILLIS|disabled>]
  actrailweb --storage <PATH> [--addr <ADDR>] [--port <PORT>] [--request-read-timeout-ms <MILLIS|disabled>]
  actrailweb --config <PATH> [--addr <ADDR>] [--port <PORT>]

Options:
  --trace <PATH>                    Path to trace.json file
  --storage <PATH>                  Path to SQLite database file
  --config <PATH>                   Operator config path (uses storage_path from config)
  --addr <ADDR>                     Listen address (default: 127.0.0.1)
  --port <PORT>                     Listen port (default: 8080)
  --request-read-timeout-ms <VALUE> Request read timeout in milliseconds, or disabled
  -h, --help                        Print help
";

pub fn is_help_request(args: &[String]) -> bool {
    args.iter().any(|arg| matches!(arg.as_str(), "-h" | "--help"))
}

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<WebConfig, String> {
    let mut flags = std::collections::BTreeMap::new();
    let mut args = args.into_iter();
    
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("missing value for {flag}"))?;
        if !matches!(flag.as_str(), "--trace" | "--storage" | "--config" | "--addr" | "--port" | "--request-read-timeout-ms") {
            return Err(format!("unknown flag {flag}"));
        }
        if flags.insert(flag.clone(), value).is_some() {
            return Err(format!("duplicate flag {flag}"));
        }
    }

    // Check for config file
    let config_storage_path = if let Some(config_path) = flags.get("--config") {
        let config_path = PathBuf::from(config_path);
        if !config_path.exists() {
            return Err(format!("config file does not exist: {}", config_path.display()));
        }
        // Read config and extract storage_path
        let raw = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("read config failed: {e}"))?;
        let storage_path = extract_storage_path(&raw)?;
        Some(PathBuf::from(storage_path))
    } else {
        None
    };

    let trace_path = flags.get("--trace").map(|p| PathBuf::from(p));
    let storage_path = flags.get("--storage").map(|p| PathBuf::from(p)).or(config_storage_path);

    // Validate: need either trace or storage
    if trace_path.is_none() && storage_path.is_none() {
        return Err("missing required flag --trace or --storage or --config".to_string());
    }

    // Validate trace path exists if provided
    if let Some(ref path) = trace_path {
        if !path.exists() {
            return Err(format!("trace file does not exist: {}", path.display()));
        }
    }

    // Validate storage path exists if provided
    if let Some(ref path) = storage_path {
        if !path.exists() {
            return Err(format!("storage file does not exist: {}", path.display()));
        }
    }

    let addr: IpAddr = flags.get("--addr")
        .map(|s| s.parse())
        .transpose()
        .map_err(|e| format!("invalid address: {e}"))?
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)));

    let port: u16 = flags.get("--port")
        .map(|s| s.parse())
        .transpose()
        .map_err(|e| format!("invalid port: {e}"))?
        .unwrap_or(8080);

    let listen_addr = SocketAddr::new(addr, port);

    let request_read_timeout = match flags.get("--request-read-timeout-ms") {
        Some(value) if value == "disabled" => None,
        Some(value) => {
            let millis: u64 = value.parse()
                .map_err(|e| format!("invalid timeout value: {e}"))?;
            Some(Duration::from_millis(millis))
        }
        None => Some(Duration::from_secs(30)),
    };

    Ok(WebConfig {
        trace_path,
        storage_path,
        listen_addr,
        request_read_timeout,
    })
}

fn extract_storage_path(raw: &str) -> Result<String, String> {
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            if key.trim() == "storage_path" {
                let value = value.trim();
                // Remove quotes if present
                if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                    return Ok(value[1..value.len()-1].to_string());
                }
                return Ok(value.to_string());
            }
        }
    }
    Err("missing storage_path in config".to_string())
}
