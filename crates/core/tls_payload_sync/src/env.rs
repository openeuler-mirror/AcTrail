//! Environment contract between launcher and preloaded runtime.

use std::ffi::OsString;
use std::path::PathBuf;

use payload_capability::DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES;
use tls_payload_core::RewriteRule;
use tls_probe_point_finder::ProbePointPlan;

use crate::SyncResult;
use crate::plan::{encode_points, runtime_plan_bundle};

pub const ENV_ENABLED: &str = "TLS_PAYLOAD_SYNC_ENABLED";
pub const ENV_BINARY: &str = "TLS_PAYLOAD_SYNC_BINARY";
pub const ENV_PROVIDER: &str = "TLS_PAYLOAD_SYNC_PROVIDER";
pub const ENV_POINTS: &str = "TLS_PAYLOAD_SYNC_POINTS";
pub const ENV_PLAN_BUNDLE: &str = "TLS_PAYLOAD_SYNC_PLAN_BUNDLE";
pub const ENV_RULES: &str = "TLS_PAYLOAD_SYNC_RULES";
pub const ENV_MAX_PAYLOAD_BYTES: &str = "TLS_PAYLOAD_SYNC_MAX_PAYLOAD_BYTES";
pub const ENV_REDACTION: &str = "TLS_PAYLOAD_SYNC_REDACTION";
pub const ENV_EVENTS: &str = "TLS_PAYLOAD_SYNC_EVENTS";
pub const ENV_TRACE_ID: &str = "TLS_PAYLOAD_SYNC_TRACE_ID";
pub const ENV_EVENT_SOCKET: &str = "TLS_PAYLOAD_SYNC_EVENT_SOCKET";
pub const ENV_EVENT_FD: &str = "TLS_PAYLOAD_SYNC_EVENT_FD";
pub const ENV_EVENT_WRITE_BUFFER_BYTES: &str = "TLS_PAYLOAD_SYNC_EVENT_WRITE_BUFFER_BYTES";
pub const ENV_FLOW_CONTROL_ENABLED: &str = "TLS_PAYLOAD_SYNC_FLOW_CONTROL_ENABLED";
pub const ENV_FLOW_SNIFF_BYTES: &str = "TLS_PAYLOAD_SYNC_FLOW_SNIFF_BYTES";
pub const ENV_FLOW_MAX_HEADER_BYTES: &str = "TLS_PAYLOAD_SYNC_FLOW_MAX_HEADER_BYTES";
pub const ENV_FLOW_LARGE_TRANSFER_BYTES: &str = "TLS_PAYLOAD_SYNC_FLOW_LARGE_TRANSFER_BYTES";
pub const ENV_FLOW_UNKNOWN_STREAM_BYTES: &str = "TLS_PAYLOAD_SYNC_FLOW_UNKNOWN_STREAM_BYTES";
pub const ENV_FLOW_H2_DATA_PROBE_BYTES: &str = "TLS_PAYLOAD_SYNC_FLOW_H2_DATA_PROBE_BYTES";
pub const ENV_LIBRARY_PATH_PREFIX: &str = "TLS_PAYLOAD_SYNC_LIBRARY_PATH_PREFIX";
pub const ENV_LIBRARY_PATH_PREFIX_GLIBC: &str = "TLS_PAYLOAD_SYNC_LIBRARY_PATH_PREFIX_GLIBC";
pub const ENV_LIBRARY_PATH_PREFIX_MUSL: &str = "TLS_PAYLOAD_SYNC_LIBRARY_PATH_PREFIX_MUSL";
pub const ENV_RUNTIME_GLIBC_LIBRARY: &str = "TLS_PAYLOAD_SYNC_RUNTIME_GLIBC_LIBRARY";
pub const ENV_RUNTIME_MUSL_LIBRARY: &str = "TLS_PAYLOAD_SYNC_RUNTIME_MUSL_LIBRARY";
pub const ENV_DEPENDENCY_GUARD_DIR: &str = "TLS_PAYLOAD_SYNC_DEPENDENCY_GUARD_DIR";
pub const ENV_SYSTEM_LIBRARY_DIRS: &str = "TLS_PAYLOAD_SYNC_SYSTEM_LIBRARY_DIRS";
pub const RUNTIME_GLIBC_LIBRARY_NAME: &str = "libactrail_tls_payload_probe_sync.so";
pub const RUNTIME_MUSL_LIBRARY_NAME: &str = "libactrail_tls_payload_probe_sync-musl.so";

#[derive(Clone, Debug)]
pub struct RuntimeEnvConfig {
    pub rules: Vec<RewriteRule>,
    pub max_payload_bytes: usize,
    pub flow_control: RuntimeFlowControlConfig,
    pub redaction: RedactionMode,
    pub events: EventFilter,
    pub trace_id: Option<u64>,
    pub event_socket_path: Option<PathBuf>,
    pub event_fd: Option<i32>,
    pub event_write_buffer_bytes: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeFlowControlConfig {
    pub enabled: bool,
    pub sniff_bytes: usize,
    pub max_header_bytes: usize,
    pub large_transfer_bytes: u64,
    pub unknown_stream_bytes: u64,
    pub h2_data_probe_bytes: u64,
}

impl Default for RuntimeFlowControlConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sniff_bytes: 65536,
            max_header_bytes: 16384,
            large_transfer_bytes: 1048576,
            unknown_stream_bytes: DEFAULT_TLS_SYNC_FLOW_UNKNOWN_STREAM_BYTES,
            h2_data_probe_bytes: 65536,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedactionMode {
    Redact,
    None,
}

impl RedactionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Redact => "redact",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventFilter {
    pub target: bool,
    pub payload: bool,
    pub decision: bool,
}

impl EventFilter {
    pub const fn all() -> Self {
        Self {
            target: true,
            payload: true,
            decision: true,
        }
    }

    pub const fn none() -> Self {
        Self {
            target: false,
            payload: false,
            decision: false,
        }
    }

    pub fn encode(&self) -> String {
        let mut events = Vec::new();
        if self.target {
            events.push("target");
        }
        if self.payload {
            events.push("payload");
        }
        if self.decision {
            events.push("decision");
        }
        events.join(",")
    }
}

pub fn runtime_env(
    config: &RuntimeEnvConfig,
    plan: &ProbePointPlan,
) -> SyncResult<Vec<(OsString, OsString)>> {
    runtime_env_for_plans(config, std::slice::from_ref(plan))
}

pub fn runtime_env_for_plans(
    config: &RuntimeEnvConfig,
    plans: &[ProbePointPlan],
) -> SyncResult<Vec<(OsString, OsString)>> {
    let mut env = vec![
        pair(ENV_ENABLED, "1"),
        pair(ENV_RULES, &encode_rules(&config.rules)),
        pair(ENV_MAX_PAYLOAD_BYTES, &config.max_payload_bytes.to_string()),
        pair(
            ENV_FLOW_CONTROL_ENABLED,
            if config.flow_control.enabled {
                "1"
            } else {
                "0"
            },
        ),
        pair(
            ENV_FLOW_SNIFF_BYTES,
            &config.flow_control.sniff_bytes.to_string(),
        ),
        pair(
            ENV_FLOW_MAX_HEADER_BYTES,
            &config.flow_control.max_header_bytes.to_string(),
        ),
        pair(
            ENV_FLOW_LARGE_TRANSFER_BYTES,
            &config.flow_control.large_transfer_bytes.to_string(),
        ),
        pair(
            ENV_FLOW_UNKNOWN_STREAM_BYTES,
            &config.flow_control.unknown_stream_bytes.to_string(),
        ),
        pair(
            ENV_FLOW_H2_DATA_PROBE_BYTES,
            &config.flow_control.h2_data_probe_bytes.to_string(),
        ),
        pair(ENV_REDACTION, config.redaction.as_str()),
        pair(ENV_EVENTS, &config.events.encode()),
    ];
    if let Some(plan) = plans.first() {
        env.push(pair(ENV_BINARY, &plan.binary.path.display().to_string()));
        env.push(pair(ENV_PROVIDER, plan.provider.as_str()));
        env.push(pair(ENV_POINTS, &encode_points(plan)?));
    }
    env.push(pair(ENV_PLAN_BUNDLE, &runtime_plan_bundle(plans)?));
    if let Some(trace_id) = config.trace_id {
        env.push(pair(ENV_TRACE_ID, &trace_id.to_string()));
    }
    if let Some(path) = &config.event_socket_path {
        env.push(pair(ENV_EVENT_SOCKET, &path.display().to_string()));
    }
    if let Some(fd) = config.event_fd {
        env.push(pair(ENV_EVENT_FD, &fd.to_string()));
    }
    if let Some(bytes) = config.event_write_buffer_bytes {
        env.push(pair(ENV_EVENT_WRITE_BUFFER_BYTES, &bytes.to_string()));
    }
    Ok(env)
}

fn pair(key: &str, value: &str) -> (OsString, OsString) {
    (OsString::from(key), OsString::from(value))
}

fn encode_rules(rules: &[RewriteRule]) -> String {
    rules
        .iter()
        .map(|rule| {
            format!(
                "{}:{}={}",
                rule.direction().as_str(),
                encode_hex(rule.from()),
                encode_hex(rule.to())
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push_str(&format!("{byte:02x}"));
    }
    value
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tls_probe_point_finder::{
        AttachPoint, CaptureStrategy, PayloadDirection as FinderDirection, ProbeBinary, ProbePoint,
        ProbePointPlan, ProbeSource, TargetIdentity, TlsProvider,
    };

    use super::{
        ENV_BINARY, ENV_EVENT_FD, ENV_PLAN_BUNDLE, EventFilter, RuntimeEnvConfig,
        RuntimeFlowControlConfig, runtime_env_for_plans,
    };

    #[test]
    fn runtime_env_encodes_multiple_probe_plans() {
        let config = RuntimeEnvConfig {
            rules: Vec::new(),
            max_payload_bytes: 4096,
            flow_control: RuntimeFlowControlConfig::default(),
            redaction: super::RedactionMode::Redact,
            events: EventFilter::none(),
            trace_id: None,
            event_socket_path: None,
            event_fd: None,
            event_write_buffer_bytes: None,
        };
        let first = plan("/bin/first", TlsProvider::BoringSsl);
        let second = plan("/bin/second", TlsProvider::Rustls);

        let env = runtime_env_for_plans(&config, &[first, second]).expect("runtime env");
        let bundle = env
            .iter()
            .find(|(key, _)| key == ENV_PLAN_BUNDLE)
            .map(|(_, value)| value.to_string_lossy().into_owned())
            .expect("plan bundle");

        assert_eq!(bundle.lines().count(), 2);
    }

    #[test]
    fn runtime_env_allows_empty_probe_plan_bundle() {
        let config = RuntimeEnvConfig {
            rules: Vec::new(),
            max_payload_bytes: 4096,
            flow_control: RuntimeFlowControlConfig::default(),
            redaction: super::RedactionMode::Redact,
            events: EventFilter::none(),
            trace_id: None,
            event_socket_path: None,
            event_fd: None,
            event_write_buffer_bytes: None,
        };

        let env = runtime_env_for_plans(&config, &[]).expect("runtime env");
        let bundle = env
            .iter()
            .find(|(key, _)| key == ENV_PLAN_BUNDLE)
            .map(|(_, value)| value.to_string_lossy().into_owned())
            .expect("plan bundle");

        assert_eq!(bundle, "");
        assert!(!env.iter().any(|(key, _)| key == ENV_BINARY));
    }

    #[test]
    fn runtime_env_encodes_inherited_event_fd() {
        let config = RuntimeEnvConfig {
            rules: Vec::new(),
            max_payload_bytes: 4096,
            flow_control: RuntimeFlowControlConfig::default(),
            redaction: super::RedactionMode::Redact,
            events: EventFilter::none(),
            trace_id: None,
            event_socket_path: None,
            event_fd: Some(3),
            event_write_buffer_bytes: None,
        };

        let env = runtime_env_for_plans(&config, &[]).expect("runtime env");

        assert!(env.iter().any(|(key, value)| {
            key == ENV_EVENT_FD && value.to_string_lossy().as_ref() == "3"
        }));
    }

    fn plan(path: &str, provider: TlsProvider) -> ProbePointPlan {
        ProbePointPlan {
            target: TargetIdentity {
                binary: PathBuf::from(path),
                architecture: "x86_64".to_string(),
                build_id: None,
            },
            provider,
            source: ProbeSource::Executable,
            resolver: "test".to_string(),
            binary: ProbeBinary {
                path: PathBuf::from(path),
                architecture: "x86_64".to_string(),
                build_id: None,
            },
            points: vec![
                point("write", FinderDirection::Outbound),
                point("read", FinderDirection::Inbound),
            ],
        }
    }

    fn point(symbol: &str, direction: FinderDirection) -> ProbePoint {
        ProbePoint {
            symbol: symbol.to_string(),
            direction,
            attach: AttachPoint::Entry,
            capture: CaptureStrategy::EntryBuffer,
            virtual_address: 0,
            file_offset: 0,
        }
    }
}
