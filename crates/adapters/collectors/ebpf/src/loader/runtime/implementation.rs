//! Loaded eBPF runtime ownership, maps, and attachment lifecycle.

use super::*;

impl EbpfRuntime {
    pub(super) fn from_object(
        mut object: Object,
        config: &EbpfCollectorConfig,
        payload: &PayloadConfig,
        attach_plan: &AttachPlan,
        static_link_teardown: StaticLinkTeardown,
    ) -> Result<Self, LoaderError> {
        let tracked_traces = map_handle(&object, "tracked_traces", "tracked_map")?;
        let process_start_times =
            map_handle(&object, "process_start_times", "process_start_time_map")?;
        let launch_bindings =
            LaunchExecBindings::from_object(&object, config.suppressed_fd_index_slots_per_process)?;
        let fork_trace_bindings =
            map_handle(&object, "fork_trace_bindings", "fork_trace_bindings")?;
        let trace_pid_namespaces =
            map_handle(&object, "trace_pid_namespaces", "trace_pid_namespaces_map")?;
        let suppressed_fds = map_handle(&object, "suppressed_fds", "suppressed_fds")?;
        let suppressed_fd_index =
            map_handle(&object, "suppressed_fd_index", "suppressed_fd_index")?;
        let file_bulk_read_fast_processes = map_handle(
            &object,
            "file_bulk_read_fast_processes",
            "file_bulk_read_fast_processes",
        )?;
        let file_bulk_read_fast_fd_stats = map_handle(
            &object,
            "file_bulk_read_fast_fd_stats",
            "file_bulk_read_fast_fd_stats",
        )?;
        let pending_tls_payload_ops = map_handle(
            &object,
            "pending_tls_payload_ops",
            "pending_tls_payload_ops",
        )?;
        let pending_tls_payload_ops_by_namespace =
            map_handle(&object, "tls_pending_ns", "tls_pending_ns")?;
        let payload_tls_diagnostics = map_handle(
            &object,
            "payload_tls_diagnostics",
            "payload_tls_diagnostics",
        )?;
        let payload_socket_fds = map_handle(&object, "payload_socket_fds", "payload_socket_fds")?;
        let event_transport_diagnostics = map_handle(
            &object,
            "event_transport_diagnostics",
            "event_transport_diagnostics",
        )?;
        let tls_diagnostics_baseline = TlsPayloadDiagnostics {
            counters: Vec::new(),
        };
        let event_transport_diagnostics_baseline = EventTransportDiagnostics::default();
        let events_map = map_handle(&object, "events", "event_buffer")?;

        let event_buffer_bytes = ring_buffer_max_bytes(config, payload);
        let consumer = EventConsumer::spawn(&events_map, event_buffer_bytes)?;
        file::configure_file_config_map(&object, config, attach_plan)?;
        fd::configure_fd_category_config_map(&object, attach_plan, config)?;
        suppressed_fd::configure_config_map(&object, config)?;
        tls::configure_payload_tls_map(&object, &payload.tls)?;
        stdio::configure_payload_stdio_map(&object, &payload.stdio)?;
        socket::configure_payload_socket_map(&object, &payload.socket)?;

        let (links, attached_programs) =
            Self::attach_loaded_programs(&mut object, payload, attach_plan)?;
        let attached_capabilities = attach_plan.attached_capabilities(&attached_programs);

        Ok(Self {
            object,
            links,
            static_link_teardown,
            attachment_state: RuntimeAttachmentState::Attached,
            attach_plan: attach_plan.clone(),
            payload: payload.clone(),
            planned_static_programs: attached_programs.clone(),
            planned_capabilities: attached_capabilities.clone(),
            attached_programs,
            attached_capabilities,
            tracked_traces,
            process_start_times,
            launch_bindings,
            fork_trace_bindings,
            trace_pid_namespaces,
            suppressed_fds,
            suppressed_fd_index,
            suppressed_fd_index_slots_per_process: config.suppressed_fd_index_slots_per_process,
            file_bulk_read_fast_processes,
            file_bulk_read_fast_fd_stats,
            pending_tls_payload_ops,
            pending_tls_payload_ops_by_namespace,
            payload_tls_diagnostics,
            tls_diagnostics_baseline,
            payload_socket_fds,
            event_transport_diagnostics,
            event_transport_diagnostics_baseline,
            events_map,
            consumer: Some(consumer),
            event_buffer_bytes,
            pending_raw_events: Vec::new(),
            last_perf_lost: 0,
            last_event_transport_loss_summary: None,
            pending_event_transport_loss_summaries: Vec::new(),
            last_raw_sample_count: 0,
        })
    }

    fn attach_loaded_programs(
        object: &mut Object,
        payload: &PayloadConfig,
        attach_plan: &AttachPlan,
    ) -> Result<(Vec<Link>, Vec<String>), LoaderError> {
        let mut links = Vec::new();
        let mut attached_programs = Vec::new();
        let mut autoloaded_programs = object
            .progs()
            .filter(|program| program.autoload())
            .map(|program| program.name().to_string_lossy().into_owned())
            .filter(|program_name| !tls::is_payload_tls_program(program_name))
            .collect::<Vec<_>>();
        autoloaded_programs.sort_by_key(|program_name| attach_plan.attach_priority(program_name));
        for program_name in autoloaded_programs {
            let program = object
                .progs_mut()
                .find(|program| program.name() == OsStr::new(&program_name))
                .ok_or_else(|| {
                    LoaderError::new(
                        "attach_program",
                        format!("BPF program {program_name} is missing"),
                    )
                })?;
            if let Some(link) = tracepoint::attach_program(
                &program,
                &program_name,
                attach_plan.allows_missing_tracepoint(&program_name),
            )? {
                links.push(link);
                attached_programs.push(program_name);
            }
        }
        for (link, program_name) in tls::attach_payload_tls_programs(object, &payload.tls)? {
            links.push(link);
            attached_programs.push(program_name);
        }
        if links.is_empty() {
            return Err(LoaderError::new(
                "attach_program",
                "eBPF object did not attach any programs",
            ));
        }
        Ok((links, attached_programs))
    }

    pub(crate) fn park_for_first_binding(&mut self) -> Result<(), LoaderError> {
        if self.attachment_state != RuntimeAttachmentState::Attached
            || self.attached_programs != self.planned_static_programs
        {
            return Err(LoaderError::new(
                "park_runtime",
                "only a newly preflighted static eBPF runtime can be parked",
            ));
        }
        let links = std::mem::take(&mut self.links);
        self.static_link_teardown.drop_all(links)?;
        self.consumer.take();
        self.pending_raw_events.clear();
        self.last_perf_lost = 0;
        self.tls_diagnostics_baseline =
            tls::read_tls_payload_diagnostics(&self.payload_tls_diagnostics)?;
        self.event_transport_diagnostics_baseline =
            read_event_transport_diagnostics(&self.event_transport_diagnostics)?;
        self.attached_programs.clear();
        self.attached_capabilities.clear();
        self.last_event_transport_loss_summary = None;
        self.pending_event_transport_loss_summaries.clear();
        self.last_raw_sample_count = 0;
        self.attachment_state = RuntimeAttachmentState::Parked;
        Ok(())
    }

    pub(crate) fn activate_static_programs(&mut self) -> Result<(), LoaderError> {
        if self.attachment_state == RuntimeAttachmentState::Attached {
            return Ok(());
        }
        let consumer = EventConsumer::spawn(&self.events_map, self.event_buffer_bytes)?;
        self.consumer = Some(consumer);
        self.pending_raw_events.clear();
        self.last_perf_lost = 0;
        let tls_diagnostics_baseline =
            tls::read_tls_payload_diagnostics(&self.payload_tls_diagnostics)?;
        let event_transport_diagnostics_baseline =
            read_event_transport_diagnostics(&self.event_transport_diagnostics)?;
        let (links, attached_programs) =
            Self::attach_loaded_programs(&mut self.object, &self.payload, &self.attach_plan)?;
        if attached_programs != self.planned_static_programs {
            return Err(LoaderError::new(
                "reattach_program",
                "parked eBPF runtime reattached a different static program set",
            ));
        }
        self.links = links;
        self.tls_diagnostics_baseline = tls_diagnostics_baseline;
        self.event_transport_diagnostics_baseline = event_transport_diagnostics_baseline;
        self.attached_programs = attached_programs;
        self.attached_capabilities = self.planned_capabilities.clone();
        self.attachment_state = RuntimeAttachmentState::Attached;
        Ok(())
    }

    pub fn poll_events(&mut self) -> Result<Vec<KernelEvent>, LoaderError> {
        if self.consumer.is_none() {
            self.last_raw_sample_count = 0;
            return Ok(Vec::new());
        }
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
            self.record_event_transport_loss_summary(format!(
                "kernel event processing failed locally: {error:?}"
            ));
        }
        if let Some(error) = drain_error {
            self.record_event_transport_loss_summary(format!(
                "kernel event consumer failed after delivering queued events: {error:?}"
            ));
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
        if self.consumer.is_none() {
            return Ok(());
        }
        let drain_error = self.drain_consumer_queue().err();
        let diagnostics_error = self.capture_event_transport_loss().err();
        for error in [drain_error, diagnostics_error].into_iter().flatten() {
            self.record_event_transport_loss_summary(format!(
                "kernel event transport flush failed locally: {error:?}"
            ));
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
        let Some(consumer) = self.consumer.as_ref() else {
            return Ok(());
        };
        consumer.clear_wakeup();
        loop {
            match consumer.try_recv() {
                Ok(EventConsumerMessage::RawBatch { raw, perf_lost }) => {
                    self.pending_raw_events.extend(raw);
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
        let perf_lost = self.last_perf_lost;
        let diagnostics = read_event_transport_diagnostics(&self.event_transport_diagnostics)?
            .saturating_delta_since(self.event_transport_diagnostics_baseline);
        if perf_lost != 0
            || diagnostics.reserve_fail != 0
            || diagnostics.output_fail != 0
            || diagnostics.output_fail_bytes != 0
            || diagnostics.stdio_pending_update_fail != 0
            || diagnostics.stdio_read_user_fail != 0
            || diagnostics.socket_state_update_fail != 0
            || diagnostics.socket_sequence_update_fail != 0
        {
            let summary = format!(
                "kernel event transport lost data: perf_lost={perf_lost}, reserve_fail={}, output_fail={}, output_fail_bytes={}, stdio_pending_update_fail={}, stdio_read_user_fail={}, socket_state_update_fail={}, socket_sequence_update_fail={}",
                diagnostics.reserve_fail,
                diagnostics.output_fail,
                diagnostics.output_fail_bytes,
                diagnostics.stdio_pending_update_fail,
                diagnostics.stdio_read_user_fail,
                diagnostics.socket_state_update_fail,
                diagnostics.socket_sequence_update_fail,
            );
            self.record_event_transport_loss_summary(summary);
        }
        Ok(())
    }

    fn record_event_transport_loss_summary(&mut self, summary: String) {
        if self.last_event_transport_loss_summary.as_deref() != Some(summary.as_str()) {
            self.last_event_transport_loss_summary = Some(summary.clone());
            self.pending_event_transport_loss_summaries.push(summary);
        }
    }

    pub fn take_event_transport_loss_summaries(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_event_transport_loss_summaries)
    }

    pub fn track_pid(
        &self,
        map_pid: u32,
        kernel_start_time: u64,
        trace_id: TraceId,
    ) -> Result<(), LoaderError> {
        let key = map_pid.to_ne_bytes();
        let value = trace_id.get().to_ne_bytes();
        self.tracked_traces
            .update(&key, &value, MapFlags::ANY)
            .map_err(|error| LoaderError::new("track_pid", error.to_string()))?;
        self.process_start_times
            .update(&key, &kernel_start_time.to_ne_bytes(), MapFlags::ANY)
            .map_err(|error| LoaderError::new("track_pid_start_time", error.to_string()))
    }

    pub(crate) fn arm_launch_binding(
        &self,
        pidfd: OwnedFd,
        host_pid: u32,
        trace_id: TraceId,
        generation: u64,
        suppressed_fds: &[InitialSuppressedFd],
    ) -> Result<ArmedLaunchBinding, LoaderError> {
        let target = LaunchBindingTarget::new(pidfd, host_pid, generation)?;
        let pending = PendingLaunchBinding::new(trace_id, suppressed_fds);
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
        self.trace_pid_namespaces
            .delete(&key)
            .map_err(|error| LoaderError::new("trace_pid_namespace", error.to_string()))
    }

    pub fn suppress_fd(
        &self,
        trace_id: TraceId,
        suppressed_fd: &ProcessSuppressedFd,
    ) -> Result<(), LoaderError> {
        suppressed_fd::suppress_fd(
            &self.suppressed_fds,
            &self.suppressed_fd_index,
            self.suppressed_fd_index_slots_per_process,
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
            self.suppressed_fd_index_slots_per_process,
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

    pub(crate) fn fork_trace_binding(
        &self,
        host_pid: u32,
    ) -> Result<Option<ForkTraceBinding>, LoaderError> {
        let key = host_pid.to_ne_bytes();
        self.fork_trace_bindings
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?
            .map(|value| parse_fork_trace_binding(&value))
            .transpose()
    }

    pub(crate) fn fork_identity_publish_failures(&self) -> Result<u64, LoaderError> {
        read_event_transport_counter(
            &self.event_transport_diagnostics,
            FORK_IDENTITY_PUBLISH_FAIL_COUNTER,
        )
    }

    pub(crate) fn untrack_fork_host_pid(&self, host_pid: u32) -> Result<(), LoaderError> {
        let key = host_pid.to_ne_bytes();
        if self
            .fork_trace_bindings
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?
            .is_some()
        {
            self.fork_trace_bindings
                .delete(&key)
                .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?;
        }
        Ok(())
    }

    pub(crate) fn untrack_fork_trace(&self, trace_id: TraceId) -> Result<(), LoaderError> {
        for key in self.fork_trace_bindings.keys().collect::<Vec<_>>() {
            let binding = self
                .fork_trace_bindings
                .lookup(&key, MapFlags::ANY)
                .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?
                .map(|value| parse_fork_trace_binding(&value))
                .transpose()?;
            if binding.is_some_and(|binding| binding.trace_id == trace_id) {
                self.fork_trace_bindings
                    .delete(&key)
                    .map_err(|error| LoaderError::new("fork_trace_binding", error.to_string()))?;
            }
        }
        Ok(())
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

    pub(crate) fn is_attached(&self) -> bool {
        self.attachment_state == RuntimeAttachmentState::Attached
    }

    pub(crate) fn planned_capabilities(&self) -> &BTreeSet<Capability> {
        &self.planned_capabilities
    }

    pub fn attached_capabilities(&self) -> &BTreeSet<Capability> {
        &self.attached_capabilities
    }

    pub fn last_raw_sample_count(&self) -> usize {
        self.last_raw_sample_count
    }

    pub fn untrack_pid(&self, pid: u32) -> Result<(), LoaderError> {
        if self.tracked_trace_id(pid)?.is_none() {
            return Ok(());
        }
        let key = pid.to_ne_bytes();
        self.tracked_traces
            .delete(&key)
            .map_err(|error| LoaderError::new("untrack_pid", error.to_string()))?;
        self.process_start_times
            .delete(&key)
            .map_err(|error| LoaderError::new("untrack_pid_start_time", error.to_string()))
    }

    pub fn mark_file_bulk_read_fast_process(
        &self,
        pid: u32,
        generation: u64,
        trace_id: TraceId,
    ) -> Result<(), LoaderError> {
        let key = file_bulk_read_fast_process_key(pid, generation)?;
        let mut value = [0_u8; FILE_BULK_READ_FAST_PROCESS_VALUE_SIZE];
        value.copy_from_slice(&trace_id.get().to_ne_bytes());
        self.file_bulk_read_fast_processes
            .update(&key, &value, MapFlags::ANY)
            .map_err(|error| LoaderError::new("file_bulk_read_fast_process", error.to_string()))
    }

    pub fn unmark_file_bulk_read_fast_process(
        &self,
        pid: u32,
        generation: u64,
    ) -> Result<(), LoaderError> {
        let key = file_bulk_read_fast_process_key(pid, generation)?;
        if self
            .file_bulk_read_fast_processes
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("file_bulk_read_fast_process", error.to_string()))?
            .is_none()
        {
            return Ok(());
        }
        self.file_bulk_read_fast_processes
            .delete(&key)
            .map_err(|error| LoaderError::new("file_bulk_read_fast_process", error.to_string()))
    }

    pub fn sweep_file_bulk_read_fast_fds_for_process(
        &self,
        pid: u32,
        generation: u64,
    ) -> Result<(), LoaderError> {
        for key in self.file_bulk_read_fast_fd_stats.keys().collect::<Vec<_>>() {
            let Some(parsed) = parse_file_bulk_read_fast_fd_key(&key) else {
                continue;
            };
            if parsed.pid == pid && parsed.generation == generation {
                self.file_bulk_read_fast_fd_stats
                    .delete(&key)
                    .map_err(|error| {
                        LoaderError::new("sweep_file_bulk_read_fast_fds", error.to_string())
                    })?;
            }
        }
        Ok(())
    }

    pub fn max_tracked_processes(&self) -> u32 {
        self.tracked_traces.max_entries()
    }

    pub fn event_poll_fd(&self) -> Result<Option<RawFd>, LoaderError> {
        Ok(self.consumer.as_ref().map(EventConsumer::wake_fd))
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
        if !self.is_attached() {
            return Ok(None);
        }
        tls::read_tls_payload_diagnostics(&self.payload_tls_diagnostics).map(|diagnostics| {
            Some(diagnostics.saturating_delta_since(&self.tls_diagnostics_baseline))
        })
    }

    pub fn attach_go_tls_executable(&mut self, binary_path: &Path) -> Result<bool, LoaderError> {
        if !self.is_attached() {
            return Err(LoaderError::new(
                "attach_go_tls_executable",
                "cannot attach a dynamic TLS probe while the eBPF runtime is parked",
            ));
        }
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
        if !self.is_attached() {
            return Err(LoaderError::new(
                "attach_dynamic_tls_plan",
                "cannot attach a dynamic TLS probe while the eBPF runtime is parked",
            ));
        }
        let links = tls::attach_dynamic_tls_programs(&mut self.object, plan)?;
        for (link, program_name) in links {
            self.links.push(link);
            self.attached_programs.push(program_name);
        }
        Ok(())
    }
}
