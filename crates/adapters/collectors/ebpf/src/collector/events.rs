//! Kernel ring event draining for the eBPF collector.

use std::time::SystemTime;

use collector_event::{RawCollectorEvent, RawObservationPayload};
use collector_instance::{CollectorError, CollectorPollBatch};
use model_core::ids::TraceId;
use model_core::process::{KernelProcessCoordinates, ProcessSuppressedFd};

use crate::decode::{
    self, decode_file_path, decode_observation, decode_socket_payload,
    decode_socket_payload_completion, decode_tls_capture_request, decode_tls_completion,
    decode_tls_diagnostic, decode_tls_direct_capture,
};
use crate::loader::{KernelEvent, KernelObservationEvent};

use super::collector_net_aggregation::ObserveOutcome;
use super::{EbpfCollector, loader_error};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExitRetire {
    trace_id: TraceId,
    map_pid: u32,
    generation: u64,
}

impl ExitRetire {
    fn from_event(event: &KernelEvent) -> Option<Self> {
        match event {
            KernelEvent::Observation(event) if event.kind == decode::PROC_EVENT_EXIT => {
                Some(Self {
                    trace_id: event.trace_id,
                    map_pid: decode::event_map_pid(event.pid, event.host_pid),
                    generation: event.pid_generation,
                })
            }
            _ => None,
        }
    }
}

/// Extract the decoded part of a close/shutdown identity. The kernel-only FD
/// object generation is carried separately from the raw lifecycle event.
fn net_flush_identity(
    event: &RawCollectorEvent,
) -> Option<(&str, &Option<String>, &Option<String>, Option<&str>)> {
    let RawObservationPayload::Net {
        transport,
        local,
        remote,
        result,
        metadata,
        ..
    } = &event.payload
    else {
        return None;
    };
    let operation = metadata.get("operation")?;
    ((operation == "close" || operation == "shutdown") && *result == Some(0)).then_some((
        transport,
        local,
        remote,
        metadata.get("fd").map(String::as_str),
    ))
}

impl EbpfCollector {
    pub fn poll_tls_payload_control_events(&mut self) -> Result<(), CollectorError> {
        let Some(runtime) = self.runtime.as_mut() else {
            return Ok(());
        };
        for event in runtime.poll_events().map_err(loader_error)? {
            if let KernelEvent::Observation(observation) = &event {
                self.promote_pending_launch_after_exec(observation)?;
            }
            self.handle_control_event(event);
        }
        Ok(())
    }

    pub(super) fn poll_batch_impl(&mut self) -> Result<CollectorPollBatch, CollectorError> {
        let Some(runtime) = self.runtime.as_mut() else {
            return Ok(CollectorPollBatch {
                observations: Vec::new(),
                payload_segments: Vec::new(),
            });
        };
        let raw_events = runtime.poll_events().map_err(loader_error)?;
        let mut batch = CollectorPollBatch {
            observations: Vec::new(),
            payload_segments: Vec::new(),
        };
        batch.observations.append(&mut self.net_aggregation_backlog);
        let mut exit_retires = Vec::new();
        for event in raw_events {
            let exit_retire = ExitRetire::from_event(&event);
            self.handle_batch_event(event, &mut batch)?;
            if let Some(exit_retire) = exit_retire {
                exit_retires.push(exit_retire);
            }
        }
        for exit_retire in exit_retires {
            let _ = self.bindings.remove_event_pid(
                exit_retire.trace_id,
                exit_retire.map_pid,
                exit_retire.generation,
            );
            batch
                .observations
                .extend(self.net_aggregator.flush_trace(exit_retire.trace_id));
        }
        batch
            .observations
            .extend(self.net_aggregator.drain_timeout(SystemTime::now()));
        Ok(batch)
    }

    fn handle_control_event(&mut self, event: KernelEvent) {
        match event {
            KernelEvent::TlsCompletion(event) => {
                self.tls_completions.push(decode_tls_completion(event));
            }
            KernelEvent::TlsCaptureRequest(event) => {
                self.tls_capture_requests
                    .push(decode_tls_capture_request(event));
            }
            KernelEvent::TlsDirectCapture(event) => {
                self.tls_direct_captures
                    .push(decode_tls_direct_capture(event));
            }
            KernelEvent::TlsDiagnostic(event) => {
                self.tls_diagnostic_events
                    .push(decode_tls_diagnostic(event));
            }
            KernelEvent::LaunchBindingFailure(event) => {
                self.launch_binding_failures.push(event);
            }
            KernelEvent::SocketPayloadCompletion(event) => {
                self.socket_completions
                    .push(decode_socket_payload_completion(event));
            }
            _ => {}
        }
    }

    fn handle_batch_event(
        &mut self,
        event: KernelEvent,
        batch: &mut CollectorPollBatch,
    ) -> Result<(), CollectorError> {
        match event {
            KernelEvent::Observation(event) => {
                self.promote_pending_launch_after_exec(&event)?;
                if self.is_superseded_fork_event(&event) {
                    return Ok(());
                }
                self.maybe_attach_go_tls_after_exec(&event)?;
                let lifecycle_event = event.clone();
                self.apply_file_lifecycle_before_decode(&lifecycle_event)?;
                let descriptor_generation = lifecycle_event.aux_generation;
                if let Some(event) =
                    decode_observation(event, &mut self.bindings, &mut self.file_tracker)
                        .map_err(|error| CollectorError::new(error.stage, error.message))?
                {
                    if let Some((transport, local, remote, fd)) = net_flush_identity(&event) {
                        batch
                            .observations
                            .extend(self.net_aggregator.flush_connection(
                                &event.envelope,
                                transport,
                                local,
                                remote,
                                descriptor_generation,
                                fd,
                            ));
                    }
                    match self.net_aggregator.observe(event, descriptor_generation) {
                        ObserveOutcome::PassThrough(event) => batch.observations.push(event),
                        ObserveOutcome::Buffered => {}
                        ObserveOutcome::Flushed(event) => batch.observations.push(event),
                    }
                }
                self.apply_file_lifecycle_after_decode(&lifecycle_event)?;
                batch
                    .observations
                    .extend(self.file_tracker.take_stdio_bundle_events());
            }
            KernelEvent::FilePath(event) => {
                match decode_file_path(event, &self.bindings, &mut self.file_tracker) {
                    Ok(Some(event)) => batch.observations.push(event),
                    Ok(None) => {}
                    Err(error) if error.stage == "file_identity" => {
                        self.record_file_binding_gap_drop(&error.message);
                    }
                    Err(error) => return Err(CollectorError::new(error.stage, error.message)),
                }
                batch
                    .observations
                    .extend(self.file_tracker.take_stdio_bundle_events());
            }
            KernelEvent::StdioPayload(event) => {
                if let Some(segment) = self.stdio_payloads.observe_payload(event, &self.bindings)? {
                    batch.payload_segments.push(segment);
                }
            }
            KernelEvent::StdioPayloadCompletion(event) => {
                if let Some(segment) = self
                    .stdio_payloads
                    .observe_completion(event, &self.bindings)?
                {
                    batch.payload_segments.push(segment);
                }
            }
            KernelEvent::SocketPayload(event) => {
                batch.payload_segments.push(
                    decode_socket_payload(event, &self.bindings)
                        .map_err(|error| CollectorError::new(error.stage, error.message))?,
                );
            }
            other => self.handle_control_event(other),
        }
        Ok(())
    }

    fn promote_pending_launch_after_exec(
        &mut self,
        event: &KernelObservationEvent,
    ) -> Result<(), CollectorError> {
        if event.kind != decode::PROC_EVENT_EXEC {
            return Ok(());
        }
        let Some(pending) = self.pending_launches.get(&event.trace_id) else {
            return Ok(());
        };
        if event.host_pid == 0 {
            return Err(CollectorError::new(
                "pending_launch_exec",
                format!(
                    "trace {} exec promotion did not report kernel PID K",
                    event.trace_id
                ),
            ));
        }
        if event.pid_generation == 0 || event.pid_generation != pending.generation {
            return Err(CollectorError::new(
                "pending_launch_exec",
                format!(
                    "trace {} exec generation {} does not match armed generation {}",
                    event.trace_id, event.pid_generation, pending.generation
                ),
            ));
        }

        let root_observation = pending.root_observation.clone();
        let root_working_directory = pending.root_working_directory.clone();
        let initial_suppressed_fds = pending.initial_suppressed_fds.clone();
        self.bindings.track_with_map_pid(
            event.trace_id,
            root_observation.clone(),
            event.host_pid,
            event.pid_generation,
        );
        self.file_tracker
            .seed_process(event.trace_id, root_observation, root_working_directory);
        let process = KernelProcessCoordinates {
            pid: event.host_pid,
            start_time: event.pid_generation,
        };
        self.suppressed_fds
            .extend(
                initial_suppressed_fds
                    .into_iter()
                    .map(|initial| super::TraceSuppressedFd {
                        trace_id: event.trace_id,
                        fd: ProcessSuppressedFd {
                            process,
                            fd: initial.fd,
                            purpose: initial.purpose,
                        },
                    }),
            );
        self.pending_launches.remove(&event.trace_id);
        Ok(())
    }

    fn is_superseded_fork_event(&self, event: &KernelObservationEvent) -> bool {
        event.kind == decode::PROC_EVENT_FORK
            && event.aux_host_pid != 0
            && self
                .bindings
                .by_host_pid(event.aux_host_pid)
                .is_some_and(|binding| binding.trace_id != event.trace_id)
    }

    fn apply_file_lifecycle_before_decode(
        &mut self,
        event: &KernelObservationEvent,
    ) -> Result<(), CollectorError> {
        match event.kind {
            decode::PROC_EVENT_EXIT => {
                self.stdio_payloads.release_process(
                    event.trace_id,
                    event.host_pid,
                    event.pid_generation,
                );
                let map_pid = decode::event_map_pid(event.pid, event.host_pid);
                if decode::resolve_bound_event_observation(
                    event.trace_id,
                    map_pid,
                    event.pid_generation,
                    &self.bindings,
                )
                .is_ok()
                {
                    self.cleanup_suppressed_fds_for_process(map_pid, event.pid_generation)?;
                    if let Some(runtime) = self.runtime.as_ref() {
                        runtime
                            .unmark_file_bulk_read_fast_process(map_pid, event.pid_generation)
                            .map_err(loader_error)?;
                        runtime
                            .sweep_file_bulk_read_fast_fds_for_process(
                                map_pid,
                                event.pid_generation,
                            )
                            .map_err(loader_error)?;
                    }
                } else {
                    self.record_exit_lifecycle_binding_gap();
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn apply_file_lifecycle_after_decode(
        &mut self,
        event: &KernelObservationEvent,
    ) -> Result<(), CollectorError> {
        match event.kind {
            decode::PROC_EVENT_FORK => {
                let parent = decode::fork_parent_observation(event, &self.bindings)
                    .map_err(|error| CollectorError::new(error.stage, error.message))?;
                let child = decode::fork_child_observation(event, &self.bindings)
                    .map_err(|error| CollectorError::new(error.stage, error.message))?;
                self.file_tracker
                    .inherit_process(event.trace_id, &parent, child);
            }
            decode::PROC_EVENT_EXEC => {
                let map_pid = decode::event_map_pid(event.pid, event.host_pid);
                let process = decode::resolve_bound_event_observation(
                    event.trace_id,
                    map_pid,
                    event.pid_generation,
                    &self.bindings,
                )
                .map_err(|error| CollectorError::new("file_lifecycle_exec", error))?;
                self.file_tracker
                    .exec_process(event.trace_id, process, event.observed_ktime_ns);
                self.configure_file_bulk_read_fast_process(event)?;
            }
            decode::PROC_EVENT_EXIT => {
                let map_pid = event.pid;
                let process = decode::resolve_bound_event_observation(
                    event.trace_id,
                    map_pid,
                    event.pid_generation,
                    &self.bindings,
                )
                .map_err(|error| CollectorError::new("file_lifecycle_exit", error))?;
                self.file_tracker
                    .exit_process(event.trace_id, process, event.observed_ktime_ns);
            }
            _ => {}
        }
        Ok(())
    }

    fn configure_file_bulk_read_fast_process(
        &mut self,
        event: &KernelObservationEvent,
    ) -> Result<(), CollectorError> {
        let Some(runtime) = self.runtime.as_ref() else {
            return Ok(());
        };
        let map_pid = decode::event_map_pid(event.pid, event.host_pid);
        runtime
            .unmark_file_bulk_read_fast_process(map_pid, event.pid_generation)
            .map_err(loader_error)?;
        if !self.file_bulk_read_fast_path.enabled {
            return Ok(());
        }
        let Some(exec_filename) = &event.exec_filename else {
            return Ok(());
        };
        if exec_filename.truncated {
            return Ok(());
        }
        let Some(command) = std::path::Path::new(&exec_filename.path)
            .file_name()
            .and_then(|value| value.to_str())
        else {
            return Ok(());
        };
        if !self
            .file_bulk_read_fast_path
            .scanner_commands
            .iter()
            .any(|candidate| candidate == command)
        {
            return Ok(());
        }
        runtime
            .mark_file_bulk_read_fast_process(map_pid, event.pid_generation, event.trace_id)
            .map_err(loader_error)
    }

    fn maybe_attach_go_tls_after_exec(
        &mut self,
        event: &KernelObservationEvent,
    ) -> Result<(), CollectorError> {
        if event.kind != decode::PROC_EVENT_EXEC {
            return Ok(());
        }
        let Some(exec_filename) = &event.exec_filename else {
            return Ok(());
        };
        if exec_filename.truncated {
            return Ok(());
        }
        self.attach_dynamic_go_tls(std::path::Path::new(&exec_filename.path))
    }

    fn record_file_binding_gap_drop(&mut self, _detail: &str) {
        self.binding_gap_drops = self.binding_gap_drops.saturating_add(1);
    }

    fn record_exit_lifecycle_binding_gap(&mut self) {
        self.binding_gap_lifecycle_skips = self.binding_gap_lifecycle_skips.saturating_add(1);
    }
}
