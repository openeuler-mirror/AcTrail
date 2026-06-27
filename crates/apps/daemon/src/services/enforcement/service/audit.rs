use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::EnforcementDecision;
use model_core::event::EnforcementPayload;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use plugin_system::DecisionScope;

use crate::services::enforcement::rules::{EnforcementRule, FileKey};

pub(in crate::services) struct EnforcementEventDraft {
    pub trace_id: TraceId,
    pub observed_at: SystemTime,
    pub process: ProcessIdentity,
    pub metadata_partial: bool,
    pub payload: EnforcementPayload,
}

pub(super) struct Decision<'a> {
    pub(super) decision: EnforcementDecision,
    pub(super) rule: Option<&'a EnforcementRule>,
    pub(super) source: DecisionSource,
}

pub(super) enum DecisionSource {
    Default,
    Rule,
    SyncPlugin {
        instance_id: String,
        timeout_ms: u64,
        concurrency_limit: u32,
        scope: DecisionScope,
        reason: Option<String>,
    },
    SyncPluginCache {
        instance_id: String,
    },
    SyncPluginFallback {
        instance_id: String,
        timeout_ms: u64,
        concurrency_limit: u32,
        reason: SyncPluginFallbackReason,
        error: Option<String>,
        in_flight: Option<u32>,
        instance_concurrency_limit: Option<u32>,
        instance_in_flight: Option<u32>,
        fallback: EnforcementDecision,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SyncPluginFallbackReason {
    PluginError,
    PluginPanic,
    ConcurrencyLimit,
    PluginInstanceConcurrencyLimit,
}

impl SyncPluginFallbackReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::PluginError => "plugin_error",
            Self::PluginPanic => "plugin_panic",
            Self::ConcurrencyLimit => "concurrency_limit",
            Self::PluginInstanceConcurrencyLimit => "plugin_instance_concurrency_limit",
        }
    }
}

pub(super) fn event_draft(
    trace_id: TraceId,
    process: ProcessIdentity,
    decision: Decision<'_>,
    file_key: Option<FileKey>,
    fallback_path: Option<String>,
    audit_metadata_error: Option<String>,
) -> EnforcementEventDraft {
    let metadata_partial = audit_metadata_error.is_some();
    let mut metadata = BTreeMap::from([("scope".to_string(), "trace".to_string())]);
    if let Some(file_key) = file_key {
        metadata.insert("file_dev".to_string(), file_key.dev.to_string());
        metadata.insert("file_ino".to_string(), file_key.ino.to_string());
    }
    if let Some(error) = audit_metadata_error {
        metadata.insert("audit_metadata_error".to_string(), error);
    }
    insert_decision_source_metadata(&mut metadata, &decision.source);
    EnforcementEventDraft {
        trace_id,
        observed_at: SystemTime::now(),
        process,
        metadata_partial,
        payload: EnforcementPayload {
            backend: "fanotify".to_string(),
            operation: "open".to_string(),
            decision: decision.decision.as_str().to_string(),
            path: decision
                .rule
                .map(|rule| rule.path.display().to_string())
                .or(fallback_path),
            rule_id: decision.rule.map(|rule| rule.rule_id.clone()),
            result: match decision.decision {
                EnforcementDecision::Allow => "allowed",
                EnforcementDecision::Deny => "denied",
            }
            .to_string(),
            metadata,
        },
    }
}

fn insert_decision_source_metadata(
    metadata: &mut BTreeMap<String, String>,
    source: &DecisionSource,
) {
    match source {
        DecisionSource::Default => {
            metadata.insert("decision_source".to_string(), "default".to_string());
        }
        DecisionSource::Rule => {
            metadata.insert("decision_source".to_string(), "rule".to_string());
        }
        DecisionSource::SyncPlugin {
            instance_id,
            timeout_ms,
            concurrency_limit,
            scope,
            reason,
        } => {
            metadata.insert("decision_source".to_string(), "sync-plugin".to_string());
            metadata.insert("plugin_instance".to_string(), instance_id.clone());
            metadata.insert("plugin_timeout_ms".to_string(), timeout_ms.to_string());
            metadata.insert(
                "plugin_concurrency_limit".to_string(),
                concurrency_limit.to_string(),
            );
            metadata.insert("decision_scope".to_string(), scope.as_str().to_string());
            if let Some(reason) = reason {
                metadata.insert("plugin_reason".to_string(), reason.clone());
            }
        }
        DecisionSource::SyncPluginCache { instance_id } => {
            metadata.insert(
                "decision_source".to_string(),
                "sync-plugin-cache".to_string(),
            );
            metadata.insert("plugin_instance".to_string(), instance_id.clone());
        }
        DecisionSource::SyncPluginFallback {
            instance_id,
            timeout_ms,
            concurrency_limit,
            reason,
            error,
            in_flight,
            instance_concurrency_limit,
            instance_in_flight,
            fallback,
        } => {
            metadata.insert(
                "decision_source".to_string(),
                "sync-plugin-fallback".to_string(),
            );
            metadata.insert("plugin_instance".to_string(), instance_id.clone());
            metadata.insert("plugin_timeout_ms".to_string(), timeout_ms.to_string());
            metadata.insert(
                "plugin_concurrency_limit".to_string(),
                concurrency_limit.to_string(),
            );
            metadata.insert("fallback_reason".to_string(), reason.as_str().to_string());
            if let Some(error) = error {
                metadata.insert("plugin_error".to_string(), error.clone());
            }
            if let Some(in_flight) = in_flight {
                metadata.insert("plugin_inflight".to_string(), in_flight.to_string());
            }
            if let Some(instance_concurrency_limit) = instance_concurrency_limit {
                metadata.insert(
                    "plugin_instance_concurrency_limit".to_string(),
                    instance_concurrency_limit.to_string(),
                );
            }
            if let Some(instance_in_flight) = instance_in_flight {
                metadata.insert(
                    "plugin_instance_inflight".to_string(),
                    instance_in_flight.to_string(),
                );
            }
            metadata.insert(
                "fallback_decision".to_string(),
                fallback.as_str().to_string(),
            );
        }
    }
}
