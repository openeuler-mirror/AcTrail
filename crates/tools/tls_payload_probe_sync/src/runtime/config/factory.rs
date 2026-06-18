use tls_payload_sync::{
    ENV_ENABLED, ENV_EVENT_FD, ENV_EVENT_SOCKET, ENV_EVENT_WRITE_BUFFER_BYTES, ENV_EVENTS,
    ENV_MAX_PAYLOAD_BYTES, ENV_REDACTION, ENV_RULES, ENV_TRACE_ID,
};

use super::codec::parse_rules;
use super::plan::{RuntimePlan, current_runtime_plan};
use super::policy::{EventFilter, RedactionMode};
use super::state::{EventTransportConfig, RuntimeConfig, RuntimeConfigParts};

pub(in crate::runtime) struct RuntimeConfigFactory;

pub(in crate::runtime) struct RuntimeBootstrap {
    pub(in crate::runtime) config: RuntimeConfig,
    pub(in crate::runtime) initial_plan: Option<RuntimePlan>,
}

impl RuntimeConfigFactory {
    pub(in crate::runtime) fn from_env_with_initial_plan(
        resolve_initial_plan: bool,
    ) -> Result<Option<RuntimeBootstrap>, String> {
        if std::env::var_os(ENV_ENABLED).is_none() {
            return Ok(None);
        }
        let initial_plan = if resolve_initial_plan {
            current_runtime_plan()?
        } else {
            None
        };
        let rules = parse_rules(&std::env::var(ENV_RULES).unwrap_or_default())?;
        let max_payload_bytes = required_payload_limit()?;
        let redaction = RedactionMode::parse(
            &std::env::var(ENV_REDACTION).unwrap_or_else(|_| "redact".to_string()),
        )?;
        let events_value = std::env::var(ENV_EVENTS).ok();
        let events = EventFilter::parse(events_value.as_deref())?;
        Ok(Some(RuntimeBootstrap {
            config: RuntimeConfig::from_parts(RuntimeConfigParts {
                rules,
                max_payload_bytes,
                redaction,
                events,
                trace_id: optional_trace_id()?,
                event_transport: optional_event_transport(max_payload_bytes)?,
            }),
            initial_plan,
        }))
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

fn optional_event_transport(
    pending_byte_budget: usize,
) -> Result<Option<EventTransportConfig>, String> {
    if let Some(value) = std::env::var_os(ENV_EVENT_FD) {
        let fd = value
            .to_string_lossy()
            .parse::<i32>()
            .map_err(|error| format!("parse {ENV_EVENT_FD}: {error}"))?;
        let write_buffer_bytes = required_event_write_buffer_bytes()?;
        return Ok(Some(EventTransportConfig::InheritedFd {
            fd,
            pending_byte_budget,
            write_buffer_bytes,
        }));
    }
    let Some(value) = std::env::var_os(ENV_EVENT_SOCKET) else {
        return Ok(None);
    };
    let path = std::path::PathBuf::from(value);
    let write_buffer_bytes = required_event_write_buffer_bytes()?;
    Ok(Some(EventTransportConfig::Socket {
        path,
        pending_byte_budget,
        write_buffer_bytes,
    }))
}

fn required_event_write_buffer_bytes() -> Result<usize, String> {
    let value = std::env::var(ENV_EVENT_WRITE_BUFFER_BYTES)
        .map_err(|_| format!("missing required runtime env {ENV_EVENT_WRITE_BUFFER_BYTES}"))?;
    let bytes = value
        .parse::<usize>()
        .map_err(|error| format!("parse {ENV_EVENT_WRITE_BUFFER_BYTES}: {error}"))?;
    if bytes == 0 {
        return Err("sync event write buffer bytes must be positive".to_string());
    }
    Ok(bytes)
}
