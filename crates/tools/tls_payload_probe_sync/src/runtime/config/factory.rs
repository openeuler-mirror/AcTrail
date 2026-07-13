use payload_capability::DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES;
use tls_payload_sync::{
    ENV_ENABLED, ENV_EVENT_FD, ENV_EVENT_SOCKET, ENV_EVENT_WRITE_BUFFER_BYTES, ENV_EVENTS,
    ENV_FLOW_CONTROL_ENABLED, ENV_FLOW_H2_DATA_PROBE_BYTES, ENV_FLOW_LARGE_TRANSFER_BYTES,
    ENV_FLOW_MAX_HEADER_BYTES, ENV_FLOW_SNIFF_BYTES, ENV_FLOW_UNKNOWN_STREAM_BYTES,
    ENV_MAX_PAYLOAD_BYTES, ENV_REDACTION, ENV_RULES, ENV_TRACE_ID,
};

use crate::runtime::flow_control::FlowControlConfig;

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
                flow_control: flow_control_config()?,
                redaction,
                events,
                trace_id: optional_trace_id()?,
                event_transport: optional_event_transport(max_payload_bytes)?,
            }),
            initial_plan,
        }))
    }
}

fn flow_control_config() -> Result<FlowControlConfig, String> {
    Ok(FlowControlConfig {
        enabled: optional_bool(ENV_FLOW_CONTROL_ENABLED, true)?,
        sniff_bytes: optional_positive_usize(ENV_FLOW_SNIFF_BYTES, 65536)?,
        max_header_bytes: optional_positive_usize(ENV_FLOW_MAX_HEADER_BYTES, 16384)?,
        large_transfer_bytes: optional_positive_u64(ENV_FLOW_LARGE_TRANSFER_BYTES, 1048576)?,
        unknown_stream_bytes: optional_positive_u64(
            ENV_FLOW_UNKNOWN_STREAM_BYTES,
            DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES,
        )?,
        h2_data_probe_bytes: optional_positive_u64(ENV_FLOW_H2_DATA_PROBE_BYTES, 65536)?,
    })
}

fn optional_bool(name: &str, default: bool) -> Result<bool, String> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(default);
    };
    match value.to_string_lossy().as_ref() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        value => Err(format!(
            "parse {name}: expected true/false or 1/0, got {value}"
        )),
    }
}

fn optional_positive_usize(name: &str, default: usize) -> Result<usize, String> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(default);
    };
    let parsed = value
        .to_string_lossy()
        .parse::<usize>()
        .map_err(|error| format!("parse {name}: {error}"))?;
    if parsed == 0 {
        return Err(format!("{name} must be positive"));
    }
    Ok(parsed)
}

fn optional_positive_u64(name: &str, default: u64) -> Result<u64, String> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(default);
    };
    let parsed = value
        .to_string_lossy()
        .parse::<u64>()
        .map_err(|error| format!("parse {name}: {error}"))?;
    if parsed == 0 {
        return Err(format!("{name} must be positive"));
    }
    Ok(parsed)
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
    let socket_path = std::env::var_os(ENV_EVENT_SOCKET).map(std::path::PathBuf::from);
    if let Some(value) = std::env::var_os(ENV_EVENT_FD) {
        let fd = value
            .to_string_lossy()
            .parse::<i32>()
            .map_err(|error| format!("parse {ENV_EVENT_FD}: {error}"))?;
        if fd < 0 {
            return Err(format!("{ENV_EVENT_FD} must be non-negative: {fd}"));
        }
        let write_buffer_bytes = required_event_write_buffer_bytes()?;
        if !event_fd_is_open(fd) {
            let Some(path) = socket_path else {
                return Err(format!(
                    "{ENV_EVENT_FD}={fd} is not open and {ENV_EVENT_SOCKET} is not set"
                ));
            };
            return Ok(Some(EventTransportConfig::Socket {
                path,
                pending_byte_budget,
                write_buffer_bytes,
            }));
        }
        return Ok(Some(EventTransportConfig::InheritedFd {
            fd,
            reconnect_path: socket_path,
            pending_byte_budget,
            write_buffer_bytes,
        }));
    }
    let Some(path) = socket_path else {
        return Ok(None);
    };
    let write_buffer_bytes = required_event_write_buffer_bytes()?;
    Ok(Some(EventTransportConfig::Socket {
        path,
        pending_byte_budget,
        write_buffer_bytes,
    }))
}

fn event_fd_is_open(fd: i32) -> bool {
    (unsafe { libc::fcntl(fd, libc::F_GETFD) }) >= 0
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
