//! Configure maps and attach the shared eBPF runtime.

use super::*;

impl EbpfProgramLoader {
    pub fn load_runtime_with_plan(
        &self,
        attach_plan: &AttachPlan,
    ) -> Result<EbpfRuntime, LoaderError> {
        if attach_plan.mcp_stdio_enabled() && !self.config.ipc_lineage.enabled {
            return Err(LoaderError::new(
                "mcp_stdio_config",
                "MCP stdio observation requires ebpf.ipc_lineage.enabled=true",
            ));
        }
        file::validate_file_config(&self.config)?;
        fd::validate_fd_config(&self.config)?;
        tls::validate_payload_config(&self.payload.tls)?;
        stdio::validate_payload_config(&self.payload.stdio)?;
        socket::validate_payload_config(&self.payload.socket)?;
        process::validate_config(&self.process)?;
        suppressed_fd::validate_config(&self.config)?;
        let effective_payload = effective_config_for_attach_plan(&self.payload, attach_plan);
        environment::ensure_tracefs_control()?;
        environment::apply_memlock_rlimit(self.config.memlock_rlimit)?;
        let object_bytes = include_bytes!(env!("ACTRAIL_EBPF_OBJECT"));
        let mut builder = ObjectBuilder::default();
        if libbpf_debug_enabled()? {
            builder.debug(true);
        }
        let mut open_object = builder
            .open_memory(object_bytes)
            .map_err(|error| LoaderError::new("open_object", error.to_string()))?;
        resize_map(
            &mut open_object,
            "tracked_traces",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "process_observation_depths",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "process_identities",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "process_identity_resolutions",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "trace_namespace_thread_identities",
            self.config.pending_operation_max_entries,
        )?;
        #[cfg(actrail_launch_binding_task_storage)]
        resize_map(
            &mut open_object,
            "pending_exec_observer_bindings",
            self.config.pending_operation_max_entries,
        )?;
        #[cfg(actrail_launch_binding_pid_generation_hash)]
        {
            resize_map(
                &mut open_object,
                "pending_exec_bindings",
                self.config.pending_operation_max_entries,
            )?;
            resize_map(
                &mut open_object,
                "pending_exec_pid_index",
                self.config.pending_operation_max_entries,
            )?;
        }
        resize_map(
            &mut open_object,
            "trace_pid_namespaces",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_net_ops",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_process_exec_ops",
            self.process.pending_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_process_exec_tgid_index",
            self.process.pending_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "process_exec_sequences",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_process_fork_ops",
            self.process.pending_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "process_fork_sequences",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "payload_socket_operation_sequence",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_ipc_fd_pair_ops",
            self.config.pending_operation_max_entries,
        )?;
        // Unified fd lifecycle table plus its dense per-process active index.
        resize_map(
            &mut open_object,
            "fd_table",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "fd_objects",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "fd_index_slots",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "fd_process_active_counts",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_fd_open_ops",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_fd_close_ops",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_fd_dup_ops",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_fd_flag_ops",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_file_completion_ops",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_file_open_observations",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "fork_trace_bindings",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "observer_fork_trace_bindings",
            self.config.tracked_process_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_exit_ops",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "suppressed_fds",
            self.config.suppressed_fd_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "suppressed_fd_index",
            self.config.suppressed_fd_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_suppressed_fd_dup_ops",
            self.config.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "file_io_objects",
            if attach_plan.file_io_summary_enabled() {
                self.file_io_summary_config.object_max_entries
            } else {
                1
            },
        )?;
        resize_map(
            &mut open_object,
            "file_io_totals",
            if attach_plan.file_io_summary_enabled() {
                self.file_io_summary_config.max_entries
            } else {
                1
            },
        )?;
        resize_map(
            &mut open_object,
            "pending_tls_payload_ops",
            effective_payload.tls.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "tls_pending_ns",
            effective_payload.tls.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "go_tls_read_buffers",
            effective_payload.tls.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_stdio_payload_ops",
            effective_payload.stdio.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "payload_stdio_stream_sequences",
            effective_payload.stdio.stream_state_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "payload_socket_fds",
            effective_payload.socket.stream_state_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "payload_socket_process_generations",
            effective_payload.socket.stream_state_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_socket_payload_ops",
            effective_payload.socket.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "pending_socket_dup_ops",
            effective_payload.socket.pending_operation_max_entries,
        )?;
        resize_map(
            &mut open_object,
            "payload_socket_stream_sequences",
            effective_payload.socket.stream_state_max_entries,
        )?;
        let event_buffer_bytes = ring_buffer_max_bytes(&self.config, &effective_payload);
        resize_map(
            &mut open_object,
            "events",
            event_map_max_entries(event_buffer_bytes)?,
        )?;
        configure_program_autoload(&mut open_object, attach_plan, &effective_payload)?;

        let object = open_object
            .load()
            .map_err(|error| LoaderError::new("load_object", error.to_string()))?;
        EbpfRuntime::from_object(
            object,
            &self.config,
            &effective_payload,
            &self.process,
            &self.file_io_summary_config,
            attach_plan,
        )
    }
}

impl EbpfRuntime {
    pub(super) fn from_object(
        mut object: Object,
        config: &EbpfCollectorConfig,
        payload: &PayloadConfig,
        process_config: &ProcessSeccompConfig,
        file_io_summary_config: &config_core::daemon::FileIoSummaryConfig,
        attach_plan: &AttachPlan,
    ) -> Result<Self, LoaderError> {
        let tracked_traces = map_handle(&object, "tracked_traces", "tracked_map")?;
        let process_observation_depths = map_handle(
            &object,
            "process_observation_depths",
            "process_observation_depths",
        )?;
        if process_observation_depths.key_size() as usize != std::mem::size_of::<u32>()
            || process_observation_depths.value_size() as usize != 16
        {
            return Err(LoaderError::new(
                "process_observation_depths",
                format!(
                    "unexpected observation scope map key/value sizes {}/{}",
                    process_observation_depths.key_size(),
                    process_observation_depths.value_size()
                ),
            ));
        }
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
        file::configure_file_config_map(&object, config, attach_plan)?;
        fd::configure_fd_category_config_map(&object, attach_plan, config)?;
        let suppressed_fd_config = suppressed_fd::SuppressedFdConfig::new(&object, config)?;
        tls::configure_payload_tls_map(&object, &payload.tls)?;
        stdio::configure_payload_stdio_map(&object, &payload.stdio)?;
        socket::configure_payload_socket_map(&object, &payload.socket)?;
        process::configure_map(
            &object,
            process_config,
            payload.tls.enabled
                && payload.tls.direct_dynamic_discovery_enabled
                && payload.tls.capture_backend
                    == config_core::daemon::PayloadTlsCaptureBackend::BpfCopy,
        )?;
        let direct_tls_ready = tls::DirectTlsBackend::validate_ready(&object, &payload.tls)?;
        let file_io_summaries = file_io::FileIoSummaryMap::from_object(
            &object,
            config,
            file_io_summary_config,
            attach_plan,
        )?;
        let consumer = EventConsumer::spawn(&events_map, event_buffer_bytes)?;

        let (links, attached_programs) =
            Self::attach_loaded_programs(&mut object, payload, attach_plan)?;
        let mut attached_capabilities = attach_plan.attached_capabilities(&attached_programs);
        if direct_tls_ready && attach_plan.contains(&Capability::TlsPlaintextPayload) {
            // For on-demand TLS this denotes backend readiness. Object coverage
            // is represented only by successful links, never by this capability.
            attached_capabilities.insert(Capability::TlsPlaintextPayload);
        }

        Ok(Self {
            attach_plan: attach_plan.clone(),
            object,
            links,
            attached_programs,
            attached_capabilities,
            tracked_traces,
            process_observation_depths,
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
            suppressed_fd_config,
            file_io_summaries,
            pending_tls_payload_ops,
            pending_tls_payload_ops_by_namespace,
            payload_tls_diagnostics,
            tls_diagnostics_baseline,
            payload_socket_fds,
            event_transport_diagnostics,
            event_transport_diagnostics_baseline,
            consumer,
            pending_raw_events: Vec::new(),
            last_perf_lost: 0,
            loss_diagnostics: runtime_loss::LossDiagnostics::new(
                config.diagnostics_summary_interval_ms,
            ),
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
}
