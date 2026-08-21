//! Deferred gray decisions, bounded admission, wakeup, and reusable cache.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, TryRecvError};
use std::time::Instant;

use collector_event::RawCollectorEvent;
use config_core::daemon::EnforcementDecision;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use plugin_system::{
    ControlDecisionBudget, ControlDecisionRequest, ControlSubject, ControlVerdict, DecisionScope,
    NETWORK_ACTION_CURRENT_CONTEXT_TOKEN,
};
use process_identity::ProcessIdentityManager;
use trace_runtime::registry::TraceRuntime;

use super::audit::{NetworkAuditBuilder, NetworkDecisionSource};
use super::request::NetworkConnectContext;
use super::rules::StoredNetworkRule;
use super::service::{NetworkControlBackend, NetworkControlService};
use crate::services::control_runtime::ControlPluginRuntime;
use crate::services::seccomp_notify::DeferredNotification;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ReusableDecisionKey {
    pub(super) trace_id: TraceId,
    pub(super) process_generation: u64,
    pub(super) owner_instance_id: String,
    pub(super) rule_id: String,
    pub(super) rule_revision: u64,
    pub(super) remote: SocketAddr,
}

#[derive(Clone, Debug)]
pub(super) struct CachedNetworkDecision {
    pub(super) decision: EnforcementDecision,
    pub(super) instance_id: String,
    pub(super) reason: Option<String>,
    inserted_sequence: u64,
}

pub(super) enum PluginDecisionOutcome {
    Plugin {
        decision: EnforcementDecision,
        scope: DecisionScope,
        reason: Option<String>,
    },
    Fallback {
        decision: EnforcementDecision,
        reason: String,
        error: Option<String>,
    },
}

pub(super) struct PluginDecisionCompletion {
    deferred: DeferredNotification,
    context: NetworkConnectContext,
    rule: StoredNetworkRule,
    outcome: PluginDecisionOutcome,
    latency_us: u64,
    target_instance_index: u64,
}

pub(super) struct GrayDecisionWorker {
    plugins: ControlPluginRuntime,
    completion_sender: SyncSender<PluginDecisionCompletion>,
    wake: Arc<NetworkWakeFd>,
    target_instance_index: u64,
}

impl GrayDecisionWorker {
    pub(super) fn new(
        plugins: ControlPluginRuntime,
        completion_sender: SyncSender<PluginDecisionCompletion>,
        wake: Arc<NetworkWakeFd>,
        target_instance_index: u64,
    ) -> Self {
        Self {
            plugins,
            completion_sender,
            wake,
            target_instance_index,
        }
    }

    pub(super) fn spawn(
        self,
        deferred: DeferredNotification,
        context: NetworkConnectContext,
        rule: StoredNetworkRule,
    ) -> Result<(), String> {
        let thread_name = format!("network-gray-{}", rule.rule_id);
        std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || self.run(deferred, context, rule))
            .map(|_| ())
            .map_err(|error| format!("spawn network gray decision worker: {error}"))
    }

    fn run(
        self,
        deferred: DeferredNotification,
        context: NetworkConnectContext,
        rule: StoredNetworkRule,
    ) {
        let started_at = Instant::now();
        let target = rule
            .gray_target
            .as_deref()
            .expect("validated gray network rule has target");
        let fallback = rule
            .fallback
            .expect("validated gray network rule has fallback");
        let timeout_ms = rule
            .timeout_ms
            .expect("validated gray network rule has timeout");
        let outcome = if !self
            .plugins
            .is_instance_index_active(self.target_instance_index)
        {
            PluginDecisionOutcome::Fallback {
                decision: EnforcementDecision::Deny,
                reason: "plugin_unloaded".to_string(),
                error: None,
            }
        } else {
            let response = self.plugins.decide(
                target,
                ControlDecisionRequest {
                    decision_id: format!("{}:{}", rule.rule_id, deferred.notification_id()),
                    trace_id: context.trace_id().to_string(),
                    subject: ControlSubject::NetworkAction,
                    actor_process_identity: context.actor(),
                    operation: "connect".to_string(),
                    target_summary: context.target_summary(),
                    context_ref: Some(NETWORK_ACTION_CURRENT_CONTEXT_TOKEN.to_string()),
                    file_policy_context: None,
                    command_execution_context: None,
                    network_action_context: Some(context.action_context()),
                },
                ControlDecisionBudget {
                    timeout_ms: Some(timeout_ms),
                },
            );
            match response {
                Ok(response)
                    if self
                        .plugins
                        .is_instance_index_active(self.target_instance_index) =>
                {
                    PluginDecisionOutcome::Plugin {
                        decision: verdict_decision(response.verdict),
                        scope: response.scope,
                        reason: response.reason,
                    }
                }
                Ok(_) => PluginDecisionOutcome::Fallback {
                    decision: EnforcementDecision::Deny,
                    reason: "plugin_unloaded".to_string(),
                    error: None,
                },
                Err(_)
                    if !self
                        .plugins
                        .is_instance_index_active(self.target_instance_index) =>
                {
                    PluginDecisionOutcome::Fallback {
                        decision: EnforcementDecision::Deny,
                        reason: "plugin_unloaded".to_string(),
                        error: None,
                    }
                }
                Err(error) => PluginDecisionOutcome::Fallback {
                    decision: fallback,
                    reason: plugin_failure_reason(&error).to_string(),
                    error: Some(format!("{}: {}", error.code, error.message)),
                },
            }
        };
        let completion = PluginDecisionCompletion {
            deferred: deferred.clone(),
            context,
            rule,
            outcome,
            latency_us: started_at.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
            target_instance_index: self.target_instance_index,
        };
        if self.completion_sender.send(completion).is_err() {
            let _ = deferred.deny_errno(libc::EPERM);
            return;
        }
        if let Err(error) = self.wake.notify() {
            tracing::error!(
                error = %error,
                "network gray completion could not wake daemon event loop"
            );
        }
    }
}

impl NetworkControlService {
    pub(in crate::services) fn drain_completions(
        &self,
        trace_runtime: &TraceRuntime,
        process_registry: &ProcessIdentityManager,
        plugins: &ControlPluginRuntime,
    ) -> Result<Vec<RawCollectorEvent>, ControlError> {
        let Some(backend_ref) = &self.backend else {
            return Ok(Vec::new());
        };
        let mut backend = backend_ref.lock().map_err(|error| {
            ControlError::new("network_control_policy", format!("lock backend: {error}"))
        })?;
        backend.wake.drain()?;
        let mut events = Vec::new();
        loop {
            let completion = match backend.completion_receiver.try_recv() {
                Ok(completion) => completion,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(ControlError::new(
                        "network_control_worker",
                        "network gray completion channel disconnected",
                    ));
                }
            };
            let target = completion
                .rule
                .gray_target
                .as_deref()
                .expect("gray network completion has target")
                .to_string();
            backend.release(&completion.rule, &target)?;
            if completion.deferred.trace_id() != completion.context.trace_id() {
                completion.deferred.deny_errno(libc::EPERM)?;
                return Err(ControlError::new(
                    "network_control_worker",
                    "deferred notification trace does not match captured network context",
                ));
            }
            if !completion.deferred.is_valid()? {
                continue;
            }
            let target_active = plugins.is_instance_index_active(completion.target_instance_index);
            let (decision, source, cache_value) = match completion.outcome {
                PluginDecisionOutcome::Plugin {
                    decision,
                    scope,
                    reason,
                } if target_active => {
                    let source = NetworkDecisionSource::GrayPlugin {
                        instance_id: target.clone(),
                        scope,
                        reason: reason.clone(),
                    };
                    let cache = (scope == DecisionScope::Reusable).then_some((
                        decision,
                        target.clone(),
                        reason,
                    ));
                    (decision, source, cache)
                }
                PluginDecisionOutcome::Plugin { .. } => (
                    EnforcementDecision::Deny,
                    NetworkDecisionSource::GrayFallback {
                        instance_id: Some(target.clone()),
                        reason: "plugin_unloaded".to_string(),
                        error: None,
                    },
                    None,
                ),
                PluginDecisionOutcome::Fallback {
                    decision,
                    reason,
                    error,
                } => (
                    decision,
                    NetworkDecisionSource::GrayFallback {
                        instance_id: Some(target),
                        reason,
                        error,
                    },
                    None,
                ),
            };
            respond_deferred(&completion.deferred, decision)?;
            if let Some((decision, instance_id, reason)) = cache_value
                && backend.rules.is_rule_current(&completion.rule)
                && trace_runtime
                    .get_trace(completion.context.trace_id())
                    .is_some_and(|entry| !entry.trace.lifecycle_state.is_terminal())
            {
                backend.cache_decision(
                    &completion.context,
                    &completion.rule,
                    decision,
                    instance_id,
                    reason,
                )?;
            }
            if backend.config.audit_enabled {
                events.push(
                    NetworkAuditBuilder::new(
                        &completion.context,
                        decision,
                        Some(&completion.rule),
                        source,
                        completion.latency_us,
                    )
                    .build(process_registry)?,
                );
            }
        }
        Ok(events)
    }
}

impl NetworkControlBackend {
    pub(super) fn admission_rejection(
        &self,
        rule: &StoredNetworkRule,
        target: &str,
        instance_limit: Option<u32>,
    ) -> Option<&'static str> {
        if self.global_pending >= self.config.pending_decision_max {
            return Some("global_pending_limit");
        }
        let rule_key = (rule.owner_instance_id.clone(), rule.rule_id.clone());
        if self.in_flight_by_rule.get(&rule_key).copied().unwrap_or(0)
            >= rule
                .concurrency_limit
                .expect("validated gray network rule has concurrency limit")
        {
            return Some("rule_concurrency_limit");
        }
        if instance_limit.is_none_or(|limit| {
            self.in_flight_by_instance.get(target).copied().unwrap_or(0) >= limit
        }) {
            return Some("plugin_instance_concurrency_limit");
        }
        None
    }

    pub(super) fn reserve(
        &mut self,
        rule: &StoredNetworkRule,
        target: &str,
    ) -> Result<(), ControlError> {
        self.global_pending = checked_increment(self.global_pending, "global pending")?;
        increment_map(
            &mut self.in_flight_by_rule,
            (rule.owner_instance_id.clone(), rule.rule_id.clone()),
            "rule in-flight",
        )?;
        increment_map(
            &mut self.in_flight_by_instance,
            target.to_string(),
            "instance in-flight",
        )
    }

    pub(super) fn release(
        &mut self,
        rule: &StoredNetworkRule,
        target: &str,
    ) -> Result<(), ControlError> {
        self.global_pending = self.global_pending.checked_sub(1).ok_or_else(|| {
            ControlError::new("network_control_accounting", "global pending underflow")
        })?;
        decrement_map(
            &mut self.in_flight_by_rule,
            &(rule.owner_instance_id.clone(), rule.rule_id.clone()),
            "rule in-flight",
        )?;
        decrement_map(
            &mut self.in_flight_by_instance,
            &target.to_string(),
            "instance in-flight",
        )
    }

    fn cache_decision(
        &mut self,
        context: &NetworkConnectContext,
        rule: &StoredNetworkRule,
        decision: EnforcementDecision,
        instance_id: String,
        reason: Option<String>,
    ) -> Result<(), ControlError> {
        self.next_cache_sequence = self
            .next_cache_sequence
            .checked_add(1)
            .ok_or_else(|| ControlError::new("network_control_cache", "cache sequence overflow"))?;
        let key = ReusableDecisionKey {
            trace_id: context.trace_id(),
            process_generation: context.process_generation(),
            owner_instance_id: rule.owner_instance_id.clone(),
            rule_id: rule.rule_id.clone(),
            rule_revision: rule.rule_revision,
            remote: context.endpoint(),
        };
        if let Some(previous) = self.reusable_decisions.get(&key) {
            self.reusable_order.remove(&previous.inserted_sequence);
        } else {
            self.reusable_by_trace
                .entry(key.trace_id)
                .or_default()
                .insert(key.clone());
        }
        self.reusable_decisions.insert(
            key.clone(),
            CachedNetworkDecision {
                decision,
                instance_id,
                reason,
                inserted_sequence: self.next_cache_sequence,
            },
        );
        self.reusable_order.insert(self.next_cache_sequence, key);
        while self.reusable_decisions.len() > self.config.reusable_cache_max_entries as usize {
            let oldest = self
                .reusable_order
                .first_key_value()
                .map(|(_, key)| key.clone());
            if let Some(oldest) = oldest {
                self.remove_cached_key(&oldest);
            }
        }
        Ok(())
    }

    pub(super) fn forget_trace_cache(&mut self, trace_id: TraceId) {
        let Some(keys) = self.reusable_by_trace.remove(&trace_id) else {
            return;
        };
        for key in keys {
            if let Some(cached) = self.reusable_decisions.remove(&key) {
                self.reusable_order.remove(&cached.inserted_sequence);
            }
        }
    }

    pub(super) fn clear_reusable_cache(&mut self) {
        self.reusable_decisions.clear();
        self.reusable_order.clear();
        self.reusable_by_trace.clear();
    }

    pub(super) fn remove_cached_decisions_by_instance(&mut self, instance_id: &str) {
        let keys = self
            .reusable_decisions
            .iter()
            .filter(|(_, cached)| cached.instance_id == instance_id)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            self.remove_cached_key(&key);
        }
    }

    fn remove_cached_key(&mut self, key: &ReusableDecisionKey) {
        let Some(cached) = self.reusable_decisions.remove(key) else {
            return;
        };
        self.reusable_order.remove(&cached.inserted_sequence);
        let remove_trace_bucket =
            self.reusable_by_trace
                .get_mut(&key.trace_id)
                .is_some_and(|keys| {
                    keys.remove(key);
                    keys.is_empty()
                });
        if remove_trace_bucket {
            self.reusable_by_trace.remove(&key.trace_id);
        }
    }
}

pub(super) struct NetworkWakeFd {
    fd: OwnedFd,
}

impl NetworkWakeFd {
    pub(super) fn new() -> Result<Self, String> {
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(format!(
                "create network control eventfd: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    pub(super) fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    pub(super) fn notify(&self) -> std::io::Result<()> {
        let value = 1_u64.to_ne_bytes();
        let written = unsafe {
            libc::write(
                self.fd(),
                value.as_ptr().cast::<libc::c_void>(),
                value.len(),
            )
        };
        if written == value.len() as isize
            || (written < 0
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock)
        {
            return Ok(());
        }
        Err(std::io::Error::last_os_error())
    }

    pub(super) fn drain(&self) -> Result<(), ControlError> {
        loop {
            let mut value = 0_u64;
            let read = unsafe {
                libc::read(
                    self.fd(),
                    (&mut value as *mut u64).cast::<libc::c_void>(),
                    std::mem::size_of::<u64>(),
                )
            };
            if read == std::mem::size_of::<u64>() as isize {
                continue;
            }
            if read < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    return Ok(());
                }
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(ControlError::new("network_control_wake", error.to_string()));
            }
            return Err(ControlError::new(
                "network_control_wake",
                format!("eventfd returned short read {read}"),
            ));
        }
    }
}

pub(super) fn respond_deferred(
    deferred: &DeferredNotification,
    decision: EnforcementDecision,
) -> Result<(), ControlError> {
    match decision {
        EnforcementDecision::Allow => deferred.continue_now(),
        EnforcementDecision::Deny => deferred.deny_errno(libc::EPERM),
    }
}

fn verdict_decision(verdict: ControlVerdict) -> EnforcementDecision {
    match verdict {
        ControlVerdict::Allow => EnforcementDecision::Allow,
        ControlVerdict::Deny => EnforcementDecision::Deny,
    }
}

fn increment_map<K: Ord>(
    values: &mut BTreeMap<K, u32>,
    key: K,
    label: &str,
) -> Result<(), ControlError> {
    let current = values.get(&key).copied().unwrap_or(0);
    values.insert(key, checked_increment(current, label)?);
    Ok(())
}

fn decrement_map<K: Ord + Clone>(
    values: &mut BTreeMap<K, u32>,
    key: &K,
    label: &str,
) -> Result<(), ControlError> {
    match values.get(key).copied() {
        Some(value) if value > 1 => {
            values.insert(key.clone(), value - 1);
            Ok(())
        }
        Some(1) => {
            values.remove(key);
            Ok(())
        }
        _ => Err(ControlError::new(
            "network_control_accounting",
            format!("{label} underflow"),
        )),
    }
}

fn checked_increment(value: u32, label: &str) -> Result<u32, ControlError> {
    value
        .checked_add(1)
        .ok_or_else(|| ControlError::new("network_control_accounting", format!("{label} overflow")))
}

fn plugin_failure_reason(error: &plugin_system::PluginRuntimeError) -> &'static str {
    if error.code.contains("timeout") || error.message.contains("timeout") {
        "plugin_timeout"
    } else if error.code == "plugin_panic" {
        "plugin_panic"
    } else {
        "plugin_error"
    }
}
