//! JSON-RPC 2.0 over HTTP(S) exporter implementation.

use std::thread;
use std::time::Duration;

use export_core::{BestEffortSink, ExportError};
use serde_json::Value;

use crate::config::JsonRpcHttpExporterConfig;

const EXPORTER_NAME: &str = "otel_live_jsonl";
const USER_AGENT: &str = concat!("actrail-otel-jsonl/", env!("CARGO_PKG_VERSION"));

pub(super) struct JsonRpcHttpExporterSink {
    agent: ureq::Agent,
    endpoint: String,
    encoded_method: String,
    response_body_max_bytes: u64,
    max_attempts: u32,
    retry_backoff: Duration,
    next_request_id: u64,
}

impl JsonRpcHttpExporterSink {
    pub(super) fn open(config: JsonRpcHttpExporterConfig) -> Result<Self, ExportError> {
        let encoded_method = serde_json::to_string(&config.method).map_err(|error| {
            ExportError::new(
                EXPORTER_NAME,
                format!("encode JSON-RPC method failed: {error}"),
            )
        })?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_millis(u64::from(
                config.connect_timeout_ms,
            ))))
            .timeout_global(Some(Duration::from_millis(u64::from(
                config.request_timeout_ms,
            ))))
            .user_agent(USER_AGENT)
            .build()
            .into();
        Ok(Self {
            agent,
            endpoint: config.endpoint,
            encoded_method,
            response_body_max_bytes: u64::from(config.response_body_max_bytes),
            max_attempts: config.max_attempts,
            retry_backoff: Duration::from_millis(u64::from(config.retry_backoff_ms)),
            next_request_id: 1,
        })
    }

    fn encode_request(&self, request_id: u64, document: String) -> String {
        let encoded_id = request_id.to_string();
        let mut body = String::with_capacity(
            document.len() + self.encoded_method.len() + encoded_id.len() + 48,
        );
        body.push_str(r#"{"jsonrpc":"2.0","id":"#);
        body.push_str(&encoded_id);
        body.push_str(r#","method":"#);
        body.push_str(&self.encoded_method);
        body.push_str(r#","params":"#);
        body.push_str(&document);
        body.push('}');
        body
    }

    fn send_request(&self, request_id: u64, body: &str) -> Result<(), RequestFailure> {
        let mut response = self
            .agent
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .send(body)
            .map_err(RequestFailure::from_http)?;
        let response_body = response
            .body_mut()
            .with_config()
            .limit(self.response_body_max_bytes.saturating_add(1))
            .read_to_vec()
            .map_err(RequestFailure::from_response_body)?;
        let response_body_size = u64::try_from(response_body.len()).map_err(|error| {
            RequestFailure::terminal(format!("JSON-RPC response size overflow: {error}"))
        })?;
        if response_body_size > self.response_body_max_bytes {
            return Err(RequestFailure::terminal(format!(
                "JSON-RPC response exceeds configured {} byte limit",
                self.response_body_max_bytes
            )));
        }
        self.validate_response(request_id, &response_body)
    }

    fn validate_response(
        &self,
        request_id: u64,
        response_body: &[u8],
    ) -> Result<(), RequestFailure> {
        let response = serde_json::from_slice::<Value>(response_body).map_err(|error| {
            RequestFailure::terminal(format!("parse JSON-RPC response failed: {error}"))
        })?;
        let object = response.as_object().ok_or_else(|| {
            RequestFailure::terminal("JSON-RPC response must be an object".to_string())
        })?;
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(RequestFailure::terminal(
                "JSON-RPC response has invalid jsonrpc version".to_string(),
            ));
        }
        if object.get("id").and_then(Value::as_u64) != Some(request_id) {
            return Err(RequestFailure::terminal(format!(
                "JSON-RPC response id does not match request {request_id}"
            )));
        }
        let has_result = object.contains_key("result");
        let error = object.get("error");
        if has_result == error.is_some() {
            return Err(RequestFailure::terminal(
                "JSON-RPC response must contain exactly one of result or error".to_string(),
            ));
        }
        if let Some(error) = error {
            return Err(RequestFailure::terminal(Self::format_remote_error(error)));
        }
        Ok(())
    }

    fn format_remote_error(error: &Value) -> String {
        let Some(error) = error.as_object() else {
            return "JSON-RPC endpoint returned a non-object error".to_string();
        };
        let code = error
            .get("code")
            .map(Value::to_string)
            .unwrap_or_else(|| "<missing>".to_string());
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        format!("JSON-RPC endpoint returned error code {code}: {message}")
    }

    fn deliver_with_retries(&self, request_id: u64, body: &str) -> Result<(), String> {
        let mut attempt = 1;
        loop {
            match self.send_request(request_id, body) {
                Ok(()) => return Ok(()),
                Err(failure) if failure.retryable && attempt < self.max_attempts => {
                    thread::sleep(self.retry_backoff);
                    attempt = attempt.saturating_add(1);
                }
                Err(failure) => {
                    return Err(format!(
                        "JSON-RPC delivery failed after {attempt} attempt(s): {}",
                        failure.message
                    ));
                }
            }
        }
    }
}

impl BestEffortSink<String> for JsonRpcHttpExporterSink {
    fn deliver(&mut self, document: String) -> Result<u64, String> {
        let request_id = self.next_request_id;
        let body = self.encode_request(request_id, document);
        self.deliver_with_retries(request_id, &body)?;
        self.next_request_id = request_id.checked_add(1).unwrap_or(1);
        Ok(1)
    }
}

struct RequestFailure {
    message: String,
    retryable: bool,
}

impl RequestFailure {
    fn terminal(message: String) -> Self {
        Self {
            message,
            retryable: false,
        }
    }

    fn from_http(error: ureq::Error) -> Self {
        let retryable = match &error {
            ureq::Error::StatusCode(code) => {
                matches!(*code, 408 | 429) || (500..=599).contains(code)
            }
            error => Self::is_retryable_transport_error(error),
        };
        Self {
            message: format!("HTTP request failed: {error}"),
            retryable,
        }
    }

    fn from_response_body(error: ureq::Error) -> Self {
        Self {
            retryable: Self::is_retryable_transport_error(&error),
            message: format!("read JSON-RPC response failed: {error}"),
        }
    }

    fn is_retryable_transport_error(error: &ureq::Error) -> bool {
        matches!(
            error,
            ureq::Error::Io(_)
                | ureq::Error::Timeout(_)
                | ureq::Error::HostNotFound
                | ureq::Error::ConnectionFailed
        )
    }
}
