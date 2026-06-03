use std::path::PathBuf;

use tls_payload_sync::{
    ENV_ENABLED, ENV_EVENT_SOCKET, ENV_EVENTS, ENV_MAX_PAYLOAD_BYTES, ENV_REDACTION, ENV_RULES,
    ENV_TRACE_ID, EventClient,
};

use super::codec::parse_rules;
use super::plan::current_runtime_plan;
use super::policy::{EventFilter, RedactionMode};
use super::state::{RuntimeConfig, RuntimeConfigParts};

pub(in crate::runtime) struct RuntimeConfigFactory;

impl RuntimeConfigFactory {
    pub(in crate::runtime) fn from_env() -> Result<Option<RuntimeConfig>, String> {
        if std::env::var_os(ENV_ENABLED).is_none() {
            return Ok(None);
        }
        let Some(plan) = current_runtime_plan()? else {
            return Ok(None);
        };
        let rules = parse_rules(&std::env::var(ENV_RULES).unwrap_or_default())?;
        let max_payload_bytes = required_payload_limit()?;
        let redaction = RedactionMode::parse(
            &std::env::var(ENV_REDACTION).unwrap_or_else(|_| "redact".to_string()),
        )?;
        let events_value = std::env::var(ENV_EVENTS).ok();
        let events = EventFilter::parse(events_value.as_deref())?;
        Ok(Some(RuntimeConfig::from_parts(RuntimeConfigParts {
            binary: plan.binary,
            provider: plan.provider,
            points: plan.points,
            rules,
            max_payload_bytes,
            redaction,
            events,
            trace_id: optional_trace_id()?,
            event_client: optional_event_client()?,
        })))
    }
}

fn required_payload_limit() -> Result<usize, String> {
    let value = std::env::var(ENV_MAX_PAYLOAD_BYTES)
        .map_err(|_| format!("missing required runtime env {ENV_MAX_PAYLOAD_BYTES}"))?;
    let max_payload_bytes = value
        .parse::<usize>()
        .map_err(|error| format!("parse {ENV_MAX_PAYLOAD_BYTES}: {error}"))?;
    if max_payload_bytes == 0 {
        return Err("max payload bytes must be positive".to_string());
    }
    Ok(max_payload_bytes)
}

fn optional_trace_id() -> Result<Option<u64>, String> {
    let Some(value) = std::env::var_os(ENV_TRACE_ID) else {
        return Ok(None);
    };
    value
        .to_string_lossy()
        .parse::<u64>()
        .map(Some)
        .map_err(|error| format!("parse {ENV_TRACE_ID}: {error}"))
}

fn optional_event_client() -> Result<Option<EventClient>, String> {
    let Some(value) = std::env::var_os(ENV_EVENT_SOCKET) else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    EventClient::connect(&path)
        .map(Some)
        .map_err(|error| format!("connect sync event socket {}: {error}", path.display()))
}
