//! Command-line input for actrailweb.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use config_core::daemon::{DEFAULT_OPERATOR_CONFIG_PATH, OperatorConfig, WebAlertsConfig};
use storage_factory::StorageConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebConfig {
    pub storage: StorageConfig,
    pub cluster_root: Option<PathBuf>,
    pub listen_addr: SocketAddr,
    pub request_read_timeout: Option<Duration>,
    pub alerts: WebAlertsConfig,
    pub operator_config_path: Option<PathBuf>,
    pub operator_config: Option<OperatorConfig>,
}

const HELP_FLAG_SHORT: &str = "-h";
const HELP_FLAG_LONG: &str = "--help";
const HELP_COMMAND: &str = "help";

pub const HELP_TEXT: &str = "\
Read AcTrail traces through a read-only web UI

Usage:
  actrailweb help
  actrailweb [--config <PATH>] [--addr <ADDR>] [--port <PORT>] [--request-read-timeout-ms <MILLIS|disabled>]
  actrailweb --storage-path <PATH> --addr <ADDR> --port <PORT> --request-read-timeout-ms <MILLIS|disabled>
  actrailweb cluster [--config <PATH>] [--cluster-root <PATH>] [--addr <ADDR>] [--port <PORT>] [--request-read-timeout-ms <MILLIS|disabled>]

Options:
  --config <PATH>                   Operator config path; defaults to /etc/actrail/actraild.conf
  --storage-path <PATH>             Storage path when no operator config is used
  --cluster-root <PATH>             Cluster center root directory; defaults to [cluster.center].root_dir
  --addr <ADDR>                     Listen address or operator config override
  --port <PORT>                     Listen port or operator config override
  --request-read-timeout-ms <VALUE> Request read timeout in milliseconds, or disabled
  help, -h, --help                  Print help
";

pub fn is_help_request(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), HELP_FLAG_SHORT | HELP_FLAG_LONG))
        || matches!(args, [command] if command == HELP_COMMAND)
}

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<WebConfig, String> {
    let (mode, flags) = parse_command_and_flags(args)?;
    let config = load_optional_config(&flags)?;
    let storage = resolve_storage_config(&flags, config.as_ref())?;
    let cluster_root = match mode {
        WebMode::Storage => None,
        WebMode::Cluster => Some(resolve_cluster_root(&flags, config.as_ref())?),
    };
    let listen_addr = resolve_listen_addr(&flags, config.as_ref())?;
    let request_read_timeout = resolve_request_read_timeout(&flags, config.as_ref())?;
    let alerts = config
        .as_ref()
        .map(|config| config.alerts)
        .unwrap_or_default();
    let operator_config = config
        .as_ref()
        .and_then(|config| config.operator_config.clone())
        .map(|mut operator_config| {
            operator_config.storage = storage.clone();
            operator_config.web.listen_addr = listen_addr;
            operator_config.web.request_read_timeout = request_read_timeout;
            operator_config.web.alerts = alerts;
            operator_config
        });
    Ok(WebConfig {
        storage,
        cluster_root,
        listen_addr,
        request_read_timeout,
        alerts,
        operator_config_path: config
            .as_ref()
            .and_then(|config| config.operator_config_path.clone()),
        operator_config,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WebMode {
    Storage,
    Cluster,
}

fn parse_command_and_flags(
    args: impl IntoIterator<Item = String>,
) -> Result<(WebMode, std::collections::BTreeMap<String, String>), String> {
    let mut args = args.into_iter();
    let mode = match args.next() {
        Some(command) if command == "cluster" => WebMode::Cluster,
        Some(first) => {
            return parse_flags(std::iter::once(first).chain(args))
                .map(|flags| (WebMode::Storage, flags));
        }
        None => WebMode::Storage,
    };
    parse_flags(args).map(|flags| (mode, flags))
}

fn parse_flags(
    args: impl IntoIterator<Item = String>,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut flags = std::collections::BTreeMap::new();
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        if !matches!(
            flag.as_str(),
            "--config"
                | "--storage-path"
                | "--cluster-root"
                | "--addr"
                | "--port"
                | "--request-read-timeout-ms"
        ) {
            return Err(format!("unknown actrailweb flag {flag}"));
        }
        if flags.insert(flag.clone(), value).is_some() {
            return Err(format!("duplicate actrailweb flag {flag}"));
        }
    }
    Ok(flags)
}

fn load_config(path: &Path) -> Result<WebConfig, String> {
    let config = OperatorConfig::load(path)?;
    Ok(WebConfig {
        storage: config.storage.clone(),
        cluster_root: None,
        listen_addr: config.web.listen_addr,
        request_read_timeout: config.web.request_read_timeout,
        alerts: config.web.alerts,
        operator_config_path: Some(path.to_path_buf()),
        operator_config: Some(config),
    })
}

fn load_optional_config(
    flags: &std::collections::BTreeMap<String, String>,
) -> Result<Option<WebConfig>, String> {
    if let Some(path) = flags.get("--config") {
        return load_config(Path::new(path)).map(Some);
    }
    if flags.contains_key("--storage-path") {
        return Ok(None);
    }
    load_config(Path::new(DEFAULT_OPERATOR_CONFIG_PATH)).map(Some)
}

fn resolve_storage_config(
    flags: &std::collections::BTreeMap<String, String>,
    config: Option<&WebConfig>,
) -> Result<StorageConfig, String> {
    if let Some(path) = flags.get("--storage-path") {
        if path.is_empty() {
            return Err("--storage-path must not be empty".to_string());
        }
        return Ok(StorageConfig::sqlite_path(path));
    }
    config
        .map(|config| config.storage.clone())
        .ok_or_else(|| "missing required flag --storage-path".to_string())
}

fn resolve_cluster_root(
    flags: &std::collections::BTreeMap<String, String>,
    config: Option<&WebConfig>,
) -> Result<PathBuf, String> {
    if let Some(path) = flags.get("--cluster-root") {
        if path.is_empty() {
            return Err("--cluster-root must not be empty".to_string());
        }
        return Ok(PathBuf::from(path));
    }
    config
        .and_then(|config| config.operator_config.as_ref())
        .map(|config| config.cluster.center.root_dir.clone())
        .ok_or_else(|| "missing required flag --cluster-root".to_string())
}

fn resolve_listen_addr(
    flags: &std::collections::BTreeMap<String, String>,
    config: Option<&WebConfig>,
) -> Result<SocketAddr, String> {
    let configured = config.map(|config| config.listen_addr);
    let addr = match flags.get("--addr") {
        Some(value) => parse_addr("--addr", value)?,
        None => configured
            .map(|listen_addr| listen_addr.ip())
            .ok_or_else(|| "missing required flag --addr".to_string())?,
    };
    let port = match flags.get("--port") {
        Some(value) => parse_port("--port", value)?,
        None => configured
            .map(|listen_addr| listen_addr.port())
            .ok_or_else(|| "missing required flag --port".to_string())?,
    };
    Ok(SocketAddr::new(addr, port))
}

fn resolve_request_read_timeout(
    flags: &std::collections::BTreeMap<String, String>,
    config: Option<&WebConfig>,
) -> Result<Option<Duration>, String> {
    if let Some(raw) = flags.get("--request-read-timeout-ms") {
        return parse_duration_millis("--request-read-timeout-ms", raw);
    }
    config
        .map(|config| config.request_read_timeout)
        .ok_or_else(|| "missing required flag --request-read-timeout-ms".to_string())
}

fn parse_duration_millis(key: &'static str, raw: &str) -> Result<Option<Duration>, String> {
    if raw == "disabled" {
        return Ok(None);
    }
    let millis = raw
        .parse::<u64>()
        .map_err(|error| format!("invalid {key}: {error}"))?;
    if millis == u64::default() {
        return Err(format!("invalid {key}: value must be positive or disabled"));
    }
    Ok(Some(Duration::from_millis(millis)))
}

fn parse_addr(key: &'static str, raw: &str) -> Result<IpAddr, String> {
    raw.parse::<IpAddr>()
        .map_err(|error| format!("invalid {key}: {error}"))
}

fn parse_port(key: &'static str, raw: &str) -> Result<u16, String> {
    raw.parse::<u16>()
        .map_err(|error| format!("invalid {key}: {error}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn help_request_recognizes_short_long_flags_and_help_command() {
        assert!(super::is_help_request(&["-h".to_string()]));
        assert!(super::is_help_request(&["--help".to_string()]));
        assert!(super::is_help_request(&["help".to_string()]));
        assert!(!super::is_help_request(&[
            "--storage-path".to_string(),
            "help".to_string()
        ]));
        assert!(!super::is_help_request(&["--config".to_string()]));
    }
}
