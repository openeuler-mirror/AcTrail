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
        let process_identities = map_handle(&object, "process_identities", "process_identity_map")?;
        let process_identity_resolutions = map_handle(
            &object,
            "process_identity_resolutions",
            "process_identity_resolution_map",
        )?;
        runtime_process_identity::validate_process_identity_resolution_map(
            &process_identity_resolutions,
        )?;
        runtime_process_identity::configure_process_identity_resolution_ticks(&object)?;
        let trace_namespace_thread_identities = map_handle(
            &object,
            "trace_namespace_thread_identities",
            "trace_namespace_thread_identities",
        )?;
        if trace_namespace_thread_identities.value_size() as usize
            != TRACE_NAMESPACE_THREAD_IDENTITY_VALUE_SIZE
        {
            return Err(LoaderError::new(
                "trace_namespace_thread_identities",
                format!(
                    "unexpected thread identity value size {}",
                    trace_namespace_thread_identities.value_size()
                ),
            ));
        }
        if process_identities.value_size() as usize != PROCESS_IDENTITY_VALUE_SIZE {
            return Err(LoaderError::new(
                "process_identity_map",
                format!(
                    "unexpected process identity value size {}",
                    process_identities.value_size()
                ),
            ));
        }
        let launch_bindings =
            LaunchExecBindings::from_object(&object, config.suppressed_fd_index_slots_per_process)?;
        let fork_trace_bindings =
            map_handle(&object, "fork_trace_bindings", "fork_trace_bindings")?;
        let observer_fork_trace_bindings = map_handle(
            &object,
            "observer_fork_trace_bindings",
            "observer_fork_trace_bindings",
        )?;
        let trace_pid_namespaces =
            map_handle(&object, "trace_pid_namespaces", "trace_pid_namespaces_map")?;
        let observer_pid_namespace =
            map_handle(&object, "observer_pid_namespace", "observer_pid_namespace")?;
        let observer_pid_diagnostics = map_handle(
            &object,
            "observer_pid_diagnostics",
            "observer_pid_diagnostics",
        )?;
        let observer_pid_diagnostics_baseline =
            read_observer_pid_diagnostics(&observer_pid_diagnostics)?;
        write_observer_pid_namespace(&observer_pid_namespace, read_observer_pid_namespace()?)?;
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
            process_identities,
            process_identity_resolutions,
            trace_namespace_thread_identities,
            observer_pid_diagnostics,
            observer_pid_diagnostics_baseline,
            launch_bindings,
            fork_trace_bindings,
            observer_fork_trace_bindings,
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
            .filter(|program_name| program_name != "resolve_process_identities")
            .collect::<Vec<_>>();
        autoloaded_programs.sort_by_key(|program_name| attach_plan.attach_priority(program_name));
        let tracepoint_policy = tracepoint::TracepointAttachPolicy::new();
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
            if let Some(link) = tracepoint_policy.attach_program(
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
        self.observer_pid_diagnostics_baseline =
            read_observer_pid_diagnostics(&self.observer_pid_diagnostics)?;
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
        let observer_pid_diagnostics_baseline =
            read_observer_pid_diagnostics(&self.observer_pid_diagnostics)?;
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
        self.observer_pid_diagnostics_baseline = observer_pid_diagnostics_baseline;
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
        let current_diagnostics =
            read_event_transport_diagnostics(&self.event_transport_diagnostics)?;
        let diagnostics =
            current_diagnostics.saturating_delta_since(self.event_transport_diagnostics_baseline);
        self.event_transport_diagnostics_baseline = current_diagnostics;
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
            self.record_event_transport_loss_delta(summary);
        }
        if diagnostics.process_identity_cache_miss != 0 {
            self.record_event_transport_loss_delta(format!(
                "kernel process identity cache missed {} typed events",
                diagnostics.process_identity_cache_miss
            ));
        }
        if diagnostics.process_identity_cleanup_fail != 0 {
            self.record_event_transport_loss_delta(format!(
                "kernel process identity cleanup failed {} times",
                diagnostics.process_identity_cleanup_fail
            ));
        }
        let current_observer_diagnostics =
            read_observer_pid_diagnostics(&self.observer_pid_diagnostics)?;
        let observer_diagnostics = current_observer_diagnostics
            .saturating_delta_since(self.observer_pid_diagnostics_baseline);
        self.observer_pid_diagnostics_baseline = current_observer_diagnostics;
        if observer_diagnostics.level_mismatch != 0
            || observer_diagnostics.resolution_fail != 0
            || observer_diagnostics.index_publish_fail != 0
        {
            self.record_event_transport_loss_delta(format!(
                "kernel observer PID identity failures: level_mismatch={}, resolution_fail={}, index_publish_fail={}",
                observer_diagnostics.level_mismatch,
                observer_diagnostics.resolution_fail,
                observer_diagnostics.index_publish_fail,
            ));
        }
        Ok(())
    }

    fn record_event_transport_loss_summary(&mut self, summary: String) {
        if self.last_event_transport_loss_summary.as_deref() != Some(summary.as_str()) {
            self.last_event_transport_loss_summary = Some(summary.clone());
            self.pending_event_transport_loss_summaries.push(summary);
        }
    }

    fn record_event_transport_loss_delta(&mut self, summary: String) {
        self.last_event_transport_loss_summary = None;
        self.pending_event_transport_loss_summaries.push(summary);
    }

    pub fn take_event_transport_loss_summaries(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_event_transport_loss_summaries)
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
        let key = pid.to_ne_bytes();
        let tracked = self.tracked_trace_id(pid)?.is_some();
        let identity = self
            .process_identities
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("untrack_pid_identity", error.to_string()))?;
        if identity.is_none() && !tracked {
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
        Ok(())
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
