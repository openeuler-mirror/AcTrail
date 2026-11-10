//! eBPF runtime selection, preflight, and transport access.

use super::*;

impl EbpfCollector {
    pub fn file_io_poll_timeout(&self) -> Option<std::time::Duration> {
        if !self.file_io_summaries.backlog.is_empty() {
            return Some(std::time::Duration::ZERO);
        }
        if self.active_binding_trace_count() == 0 {
            return None;
        }
        self.runtime
            .as_ref()
            .and_then(EbpfRuntime::file_io_poll_timeout)
    }

    pub fn probe_result(&self) -> &EbpfProbeResult {
        &self.probe_result
    }

    pub const fn event_transport(&self) -> &'static str {
        env!("ACTRAIL_EBPF_EVENT_TRANSPORT")
    }

    pub fn event_poll_fd(&self) -> Result<Option<RawFd>, CollectorError> {
        let Some(runtime) = self.runtime.as_ref() else {
            return Ok(None);
        };
        runtime.event_poll_fd().map_err(loader_error)
    }

    pub fn lookup_pending_tls_payload_op(
        &self,
        tid: u32,
    ) -> Result<Option<PendingTlsPayloadOp>, CollectorError> {
        self.runtime_ref()?
            .lookup_pending_tls_payload_op(tid)
            .map_err(loader_error)
    }

    /// Resolve a live observer PID through the existing trace binding. Returned
    /// coordinates are the kernel TGID and its start-time generation.
    pub fn socket_capture_process(
        &self,
        observer_pid: u32,
        trace_id: TraceId,
    ) -> Option<(u32, u64)> {
        let process = self.bindings.by_host_pid(observer_pid)?;
        (process.trace_id == trace_id && process.kernel_start_time != 0)
            .then_some((process.kernel_tgid, process.kernel_start_time))
    }

    /// `kernel_pid` indexes the BPF socket map, unlike observer `/proc` PIDs.
    pub fn lookup_socket_fd_state(
        &self,
        kernel_pid: u32,
        fd: u32,
    ) -> Result<Option<SocketPayloadFdState>, CollectorError> {
        self.runtime_ref()?
            .lookup_socket_fd_state(kernel_pid, fd)
            .map_err(loader_error)
    }

    pub fn mark_socket_fd_tls_owned(
        &self,
        kernel_pid: u32,
        fd: u32,
        expected_socket_generation: u32,
    ) -> Result<bool, CollectorError> {
        self.runtime_ref()?
            .mark_socket_fd_tls_owned(kernel_pid, fd, expected_socket_generation)
            .map_err(loader_error)
    }

    pub fn take_tls_completions(&mut self) -> Vec<TlsPayloadCompletion> {
        std::mem::take(&mut self.tls_completions)
    }

    pub fn take_tls_capture_requests(&mut self) -> Vec<TlsPayloadCaptureRequest> {
        std::mem::take(&mut self.tls_capture_requests)
    }

    pub fn take_tls_direct_captures(&mut self) -> Vec<TlsPayloadDirectCapture> {
        std::mem::take(&mut self.tls_direct_captures)
    }

    pub fn take_tls_mapping_events(&mut self) -> Vec<crate::loader::KernelTlsMappingEvent> {
        std::mem::take(&mut self.tls_mapping_events)
    }

    pub fn take_tls_diagnostic_events(&mut self) -> Vec<TlsDiagnosticEvent> {
        std::mem::take(&mut self.tls_diagnostic_events)
    }

    pub fn take_launch_binding_failures(&mut self) -> Vec<LaunchBindingFailure> {
        std::mem::take(&mut self.launch_binding_failures)
    }

    pub fn take_socket_completions(&mut self) -> Vec<SocketPayloadCompletion> {
        std::mem::take(&mut self.socket_completions)
    }

    pub fn tls_payload_diagnostics(&self) -> Result<Option<TlsPayloadDiagnostics>, CollectorError> {
        let Some(runtime) = self.runtime.as_ref() else {
            return Ok(None);
        };
        runtime.tls_payload_diagnostics().map_err(loader_error)
    }

    pub fn event_transport_loss_poll_timeout(&self) -> Option<std::time::Duration> {
        self.runtime
            .as_ref()
            .and_then(EbpfRuntime::event_transport_loss_poll_timeout)
    }

    /// The bool reports newly observed loss immediately; the optional summary
    /// is emitted only at the configured interval (or during final shutdown).
    pub fn take_event_transport_loss_summary(&mut self, force: bool) -> (bool, Option<String>) {
        let Some(runtime) = self.runtime.as_mut() else {
            return (false, None);
        };
        for (category, count) in self.stdio_payloads.take_loss_counts() {
            runtime.record_event_transport_loss_count(category, count);
        }
        runtime.take_event_transport_loss_summary(force)
    }

    pub fn flush_transport(&mut self) -> Result<(), CollectorError> {
        self.runtime
            .as_mut()
            .map(EbpfRuntime::flush_transport)
            .transpose()
            .map(|_| ())
            .map_err(loader_error)
    }

    pub fn debug_snapshot_for_pid(
        &self,
        pid: u32,
    ) -> Result<EbpfCollectorDebugSnapshot, CollectorError> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| CollectorError::new("runtime", "eBPF runtime was not initialized"))?;
        Ok(EbpfCollectorDebugSnapshot {
            active_binding_traces: self.active_binding_trace_count(),
            attached_programs: runtime.attached_programs().to_vec(),
            last_raw_sample_count: runtime.last_raw_sample_count(),
            tracked_trace_id: runtime
                .tracked_trace_id(
                    self.bindings
                        .by_host_pid(pid)
                        .map(|tracked| tracked.kernel_tgid)
                        .unwrap_or(pid),
                )
                .map_err(loader_error)?,
        })
    }

    pub(super) fn active_binding_trace_count(&self) -> usize {
        self.bindings.trace_count() + self.pending_launches.len()
    }

    pub(super) fn runtime_ref(&self) -> Result<&EbpfRuntime, CollectorError> {
        self.runtime
            .as_ref()
            .ok_or_else(|| CollectorError::new("runtime", "eBPF runtime was not initialized"))
    }

    pub(super) fn runtime_mut(&mut self) -> Result<&mut EbpfRuntime, CollectorError> {
        self.runtime
            .as_mut()
            .ok_or_else(|| CollectorError::new("runtime", "eBPF runtime was not initialized"))
    }
}
