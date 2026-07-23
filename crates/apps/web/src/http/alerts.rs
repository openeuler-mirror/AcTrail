use config_core::daemon::WebAlertsConfig;
use storage_factory::StorageConfig;

use crate::view::{AlertProjection, AlertProjectionError};

use super::query::{optional_query_param, parse_u64};
use super::{Response, STATUS_BAD_REQUEST, STATUS_NOT_FOUND};

pub(super) struct AlertHttp<'context> {
    storage: &'context StorageConfig,
    limits: WebAlertsConfig,
}

impl<'context> AlertHttp<'context> {
    pub(super) const fn new(storage: &'context StorageConfig, limits: WebAlertsConfig) -> Self {
        Self { storage, limits }
    }

    pub(super) fn route(&self, path: &str, query: &str) -> Option<Result<Response, String>> {
        if path == "/api/alerts" {
            return Some(self.latest(query));
        }
        if let Some(raw_alert_id) = path.strip_prefix("/api/alerts/") {
            return Some(self.detail(raw_alert_id));
        }
        let parts = path
            .strip_prefix("/api/traces/")
            .map(|suffix| suffix.split('/').collect::<Vec<_>>());
        match parts.as_deref() {
            Some([trace_id, "alerts"]) => Some(self.trace_alerts(trace_id, query)),
            _ => None,
        }
    }

    fn latest(&self, query: &str) -> Result<Response, String> {
        let limit = match self.limit(query) {
            Ok(limit) => limit,
            Err(error) => return Ok(Response::text(STATUS_BAD_REQUEST, error)),
        };
        self.respond(AlertProjection::open(self.storage).and_then(|view| view.latest_json(limit)))
    }

    fn detail(&self, raw_alert_id: &str) -> Result<Response, String> {
        if raw_alert_id.is_empty() || raw_alert_id.contains('/') {
            return Ok(Response::text(STATUS_NOT_FOUND, "not found"));
        }
        let alert_id = match parse_u64(raw_alert_id) {
            Ok(alert_id) => alert_id,
            Err(error) => return Ok(Response::text(STATUS_BAD_REQUEST, error)),
        };
        self.respond(
            AlertProjection::open(self.storage).and_then(|view| view.detail_json(alert_id)),
        )
    }

    fn trace_alerts(&self, raw_trace_id: &str, query: &str) -> Result<Response, String> {
        let trace_id = match parse_u64(raw_trace_id) {
            Ok(trace_id) => trace_id,
            Err(error) => return Ok(Response::text(STATUS_BAD_REQUEST, error)),
        };
        let limit = match self.limit(query) {
            Ok(limit) => limit,
            Err(error) => return Ok(Response::text(STATUS_BAD_REQUEST, error)),
        };
        self.respond(
            AlertProjection::open(self.storage).and_then(|view| view.trace_json(trace_id, limit)),
        )
    }

    fn limit(&self, query: &str) -> Result<usize, String> {
        let raw = optional_query_param(query, "limit")?;
        let limit = match raw {
            Some(raw) => raw
                .parse::<u32>()
                .map_err(|error| format!("invalid query parameter limit: {error}"))?,
            None => self.limits.default_limit,
        };
        if limit == 0 {
            return Err("invalid query parameter limit: value must be positive".to_string());
        }
        if limit > self.limits.max_limit {
            return Err(format!(
                "invalid query parameter limit: {limit} exceeds configured maximum {}",
                self.limits.max_limit
            ));
        }
        usize::try_from(limit).map_err(|error| format!("invalid query parameter limit: {error}"))
    }

    fn respond(&self, result: Result<String, AlertProjectionError>) -> Result<Response, String> {
        match result {
            Ok(body) => Ok(Response::json(body)),
            Err(AlertProjectionError::InvalidLimit(message)) => {
                Ok(Response::text(STATUS_BAD_REQUEST, message))
            }
            Err(AlertProjectionError::NotFound(message)) => {
                Ok(Response::text(STATUS_NOT_FOUND, message))
            }
            Err(AlertProjectionError::Storage(message)) => Err(message),
        }
    }
}
