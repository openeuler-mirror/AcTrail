//! Enforcement event and command-boundary alert drafts.

use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::EnforcementDecision;
use model_core::event::EnforcementPayload;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use plugin_system::DecisionScope;

use super::decision::ExecNotificationContext;
use super::rules::StoredCommandRule;
use crate::services::alert_ingress::{CommandExecutionBoundaryAlert, CommandExecutionDenySource};

pub(crate) struct CommandControlDrain {
    pub(crate) outcomes: Vec<CommandEnforcementDraft>,
    pub(crate) allowed_execs: Vec<ExecNotificationContext>,
}

impl CommandControlDrain {
    pub(super) fn empty() -> Self {
        Self {
            outcomes: Vec::new(),
            allowed_execs: Vec::new(),
        }
    }
}

pub(super) fn failure_draft(
    trace_id: TraceId,
    process: ProcessIdentity,
    operation: &str,
    decision: EnforcementDecision,
    target_pid: u32,
    error: String,
) -> CommandEnforcementDraft {
    let metadata = BTreeMap::from([
        ("scope".to_string(), "trace".to_string()),
        ("decision_source".to_string(), "failure".to_string()),
        ("fallback_reason".to_string(), "capture_failure".to_string()),
        ("command_control_error".to_string(), error),
        ("target_pid".to_string(), target_pid.to_string()),
        ("path_truncated".to_string(), "unknown".to_string()),
        ("argv_truncated".to_string(), "unknown".to_string()),
    ]);
    CommandEnforcementDraft {
        trace_id,
        observed_at: SystemTime::now(),
        process,
        metadata_partial: true,
        payload: EnforcementPayload {
            backend: "seccomp-user-notify".to_string(),
            operation: operation.to_string(),
            decision: decision.as_str().to_string(),
            path: None,
            rule_id: None,
            result: match decision {
                EnforcementDecision::Allow => "allowed",
                EnforcementDecision::Deny => "denied",
            }
            .to_string(),
            metadata,
        },
        boundary_alert: None,
    }
}

pub(crate) struct CommandEnforcementDraft {
    pub(crate) trace_id: TraceId,
    pub(crate) observed_at: SystemTime,
    pub(crate) process: ProcessIdentity,
    pub(crate) metadata_partial: bool,
    pub(crate) payload: EnforcementPayload,
    pub(crate) boundary_alert: Option<CommandExecutionBoundaryAlert>,
}

#[derive(Clone, Debug)]
pub(super) enum CommandDecisionSource {
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
    Failure {
        reason: String,
        error: String,
    },
}

pub(super) struct CommandAuditBuilder<'a> {
    context: &'a ExecNotificationContext,
    decision: EnforcementDecision,
    rule: Option<&'a StoredCommandRule>,
    source: CommandDecisionSource,
    latency_us: u64,
}

impl<'a> CommandAuditBuilder<'a> {
    pub(super) fn new(
        context: &'a ExecNotificationContext,
        decision: EnforcementDecision,
        rule: Option<&'a StoredCommandRule>,
        source: CommandDecisionSource,
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

    pub(super) fn argv_failure(
        context: &'a ExecNotificationContext,
        decision: EnforcementDecision,
        rule: Option<&'a StoredCommandRule>,
        error: String,
        latency_us: u64,
    ) -> Self {
        Self::new(
            context,
            decision,
            rule,
            CommandDecisionSource::Failure {
                reason: "argv_limit_or_read_failure".to_string(),
                error,
            },
            latency_us,
        )
    }

    pub(super) fn build(self) -> CommandEnforcementDraft {
        let mut metadata = BTreeMap::from([
            ("scope".to_string(), "trace".to_string()),
            (
                "requested_executable".to_string(),
                self.context.requested_path().to_string(),
            ),
            ("path_truncated".to_string(), "false".to_string()),
            ("argv_truncated".to_string(), "false".to_string()),
            (
                "decision_latency_us".to_string(),
                self.latency_us.to_string(),
            ),
        ]);
        if let Some(rule) = self.rule {
            metadata.insert(
                "policy_owner_instance_id".to_string(),
                rule.owner_instance_id.clone(),
            );
            metadata.insert("rule_revision".to_string(), rule.rule_revision.to_string());
            if let Some(args) = rule.args_view() {
                metadata.insert(
                    "rule_args".to_string(),
                    serde_json::to_string(&args)
                        .expect("command rule args serialization cannot fail"),
                );
            }
        }
        if !self.context.argv().is_empty() || self.context.argv_digest().is_some() {
            metadata.insert(
                "argv_count".to_string(),
                self.context.argv().len().to_string(),
            );
        }
        if let Some(digest) = self.context.argv_digest() {
            metadata.insert("argv_digest".to_string(), digest.to_string());
        }
        self.source.insert_metadata(&mut metadata);
        let boundary_alert = self.boundary_alert();
        CommandEnforcementDraft {
            trace_id: self.context.trace_id(),
            observed_at: SystemTime::now(),
            process: self.context.process(),
            metadata_partial: matches!(self.source, CommandDecisionSource::Failure { .. }),
            payload: EnforcementPayload {
                backend: "seccomp-user-notify".to_string(),
                operation: self.context.syscall().as_str().to_string(),
                decision: self.decision.as_str().to_string(),
                path: Some(self.context.resolved_path().display().to_string()),
                rule_id: self.rule.map(|rule| rule.rule_id.clone()),
                result: match self.decision {
                    EnforcementDecision::Allow => "allowed",
                    EnforcementDecision::Deny => "denied",
                }
                .to_string(),
                metadata,
            },
            boundary_alert,
        }
    }

    fn boundary_alert(&self) -> Option<CommandExecutionBoundaryAlert> {
        if self.decision != EnforcementDecision::Deny {
            return None;
        }
        let rule = self.rule?;
        let (source, plugin_instance, plugin_reason) = match &self.source {
            CommandDecisionSource::Rule => (CommandExecutionDenySource::FastPathDeny, None, None),
            CommandDecisionSource::GrayPlugin {
                instance_id,
                reason,
                ..
            } => (
                CommandExecutionDenySource::GrayPluginDeny,
                Some(instance_id.clone()),
                reason.clone(),
            ),
            CommandDecisionSource::GrayPluginCache {
                instance_id,
                reason,
            } => (
                CommandExecutionDenySource::GrayPluginCacheDeny,
                Some(instance_id.clone()),
                reason.clone(),
            ),
            CommandDecisionSource::Default
            | CommandDecisionSource::GrayFallback { .. }
            | CommandDecisionSource::Failure { .. } => return None,
        };
        Some(CommandExecutionBoundaryAlert::new(
            self.context.syscall().as_str().to_string(),
            self.context.resolved_path().display().to_string(),
            rule.rule_id.clone(),
            rule.owner_instance_id.clone(),
            self.context.process().get(),
            source,
            plugin_instance,
            plugin_reason,
        ))
    }
}

impl CommandDecisionSource {
    fn insert_metadata(&self, metadata: &mut BTreeMap<String, String>) {
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
                metadata.insert("decision_scope".to_string(), scope.as_str().to_string());
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
                metadata.insert("decision_scope".to_string(), "reusable".to_string());
                if let Some(reason) = reason {
                    metadata.insert("plugin_reason".to_string(), reason.clone());
                }
            }
            Self::GrayFallback {
                instance_id,
                reason,
                error,
            } => {
                metadata.insert("decision_source".to_string(), "gray-fallback".to_string());
                metadata.insert("fallback_reason".to_string(), reason.clone());
                if let Some(instance_id) = instance_id {
                    metadata.insert("plugin_instance".to_string(), instance_id.clone());
                }
                if let Some(error) = error {
                    metadata.insert("plugin_error".to_string(), error.clone());
                }
            }
            Self::Failure { reason, error } => {
                metadata.insert("decision_source".to_string(), "failure".to_string());
                metadata.insert("fallback_reason".to_string(), reason.clone());
                metadata.insert("command_control_error".to_string(), error.clone());
            }
        }
    }
}
