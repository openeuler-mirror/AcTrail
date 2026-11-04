//! Parses the complete Xiaoo integration configuration at construction time.

use std::{path::PathBuf, time::Duration};

use serde::Deserialize;

use crate::{constants, error::Error, protocol::RestoreMode};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    endpoint: PathBuf,
    request_timeout_ms: Option<u64>,
    max_response_bytes: Option<usize>,
    restore_mode: Option<RestoreMode>,
}

pub(super) struct Config {
    pub endpoint: PathBuf,
    pub request_timeout: Duration,
    pub max_response_bytes: usize,
    pub restore_mode: RestoreMode,
}

impl Config {
    pub fn from_json(json: &str) -> Result<Self, Error> {
        let input: Input =
            serde_json::from_str(json).map_err(|error| Error::Configuration(error.to_string()))?;
        let timeout = input
            .request_timeout_ms
            .unwrap_or(constants::REQUEST_TIMEOUT_MS);
        let limit = input
            .max_response_bytes
            .unwrap_or(constants::MAX_RESPONSE_BYTES);
        if !input.endpoint.is_absolute() {
            return Err(Error::Configuration(
                "endpoint must be an absolute socket path".into(),
            ));
        }
        socket2::SockAddr::unix(&input.endpoint)
            .map_err(|error| Error::Configuration(error.to_string()))?;
        if timeout == 0 || limit == 0 {
            return Err(Error::Configuration(
                "request_timeout_ms and max_response_bytes must be positive".into(),
            ));
        }
        Ok(Self {
            endpoint: input.endpoint,
            request_timeout: Duration::from_millis(timeout),
            max_response_bytes: limit,
            restore_mode: input.restore_mode.unwrap_or(RestoreMode::PromiseMode),
        })
    }
}
