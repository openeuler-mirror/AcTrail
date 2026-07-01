use std::os::fd::RawFd;
use std::panic::{AssertUnwindSafe, catch_unwind};

use config_core::daemon::EnforcementDecision;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use plugin_system::{
    ControlDecisionBudget, ControlDecisionRequest, ControlDecisionResponse, ControlVerdict,
    DecisionScope,
};

use super::audit::{
    Decision, DecisionSource, EnforcementEventDraft, SyncPluginFallbackReason, event_draft,
};
use super::wake::notify_wake_fd;
use crate::services::control_runtime::ControlPluginRuntime;
use crate::services::enforcement::fanotify::{PermissionEventFd, respond};
use crate::services::enforcement::rules::{EnforcementRule, FileKey};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ReusableDecisionKey {
    pub(super) trace_id: TraceId,
    pub(super) rule_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CachedDecision {
    pub(super) decision: EnforcementDecision,
}

pub(super) struct PluginDecisionCompletion {
    pub(super) rule_id: String,
    pub(super) instance_id: String,
    pub(super) cache_update: Option<(ReusableDecisionKey, CachedDecision)>,
    pub(super) draft: Option<EnforcementEventDraft>,
    pub(super) error: Option<String>,
}

pub(super) struct PendingPluginDecision {
    pub(super) fanotify_fd: RawFd,
    pub(super) wake_fd: RawFd,
    pub(super) event_fd: PermissionEventFd,
    pub(super) trace_id: TraceId,
    pub(super) process: ProcessIdentity,
    pub(super) rule: EnforcementRule,
    pub(super) instance_id: String,
    pub(super) timeout_ms: u64,
    pub(super) concurrency_limit: u32,
    pub(super) fallback: EnforcementDecision,
    pub(super) request: ControlDecisionRequest,
    pub(super) budget: ControlDecisionBudget,
    pub(super) control_plugins: ControlPluginRuntime,
    pub(super) audit_enabled: bool,
    pub(super) observed_path: Option<String>,
    pub(super) file_key: Option<FileKey>,
    pub(super) audit_metadata_error: Option<String>,
    pub(super) cache_key: ReusableDecisionKey,
    pub(super) completion_tx: std::sync::mpsc::Sender<PluginDecisionCompletion>,
}

pub(super) fn complete_plugin_decision(pending: PendingPluginDecision) {
    let PendingPluginDecision {
        fanotify_fd,
        wake_fd,
        event_fd,
        trace_id,
        process,
        rule,
        instance_id,
        timeout_ms,
        concurrency_limit,
        fallback,
        request,
        budget,
        control_plugins,
        audit_enabled,
        observed_path,
        file_key,
        audit_metadata_error,
        cache_key,
        completion_tx,
    } = pending;

    let completion_instance_id = instance_id.clone();
    let plugin_result = control_plugins.is_instance_active(&instance_id).then(|| {
        catch_unwind(AssertUnwindSafe(|| {
            control_plugins.decide(&instance_id, request, budget)
        }))
    });
    let (decision, cache_update) = match plugin_result {
        Some(Ok(Ok(response))) if control_plugins.is_instance_active(&instance_id) => {
            let decision = enforcement_decision_from_response(&response);
            let cache_update = (response.scope == DecisionScope::Reusable)
                .then_some((cache_key, CachedDecision { decision }));
            (
                Decision {
                    decision,
                    rule: Some(&rule),
                    source: DecisionSource::SyncPlugin {
                        instance_id,
                        timeout_ms,
                        concurrency_limit,
                        scope: response.scope,
                        reason: response.reason,
                    },
                },
                cache_update,
            )
        }
        None | Some(Ok(Ok(_))) => (
            Decision {
                decision: EnforcementDecision::Deny,
                rule: Some(&rule),
                source: DecisionSource::SyncPluginFallback {
                    instance_id,
                    timeout_ms,
                    concurrency_limit,
                    reason: SyncPluginFallbackReason::PluginError,
                    error: Some("control plugin unloaded during file-policy decision".to_string()),
                    in_flight: None,
                    instance_concurrency_limit: None,
                    instance_in_flight: None,
                    fallback: EnforcementDecision::Deny,
                },
            },
            None,
        ),
        Some(Ok(Err(error))) => (
            Decision {
                decision: fallback,
                rule: Some(&rule),
                source: DecisionSource::SyncPluginFallback {
                    instance_id,
                    timeout_ms,
                    concurrency_limit,
                    reason: SyncPluginFallbackReason::PluginError,
                    error: Some(format!("{}: {}", error.code, error.message)),
                    in_flight: None,
                    instance_concurrency_limit: None,
                    instance_in_flight: None,
                    fallback,
                },
            },
            None,
        ),
        Some(Err(_)) => (
            Decision {
                decision: fallback,
                rule: Some(&rule),
                source: DecisionSource::SyncPluginFallback {
                    instance_id,
                    timeout_ms,
                    concurrency_limit,
                    reason: SyncPluginFallbackReason::PluginPanic,
                    error: Some("control plugin panicked".to_string()),
                    in_flight: None,
                    instance_concurrency_limit: None,
                    instance_in_flight: None,
                    fallback,
                },
            },
            None,
        ),
    };

    let mut completion = PluginDecisionCompletion {
        rule_id: rule.rule_id.clone(),
        instance_id: completion_instance_id,
        cache_update,
        draft: None,
        error: None,
    };
    match respond(
        fanotify_fd,
        event_fd.raw_fd(),
        matches!(decision.decision, EnforcementDecision::Allow),
    ) {
        Ok(()) if audit_enabled => {
            completion.draft = Some(event_draft(
                trace_id,
                process,
                decision,
                file_key,
                observed_path,
                audit_metadata_error,
            ));
        }
        Ok(()) => {}
        Err(error) => {
            completion.cache_update = None;
            completion.error = Some(error);
        }
    }
    let _ = completion_tx.send(completion);
    let _ = notify_wake_fd(wake_fd);
}

fn enforcement_decision_from_response(response: &ControlDecisionResponse) -> EnforcementDecision {
    match response.verdict {
        ControlVerdict::Allow => EnforcementDecision::Allow,
        ControlVerdict::Deny => EnforcementDecision::Deny,
    }
}
