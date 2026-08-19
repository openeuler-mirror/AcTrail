//! Network-control service façade, policy host, and gray admission.

use std::collections::BTreeMap;
use std::os::fd::RawFd;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use collector_event::RawCollectorEvent;
use config_core::daemon::{EnforcementDecision, NetworkControlConfig};
use control_contract::reply::ControlError;
use model_core::{capability::Capability, ids::TraceId, process::ProcessIdentity};
use plugin_system::{
    NetworkPolicyApplyRequest, NetworkPolicyApplyResult, NetworkPolicyApplyStatus,
    NetworkPolicyDecision, NetworkPolicyHost, NetworkPolicyListFilter, NetworkPolicyListResult,
    NetworkPolicyMatchDryRunRequest, NetworkPolicyMatchDryRunResult, NetworkPolicyRulesApplyGrant,
    PluginRuntimeError,
};
use process_identity::ProcessIdentityManager;
use trace_runtime::registry::TraceRuntime;

use super::audit::{NetworkAuditBuilder, NetworkDecisionSource, failure_event};
use super::request::{NetworkConnectContext, NetworkRemote};
use super::rules::{NetworkPolicyStore, StoredNetworkRule};
use super::worker::{
    CachedNetworkDecision, GrayDecisionWorker, NetworkWakeFd, PluginDecisionCompletion,
    ReusableDecisionKey, respond_deferred,
};
use crate::services::control_runtime::ControlPluginRuntime;
use crate::services::identity::ResolvedTraceProcess;
use crate::services::seccomp_notify::NotificationContinuation;

pub(crate) struct NetworkControlService {
    pub(super) backend: Option<Arc<Mutex<NetworkControlBackend>>>,
}

#[derive(Clone)]
pub(in crate::services) struct NetworkPolicyHostFacade {
    backend: Option<Arc<Mutex<NetworkControlBackend>>>,
    plugins: ControlPluginRuntime,
}

pub(super) struct NetworkControlBackend {
    pub(super) config: NetworkControlConfig,
    pub(super) rules: NetworkPolicyStore,
    pub(super) reusable_decisions: BTreeMap<ReusableDecisionKey, CachedNetworkDecision>,
    pub(super) next_cache_sequence: u64,
    pub(super) in_flight_by_rule: BTreeMap<(String, String), u32>,
    pub(super) in_flight_by_instance: BTreeMap<String, u32>,
    pub(super) global_pending: u32,
    pub(super) completion_sender: SyncSender<PluginDecisionCompletion>,
    pub(super) completion_receiver: Receiver<PluginDecisionCompletion>,
    pub(super) wake: Arc<NetworkWakeFd>,
}

impl NetworkControlService {
    pub(crate) fn new(config: &NetworkControlConfig) -> Result<Self, ControlError> {
        if !config.enabled {
            return Ok(Self { backend: None });
        }
        let rules = NetworkPolicyStore::load(&config.rules_path)
            .map_err(|message| ControlError::new("network_control_policy", message))?;
        let queue_capacity = usize::try_from(config.pending_decision_max).map_err(|error| {
            ControlError::new(
                "network_control_config",
                format!("pending decision capacity overflow: {error}"),
            )
        })?;
        let (completion_sender, completion_receiver) = mpsc::sync_channel(queue_capacity);
        let wake = Arc::new(
            NetworkWakeFd::new()
                .map_err(|message| ControlError::new("network_control_wake", message))?,
        );
        Ok(Self {
            backend: Some(Arc::new(Mutex::new(NetworkControlBackend {
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

    pub(in crate::services) fn network_policy_host(
        &self,
        plugins: ControlPluginRuntime,
    ) -> NetworkPolicyHostFacade {
        NetworkPolicyHostFacade {
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
            .map_err(|message| ControlError::new("network_control_policy", message))?;
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
            && i64::from(notification.data.nr) == libc::SYS_connect
            && Self::enabled_for_trace(trace_runtime, listener_trace_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) fn handle_notification(
        &self,
        listener_trace_id: TraceId,
        trace_runtime: &TraceRuntime,
        process_registry: &ProcessIdentityManager,
        prepared_process: Result<ResolvedTraceProcess, String>,
        notification: &libc::seccomp_notif,
        continuation: &mut NotificationContinuation,
        plugins: &ControlPluginRuntime,
    ) -> Result<Vec<RawCollectorEvent>, ControlError> {
        let Some(backend_ref) = &self.backend else {
            return Ok(Vec::new());
        };
        if i64::from(notification.data.nr) != libc::SYS_connect {
            return Ok(Vec::new());
        }
        if !Self::enabled_for_trace(trace_runtime, listener_trace_id) {
            return Ok(Vec::new());
        }
        if !continuation.is_valid()? {
            return Ok(Vec::new());
        }
        let remote = match NetworkRemote::read(
            notification.pid,
            notification.data.args[1],
            notification.data.args[2],
        ) {
            Ok(Some(remote)) => remote,
            Ok(None) => return Ok(Vec::new()),
            Err(error) => {
                let config = backend_ref
                    .lock()
                    .map_err(lock_control_error)?
                    .config
                    .clone();
                respond_continuation(continuation, config.failure_decision)?;
                tracing::error!(
                    trace_id = %listener_trace_id,
                    pid = notification.pid,
                    error.code = %error.code,
                    error.message = %error.message,
                    "network control could not capture connect sockaddr"
                );
                return Ok(Vec::new());
            }
        };
        let prepared_identity = prepared_process
            .as_ref()
            .ok()
            .map(|resolved| resolved.process);
        let context = match prepared_process.and_then(|resolved| {
            NetworkConnectContext::capture(
                listener_trace_id,
                resolved,
                process_registry,
                notification.pid,
                notification.data.args[0],
                remote.clone(),
            )
            .map_err(|error| format!("{}: {}", error.code, error.message))
        }) {
            Ok(context) => context,
            Err(error) => {
                return self.handle_capture_failure(
                    backend_ref,
                    listener_trace_id,
                    trace_runtime,
                    process_registry,
                    notification,
                    &remote,
                    continuation,
                    prepared_identity,
                    error,
                );
            }
        };
        let started_at = Instant::now();
        let (rule, default_decision, audit_enabled, audit_default_allow) = {
            let backend = backend_ref.lock().map_err(lock_control_error)?;
            (
                backend.rules.find(&context.endpoint()).cloned(),
                backend.config.default_decision,
                backend.config.audit_enabled,
                backend.config.audit_default_allow,
            )
        };
        let Some(rule) = rule else {
            respond_continuation(continuation, default_decision)?;
            if !audit_enabled
                || (default_decision == EnforcementDecision::Allow && !audit_default_allow)
            {
                return Ok(Vec::new());
            }
            return Ok(vec![
                NetworkAuditBuilder::new(
                    &context,
                    default_decision,
                    None,
                    NetworkDecisionSource::Default,
                    elapsed_us(started_at),
                )
                .build(process_registry)?,
            ]);
        };
        match rule.decision {
            NetworkPolicyDecision::Allow | NetworkPolicyDecision::Deny => {
                let decision = policy_decision(rule.decision)?;
                respond_continuation(continuation, decision)?;
                if !audit_enabled {
                    return Ok(Vec::new());
                }
                Ok(vec![
                    NetworkAuditBuilder::new(
                        &context,
                        decision,
                        Some(&rule),
                        NetworkDecisionSource::Rule,
                        elapsed_us(started_at),
                    )
                    .build(process_registry)?,
                ])
            }
            NetworkPolicyDecision::Gray => self.handle_gray(
                backend_ref,
                plugins,
                process_registry,
                context,
                rule,
                continuation,
                started_at,
            ),
            NetworkPolicyDecision::Default => Err(ControlError::new(
                "network_control_policy",
                "stored network rule has default decision",
            )),
        }
    }

    fn enabled_for_trace(trace_runtime: &TraceRuntime, trace_id: TraceId) -> bool {
        trace_runtime.get_trace(trace_id).is_some_and(|entry| {
            entry.sensor_plan.collectors.iter().any(|collector| {
                collector
                    .capabilities
                    .contains(&Capability::EnforcementNetworkConnectSeccomp)
            })
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_capture_failure(
        &self,
        backend_ref: &Arc<Mutex<NetworkControlBackend>>,
        listener_trace_id: TraceId,
        trace_runtime: &TraceRuntime,
        process_registry: &ProcessIdentityManager,
        notification: &libc::seccomp_notif,
        remote: &NetworkRemote,
        continuation: &mut NotificationContinuation,
        prepared_identity: Option<ProcessIdentity>,
        error: String,
    ) -> Result<Vec<RawCollectorEvent>, ControlError> {
        let config = backend_ref
            .lock()
            .map_err(lock_control_error)?
            .config
            .clone();
        respond_continuation(continuation, config.failure_decision)?;
        if !config.audit_enabled {
            return Ok(Vec::new());
        }
        let process = prepared_identity.or_else(|| {
            trace_runtime
                .get_trace(listener_trace_id)
                .map(|entry| entry.trace.root_process_identity)
        });
        let Some(process) = process else {
            tracing::error!(
                trace_id = %listener_trace_id,
                pid = notification.pid,
                error,
                "network control capture failed without an auditable process"
            );
            return Ok(Vec::new());
        };
        Ok(vec![failure_event(
            listener_trace_id,
            process,
            remote,
            notification.data.args[0],
            config.failure_decision,
            error,
            process_registry,
        )?])
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_gray(
        &self,
        backend_ref: &Arc<Mutex<NetworkControlBackend>>,
        plugins: &ControlPluginRuntime,
        process_registry: &ProcessIdentityManager,
        context: NetworkConnectContext,
        rule: StoredNetworkRule,
        continuation: &mut NotificationContinuation,
        started_at: Instant,
    ) -> Result<Vec<RawCollectorEvent>, ControlError> {
        let cache_key = ReusableDecisionKey {
            trace_id: context.trace_id(),
            process_generation: context.process_generation(),
            owner_instance_id: rule.owner_instance_id.clone(),
            rule_id: rule.rule_id.clone(),
            rule_revision: rule.rule_revision,
            remote: context.endpoint(),
        };
        let target = rule
            .gray_target
            .as_deref()
            .ok_or_else(|| ControlError::new("network_control_policy", "gray rule has no target"))?
            .to_string();
        let mut backend = backend_ref.lock().map_err(lock_control_error)?;
        if let Some(cached) = backend.reusable_decisions.get(&cache_key).cloned() {
            respond_continuation(continuation, cached.decision)?;
            if !backend.config.audit_enabled {
                return Ok(Vec::new());
            }
            return Ok(vec![
                NetworkAuditBuilder::new(
                    &context,
                    cached.decision,
                    Some(&rule),
                    NetworkDecisionSource::GrayPluginCache {
                        instance_id: cached.instance_id,
                        reason: cached.reason,
                    },
                    elapsed_us(started_at),
                )
                .build(process_registry)?,
            ]);
        }
        let Some((target_instance_index, instance_limit)) =
            plugins.active_instance_registration(&target)
        else {
            return immediate_gray_fallback(
                &backend.config,
                continuation,
                process_registry,
                &context,
                &rule,
                EnforcementDecision::Deny,
                "plugin_unloaded",
                None,
                started_at,
            );
        };
        if let Some(reason) = backend.admission_rejection(&rule, &target, Some(instance_limit)) {
            let fallback = rule.fallback.ok_or_else(|| {
                ControlError::new("network_control_policy", "gray rule has no fallback")
            })?;
            return immediate_gray_fallback(
                &backend.config,
                continuation,
                process_registry,
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
            target_instance_index,
        );
        if let Err(error) = worker.spawn(deferred.clone(), context.clone(), rule.clone()) {
            backend.release(&rule, &target)?;
            let fallback = rule.fallback.ok_or_else(|| {
                ControlError::new("network_control_policy", "gray rule has no fallback")
            })?;
            respond_deferred(&deferred, fallback)?;
            if !backend.config.audit_enabled {
                return Ok(Vec::new());
            }
            return Ok(vec![
                NetworkAuditBuilder::new(
                    &context,
                    fallback,
                    Some(&rule),
                    NetworkDecisionSource::GrayFallback {
                        instance_id: Some(target),
                        reason: "worker_spawn_failed".to_string(),
                        error: Some(error),
                    },
                    elapsed_us(started_at),
                )
                .build(process_registry)?,
            ]);
        }
        Ok(Vec::new())
    }
}

impl NetworkPolicyHost for NetworkPolicyHostFacade {
    fn rules_version_get(&self) -> Result<u64, PluginRuntimeError> {
        Ok(self.lock_backend()?.rules.revision())
    }

    fn rules_list(
        &self,
        filter: NetworkPolicyListFilter,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<NetworkPolicyListResult, PluginRuntimeError> {
        self.lock_backend()?
            .rules
            .list(filter, cursor.as_deref(), limit)
            .map_err(policy_runtime_error)
    }

    fn rules_match_dry_run(
        &self,
        request: NetworkPolicyMatchDryRunRequest,
    ) -> Result<NetworkPolicyMatchDryRunResult, PluginRuntimeError> {
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
        grants: &[NetworkPolicyRulesApplyGrant],
        request: &NetworkPolicyApplyRequest,
    ) -> Result<NetworkPolicyApplyResult, PluginRuntimeError> {
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
        grants: &[NetworkPolicyRulesApplyGrant],
        request: NetworkPolicyApplyRequest,
    ) -> Result<NetworkPolicyApplyResult, PluginRuntimeError> {
        let mut backend = self.lock_backend()?;
        let result = backend
            .rules
            .apply(owner_instance_id, grants, request, |target| {
                self.plugins.is_instance_active(target)
            });
        if result.status == NetworkPolicyApplyStatus::Accepted && result.applied_count > 0 {
            backend.reusable_decisions.clear();
        }
        Ok(result)
    }
}

impl NetworkPolicyHostFacade {
    fn lock_backend(&self) -> Result<MutexGuard<'_, NetworkControlBackend>, PluginRuntimeError> {
        let backend = self.backend.as_ref().ok_or_else(|| {
            PluginRuntimeError::new("network_policy", "network control is disabled")
        })?;
        backend.lock().map_err(|error| {
            PluginRuntimeError::new("network_policy", format!("lock backend: {error}"))
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn immediate_gray_fallback(
    config: &NetworkControlConfig,
    continuation: &mut NotificationContinuation,
    process_registry: &ProcessIdentityManager,
    context: &NetworkConnectContext,
    rule: &StoredNetworkRule,
    decision: EnforcementDecision,
    reason: &str,
    error: Option<String>,
    started_at: Instant,
) -> Result<Vec<RawCollectorEvent>, ControlError> {
    respond_continuation(continuation, decision)?;
    if !config.audit_enabled {
        return Ok(Vec::new());
    }
    Ok(vec![
        NetworkAuditBuilder::new(
            context,
            decision,
            Some(rule),
            NetworkDecisionSource::GrayFallback {
                instance_id: rule.gray_target.clone(),
                reason: reason.to_string(),
                error,
            },
            elapsed_us(started_at),
        )
        .build(process_registry)?,
    ])
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

fn policy_decision(decision: NetworkPolicyDecision) -> Result<EnforcementDecision, ControlError> {
    match decision {
        NetworkPolicyDecision::Allow => Ok(EnforcementDecision::Allow),
        NetworkPolicyDecision::Deny => Ok(EnforcementDecision::Deny),
        NetworkPolicyDecision::Default | NetworkPolicyDecision::Gray => Err(ControlError::new(
            "network_control_policy",
            format!("{} is not a local network decision", decision.as_str()),
        )),
    }
}

fn default_policy_decision(decision: EnforcementDecision) -> NetworkPolicyDecision {
    match decision {
        EnforcementDecision::Allow => NetworkPolicyDecision::Allow,
        EnforcementDecision::Deny => NetworkPolicyDecision::Deny,
    }
}

fn elapsed_us(started_at: Instant) -> u64 {
    started_at.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

fn lock_control_error<T>(error: std::sync::PoisonError<T>) -> ControlError {
    ControlError::new("network_control_policy", format!("lock backend: {error}"))
}

fn policy_runtime_error(message: String) -> PluginRuntimeError {
    PluginRuntimeError::new("network_policy", message)
}
