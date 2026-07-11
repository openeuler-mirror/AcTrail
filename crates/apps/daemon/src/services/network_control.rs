//! Network-action control policy and seccomp dispatch helpers.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Mutex;
use std::time::SystemTime;

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use config_core::daemon::{EnforcementDecision, NetworkControlConfig};
use control_contract::reply::ControlError;
use model_core::ids::{CollectorName, TraceId};
use model_core::process::ProcessIdentity;
use plugin_system::{
    CONTROL_CURRENT_CONTEXT_TOKEN, ControlDecisionBudget, ControlDecisionRequest, ControlSubject,
    ControlVerdict,
};
use process_identity::ProcessIdentityManager;
use process_identity::ProcessIdentityReader;
use trace_runtime::registry::TraceRuntime;

use crate::services::control_runtime::ControlPluginRuntime;
use crate::services::identity::ControlActorIdentityResolver;
use crate::services::identity::TraceIdentityResolver;
use crate::services::seccomp_notify::{
    NotificationContinuation, read_process_bytes, target_exited,
};

pub(crate) const NETWORK_CONTROL_COLLECTOR_NAME: &str = "network-control";

#[derive(Debug)]
pub(crate) struct NetworkControlService {
    enabled: bool,
    rules: NetworkControlRules,
    in_flight_by_rule: Mutex<BTreeMap<String, u32>>,
}

impl NetworkControlService {
    pub(crate) fn new(config: &NetworkControlConfig) -> Result<Self, ControlError> {
        let rules = if config.enabled {
            NetworkControlRules::load(&config.rules_path)?
        } else {
            NetworkControlRules::default()
        };
        Ok(Self {
            enabled: config.enabled,
            rules,
            in_flight_by_rule: Mutex::new(BTreeMap::new()),
        })
    }

    pub(in crate::services) fn handle_notification(
        &self,
        trace_runtime: &TraceRuntime,
        process_registry: &ProcessIdentityManager,
        identity_reader: &impl ProcessIdentityReader,
        notification: &libc::seccomp_notif,
        continuation: &mut NotificationContinuation,
        control_plugins: &ControlPluginRuntime,
    ) -> Result<Vec<RawCollectorEvent>, ControlError> {
        if !self.enabled || i64::from(notification.data.nr) != libc::SYS_connect {
            return Ok(Vec::new());
        }
        let Some(remote) = read_connect_remote(
            notification.pid,
            notification.data.args[1],
            notification.data.args[2],
        )?
        else {
            return Ok(Vec::new());
        };
        let Some(rule) = self.rules.find_endpoint(&remote.endpoint) else {
            return Ok(Vec::new());
        };
        let Some(candidate) = self.candidate(
            trace_runtime,
            process_registry,
            identity_reader,
            notification,
            remote,
        )?
        else {
            return Ok(Vec::new());
        };
        let outcome = self.decide_connect(&candidate, process_registry, control_plugins, rule)?;
        if network_control_decision(&outcome) == EnforcementDecision::Deny {
            continuation.deny_errno(libc::EPERM)?;
        } else {
            continuation.continue_now()?;
        }
        Ok(vec![network_control_event(
            candidate,
            outcome,
            process_registry,
        )?])
    }

    fn candidate(
        &self,
        trace_runtime: &TraceRuntime,
        process_registry: &ProcessIdentityManager,
        identity_reader: &impl ProcessIdentityReader,
        notification: &libc::seccomp_notif,
        remote: NetworkRemote,
    ) -> Result<Option<NetworkConnectCandidate>, ControlError> {
        let resolver = TraceIdentityResolver::new(trace_runtime, process_registry);
        let Some(process) = resolver.runtime_or_read_pid_identity(
            identity_reader,
            notification.pid,
            "network_control_identity",
        )?
        else {
            return Ok(None);
        };
        let parent_pid = parent_pid(notification.pid)?;
        let trace_id = process_registry
            .active_host_pid(notification.pid)
            .and_then(|identity| trace_runtime.find_membership(&identity))
            .map(|(trace_id, _)| trace_id)
            .or_else(|| {
                parent_pid
                    .and_then(|pid| process_registry.active_host_pid(pid))
                    .and_then(|identity| trace_runtime.find_membership(&identity))
                    .map(|(trace_id, _)| trace_id)
            });
        let Some(trace_id) = trace_id else {
            return Ok(None);
        };
        Ok(Some(NetworkConnectCandidate {
            trace_id,
            process,
            fd: notification.data.args[0],
            remote,
        }))
    }

    fn decide_connect(
        &self,
        candidate: &NetworkConnectCandidate,
        process_registry: &ProcessIdentityManager,
        control_plugins: &ControlPluginRuntime,
        rule: &NetworkControlRule,
    ) -> Result<NetworkControlOutcome, ControlError> {
        if !self.try_reserve_rule(rule)? {
            return Ok(NetworkControlOutcome::DecisionError {
                decision: rule.fallback,
                rule_id: rule.rule_id.clone(),
                plugin_instance: rule.instance_id.clone(),
                timeout_ms: rule.timeout_ms,
                concurrency_limit: rule.concurrency_limit,
                error: "concurrency_limit".to_string(),
            });
        }
        let result = self.decide_rule(candidate, process_registry, control_plugins, rule);
        self.release_rule(&rule.rule_id);
        result
    }

    fn decide_rule(
        &self,
        candidate: &NetworkConnectCandidate,
        process_registry: &ProcessIdentityManager,
        control_plugins: &ControlPluginRuntime,
        rule: &NetworkControlRule,
    ) -> Result<NetworkControlOutcome, ControlError> {
        let request = ControlDecisionRequest {
            decision_id: format!("{}:{}", rule.rule_id, candidate.trace_id),
            trace_id: candidate.trace_id.to_string(),
            subject: ControlSubject::NetworkAction,
            actor_process_identity: ControlActorIdentityResolver::new(process_registry)
                .resolve(candidate.process)?,
            operation: "connect".to_string(),
            target_summary: format!(
                "remote={} family={} fd={}",
                candidate.remote.endpoint, candidate.remote.family, candidate.fd
            ),
            context_ref: Some(CONTROL_CURRENT_CONTEXT_TOKEN.to_string()),
            file_policy_context: None,
        };
        let response = control_plugins
            .decide(
                &rule.instance_id,
                request,
                ControlDecisionBudget {
                    timeout_ms: Some(rule.timeout_ms),
                },
            )
            .map_err(|error| {
                ControlError::new(
                    "network_control_plugin",
                    format!("{}: {}", error.code, error.message),
                )
            });
        match response {
            Ok(response) => Ok(NetworkControlOutcome::Decision {
                decision: decision_to_enforcement(response.verdict),
                rule_id: rule.rule_id.clone(),
                plugin_instance: rule.instance_id.clone(),
                timeout_ms: rule.timeout_ms,
                concurrency_limit: rule.concurrency_limit,
            }),
            Err(error) => Ok(NetworkControlOutcome::Decision {
                decision: rule.fallback,
                rule_id: rule.rule_id.clone(),
                plugin_instance: rule.instance_id.clone(),
                timeout_ms: rule.timeout_ms,
                concurrency_limit: rule.concurrency_limit,
            }
            .with_error(error.message)),
        }
    }

    fn try_reserve_rule(&self, rule: &NetworkControlRule) -> Result<bool, ControlError> {
        let mut in_flight = self.in_flight_by_rule.lock().map_err(|error| {
            ControlError::new(
                "network_control_policy",
                format!("in-flight lock poisoned: {error}"),
            )
        })?;
        let current = in_flight.get(&rule.rule_id).copied().unwrap_or_default();
        if current >= rule.concurrency_limit {
            return Ok(false);
        }
        in_flight.insert(rule.rule_id.clone(), current + 1);
        Ok(true)
    }

    fn release_rule(&self, rule_id: &str) {
        let Ok(mut in_flight) = self.in_flight_by_rule.lock() else {
            return;
        };
        match in_flight.get_mut(rule_id) {
            Some(count) if *count > 1 => *count -= 1,
            Some(_) => {
                in_flight.remove(rule_id);
            }
            None => {}
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NetworkControlOutcome {
    Decision {
        decision: EnforcementDecision,
        rule_id: String,
        plugin_instance: String,
        timeout_ms: u64,
        concurrency_limit: u32,
    },
    DecisionError {
        decision: EnforcementDecision,
        rule_id: String,
        plugin_instance: String,
        timeout_ms: u64,
        concurrency_limit: u32,
        error: String,
    },
}

impl NetworkControlOutcome {
    fn with_error(self, error: String) -> Self {
        match self {
            Self::Decision {
                decision,
                rule_id,
                plugin_instance,
                timeout_ms,
                concurrency_limit,
            } => Self::DecisionError {
                decision,
                rule_id,
                plugin_instance,
                timeout_ms,
                concurrency_limit,
                error,
            },
            other => other,
        }
    }
}

#[derive(Debug, Default)]
struct NetworkControlRules {
    by_endpoint: BTreeMap<SocketAddr, NetworkControlRule>,
}

impl NetworkControlRules {
    fn load(path: &std::path::Path) -> Result<Self, ControlError> {
        let raw = std::fs::read_to_string(path).map_err(|error| {
            ControlError::new(
                "network_control_policy",
                format!(
                    "read network control rules {} failed: {error}",
                    path.display()
                ),
            )
        })?;
        let mut rules = Self::default();
        for (index, raw_line) in raw.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let rule = parse_rule(line).map_err(|message| {
                ControlError::new(
                    "network_control_policy",
                    format!("{}:{}: {message}", path.display(), index + 1),
                )
            })?;
            if rules.by_endpoint.insert(rule.endpoint, rule).is_some() {
                return Err(ControlError::new(
                    "network_control_policy",
                    format!(
                        "{}:{}: duplicate network endpoint",
                        path.display(),
                        index + 1
                    ),
                ));
            }
        }
        Ok(rules)
    }

    fn find_endpoint(&self, endpoint: &SocketAddr) -> Option<&NetworkControlRule> {
        self.by_endpoint.get(endpoint)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NetworkControlRule {
    rule_id: String,
    instance_id: String,
    timeout_ms: u64,
    concurrency_limit: u32,
    fallback: EnforcementDecision,
    endpoint: SocketAddr,
}

#[derive(Clone, Debug)]
struct NetworkConnectCandidate {
    trace_id: TraceId,
    process: ProcessIdentity,
    fd: u64,
    remote: NetworkRemote,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NetworkRemote {
    endpoint: SocketAddr,
    family: &'static str,
}

fn parse_rule(line: &str) -> Result<NetworkControlRule, String> {
    let parts = line.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 11 {
        return Err("expected: <rule_id> sync-plugin <instance> timeout-ms <positive-ms> concurrency <positive-limit> fallback <allow|deny> connect <ip:port>".to_string());
    }
    if parts[1] != "sync-plugin"
        || parts[3] != "timeout-ms"
        || parts[5] != "concurrency"
        || parts[7] != "fallback"
        || parts[9] != "connect"
    {
        return Err("expected: <rule_id> sync-plugin <instance> timeout-ms <positive-ms> concurrency <positive-limit> fallback <allow|deny> connect <ip:port>".to_string());
    }
    Ok(NetworkControlRule {
        rule_id: parts[0].to_string(),
        instance_id: parts[2].to_string(),
        timeout_ms: positive_u64("timeout-ms", parts[4])?,
        concurrency_limit: positive_u32("concurrency", parts[6])?,
        fallback: parse_decision(parts[8])?,
        endpoint: parts[10]
            .parse::<SocketAddr>()
            .map_err(|error| format!("connect endpoint must be ip:port: {error}"))?,
    })
}

fn read_connect_remote(
    pid: u32,
    sockaddr_ptr: u64,
    sockaddr_len: u64,
) -> Result<Option<NetworkRemote>, ControlError> {
    if sockaddr_ptr == 0 {
        return Ok(None);
    }
    let read_len = sockaddr_len.min(28);
    let read_len = usize::try_from(read_len).map_err(|error| {
        ControlError::new(
            "network_control_sockaddr",
            format!("sockaddr len overflow: {error}"),
        )
    })?;
    let Some(bytes) = read_process_bytes(pid, sockaddr_ptr, read_len)? else {
        return Ok(None);
    };
    if bytes.len() < 2 {
        return Ok(None);
    }
    let family = u16::from_ne_bytes([bytes[0], bytes[1]]);
    if family == libc::AF_INET as u16 {
        return parse_sockaddr_in(&bytes).map(Some);
    }
    if family == libc::AF_INET6 as u16 {
        return parse_sockaddr_in6(&bytes).map(Some);
    }
    Ok(None)
}

fn parse_sockaddr_in(bytes: &[u8]) -> Result<NetworkRemote, ControlError> {
    if bytes.len() < 8 {
        return Err(ControlError::new(
            "network_control_sockaddr",
            "short AF_INET sockaddr",
        ));
    }
    let port = u16::from_be_bytes([bytes[2], bytes[3]]);
    let addr = Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7]);
    Ok(NetworkRemote {
        endpoint: SocketAddr::from((addr, port)),
        family: "inet",
    })
}

fn parse_sockaddr_in6(bytes: &[u8]) -> Result<NetworkRemote, ControlError> {
    if bytes.len() < 24 {
        return Err(ControlError::new(
            "network_control_sockaddr",
            "short AF_INET6 sockaddr",
        ));
    }
    let port = u16::from_be_bytes([bytes[2], bytes[3]]);
    let mut raw_addr = [0_u8; 16];
    raw_addr.copy_from_slice(&bytes[8..24]);
    Ok(NetworkRemote {
        endpoint: SocketAddr::from((Ipv6Addr::from(raw_addr), port)),
        family: "inet6",
    })
}

fn network_control_event(
    candidate: NetworkConnectCandidate,
    outcome: NetworkControlOutcome,
    process_registry: &ProcessIdentityManager,
) -> Result<RawCollectorEvent, ControlError> {
    let mut metadata = network_control_metadata(&outcome);
    metadata.insert("operation".to_string(), "connect".to_string());
    metadata.insert("remote".to_string(), candidate.remote.endpoint.to_string());
    metadata.insert(
        "address_family".to_string(),
        candidate.remote.family.to_string(),
    );
    metadata.insert("fd".to_string(), candidate.fd.to_string());
    let process = process_registry
        .record(candidate.process)
        .ok_or_else(|| ControlError::new("network_control", "process record is missing"))?
        .observation();
    Ok(RawCollectorEvent {
        envelope: RawEventEnvelope {
            observed_at: SystemTime::now(),
            process,
            collector: CollectorName::new(NETWORK_CONTROL_COLLECTOR_NAME),
        },
        payload: RawObservationPayload::Net {
            transport: "tcp".to_string(),
            local: None,
            remote: Some(candidate.remote.endpoint.to_string()),
            size: None,
            result: (network_control_decision(&outcome) == EnforcementDecision::Deny)
                .then_some(-libc::EPERM),
            metadata,
        },
    })
}

fn network_control_metadata(outcome: &NetworkControlOutcome) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();
    metadata.insert("subject".to_string(), "network-action".to_string());
    metadata.insert("decision_source".to_string(), "sync-plugin".to_string());
    metadata.insert(
        "decision".to_string(),
        network_control_decision(outcome).as_str().to_string(),
    );
    match outcome {
        NetworkControlOutcome::Decision {
            rule_id,
            plugin_instance,
            timeout_ms,
            concurrency_limit,
            ..
        }
        | NetworkControlOutcome::DecisionError {
            rule_id,
            plugin_instance,
            timeout_ms,
            concurrency_limit,
            ..
        } => {
            metadata.insert("rule_id".to_string(), rule_id.clone());
            metadata.insert("plugin_instance".to_string(), plugin_instance.clone());
            metadata.insert("plugin_timeout_ms".to_string(), timeout_ms.to_string());
            metadata.insert(
                "plugin_concurrency_limit".to_string(),
                concurrency_limit.to_string(),
            );
        }
    }
    if let NetworkControlOutcome::DecisionError { error, .. } = outcome {
        metadata.insert("plugin_error".to_string(), error.clone());
        let fallback_reason = if error == "concurrency_limit" {
            "concurrency_limit"
        } else {
            "plugin_error"
        };
        metadata.insert("fallback_reason".to_string(), fallback_reason.to_string());
    }
    metadata
}

fn network_control_decision(outcome: &NetworkControlOutcome) -> EnforcementDecision {
    match outcome {
        NetworkControlOutcome::Decision { decision, .. }
        | NetworkControlOutcome::DecisionError { decision, .. } => *decision,
    }
}

fn parse_decision(value: &str) -> Result<EnforcementDecision, String> {
    match value {
        "allow" => Ok(EnforcementDecision::Allow),
        "deny" => Ok(EnforcementDecision::Deny),
        _ => Err("fallback must be allow or deny".to_string()),
    }
}

fn decision_to_enforcement(verdict: ControlVerdict) -> EnforcementDecision {
    match verdict {
        ControlVerdict::Allow => EnforcementDecision::Allow,
        ControlVerdict::Deny => EnforcementDecision::Deny,
    }
}

fn positive_u64(label: &str, value: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|error| format!("{label} must be a positive integer: {error}"))?;
    if parsed == 0 {
        return Err(format!("{label} must be greater than zero"));
    }
    Ok(parsed)
}

fn positive_u32(label: &str, value: &str) -> Result<u32, String> {
    let parsed = value
        .parse::<u32>()
        .map_err(|error| format!("{label} must be a positive integer: {error}"))?;
    if parsed == 0 {
        return Err(format!("{label} must be greater than zero"));
    }
    Ok(parsed)
}

fn parent_pid(pid: u32) -> Result<Option<u32>, ControlError> {
    let raw = match std::fs::read_to_string(format!("/proc/{pid}/status")) {
        Ok(raw) => raw,
        Err(error) if target_exited(&error) => return Ok(None),
        Err(error) => {
            return Err(ControlError::new(
                "network_control_procfs",
                error.to_string(),
            ));
        }
    };
    for line in raw.lines() {
        if let Some(value) = line.strip_prefix("PPid:") {
            return value.trim().parse::<u32>().map(Some).map_err(|error| {
                ControlError::new("network_control_procfs", format!("parse PPid: {error}"))
            });
        }
    }
    Err(ControlError::new(
        "network_control_procfs",
        format!("missing PPid for pid {pid}"),
    ))
}
