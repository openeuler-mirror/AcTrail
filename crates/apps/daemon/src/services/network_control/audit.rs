//! Network control event construction and decision attribution.

use std::collections::BTreeMap;
use std::time::SystemTime;

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use config_core::daemon::EnforcementDecision;
use control_contract::reply::ControlError;
use model_core::ids::{CollectorName, TraceId};
use model_core::process::ProcessIdentity;
use plugin_system::DecisionScope;
use process_identity::ProcessIdentityManager;

use super::request::{NetworkConnectContext, NetworkRemote};
use super::rules::StoredNetworkRule;

pub(super) const NETWORK_CONTROL_COLLECTOR_NAME: &str = "network-control";

pub(super) enum NetworkDecisionSource {
    Default,
    Rule,
    GrayPlugin {
        instance_id: String,
        scope: DecisionScope,
        reason: Option<String>,
    },
    GrayPluginCache {
        instance_id: String,
        reason: Option<String>,
    },
    GrayFallback {
        instance_id: Option<String>,
        reason: String,
        error: Option<String>,
    },
}

pub(super) struct NetworkAuditBuilder<'a> {
    context: &'a NetworkConnectContext,
    decision: EnforcementDecision,
    rule: Option<&'a StoredNetworkRule>,
    source: NetworkDecisionSource,
    latency_us: u64,
}

impl<'a> NetworkAuditBuilder<'a> {
    pub(super) fn new(
        context: &'a NetworkConnectContext,
        decision: EnforcementDecision,
        rule: Option<&'a StoredNetworkRule>,
        source: NetworkDecisionSource,
        latency_us: u64,
    ) -> Self {
        Self {
            context,
            decision,
            rule,
            source,
            latency_us,
        }
    }

    pub(super) fn build(
        self,
        process_registry: &ProcessIdentityManager,
    ) -> Result<RawCollectorEvent, ControlError> {
        let mut metadata = self.metadata();
        Self::remote_metadata(&mut metadata, self.context.remote(), self.context.fd());
        let process = process_registry
            .record(self.context.process())
            .ok_or_else(|| ControlError::new("network_control", "process record is missing"))?
            .observation();
        Ok(Self::event(
            self.context.trace_id(),
            process,
            self.context.remote(),
            self.decision,
            metadata,
        ))
    }

    fn metadata(&self) -> BTreeMap<String, String> {
        let mut metadata = BTreeMap::new();
        metadata.insert("subject".to_string(), "network-action".to_string());
        metadata.insert("operation".to_string(), "connect".to_string());
        metadata.insert("decision".to_string(), self.decision.as_str().to_string());
        metadata.insert(
            "decision_latency_us".to_string(),
            self.latency_us.to_string(),
        );
        if let Some(rule) = self.rule {
            metadata.insert("rule_id".to_string(), rule.rule_id.clone());
            metadata.insert("policy_remote_scope".to_string(), rule.remote.to_string());
            metadata.insert(
                "policy_owner_instance_id".to_string(),
                rule.owner_instance_id.clone(),
            );
            metadata.insert("rule_revision".to_string(), rule.rule_revision.to_string());
            if let Some(target) = &rule.gray_target {
                metadata.insert("gray_target".to_string(), target.clone());
            }
            if let Some(timeout_ms) = rule.timeout_ms {
                metadata.insert("plugin_timeout_ms".to_string(), timeout_ms.to_string());
            }
            if let Some(limit) = rule.concurrency_limit {
                metadata.insert("rule_concurrency_limit".to_string(), limit.to_string());
            }
            if let Some(fallback) = rule.fallback {
                metadata.insert(
                    "fallback_decision".to_string(),
                    fallback.as_str().to_string(),
                );
            }
        }
        self.source.add_metadata(&mut metadata);
        metadata
    }

    fn remote_metadata(metadata: &mut BTreeMap<String, String>, remote: &NetworkRemote, fd: u64) {
        metadata.insert("remote".to_string(), remote.endpoint().to_string());
        metadata.insert(
            "address_family".to_string(),
            remote.address_family().to_string(),
        );
        metadata.insert("fd".to_string(), fd.to_string());
        if remote.ipv6_scope_id() != 0 {
            metadata.insert(
                "ipv6_scope_id".to_string(),
                remote.ipv6_scope_id().to_string(),
            );
        }
    }

    fn event(
        trace_id: TraceId,
        process: model_core::process::ProcessObservation,
        remote: &NetworkRemote,
        decision: EnforcementDecision,
        metadata: BTreeMap<String, String>,
    ) -> RawCollectorEvent {
        RawCollectorEvent {
            envelope: RawEventEnvelope {
                trace_id: Some(trace_id),
                observed_at: SystemTime::now(),
                process,
                collector: CollectorName::new(NETWORK_CONTROL_COLLECTOR_NAME),
            },
            payload: RawObservationPayload::Net {
                transport: "inet".to_string(),
                local: None,
                remote: Some(remote.endpoint().to_string()),
                size: None,
                result: (decision == EnforcementDecision::Deny).then_some(-libc::EPERM),
                metadata,
            },
        }
    }
}

impl NetworkDecisionSource {
    fn add_metadata(&self, metadata: &mut BTreeMap<String, String>) {
        match self {
            Self::Default => {
                metadata.insert("decision_source".to_string(), "default".to_string());
            }
            Self::Rule => {
                metadata.insert("decision_source".to_string(), "rule".to_string());
            }
            Self::GrayPlugin {
                instance_id,
                scope,
                reason,
            } => {
                metadata.insert("decision_source".to_string(), "gray-plugin".to_string());
                metadata.insert("plugin_instance".to_string(), instance_id.clone());
                metadata.insert("plugin_scope".to_string(), scope.as_str().to_string());
                if let Some(reason) = reason {
                    metadata.insert("plugin_reason".to_string(), reason.clone());
                }
            }
            Self::GrayPluginCache {
                instance_id,
                reason,
            } => {
                metadata.insert(
                    "decision_source".to_string(),
                    "gray-plugin-cache".to_string(),
                );
                metadata.insert("plugin_instance".to_string(), instance_id.clone());
                if let Some(reason) = reason {
                    metadata.insert("plugin_reason".to_string(), reason.clone());
                }
            }
            Self::GrayFallback {
                instance_id,
                reason,
                error,
            } => {
                metadata.insert("decision_source".to_string(), "fallback".to_string());
                metadata.insert("fallback_reason".to_string(), reason.clone());
                if let Some(instance_id) = instance_id {
                    metadata.insert("plugin_instance".to_string(), instance_id.clone());
                }
                if let Some(error) = error {
                    metadata.insert("plugin_error".to_string(), error.clone());
                }
            }
        }
    }
}

pub(super) fn failure_event(
    trace_id: TraceId,
    process: ProcessIdentity,
    remote: &NetworkRemote,
    fd: u64,
    decision: EnforcementDecision,
    error: String,
    process_registry: &ProcessIdentityManager,
) -> Result<RawCollectorEvent, ControlError> {
    let mut metadata = BTreeMap::new();
    metadata.insert("subject".to_string(), "network-action".to_string());
    metadata.insert("operation".to_string(), "connect".to_string());
    metadata.insert("decision".to_string(), decision.as_str().to_string());
    metadata.insert("decision_source".to_string(), "failure".to_string());
    metadata.insert("failure_error".to_string(), error);
    NetworkAuditBuilder::remote_metadata(&mut metadata, remote, fd);
    let process = process_registry
        .record(process)
        .ok_or_else(|| ControlError::new("network_control", "failure process record is missing"))?
        .observation();
    Ok(NetworkAuditBuilder::event(
        trace_id, process, remote, decision, metadata,
    ))
}
