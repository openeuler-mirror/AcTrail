//! Command-control service façade, host API, admission, and completion drain.

use std::collections::BTreeMap;
use std::os::fd::RawFd;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use config_core::daemon::{CommandControlConfig, EnforcementDecision};
use control_contract::reply::ControlError;
use model_core::capability::Capability;
use model_core::ids::TraceId;
use plugin_system::{
    CommandPolicyApplyRequest, CommandPolicyApplyResult, CommandPolicyApplyStatus,
    CommandPolicyDecision, CommandPolicyHost, CommandPolicyListFilter, CommandPolicyListResult,
    CommandPolicyMatchDryRunRequest, CommandPolicyMatchDryRunResult, CommandPolicyRulesApplyGrant,
    PluginRuntimeError,
};
use process_identity::ProcessIdentityManager;
use trace_runtime::registry::TraceRuntime;

use super::audit::{
    CommandAuditBuilder, CommandDecisionSource, CommandEnforcementDraft, failure_draft,
};
use super::decision::{CommandSyscall, ExecNotificationContext};
use super::rules::{CommandPolicyStore, StoredCommandRule};
use super::worker::{
    CachedCommandDecision, CommandWakeFd, GrayDecisionWorker, PluginDecisionCompletion,
    ReusableDecisionKey, respond_deferred,
};
use crate::services::control_runtime::ControlPluginRuntime;
use crate::services::identity::ResolvedTraceProcess;
use crate::services::seccomp_notify::NotificationContinuation;

pub(crate) struct CommandControlService {
    pub(super) backend: Option<Arc<Mutex<CommandControlBackend>>>,
}

#[derive(Clone)]
pub(in crate::services) struct CommandPolicyHostFacade {
    backend: Option<Arc<Mutex<CommandControlBackend>>>,
    plugins: ControlPluginRuntime,
}

pub(super) struct CommandControlBackend {
    pub(super) config: CommandControlConfig,
    pub(super) rules: CommandPolicyStore,
    pub(super) reusable_decisions: BTreeMap<ReusableDecisionKey, CachedCommandDecision>,
    pub(super) next_cache_sequence: u64,
    pub(super) in_flight_by_rule: BTreeMap<(String, String), u32>,
    pub(super) in_flight_by_instance: BTreeMap<String, u32>,
    pub(super) global_pending: u32,
    pub(super) completion_sender: SyncSender<PluginDecisionCompletion>,
    pub(super) completion_receiver: Receiver<PluginDecisionCompletion>,
    pub(super) wake: Arc<CommandWakeFd>,
}

impl CommandControlService {
    pub(crate) fn new(config: &CommandControlConfig) -> Result<Self, ControlError> {
        if !config.enabled {
            return Ok(Self { backend: None });
        }
        let rules = CommandPolicyStore::load(&config.rules_path)
            .map_err(|message| ControlError::new("command_control_policy", message))?;
        let queue_capacity = usize::try_from(config.pending_decision_max).map_err(|error| {
            ControlError::new(
                "command_control_config",
                format!("pending decision capacity overflow: {error}"),
            )
        })?;
        let (completion_sender, completion_receiver) = mpsc::sync_channel(queue_capacity);
        let wake = Arc::new(
            CommandWakeFd::new()
                .map_err(|message| ControlError::new("command_control_wake", message))?,
        );
        Ok(Self {
            backend: Some(Arc::new(Mutex::new(CommandControlBackend {
                config: config.clone(),
                rules,
                reusable_decisions: BTreeMap::new(),
                next_cache_sequence: 0,
                in_flight_by_rule: BTreeMap::new(),
                in_flight_by_instance: BTreeMap::new(),
                global_pending: 0,
                completion_sender,
                completion_receiver,
                wake,
            }))),
        })
    }

    pub(crate) fn event_poll_fds(&self) -> Vec<RawFd> {
        self.backend
            .as_ref()
            .and_then(|backend| backend.lock().ok().map(|backend| backend.wake.fd()))
            .into_iter()
            .collect()
    }

    pub(in crate::services) fn command_policy_host(
        &self,
        plugins: ControlPluginRuntime,
    ) -> CommandPolicyHostFacade {
        CommandPolicyHostFacade {
            backend: self.backend.clone(),
            plugins,
        }
    }

    pub(crate) fn remove_plugin_policy_owner(&self, instance_id: &str) -> Result<(), ControlError> {
        let Some(backend) = &self.backend else {
            return Ok(());
        };
        let mut backend = backend.lock().map_err(lock_control_error)?;
        let removed = backend
            .rules
            .remove_owner(instance_id)
            .map_err(|message| ControlError::new("command_control_policy", message))?;
        if removed {
            backend.reusable_decisions.clear();
        } else {
            backend
                .reusable_decisions
                .retain(|_, cached| cached.instance_id != instance_id);
        }
        Ok(())
    }

    pub(in crate::services) fn requires_notification_identity(
        &self,
        trace_runtime: &TraceRuntime,
        listener_trace_id: TraceId,
        notification: &libc::seccomp_notif,
    ) -> bool {
        self.backend.is_some()
            && CommandSyscall::notification_name(notification).is_some()
            && trace_has_command_control(trace_runtime, listener_trace_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) fn handle_notification(
        &self,
        listener_trace_id: TraceId,
        trace_runtime: &TraceRuntime,
        process_registry: &ProcessIdentityManager,
        prepared_process: Result<ResolvedTraceProcess, String>,
        plugins: &ControlPluginRuntime,
        notification: &libc::seccomp_notif,
        continuation: &mut NotificationContinuation,
    ) -> Result<Vec<CommandEnforcementDraft>, ControlError> {
        let Some(backend) = &self.backend else {
            return Ok(Vec::new());
        };
        let Some(operation) = CommandSyscall::notification_name(notification) else {
            return Ok(Vec::new());
        };
        if !trace_has_command_control(trace_runtime, listener_trace_id) {
            return Ok(Vec::new());
        }
        if !continuation.is_valid()? {
            return Ok(Vec::new());
        }
        let path_max_bytes = backend
            .lock()
            .map_err(lock_control_error)?
            .config
            .path_max_bytes;
        let mut context = match prepared_process.and_then(|resolved| {
            ExecNotificationContext::capture(
                listener_trace_id,
                resolved,
                process_registry,
                notification,
                path_max_bytes,
            )
            .and_then(|context| {
                context.ok_or_else(|| "notification is not an exec syscall".to_string())
            })
        }) {
            Ok(context) => context,
            Err(error) => {
                let backend = backend.lock().map_err(lock_control_error)?;
                let decision = backend.config.failure_decision;
                respond_continuation(continuation, decision)?;
                let Some(process) = trace_runtime
                    .get_trace(listener_trace_id)
                    .map(|entry| entry.trace.root_process_identity)
                else {
                    tracing::error!(
                        trace_id = %listener_trace_id,
                        pid = notification.pid,
                        error,
                        "command control capture failed after trace disappeared"
                    );
                    return Ok(Vec::new());
                };
                return Ok(vec![failure_draft(
                    listener_trace_id,
                    process,
                    operation,
                    decision,
                    notification.pid,
                    error,
                )]);
            }
        };
        let started_at = Instant::now();
        let (rule, default_decision, audit_enabled, audit_default_allow) = loop {
            let backend = backend.lock().map_err(lock_control_error)?;
            if !backend.rules.requires_args(context.resolved_path())
                || context.argv_was_snapshotted()
            {
                break (
                    backend
                        .rules
                        .find(context.resolved_path(), context.arguments())
                        .cloned(),
                    backend.config.default_decision,
                    backend.config.audit_enabled,
                    backend.config.audit_default_allow,
                );
            }
            let limits = (
                backend.config.argv_max_count,
                backend.config.argv_max_arg_bytes,
                backend.config.argv_max_total_bytes,
                backend.config.failure_decision,
            );
            drop(backend);
            if let Err(error) = context.snapshot_argv(limits.0, limits.1, limits.2) {
                respond_continuation(continuation, limits.3)?;
                return Ok(vec![
                    CommandAuditBuilder::argv_failure(
                        &context,
                        limits.3,
                        None,
                        error,
                        elapsed_us(started_at),
                    )
                    .build(),
                ]);
            }
        };
        let Some(rule) = rule else {
            respond_if_denied(continuation, default_decision)?;
            if !audit_enabled
                || (default_decision == EnforcementDecision::Allow && !audit_default_allow)
            {
                return Ok(Vec::new());
            }
            return Ok(vec![
                CommandAuditBuilder::new(
                    &context,
                    default_decision,
                    None,
                    CommandDecisionSource::Default,
                    elapsed_us(started_at),
                )
                .build(),
            ]);
        };
        match rule.decision {
            CommandPolicyDecision::Allow | CommandPolicyDecision::Deny => {
                let decision = policy_decision(rule.decision)?;
                respond_if_denied(continuation, decision)?;
                if !audit_enabled {
                    return Ok(Vec::new());
                }
                Ok(vec![
                    CommandAuditBuilder::new(
                        &context,
                        decision,
                        Some(&rule),
                        CommandDecisionSource::Rule,
                        elapsed_us(started_at),
                    )
                    .build(),
                ])
            }
            CommandPolicyDecision::Gray => {
                self.handle_gray(backend, plugins, context, rule, continuation, started_at)
            }
            CommandPolicyDecision::Default => Err(ControlError::new(
                "command_control_policy",
                "stored command rule has default decision",
            )),
        }
    }

    fn handle_gray(
        &self,
        backend_ref: &Arc<Mutex<CommandControlBackend>>,
        plugins: &ControlPluginRuntime,
        mut context: ExecNotificationContext,
        rule: StoredCommandRule,
        continuation: &mut NotificationContinuation,
        started_at: Instant,
    ) -> Result<Vec<CommandEnforcementDraft>, ControlError> {
        let limits = {
            let backend = backend_ref.lock().map_err(lock_control_error)?;
            (
                backend.config.argv_max_count,
                backend.config.argv_max_arg_bytes,
                backend.config.argv_max_total_bytes,
                backend.config.failure_decision,
                backend.config.audit_enabled,
            )
        };
        if let Err(error) = context.snapshot_argv(limits.0, limits.1, limits.2) {
            respond_continuation(continuation, limits.3)?;
            return Ok(vec![
                CommandAuditBuilder::argv_failure(
                    &context,
                    limits.3,
                    Some(&rule),
                    error,
                    elapsed_us(started_at),
                )
                .build(),
            ]);
        }
        let cache_key = ReusableDecisionKey {
            trace_id: context.trace_id(),
            process_generation: context.process_generation(),
            rule_id: rule.rule_id.clone(),
            rule_revision: rule.rule_revision,
            resolved_path: context.resolved_path().to_path_buf(),
            argv_digest: context
                .argv_digest()
                .expect("snapshotted command argv has digest")
                .to_string(),
        };
        let target = rule
            .gray_target
            .as_deref()
            .ok_or_else(|| ControlError::new("command_control_policy", "gray rule has no target"))?
            .to_string();
        let mut backend = backend_ref.lock().map_err(lock_control_error)?;
        if let Some(cached) = backend.reusable_decisions.get(&cache_key).cloned() {
            respond_continuation(continuation, cached.decision)?;
            if !backend.config.audit_enabled {
                return Ok(Vec::new());
            }
            return Ok(vec![
                CommandAuditBuilder::new(
                    &context,
                    cached.decision,
                    Some(&rule),
                    CommandDecisionSource::GrayPluginCache {
                        instance_id: cached.instance_id,
                        reason: cached.reason,
                    },
                    elapsed_us(started_at),
                )
                .build(),
            ]);
        }
        let Some((target_instance_index, instance_limit)) =
            plugins.active_instance_registration(&target)
        else {
            return immediate_gray_fallback(
                &backend.config,
                continuation,
                &context,
                &rule,
                EnforcementDecision::Deny,
                "plugin_unloaded",
                None,
                started_at,
            );
        };
        if let Some(reason) = backend.admission_rejection(&rule, &target, Some(instance_limit)) {
            let fallback = backend.config.gray.fallback;
            return immediate_gray_fallback(
                &backend.config,
                continuation,
                &context,
                &rule,
                fallback,
                reason,
                None,
                started_at,
            );
        }
        backend.reserve(&rule, &target)?;
        let deferred = continuation.defer()?;
        let worker = GrayDecisionWorker::new(
            plugins.clone(),
            backend.completion_sender.clone(),
            backend.wake.clone(),
            backend.config.gray.timeout_ms,
            backend.config.gray.fallback,
            target_instance_index,
        );
        if let Err(error) = worker.spawn(deferred.clone(), context.clone(), rule.clone()) {
            backend.release(&rule, &target)?;
            let fallback = backend.config.gray.fallback;
            respond_deferred(&deferred, fallback)?;
            if !backend.config.audit_enabled {
                return Ok(Vec::new());
            }
            return Ok(vec![
                CommandAuditBuilder::new(
                    &context,
                    fallback,
                    Some(&rule),
                    CommandDecisionSource::GrayFallback {
                        instance_id: Some(target),
                        reason: "worker_spawn_failed".to_string(),
                        error: Some(error),
                    },
                    elapsed_us(started_at),
                )
                .build(),
            ]);
        }
        Ok(Vec::new())
    }
}

impl CommandPolicyHost for CommandPolicyHostFacade {
    fn rules_version_get(&self) -> Result<u64, PluginRuntimeError> {
        Ok(self.lock_backend()?.rules.revision())
    }

    fn rules_list(
        &self,
        filter: CommandPolicyListFilter,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<CommandPolicyListResult, PluginRuntimeError> {
        self.lock_backend()?
            .rules
            .list(filter, cursor.as_deref(), limit)
            .map_err(policy_runtime_error)
    }

    fn rules_match_dry_run(
        &self,
        request: CommandPolicyMatchDryRunRequest,
    ) -> Result<CommandPolicyMatchDryRunResult, PluginRuntimeError> {
        let backend = self.lock_backend()?;
        backend
            .rules
            .match_dry_run(
                request,
                default_policy_decision(backend.config.default_decision),
            )
            .map_err(policy_runtime_error)
    }

    fn rules_validate(
        &self,
        owner_instance_id: &str,
        grants: &[CommandPolicyRulesApplyGrant],
        request: &CommandPolicyApplyRequest,
    ) -> Result<CommandPolicyApplyResult, PluginRuntimeError> {
        let backend = self.lock_backend()?;
        Ok(backend
            .rules
            .validate_apply(owner_instance_id, grants, request, |target| {
                self.plugins.is_instance_active(target)
            }))
    }

    fn rules_apply(
        &self,
        owner_instance_id: &str,
        grants: &[CommandPolicyRulesApplyGrant],
        request: CommandPolicyApplyRequest,
    ) -> Result<CommandPolicyApplyResult, PluginRuntimeError> {
        let mut backend = self.lock_backend()?;
        let result = backend
            .rules
            .apply(owner_instance_id, grants, request, |target| {
                self.plugins.is_instance_active(target)
            });
        if result.status == CommandPolicyApplyStatus::Accepted && result.applied_count > 0 {
            backend.reusable_decisions.clear();
        }
        Ok(result)
    }
}

impl CommandPolicyHostFacade {
    fn lock_backend(&self) -> Result<MutexGuard<'_, CommandControlBackend>, PluginRuntimeError> {
        let backend = self.backend.as_ref().ok_or_else(|| {
            PluginRuntimeError::new("command_policy", "command control is disabled")
        })?;
        backend.lock().map_err(|error| {
            PluginRuntimeError::new("command_policy", format!("lock backend: {error}"))
        })
    }
}

fn immediate_gray_fallback(
    config: &CommandControlConfig,
    continuation: &mut NotificationContinuation,
    context: &ExecNotificationContext,
    rule: &StoredCommandRule,
    decision: EnforcementDecision,
    reason: &str,
    error: Option<String>,
    started_at: Instant,
) -> Result<Vec<CommandEnforcementDraft>, ControlError> {
    respond_continuation(continuation, decision)?;
    if !config.audit_enabled {
        return Ok(Vec::new());
    }
    Ok(vec![
        CommandAuditBuilder::new(
            context,
            decision,
            Some(rule),
            CommandDecisionSource::GrayFallback {
                instance_id: rule.gray_target.clone(),
                reason: reason.to_string(),
                error,
            },
            elapsed_us(started_at),
        )
        .build(),
    ])
}

fn trace_has_command_control(trace_runtime: &TraceRuntime, trace_id: TraceId) -> bool {
    trace_runtime.get_trace(trace_id).is_some_and(|entry| {
        entry.sensor_plan.collectors.iter().any(|collector| {
            collector
                .capabilities
                .contains(&Capability::EnforcementCommandExecutionSeccomp)
        })
    })
}

fn respond_if_denied(
    continuation: &mut NotificationContinuation,
    decision: EnforcementDecision,
) -> Result<(), ControlError> {
    if decision == EnforcementDecision::Deny {
        continuation.deny_errno(libc::EPERM)?;
    }
    Ok(())
}

fn respond_continuation(
    continuation: &mut NotificationContinuation,
    decision: EnforcementDecision,
) -> Result<(), ControlError> {
    match decision {
        EnforcementDecision::Allow => continuation.continue_now(),
        EnforcementDecision::Deny => continuation.deny_errno(libc::EPERM),
    }
}

fn policy_decision(decision: CommandPolicyDecision) -> Result<EnforcementDecision, ControlError> {
    match decision {
        CommandPolicyDecision::Allow => Ok(EnforcementDecision::Allow),
        CommandPolicyDecision::Deny => Ok(EnforcementDecision::Deny),
        CommandPolicyDecision::Default | CommandPolicyDecision::Gray => Err(ControlError::new(
            "command_control_policy",
            format!("{} is not a local command decision", decision.as_str()),
        )),
    }
}

fn default_policy_decision(decision: EnforcementDecision) -> CommandPolicyDecision {
    match decision {
        EnforcementDecision::Allow => CommandPolicyDecision::Allow,
        EnforcementDecision::Deny => CommandPolicyDecision::Deny,
    }
}

fn elapsed_us(started_at: Instant) -> u64 {
    started_at.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

fn lock_control_error<T>(error: std::sync::PoisonError<T>) -> ControlError {
    ControlError::new("command_control_policy", format!("lock backend: {error}"))
}

fn policy_runtime_error(message: String) -> PluginRuntimeError {
    PluginRuntimeError::new("command_policy", message)
}
