//! Environment contract between launcher and preloaded runtime.

use std::ffi::OsString;
use std::path::PathBuf;

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

#[derive(Clone, Debug)]
pub struct RuntimeEnvConfig {
    pub rules: Vec<RewriteRule>,
    pub max_payload_bytes: usize,
    pub redaction: RedactionMode,
    pub events: EventFilter,
    pub trace_id: Option<u64>,
    pub event_socket_path: Option<PathBuf>,
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
        ENV_BINARY, ENV_PLAN_BUNDLE, EventFilter, RuntimeEnvConfig, runtime_env_for_plans,
    };

    #[test]
    fn runtime_env_encodes_multiple_probe_plans() {
        let config = RuntimeEnvConfig {
            rules: Vec::new(),
            max_payload_bytes: 4096,
            redaction: super::RedactionMode::Redact,
            events: EventFilter::none(),
            trace_id: None,
            event_socket_path: None,
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
            redaction: super::RedactionMode::Redact,
            events: EventFilter::none(),
            trace_id: None,
            event_socket_path: None,
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
