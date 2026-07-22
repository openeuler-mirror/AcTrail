//! Kernel ring event draining for the eBPF collector.

use collector_instance::{CollectorError, CollectorPollBatch};
use model_core::ids::TraceId;

use crate::decode::{
    self, decode_file_path, decode_observation, decode_socket_payload,
    decode_socket_payload_completion, decode_stdio_payload, decode_tls_capture_request,
    decode_tls_completion, decode_tls_diagnostic, decode_tls_direct_capture,
};
use crate::loader::{KernelEvent, KernelObservationEvent};

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
                    map_pid: event.pid,
                    generation: event.pid_generation,
                })
            }
            _ => None,
        }
    }
}

impl EbpfCollector {
    pub fn poll_tls_payload_control_events(&mut self) -> Result<(), CollectorError> {
        let Some(runtime) = self.runtime.as_mut() else {
            return Ok(());
        };
        for event in runtime.poll_events().map_err(loader_error)? {
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
        }
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
                self.maybe_attach_go_tls_after_exec(&event)?;
                let lifecycle_event = event.clone();
                self.apply_file_lifecycle_before_decode(&lifecycle_event)?;
                if let Some(event) =
                    decode_observation(event, &mut self.bindings, &mut self.file_tracker)
                        .map_err(|error| CollectorError::new(error.stage, error.message))?
                {
                    batch.observations.push(event);
                }
                self.apply_file_lifecycle_after_decode(&lifecycle_event)?;
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
            }
            KernelEvent::StdioPayload(event) => {
                batch.payload_segments.push(
                    decode_stdio_payload(event, &self.bindings)
                        .map_err(|error| CollectorError::new(error.stage, error.message))?,
                );
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

    fn apply_file_lifecycle_before_decode(
        &mut self,
        event: &KernelObservationEvent,
    ) -> Result<(), CollectorError> {
        match event.kind {
            decode::PROC_EVENT_EXIT => {
                let map_pid = event.pid;
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
                let map_pid = event.pid;
                let process = decode::resolve_bound_event_observation(
                    event.trace_id,
                    map_pid,
                    event.pid_generation,
                    &self.bindings,
                )
                .map_err(|error| CollectorError::new("file_lifecycle_exec", error))?;
                self.file_tracker.exec_process(event.trace_id, process);
                self.configure_file_bulk_read_fast_process(event)?;
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
        runtime
            .unmark_file_bulk_read_fast_process(event.pid, event.pid_generation)
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
            .mark_file_bulk_read_fast_process(event.pid, event.pid_generation, event.trace_id)
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
