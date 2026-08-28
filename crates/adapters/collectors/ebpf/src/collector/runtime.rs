//! eBPF runtime selection, preflight, and transport access.

use super::*;

impl EbpfCollector {
    pub fn probe_result(&self) -> &EbpfProbeResult {
        &self.probe_result
    }

    pub const fn event_transport(&self) -> &'static str {
        env!("ACTRAIL_EBPF_EVENT_TRANSPORT")
    }

    pub fn preflight_key(&self, requests: &[CapabilityRequest]) -> EbpfPreflightKey {
        EbpfPreflightKey(
            self.effective_preflight_requests(requests)
                .into_iter()
                .map(|request| (request.capability, request.mode))
                .collect(),
        )
    }

    pub fn preflight_capability_requests(
        &mut self,
        requests: &[CapabilityRequest],
    ) -> Result<(), CollectorError> {
        if let Some(reason) = &self.probe_result.reason_unavailable {
            return Err(CollectorError::new("ebpf_preflight", reason.clone()));
        }

        let requests = self.effective_preflight_requests(requests);
        if requests.is_empty() {
            return Err(CollectorError::new(
                "ebpf_preflight",
                "capture profile requests host eBPF observation, but no requested capability is exposed by the eBPF collector descriptor",
            ));
        }
        if let Some(unsupported_required) = requests.iter().find(|request| {
            !supported_required_capability(
                &request.capability,
                self.loader.config(),
                self.loader.payload_config(),
            ) && request.mode == RequestMode::Required
        }) {
            return Err(CollectorError::new(
                "ebpf_preflight",
                format!(
                    "current libbpf-rs collector path does not support required capability {:?}",
                    unsupported_required.capability
                ),
            ));
        }

        let attach_plan = AttachPlan::from_requests(
            &requests,
            self.loader.config(),
            self.loader.payload_config(),
        );
        self.runtime = None;
        let mut runtime = self
            .loader
            .load_runtime_with_plan(&attach_plan)
            .map_err(loader_error)?;
        if let Some(missing) = requests.iter().find(|request| {
            request.mode == RequestMode::Required
                && !runtime
                    .attached_capabilities()
                    .contains(&request.capability)
        }) {
            return Err(CollectorError::new(
                "ebpf_preflight",
                format!(
                    "preflight loaded eBPF runtime without required capability {:?}",
                    missing.capability
                ),
            ));
        }
        runtime.park_for_first_binding().map_err(loader_error)?;
        self.runtime = Some(runtime);
        Ok(())
    }

    fn effective_preflight_requests(
        &self,
        requests: &[CapabilityRequest],
    ) -> Vec<CapabilityRequest> {
        let mut effective = BTreeMap::<Capability, RequestMode>::new();
        for request in requests.iter().filter(|request| {
            request.mode != RequestMode::Disabled
                && self
                    .probe_result
                    .descriptor
                    .capabilities
                    .iter()
                    .any(|descriptor| descriptor.capability == request.capability)
        }) {
            effective
                .entry(request.capability.clone())
                .and_modify(|mode| {
                    if request.mode == RequestMode::Required {
                        *mode = RequestMode::Required;
                    }
                })
                .or_insert(request.mode);
        }
        effective
            .into_iter()
            .map(|(capability, mode)| CapabilityRequest { capability, mode })
            .collect()
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

    pub fn lookup_socket_fd_state(
        &self,
        pid: u32,
        fd: u32,
    ) -> Result<Option<SocketPayloadFdState>, CollectorError> {
        self.runtime_ref()?
            .lookup_socket_fd_state(pid, fd)
            .map_err(loader_error)
    }

    pub fn mark_socket_fd_tls_owned(
        &self,
        pid: u32,
        fd: u32,
        expected_generation: u32,
    ) -> Result<bool, CollectorError> {
        self.runtime_ref()?
            .mark_socket_fd_tls_owned(pid, fd, expected_generation)
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

    pub fn take_event_transport_loss_summaries(&mut self) -> Vec<String> {
        let mut summaries = self
            .runtime
            .as_mut()
            .map(EbpfRuntime::take_event_transport_loss_summaries)
            .unwrap_or_default();
        summaries.extend(self.stdio_payloads.take_loss_summaries());
        summaries
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

    pub(super) fn ensure_runtime_for_requests(
        &mut self,
        requests: &[CapabilityRequest],
    ) -> Result<(), CollectorError> {
        let attach_plan =
            AttachPlan::from_requests(requests, self.loader.config(), self.loader.payload_config());
        if self.idle_runtime_needs_replan(&attach_plan) {
            self.runtime = None;
        }
        if self.runtime.is_none() {
            let runtime = self
                .loader
                .load_runtime_with_plan(&attach_plan)
                .map_err(loader_error)?;
            self.runtime = Some(runtime);
        }
        self.runtime_mut()?
            .activate_static_programs()
            .map_err(loader_error)?;
        self.ensure_required_capabilities_attached(requests)
    }

    fn idle_runtime_needs_replan(&self, attach_plan: &AttachPlan) -> bool {
        self.active_binding_trace_count() == 0
            && self
                .runtime
                .as_ref()
                .is_some_and(|runtime| !attach_plan.is_satisfied_by(runtime.planned_capabilities()))
    }

    pub(super) fn active_binding_trace_count(&self) -> usize {
        self.bindings.trace_count() + self.pending_launches.len()
    }

    fn ensure_required_capabilities_attached(
        &self,
        requests: &[CapabilityRequest],
    ) -> Result<(), CollectorError> {
        let runtime = self.runtime_ref()?;
        if let Some(missing) = requests.iter().find(|request| {
            request.mode == RequestMode::Required
                && !runtime
                    .attached_capabilities()
                    .contains(&request.capability)
        }) {
            return Err(CollectorError::new(
                "bind_trace",
                format!(
                    "active eBPF runtime is attached without required capability {:?}; finish active traces and bind again with the requested capability set, or restart the daemon",
                    missing.capability
                ),
            ));
        }
        Ok(())
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
