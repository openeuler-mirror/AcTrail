//! Trace shutdown draining before root removal and collector unbind.

use std::time::SystemTime;

use collector_instance::CollectorInstance;
use config_core::daemon::DiagnosticLogLevel;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use model_core::trace::TraceLifecycleState;
use trace_runtime::commands::RootRemovalRequest;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::SqliteAttachService;

impl SqliteAttachService {
    fn drain_trace_shutdown_events_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        trace_id: TraceId,
    ) -> Result<(), ControlError> {
        self.drain_tls_sync_events_impl(trace_runtime)?;
        self.drain_seccomp_notifications_impl(trace_runtime)?;
        if self.trace_uses_ebpf_collector(trace_runtime, trace_id)? {
            if !self.collector_ready() {
                return Err(ControlError::new(
                    "track_remove",
                    "cannot final-drain trace because the eBPF collector is not ready",
                ));
            }
            if self.collector.stats().active_bindings == 0 {
                return Err(ControlError::new(
                    "track_remove",
                    "cannot final-drain trace because no eBPF bindings are active",
                ));
            }
            let batch = self
                .collector
                .poll_batch()
                .map_err(|error| ControlError::new(error.stage, error.message))?;
            self.log_tls_diagnostic_events_impl();
            self.process_live_event_batch(trace_runtime, batch.observations)?;
            self.process_payload_segments_impl(trace_runtime, batch.payload_segments)?;
        } else {
            self.collector
                .poll_tls_payload_control_events()
                .map_err(|error| ControlError::new(error.stage, error.message))?;
            self.log_tls_diagnostic_events_impl();
        }
        self.ingest_polled_seccomp_tls_controls_impl()?;
        self.drain_seccomp_notifications_impl(trace_runtime)?;
        self.flush_process_seccomp_observations_impl(trace_runtime)?;
        self.persist_completed_seccomp_tls_operations_impl(trace_runtime)?;
        self.persist_completed_seccomp_socket_operations_impl(trace_runtime)
    }

    fn trace_uses_ebpf_collector(
        &self,
        trace_runtime: &TraceRuntime,
        trace_id: TraceId,
    ) -> Result<bool, ControlError> {
        let collector_name = self.collector.descriptor().name.clone();
        let entry = trace_runtime
            .get_trace(trace_id)
            .ok_or_else(|| ControlError::new("track_remove", "trace not found"))?;
        Ok(entry
            .sensor_plan
            .collectors
            .iter()
            .any(|collector| collector.collector_name == collector_name))
    }

    pub(in crate::services) fn remove_root_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        trace_id: TraceId,
        removed_at: SystemTime,
    ) -> Result<(), ControlError> {
        let root_identity = trace_runtime
            .get_trace(trace_id)
            .map(|entry| entry.trace.root_process_identity.clone())
            .ok_or_else(|| ControlError::new("track_remove", "trace not found"))?;

        // Drain trace-bearing sources before disabling root capture. Descendant
        // processes stay tracked while the trace is draining, so the normal
        // live loop gets the first chance to consume their lifecycle exit events.
        self.drain_trace_shutdown_events_impl(trace_runtime, trace_id)?;
        trace_runtime
            .track_remove_root(RootRemovalRequest {
                trace_id,
                removed_at,
            })
            .map_err(|error| ControlError::new("track_remove", format!("{:?}", error)))?;
        self.collector
            .stop_tracking_process(root_identity.pid)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        self.persist_trace_state(trace_runtime, trace_id)?;
        if trace_runtime.get_trace(trace_id).is_some_and(|entry| {
            matches!(
                entry.trace.lifecycle_state,
                TraceLifecycleState::Completed | TraceLifecycleState::Failed
            )
        }) {
            let semantic_actions = self.finalize_semantic_actions_for_trace(trace_id, removed_at);
            self.write_semantic_action_batch(&semantic_actions)?;
            self.publish_live_otel_action_batch(trace_runtime, &semantic_actions)?;
            self.application_protocol.forget_trace(trace_id);
            self.semantic_actions.forget_trace(trace_id);
        }
        self.log_diagnostic(
            DiagnosticLogLevel::Info,
            format_args!(
                "agent_launch closed trace_id={} pid={} generation={}",
                trace_id, root_identity.pid, root_identity.generation
            ),
        );
        Ok(())
    }
}
