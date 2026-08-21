//! Seccomp notification dispatch and deferred exec observation materialization.

use collector_instance::CollectorInstance;
use control_contract::reply::{ControlError, LaunchTlsPlanStatus};
use ebpf_collector::loader::DynamicTlsProbePlan;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;
use crate::services::identity::SeccompNotificationIdentityRegistrar;
use crate::services::tls_sync::ExecTlsPlanMode;

impl StorageAttachService {
    pub(super) fn drain_seccomp_notifications_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        let command_drain = self
            .command_control
            .drain_completions(trace_runtime, &self.control_plugins)?;
        for context in &command_drain.allowed_execs {
            if let Some(observation) = self.process_seccomp.deferred_exec_observation(context)? {
                self.pending_process_seccomp_observations.push(observation);
            }
        }
        self.persist_command_enforcement_outcomes(trace_runtime, command_drain.outcomes)?;
        let mut network_events = self.network_control.drain_completions(
            trace_runtime,
            &self.process_registry,
            &self.control_plugins,
        )?;
        let seccomp_notify = &mut self.seccomp_notify;
        let seccomp_tls = &mut self.seccomp_tls;
        let seccomp_socket = &mut self.seccomp_socket;
        let tls_sync = &self.tls_sync;
        let collector = &mut self.collector;
        let process_registry = &mut self.process_registry;
        let storage = self.storage.as_mut();
        let process_id_block_size = self.process_id_block_size;
        let mut enforcement_outcomes = Vec::new();
        let mut command_enforcement_outcomes = Vec::new();
        let pending_process_observations = &mut self.pending_process_seccomp_observations;
        {
            let process_seccomp = &self.process_seccomp;
            let command_control = &self.command_control;
            let network_control = &self.network_control;
            let control_plugins = &self.control_plugins;
            let enforcement = &mut self.enforcement;
            let identity_reader = &self.identity_reader;
            seccomp_notify.drain_notifications(|listener_trace_id, notification, continuation| {
                if let Some(outcome) = enforcement.handle_seccomp_notification(
                    trace_runtime,
                    process_registry,
                    identity_reader,
                    control_plugins,
                    notification,
                    continuation,
                )? {
                    enforcement_outcomes.push(outcome);
                }
                if continuation.is_finished() {
                    return Ok(());
                }
                let prepared_process = if command_control.requires_notification_identity(
                    trace_runtime,
                    listener_trace_id,
                    notification,
                ) || network_control.requires_notification_identity(
                    trace_runtime,
                    listener_trace_id,
                    notification,
                )
                {
                    SeccompNotificationIdentityRegistrar::new(
                        process_registry,
                        identity_reader,
                        storage,
                        process_id_block_size,
                    )
                    .ensure(trace_runtime, listener_trace_id, notification.pid)
                    .and_then(|preparation| {
                        if let Some(record) = preparation.inherited_record
                            && collector.active_binding_trace_count() > 0
                        {
                            collector
                                .seed_fork_bound_membership(listener_trace_id, record)
                                .map_err(|error| ControlError::new(error.stage, error.message))?;
                        }
                        Ok(preparation.resolved)
                    })
                    .map_err(|error| format!("{}: {}", error.code, error.message))
                } else {
                    Err("command control identity was not requested".to_string())
                };
                command_enforcement_outcomes.extend(command_control.handle_notification(
                    listener_trace_id,
                    trace_runtime,
                    process_registry,
                    prepared_process.clone(),
                    control_plugins,
                    notification,
                    continuation,
                )?);
                if continuation.is_finished() {
                    return Ok(());
                }
                network_events.extend(network_control.handle_notification(
                    listener_trace_id,
                    trace_runtime,
                    process_registry,
                    prepared_process,
                    notification,
                    continuation,
                    control_plugins,
                )?);
                pending_process_observations.extend(process_seccomp.handle_notification(
                    trace_runtime,
                    process_registry,
                    identity_reader,
                    notification,
                    continuation,
                    &mut |candidate| {
                        if candidate.path_truncated {
                            return Ok(());
                        }
                        let Some(path) = candidate.path.as_deref() else {
                            return Ok(());
                        };
                        let Some(host_path) = crate::services::process_seccomp::host_exec_path(
                            candidate.pid,
                            path,
                            candidate.execveat_dirfd,
                        ) else {
                            return Ok(());
                        };
                        if candidate.trace_id.is_some() {
                            match tls_sync.resolve_exec_plan(&host_path) {
                                Ok(resolution) => {
                                    let cache_hit = resolution.reply.cache_hit;
                                    let elapsed_micros = resolution.reply.resolve_elapsed_micros;
                                    match resolution.reply.status {
                                        LaunchTlsPlanStatus::Found(plans)
                                            if resolution.mode == ExecTlsPlanMode::Direct =>
                                        {
                                            for plan in plans {
                                                let provider = plan.provider.clone();
                                                let source = plan.source.clone();
                                                let dynamic_plan = DynamicTlsProbePlan {
                                                    target: plan.target,
                                                    target_identity: plan.target_identity,
                                                    binary: plan.binary,
                                                    binary_identity: plan.binary_identity,
                                                    provider: plan.provider,
                                                    points: plan.points,
                                                };
                                                match collector.attach_dynamic_tls_plan(
                                                    &dynamic_plan,
                                                ) {
                                                    Ok(()) => tracing::info!(
                                                        target: "actrail::tls_sync",
                                                        pid = candidate.pid,
                                                        binary = %host_path.display(),
                                                        provider,
                                                        source,
                                                        cache_hit,
                                                        elapsed_micros,
                                                        "attached pre-resume TLS plan for exec candidate"
                                                    ),
                                                    Err(error) => tracing::warn!(
                                                        target: "actrail::tls_sync",
                                                        pid = candidate.pid,
                                                        binary = %host_path.display(),
                                                        provider,
                                                        error = %error.message,
                                                        "failed to attach pre-resume TLS plan; continuing exec"
                                                    ),
                                                }
                                            }
                                        }
                                        LaunchTlsPlanStatus::Found(plans) => tracing::debug!(
                                            target: "actrail::tls_sync",
                                            pid = candidate.pid,
                                            binary = %host_path.display(),
                                            plan_count = plans.len(),
                                            cache_hit,
                                            elapsed_micros,
                                            "resolved pre-resume sync TLS plan for exec candidate"
                                        ),
                                        LaunchTlsPlanStatus::Unsupported { reason } => {
                                            tracing::debug!(
                                                target: "actrail::tls_sync",
                                                pid = candidate.pid,
                                                binary = %host_path.display(),
                                                reason,
                                                cache_hit,
                                                elapsed_micros,
                                                "exec candidate has no supported TLS plan"
                                            );
                                        }
                                    }
                                }
                                Err(error) => tracing::warn!(
                                    target: "actrail::tls_sync",
                                    pid = candidate.pid,
                                    binary = %host_path.display(),
                                    error_code = %error.code,
                                    error = %error.message,
                                    "failed to resolve pre-resume TLS plan; continuing exec"
                                ),
                            }
                        }
                        collector
                            .attach_dynamic_go_tls(&host_path)
                            .map_err(|error| ControlError::new(error.stage, error.message))
                    },
                )?);
                let tls_consumed = seccomp_tls.handle_notification(collector, notification)?;
                if !tls_consumed {
                    seccomp_socket.handle_notification(
                        collector,
                        trace_runtime,
                        process_registry,
                        notification,
                    )?;
                }
                Ok(())
            })?;
        }
        self.persist_enforcement_outcomes(
            trace_runtime,
            crate::services::enforcement::EnforcementDrain {
                outcomes: enforcement_outcomes,
                process_records: Vec::new(),
            },
        )?;
        self.persist_command_enforcement_outcomes(trace_runtime, command_enforcement_outcomes)?;
        self.process_live_event_batch(trace_runtime, network_events)?;
        Ok(())
    }

    pub(super) fn materialize_process_seccomp_observations_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        if self.pending_process_seccomp_observations.is_empty() {
            return Ok(());
        }
        let batch_size = self.process_seccomp.pending_observation_batch_size()?;
        while !self.pending_process_seccomp_observations.is_empty() {
            let batch_len = self
                .pending_process_seccomp_observations
                .len()
                .min(batch_size);
            let raw_events = self.pending_process_seccomp_observations[..batch_len]
                .iter()
                .map(|observation| {
                    self.process_seccomp.materialize_observation(
                        trace_runtime,
                        &self.process_registry,
                        observation,
                    )
                })
                .collect();
            self.process_live_event_batch(trace_runtime, raw_events)?;
            self.pending_process_seccomp_observations.drain(..batch_len);
        }
        let evicted_intents = self.semantic_actions.take_pending_exec_intent_evictions();
        if evicted_intents > 0 {
            tracing::warn!(
                evicted_intents,
                "semantic exec intent capacity reached; completed exec actions remain valid but may omit seccomp argument evidence"
            );
        }
        Ok(())
    }
}
