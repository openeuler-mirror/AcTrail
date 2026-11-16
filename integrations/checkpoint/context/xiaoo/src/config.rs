//! Separates daemon-owned RPC settings from the endpoint selected for one Trace.

use std::{path::PathBuf, time::Duration};

use checkpoint::BackendConfig;
use serde::Deserialize;

use crate::{constants, error::Error, protocol::RestoreMode};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeInput {
    request_timeout_ms: Option<u64>,
    max_response_bytes: Option<usize>,
    restore_mode: Option<RestoreMode>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceInput {
    endpoint: PathBuf,
}

pub(super) struct Config {
    pub endpoint: PathBuf,
    pub request_timeout: Duration,
    pub max_response_bytes: usize,
    pub restore_mode: RestoreMode,
}

impl Config {
    /// Parse the daemon's fixed runtime settings and load only this Trace's binding file.
    pub fn load(config: BackendConfig) -> Result<Self, Error> {
        let runtime: RuntimeInput = serde_json::from_str(&config.runtime_json)
            .map_err(|error| Error::Configuration(format!("runtime configuration: {error}")))?;
        let path = config
            .trace_config_path
            .as_ref()
            .into_option()
            .ok_or_else(|| {
                Error::Configuration("Xiaoo requires a Trace configuration path".into())
            })?;
        let path = path.as_str();
        if !std::path::Path::new(path).is_absolute() {
            return Err(Error::Configuration(
                "trace_config_path must be absolute".into(),
            ));
        }
        let json = std::fs::read_to_string(path)
            .map_err(|error| Error::Configuration(format!("{path}: {error}")))?;
        let trace: TraceInput = serde_json::from_str(&json).map_err(|error| {
            Error::Configuration(format!("Trace configuration {path}: {error}"))
        })?;
        let timeout = runtime
            .request_timeout_ms
            .unwrap_or(constants::REQUEST_TIMEOUT_MS);
        let limit = runtime
            .max_response_bytes
            .unwrap_or(constants::MAX_RESPONSE_BYTES);
        if !trace.endpoint.is_absolute() {
            return Err(Error::Configuration(
                "endpoint must be an absolute socket path".into(),
            ));
        }
        socket2::SockAddr::unix(&trace.endpoint)
            .map_err(|error| Error::Configuration(error.to_string()))?;
        if timeout == 0 || limit == 0 {
            return Err(Error::Configuration(
                "request_timeout_ms and max_response_bytes must be positive".into(),
            ));
        }
        Ok(Self {
            endpoint: trace.endpoint,
            request_timeout: Duration::from_millis(timeout),
            max_response_bytes: limit,
            restore_mode: runtime.restore_mode.unwrap_or(RestoreMode::PromiseMode),
        })
    }
}
