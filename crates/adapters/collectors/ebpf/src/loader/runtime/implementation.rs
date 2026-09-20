//! Loaded eBPF runtime transport, maps, and dynamic probe ownership.

use super::*;

impl EbpfRuntime {
    pub fn poll_events(&mut self) -> Result<Vec<KernelEvent>, LoaderError> {
        let drain_error = self.drain_consumer_queue().err();
        let raw_events = std::mem::take(&mut self.pending_raw_events);
        self.last_raw_sample_count = raw_events.len();
        let mut events = Vec::with_capacity(raw_events.len());
        let mut decode_error = None;
        for raw in raw_events {
            match decode_kernel_event(&raw) {
                Ok(event) => events.push(event),
                Err(error) if decode_error.is_none() => decode_error = Some(error),
                Err(_) => {}
            }
        }
        let diagnostics_error = self.capture_event_transport_loss().err();
        for error in [decode_error, diagnostics_error].into_iter().flatten() {
            self.loss_diagnostics
                .record_error("event_processing_fail", || {
                    format!("kernel event processing failed locally: {error:?}")
                });
        }
        if let Some(error) = drain_error {
            self.loss_diagnostics
                .record_error("event_consumer_fail", || {
                    format!(
                        "kernel event consumer failed after delivering queued events: {error:?}"
                    )
                });
            if events.is_empty() {
                return Err(error);
            }
        }
        // Both ring buffers and perf buffers are drained per CPU, so callback
        // order is not a global causal order. Timestamped events are sorted
        // causally; untimestamped control diagnostics remain last in arrival
        // order.
        events.sort_by_key(|event| {
            let observed_ktime_ns = event.observed_ktime_ns();
            (observed_ktime_ns.is_none(), observed_ktime_ns)
        });
        Ok(events)
    }

    /// Drain the kernel transport buffer into userspace without decoding.
    ///
    /// Call this after a drain cycle's expensive processing to shrink the
    /// starvation window — events that arrived while the pipeline was busy
    /// are moved into the userspace raw buffer so the kernel ring buffer can
    /// accept new submissions. The buffered bytes are decoded on the next
    /// `poll_events()` call.
    pub fn flush_transport(&mut self) -> Result<(), LoaderError> {
        let drain_error = self.drain_consumer_queue().err();
        let diagnostics_error = self.capture_event_transport_loss().err();
        for error in [drain_error, diagnostics_error].into_iter().flatten() {
            self.loss_diagnostics
                .record_error("event_transport_flush_fail", || {
                    format!("kernel event transport flush failed locally: {error:?}")
                });
        }
        Ok(())
    }

    /// Pull queued raw batches from the consumer thread into the pending raw
    /// buffer without decoding, resetting the daemon wakeup first.
    fn drain_consumer_queue(&mut self) -> Result<(), LoaderError> {
        // Reset the wakeup before draining: a consumer write that lands after
        // this reset but before the drain finishes leaves the eventfd counter
        // non-zero, so the daemon wakes again instead of stranding a batch
        // until the next background poll.
        let consumer = &self.consumer;
        consumer.clear_wakeup();
        loop {
            match consumer.try_recv() {
                Ok(EventConsumerMessage::RawBatch { raw, perf_lost }) => {
                    self.pending_raw_events.extend(raw);
                    self.loss_diagnostics
                        .record_count("perf_lost", perf_lost.saturating_sub(self.last_perf_lost));
                    self.last_perf_lost = perf_lost;
                }
                Ok(EventConsumerMessage::Failure { stage, message }) => {
                    return Err(LoaderError::new(format!("event_consumer_{stage}"), message));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err(LoaderError::new(
                        "event_consumer",
                        "event consumer thread exited unexpectedly",
                    ));
                }
            }
        }
        Ok(())
    }

    fn capture_event_transport_loss(&mut self) -> Result<(), LoaderError> {
        let current_diagnostics =
            read_event_transport_diagnostics(&self.event_transport_diagnostics)?;
        let diagnostics =
            current_diagnostics.saturating_delta_since(self.event_transport_diagnostics_baseline);
        self.event_transport_diagnostics_baseline = current_diagnostics;
        for (category, count) in [
            ("reserve_fail", diagnostics.reserve_fail),
            ("output_fail", diagnostics.output_fail),
            ("output_fail_bytes", diagnostics.output_fail_bytes),
            (
                "stdio_pending_update_fail",
                diagnostics.stdio_pending_update_fail,
            ),
            ("stdio_read_user_fail", diagnostics.stdio_read_user_fail),
            (
                "socket_state_update_fail",
                diagnostics.socket_state_update_fail,
            ),
            (
                "socket_sequence_update_fail",
                diagnostics.socket_sequence_update_fail,
            ),
            ("socket_read_user_fail", diagnostics.socket_read_user_fail),
            ("socket_reserve_fail", diagnostics.socket_reserve_fail),
            (
                "process_identity_cache_miss",
                diagnostics.process_identity_cache_miss,
            ),
            (
                "file_pending_update_fail",
                diagnostics.file_pending_update_fail,
            ),
            (
                "process_identity_cleanup_fail",
                diagnostics.process_identity_cleanup_fail,
            ),
        ] {
            self.loss_diagnostics.record_count(category, count);
        }
        let current_observer_diagnostics =
            read_observer_pid_diagnostics(&self.observer_pid_diagnostics)?;
        let observer_diagnostics = current_observer_diagnostics
            .saturating_delta_since(self.observer_pid_diagnostics_baseline);
        self.observer_pid_diagnostics_baseline = current_observer_diagnostics;
        for (category, count) in [
            (
                "observer_pid_level_mismatch",
                observer_diagnostics.level_mismatch,
            ),
            (
                "observer_pid_resolution_fail",
                observer_diagnostics.resolution_fail,
            ),
            (
                "observer_pid_index_publish_fail",
                observer_diagnostics.index_publish_fail,
            ),
        ] {
            self.loss_diagnostics.record_count(category, count);
        }
        Ok(())
    }

    pub fn event_transport_loss_poll_timeout(&self) -> Option<std::time::Duration> {
        self.loss_diagnostics.poll_timeout()
    }

    pub(crate) fn record_event_transport_loss_count(&mut self, category: &'static str, count: u64) {
        self.loss_diagnostics.record_count(category, count);
    }

    pub fn take_event_transport_loss_summary(&mut self, force: bool) -> (bool, Option<String>) {
        self.loss_diagnostics.take_summary(force)
    }

    pub fn track_pid(
        &self,
        kernel_tgid: u32,
        kernel_start_time: u64,
        observer_tgid: u32,
        trace_id: TraceId,
    ) -> Result<(), LoaderError> {
        if kernel_tgid == 0 || kernel_start_time == 0 || observer_tgid == 0 {
            return Err(LoaderError::new(
                "track_pid_identity",
                "process identity requires non-zero kernel TGID, observer TGID, and generation",
            ));
        }
        let key = kernel_tgid.to_ne_bytes();
        let value = trace_id.get().to_ne_bytes();
        self.tracked_traces
            .update(&key, &value, MapFlags::ANY)
            .map_err(|error| LoaderError::new("track_pid", error.to_string()))?;
        let mut identity = [0_u8; PROCESS_IDENTITY_VALUE_SIZE];
        identity[0..8].copy_from_slice(&kernel_start_time.to_ne_bytes());
        identity[8..12].copy_from_slice(&observer_tgid.to_ne_bytes());
        if let Err(error) = self
            .process_identities
            .update(&key, &identity, MapFlags::ANY)
        {
            let rollback = self.tracked_traces.delete(&key);
            return Err(LoaderError::new(
                "track_pid_identity",
                match rollback {
                    Ok(()) => error.to_string(),
                    Err(rollback_error) => {
                        format!("{error}; tracked trace rollback failed: {rollback_error}")
                    }
                },
            ));
        }
        Ok(())
    }

    pub(crate) fn arm_launch_binding(
        &self,
        pidfd: OwnedFd,
        observer_tgid: u32,
        trace_id: TraceId,
        generation: u64,
        suppressed_fds: &[InitialSuppressedFd],
    ) -> Result<ArmedLaunchBinding, LoaderError> {
        let target = LaunchBindingTarget::new(pidfd, observer_tgid, generation)?;
        let pending = PendingLaunchBinding::new(trace_id, suppressed_fds);
        if !suppressed_fds.is_empty() {
            self.suppressed_fd_config.activate()?;
        }
        self.launch_bindings.arm(target, &pending)
    }

    pub(crate) fn cancel_launch_binding(
        &self,
        armed: &ArmedLaunchBinding,
    ) -> Result<bool, LoaderError> {
        self.launch_bindings.cancel(armed)
    }

    pub fn register_trace_pid_namespace(
        &self,
        trace_id: TraceId,
        pid: u32,
    ) -> Result<(), LoaderError> {
        let namespace = read_pid_namespace_for_pid(pid)?;
        write_trace_pid_namespace(
            &self.trace_pid_namespaces,
            trace_id,
            namespace,
            "trace_pid_namespace",
        )
    }

    pub fn unregister_trace_pid_namespace(&self, trace_id: TraceId) -> Result<(), LoaderError> {
        let key = trace_id.get().to_ne_bytes();
        if self
            .trace_pid_namespaces
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("trace_pid_namespace", error.to_string()))?
            .is_none()
        {
            return Ok(());
        }
        for thread_key in self
            .trace_namespace_thread_identities
            .keys()
            .collect::<Vec<_>>()
        {
            let cached_trace_id = self
                .trace_namespace_thread_identities
                .lookup(&thread_key, MapFlags::ANY)
                .map_err(|error| {
                    LoaderError::new("trace_namespace_thread_identities", error.to_string())
                })?
                .and_then(|value| value.get(..8).and_then(|raw| raw.try_into().ok()))
                .map(u64::from_ne_bytes);
            if cached_trace_id == Some(trace_id.get()) {
                self.trace_namespace_thread_identities
                    .delete(&thread_key)
                    .map_err(|error| {
                        LoaderError::new("trace_namespace_thread_identities", error.to_string())
                    })?;
            }
        }
        self.trace_pid_namespaces
            .delete(&key)
            .map_err(|error| LoaderError::new("trace_pid_namespace", error.to_string()))
    }

    pub fn suppress_fd(
        &self,
        trace_id: TraceId,
        suppressed_fd: &ProcessSuppressedFd,
    ) -> Result<(), LoaderError> {
        self.suppressed_fd_config.activate()?;
        suppressed_fd::suppress_fd(
            &self.suppressed_fds,
            &self.suppressed_fd_index,
            self.suppressed_fd_config.capacity(),
            trace_id,
            suppressed_fd,
        )
    }

    pub fn unsuppress_fd(
        &self,
        process: &KernelProcessCoordinates,
        fd: i32,
    ) -> Result<(), LoaderError> {
        suppressed_fd::unsuppress_fd(
            &self.suppressed_fds,
            &self.suppressed_fd_index,
            self.suppressed_fd_config.capacity(),
            process,
            fd,
        )
    }

    pub fn sweep_suppressed_fds_for_process(
        &self,
        pid: u32,
        generation: u64,
    ) -> Result<(), LoaderError> {
        suppressed_fd::sweep_process(
            &self.suppressed_fds,
            &self.suppressed_fd_index,
            pid,
            generation,
        )
    }

    pub fn sweep_suppressed_fds_for_trace(&self, trace_id: TraceId) -> Result<(), LoaderError> {
        suppressed_fd::sweep_trace(&self.suppressed_fds, &self.suppressed_fd_index, trace_id)
    }

    pub fn tracked_trace_id(&self, pid: u32) -> Result<Option<TraceId>, LoaderError> {
        let key = pid.to_ne_bytes();
        self.tracked_traces
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("lookup_tracked_pid", error.to_string()))?
            .map(|value| {
                value
                    .get(..8)
                    .and_then(|value| value.try_into().ok())
                    .map(u64::from_ne_bytes)
                    .map(TraceId::new)
                    .ok_or_else(|| {
                        LoaderError::new(
                            "lookup_tracked_pid",
                            format!("unexpected tracked trace value size {}", value.len()),
                        )
                    })
            })
            .transpose()
    }

    pub fn lookup_socket_fd_state(
        &self,
        pid: u32,
        fd: u32,
    ) -> Result<Option<SocketPayloadFdState>, LoaderError> {
        socket::lookup_fd_state(&self.payload_socket_fds, pid, fd)
    }

    pub fn mark_socket_fd_tls_owned(
        &self,
        pid: u32,
        fd: u32,
        expected_generation: u32,
    ) -> Result<bool, LoaderError> {
        socket::mark_fd_tls_owned(&self.payload_socket_fds, pid, fd, expected_generation)
    }

    pub fn attached_programs(&self) -> &[String] {
        &self.attached_programs
    }

    pub fn attached_capabilities(&self) -> &BTreeSet<Capability> {
        &self.attached_capabilities
    }

    pub fn last_raw_sample_count(&self) -> usize {
        self.last_raw_sample_count
    }

    pub fn untrack_pid(&self, pid: u32) -> Result<(), LoaderError> {
        let key = pid.to_ne_bytes();
        let tracked = self.tracked_trace_id(pid)?.is_some();
        let identity = self
            .process_identities
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("untrack_pid_identity", error.to_string()))?;
        if identity.is_none() && !tracked {
            self.clear_process_observation_depth(pid)?;
            return Ok(());
        }
        if identity.is_some() {
            self.process_identities
                .delete(&key)
                .map_err(|error| LoaderError::new("untrack_pid_identity", error.to_string()))?;
        }
        if tracked && let Err(error) = self.tracked_traces.delete(&key) {
            let rollback = identity
                .as_ref()
                .map(|value| self.process_identities.update(&key, value, MapFlags::ANY));
            return Err(LoaderError::new(
                "untrack_pid",
                match rollback {
                    Some(Err(rollback_error)) => {
                        format!("{error}; process identity rollback failed: {rollback_error}")
                    }
                    _ => error.to_string(),
                },
            ));
        }
        self.clear_process_observation_depth(pid)?;
        Ok(())
    }

    pub fn max_tracked_processes(&self) -> u32 {
        self.tracked_traces.max_entries()
    }

    pub fn event_poll_fd(&self) -> Result<Option<RawFd>, LoaderError> {
        Ok(Some(self.consumer.wake_fd()))
    }

    pub fn lookup_pending_tls_payload_op(
        &self,
        tid: u32,
    ) -> Result<Option<PendingTlsPayloadOp>, LoaderError> {
        tls::lookup_pending_payload_op(
            &self.pending_tls_payload_ops_by_namespace,
            &self.pending_tls_payload_ops,
            tid,
        )
    }

    pub fn tls_payload_diagnostics(&self) -> Result<Option<TlsPayloadDiagnostics>, LoaderError> {
        tls::read_tls_payload_diagnostics(&self.payload_tls_diagnostics).map(|diagnostics| {
            Some(diagnostics.saturating_delta_since(&self.tls_diagnostics_baseline))
        })
    }

    pub fn attach_go_tls_executable(&mut self, binary_path: &Path) -> Result<bool, LoaderError> {
        let outcome = tls::attach_go_tls_programs(&mut self.object, binary_path)?;
        let GoTlsAttachOutcome::Attached(links) = outcome else {
            return Ok(false);
        };
        for (link, program_name) in links {
            self.links.push(link);
            self.attached_programs.push(program_name);
        }
        Ok(true)
    }

    pub fn attach_dynamic_tls_plan(
        &mut self,
        plan: &DynamicTlsProbePlan,
    ) -> Result<(), LoaderError> {
        let links = tls::attach_dynamic_tls_programs(&mut self.object, plan)?;
        for (link, program_name) in links {
            self.links.push(link);
            self.attached_programs.push(program_name);
        }
        Ok(())
    }
}
