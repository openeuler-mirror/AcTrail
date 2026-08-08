//! Short-lived gray-decision workers, wakeup signaling, and cache records.

use std::collections::BTreeMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, TryRecvError};
use std::time::Instant;

use config_core::daemon::EnforcementDecision;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use plugin_system::{
    COMMAND_EXECUTION_CURRENT_CONTEXT_TOKEN, ControlDecisionBudget, ControlDecisionRequest,
    ControlSubject, ControlVerdict, DecisionScope,
};
use trace_runtime::registry::TraceRuntime;

use super::audit::{CommandAuditBuilder, CommandControlDrain, CommandDecisionSource};
use super::decision::ExecNotificationContext;
use super::rules::StoredCommandRule;
use super::service::{CommandControlBackend, CommandControlService};
use crate::services::control_runtime::ControlPluginRuntime;
use crate::services::seccomp_notify::DeferredNotification;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ReusableDecisionKey {
    pub(super) trace_id: TraceId,
    pub(super) process_generation: u64,
    pub(super) rule_id: String,
    pub(super) rule_revision: u64,
    pub(super) resolved_path: PathBuf,
    pub(super) argv_digest: String,
}

#[derive(Clone, Debug)]
pub(super) struct CachedCommandDecision {
    pub(super) decision: EnforcementDecision,
    pub(super) instance_id: String,
    pub(super) reason: Option<String>,
    pub(super) inserted_sequence: u64,
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
    pub(super) deferred: DeferredNotification,
    pub(super) context: ExecNotificationContext,
    pub(super) rule: StoredCommandRule,
    pub(super) outcome: PluginDecisionOutcome,
    pub(super) latency_us: u64,
    pub(super) target_instance_index: u64,
}

pub(super) struct GrayDecisionWorker {
    plugins: ControlPluginRuntime,
    completion_sender: SyncSender<PluginDecisionCompletion>,
    wake: Arc<CommandWakeFd>,
    timeout_ms: u64,
    fallback: EnforcementDecision,
    target_instance_index: u64,
}

impl GrayDecisionWorker {
    pub(super) fn new(
        plugins: ControlPluginRuntime,
        completion_sender: SyncSender<PluginDecisionCompletion>,
        wake: Arc<CommandWakeFd>,
        timeout_ms: u64,
        fallback: EnforcementDecision,
        target_instance_index: u64,
    ) -> Self {
        Self {
            plugins,
            completion_sender,
            wake,
            timeout_ms,
            fallback,
            target_instance_index,
        }
    }

    pub(super) fn spawn(
        self,
        deferred: DeferredNotification,
        context: ExecNotificationContext,
        rule: StoredCommandRule,
    ) -> Result<(), String> {
        let thread_name = format!("command-gray-{}", rule.rule_id);
        std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || self.run(deferred, context, rule))
            .map(|_| ())
            .map_err(|error| format!("spawn command gray decision worker: {error}"))
    }

    fn run(
        self,
        deferred: DeferredNotification,
        context: ExecNotificationContext,
        rule: StoredCommandRule,
    ) {
        let started_at = Instant::now();
        let target = rule
            .gray_target
            .as_deref()
            .expect("gray stored rule has target");
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
                    subject: ControlSubject::CommandExecution,
                    actor_process_identity: context.actor(),
                    operation: context.syscall().as_str().to_string(),
                    target_summary: context.resolved_path().display().to_string(),
                    context_ref: Some(COMMAND_EXECUTION_CURRENT_CONTEXT_TOKEN.to_string()),
                    file_policy_context: None,
                    command_execution_context: Some(context.command_execution_context()),
                },
                ControlDecisionBudget {
                    timeout_ms: Some(self.timeout_ms),
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
                    decision: self.fallback,
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
                "command gray completion could not wake daemon event loop"
            );
        }
    }
}

impl CommandControlService {
    pub(in crate::services) fn drain_completions(
        &self,
        trace_runtime: &TraceRuntime,
        plugins: &ControlPluginRuntime,
    ) -> Result<CommandControlDrain, ControlError> {
        let Some(backend_ref) = &self.backend else {
            return Ok(CommandControlDrain::empty());
        };
        let mut backend = backend_ref.lock().map_err(|error| {
            ControlError::new("command_control_policy", format!("lock backend: {error}"))
        })?;
        backend.wake.drain()?;
        let mut drain = CommandControlDrain::empty();
        loop {
            let completion = match backend.completion_receiver.try_recv() {
                Ok(completion) => completion,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(ControlError::new(
                        "command_control_worker",
                        "command gray completion channel disconnected",
                    ));
                }
            };
            let target = completion
                .rule
                .gray_target
                .as_deref()
                .expect("gray completion has target")
                .to_string();
            backend.release(&completion.rule, &target)?;
            if completion.deferred.trace_id() != completion.context.trace_id() {
                completion.deferred.deny_errno(libc::EPERM)?;
                return Err(ControlError::new(
                    "command_control_worker",
                    "deferred notification trace does not match captured command context",
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
                    let source = CommandDecisionSource::GrayPlugin {
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
                    CommandDecisionSource::GrayFallback {
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
                    CommandDecisionSource::GrayFallback {
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
            {
                backend.cache_decision(
                    &completion.context,
                    &completion.rule,
                    decision,
                    instance_id,
                    reason,
                )?;
            }
            if decision == EnforcementDecision::Allow {
                drain.allowed_execs.push(completion.context.clone());
            }
            if backend.config.audit_enabled {
                drain.outcomes.push(
                    CommandAuditBuilder::new(
                        &completion.context,
                        decision,
                        Some(&completion.rule),
                        source,
                        completion.latency_us,
                    )
                    .build(),
                );
            }
        }
        backend.reusable_decisions.retain(|key, _| {
            trace_runtime
                .get_trace(key.trace_id)
                .is_some_and(|entry| !entry.trace.lifecycle_state.is_terminal())
        });
        Ok(drain)
    }
}

impl CommandControlBackend {
    pub(super) fn admission_rejection(
        &self,
        rule: &StoredCommandRule,
        target: &str,
        instance_limit: Option<u32>,
    ) -> Option<&'static str> {
        if self.global_pending >= self.config.pending_decision_max {
            return Some("global_pending_limit");
        }
        let rule_key = (rule.owner_instance_id.clone(), rule.rule_id.clone());
        if self.in_flight_by_rule.get(&rule_key).copied().unwrap_or(0)
            >= self.config.gray.concurrency_limit
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
        rule: &StoredCommandRule,
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
        rule: &StoredCommandRule,
        target: &str,
    ) -> Result<(), ControlError> {
        self.global_pending = self.global_pending.checked_sub(1).ok_or_else(|| {
            ControlError::new("command_control_accounting", "global pending underflow")
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
        context: &ExecNotificationContext,
        rule: &StoredCommandRule,
        decision: EnforcementDecision,
        instance_id: String,
        reason: Option<String>,
    ) -> Result<(), ControlError> {
        self.next_cache_sequence = self
            .next_cache_sequence
            .checked_add(1)
            .ok_or_else(|| ControlError::new("command_control_cache", "cache sequence overflow"))?;
        let key = ReusableDecisionKey {
            trace_id: context.trace_id(),
            process_generation: context.process_generation(),
            rule_id: rule.rule_id.clone(),
            rule_revision: rule.rule_revision,
            resolved_path: context.resolved_path().to_path_buf(),
            argv_digest: context
                .argv_digest()
                .expect("gray completion context has argv digest")
                .to_string(),
        };
        self.reusable_decisions.insert(
            key,
            CachedCommandDecision {
                decision,
                instance_id,
                reason,
                inserted_sequence: self.next_cache_sequence,
            },
        );
        while self.reusable_decisions.len() > self.config.reusable_cache_max_entries as usize {
            let oldest = self
                .reusable_decisions
                .iter()
                .min_by_key(|(_, value)| value.inserted_sequence)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.reusable_decisions.remove(&oldest);
            }
        }
        Ok(())
    }
}

pub(super) struct CommandWakeFd {
    fd: OwnedFd,
}

impl CommandWakeFd {
    pub(super) fn new() -> Result<Self, String> {
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(format!(
                "create command control eventfd: {}",
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
                return Err(ControlError::new("command_control_wake", error.to_string()));
            }
            return Err(ControlError::new(
                "command_control_wake",
                format!("eventfd returned short read {read}"),
            ));
        }
    }
}

fn verdict_decision(verdict: ControlVerdict) -> EnforcementDecision {
    match verdict {
        ControlVerdict::Allow => EnforcementDecision::Allow,
        ControlVerdict::Deny => EnforcementDecision::Deny,
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
            "command_control_accounting",
            format!("{label} underflow"),
        )),
    }
}

fn checked_increment(value: u32, label: &str) -> Result<u32, ControlError> {
    value
        .checked_add(1)
        .ok_or_else(|| ControlError::new("command_control_accounting", format!("{label} overflow")))
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
