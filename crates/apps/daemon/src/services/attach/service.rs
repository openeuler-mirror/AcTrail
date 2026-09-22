//! Attach service interface and launch descriptor validation.

use super::*;

impl StorageAttachService {
    pub(super) fn finalize_trace(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        trace_id: model_core::ids::TraceId,
        root_identity: ProcessIdentity,
        process_records: Vec<ProcessRecord>,
        launch_mode: bool,
        diagnostic_kind: DiagnosticKind,
        diagnostic_message: &'static str,
    ) -> Result<TrackAddReply, ControlError> {
        let emit_bootstrap_diagnostic =
            !launch_mode || matches!(diagnostic_kind, DiagnosticKind::BootstrapPartial);
        if emit_bootstrap_diagnostic {
            trace_runtime
                .mark_degraded(trace_id)
                .map_err(|error| ControlError::new("mark_degraded", format!("{:?}", error)))?;
        }
        trace_runtime
            .activate_trace(trace_id, SystemTime::now())
            .map_err(|error| ControlError::new("activate_trace", format!("{:?}", error)))?;

        let entry = trace_runtime.get_trace(trace_id).ok_or_else(|| {
            ControlError::new("trace_missing", "trace disappeared after activation")
        })?;
        let trace = entry.trace.clone();
        let memberships = entry.memberships.memberships().cloned().collect::<Vec<_>>();
        if let Err(error) = persist_admitted_trace(
            self.storage.as_mut(),
            trace.clone(),
            memberships,
            process_records,
            self.resource_metrics.external_binding(trace_id),
        ) {
            self.resource_metrics.finish_external_finalization(trace_id);
            return Err(ControlError::new(error.stage, error.message));
        }
        if emit_bootstrap_diagnostic {
            let diagnostic = DiagnosticRecord::new(
                self.next_diagnostic_id()?,
                Some(trace_id),
                diagnostic_kind,
                DiagnosticSeverity::Warning,
                SystemTime::now(),
                diagnostic_message,
            )
            .with_process(root_identity);
            if let Err(error) =
                RecordingWriter::new(self.storage.as_mut()).persist_diagnostic(diagnostic)
            {
                tracing::warn!(trace_id = %trace_id, stage = %error.stage, message = %error.message,
                    "Bootstrap diagnostic storage failed locally");
            }
        }
        if launch_mode {
            self.log_diagnostic(
                DiagnosticLogLevel::Info,
                format_args!(
                    "agent_launch started trace_id={} name={} process_id={}",
                    trace_id,
                    trace.display_name,
                    trace.root_process_identity.get()
                ),
            );
        }

        Ok(TrackAddReply {
            trace_id,
            lifecycle_state: trace.lifecycle_state,
        })
    }

    fn validate_launch_pidfd(
        &self,
        command: &TrackAddCommand,
        pidfd: &OwnedFd,
    ) -> Result<(), ControlError> {
        let observation = resolve_process_ref(&command.root)?;
        let expected_pid = observation
            .host
            .as_ref()
            .map(|host| host.pid)
            .ok_or_else(|| ControlError::new("launch_pidfd", "root host PID is missing"))?;
        let path = format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd());
        let fdinfo = std::fs::read_to_string(&path).map_err(|error| {
            ControlError::new(
                "launch_pidfd",
                format!("read received pidfd metadata {path}: {error}"),
            )
        })?;
        let pid = fdinfo
            .lines()
            .find_map(|line| line.strip_prefix("Pid:"))
            .map(str::trim)
            .ok_or_else(|| {
                ControlError::new(
                    "launch_pidfd",
                    "received descriptor is not a pidfd with a Pid field",
                )
            })?
            .parse::<u32>()
            .map_err(|error| {
                ControlError::new(
                    "launch_pidfd",
                    format!("parse received pidfd target PID: {error}"),
                )
            })?;
        if pid != expected_pid {
            return Err(ControlError::new(
                "launch_pidfd",
                format!(
                    "received pidfd targets daemon PID {pid}, but ProcessRef resolved to {expected_pid}"
                ),
            ));
        }
        Ok(())
    }
}

impl AttachService for StorageAttachService {
    fn host_pid_for_process(&self, process: ProcessIdentity) -> Result<u32, ControlError> {
        self.process_registry
            .record(process)
            .and_then(|record| record.host.as_ref())
            .map(|host| host.pid)
            .ok_or_else(|| {
                ControlError::new(
                    "process_host_pid",
                    format!("process {} has no host PID", process.get()),
                )
            })
    }

    fn resolve_launch_permissions(
        &mut self,
        command: &ResolveLaunchPermissionsCommand,
        host_ebpf_available: bool,
    ) -> Result<LaunchPermissionsReply, ControlError> {
        let profile = self
            .profiles
            .capture_profile(&command.profile_name)
            .ok_or_else(|| {
                ControlError::new("unknown_profile", "capture profile does not exist")
            })?;
        let policy = DeploymentPermissionPolicy {
            host_ebpf: permission_mode(command.host_ebpf),
            seccomp_notify: permission_mode(command.seccomp_notify),
        };
        let launch_seccomp_requirements = self
            .launch_seccomp_requirements
            .with_network_control(
                self.launch_seccomp_requirements.network_control
                    && capability_requested(
                        &profile.capabilities,
                        &Capability::EnforcementNetworkConnectSeccomp,
                    ),
            )
            .with_command_control(
                self.launch_seccomp_requirements.command_control
                    && capability_requested(
                        &profile.capabilities,
                        &Capability::EnforcementCommandExecutionSeccomp,
                    ),
            )
            .with_file_enforcement(self.enforcement.seccomp_requirements()?);
        let decision = resolve_deployment_permissions(
            policy,
            profile,
            launch_seccomp_requirements,
            &DeploymentPermissionAvailability {
                host_ebpf: Some(host_ebpf_available),
                seccomp_notify: Some(command.seccomp_notify_available),
                seccomp_notify_detail: command.seccomp_notify_detail.clone(),
            },
        )
        .map_err(|message| ControlError::new("deployment_permissions", message))?;
        let selected_profile = decision.selected_profile(profile);
        if self
            .profiles
            .capture_profile(&selected_profile.name)
            .is_none()
        {
            return Err(ControlError::new(
                "unknown_profile",
                "daemon did not register the selected deployment profile",
            ));
        }
        let effective_seccomp =
            launch_seccomp_requirements.enabled_by(decision.selected.seccomp_notify);
        Ok(LaunchPermissionsReply {
            requested_host_ebpf: contract_permission_mode(decision.requested_host_ebpf),
            requested_seccomp_notify: contract_permission_mode(decision.requested_seccomp_notify),
            selected_host_ebpf: decision.selected.host_ebpf,
            selected_seccomp_notify: decision.selected.seccomp_notify,
            selected_profile_name: selected_profile.name,
            payload_tls_seccomp: effective_seccomp.payload_tls,
            payload_socket_seccomp: effective_seccomp.payload_socket,
            process_seccomp: effective_seccomp.process_seccomp,
            network_control_seccomp: effective_seccomp.network_control,
            command_control_seccomp: effective_seccomp.command_control,
            file_mkdir_seccomp: effective_seccomp.file_enforcement.mkdir,
            file_rmdir_seccomp: effective_seccomp.file_enforcement.rmdir,
            required_capabilities: decision.required_capabilities,
            degraded: decision.degraded,
            reasons: decision.reasons,
        })
    }

    fn host_ebpf_available_for_profile(&self, profile_name: &model_core::ids::ProfileName) -> bool {
        self.host_ebpf_preflight_available_for_profile(profile_name)
    }

    fn resolve_launch_tls_plan(
        &mut self,
        command: &ResolveLaunchTlsPlanCommand,
        path_view_pid: u32,
    ) -> Result<LaunchTlsPlanReply, ControlError> {
        self.tls_sync
            .resolve_launch_plan(&command.binary, path_view_pid)
    }

    fn attach_existing(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
    ) -> Result<TrackAddReply, ControlError> {
        if command.launch_mode {
            return Err(ControlError::new(
                "launch_pidfd",
                "launch-mode track-add must use pidfd launch registration",
            ));
        }
        self.attach_command(trace_runtime, command, None)
    }

    fn attach_launch(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
        pidfd: OwnedFd,
    ) -> Result<TrackAddReply, ControlError> {
        if !command.launch_mode {
            return Err(ControlError::new(
                "launch_pidfd",
                "pidfd launch registration requires launch_mode=true",
            ));
        }
        self.validate_launch_pidfd(command, &pidfd)?;
        self.attach_command(trace_runtime, command, Some(pidfd))
    }

    fn drain_live_events(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        self.drain_live_events_impl(trace_runtime)
    }

    fn event_poll_fds(&self) -> Result<Vec<RawFd>, ControlError> {
        let mut fds = Vec::new();
        if let Some(fd) = self
            .collector
            .event_poll_fd()
            .map_err(|error| ControlError::new(error.stage, error.message))?
        {
            fds.push(fd);
        }
        fds.extend(self.enforcement.event_poll_fds());
        fds.extend(self.command_control.event_poll_fds());
        fds.extend(self.network_control.event_poll_fds());
        fds.extend(self.tls_sync.event_poll_fds());
        fds.extend(self.seccomp_notify.event_poll_fds());
        fds.push(self.alert_ingress.event_poll_fd());
        fds.push(self.post_trace_broker.event_poll_fd());
        Ok(fds)
    }

    fn background_poll_timeout(&self) -> Result<Option<Duration>, ControlError> {
        let mut timeout = min_optional_timeout(
            self.resource_metrics.poll_timeout(),
            self.idle_detection.poll_timeout(),
        );
        timeout = min_optional_timeout(
            timeout,
            (!self.pending_terminal_finalizations.is_empty()
                || self.post_trace_coordinator.has_running_tasks()
                || self.alert_ingress.has_outstanding_writes()?)
            .then_some(self.finalization_poll_interval),
        );
        timeout = min_optional_timeout(timeout, self.storage_retention.poll_timeout());
        timeout = min_optional_timeout(timeout, self.collector.file_io_poll_timeout());
        timeout = min_optional_timeout(timeout, self.collector.event_transport_loss_poll_timeout());
        Ok(timeout)
    }

    fn shutdown(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        self.shutdown_impl(trace_runtime)
    }

    fn remove_root(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        trace_id: model_core::ids::TraceId,
        removed_at: SystemTime,
    ) -> Result<(), ControlError> {
        self.remove_root_impl(trace_runtime, trace_id, removed_at)
    }

    fn register_seccomp_listener(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: control_contract::command::RegisterSeccompListenerCommand,
    ) -> Result<(), ControlError> {
        let target_observation = resolve_process_ref(&command.target)?;
        let target_pid = target_observation
            .host
            .as_ref()
            .map(|host| host.pid)
            .ok_or_else(|| ControlError::new("seccomp_listener", "target host PID is missing"))?;
        let (target_identity, target_record) =
            self.resolve_process_observation(target_observation)?;
        let trace = trace_runtime
            .get_trace(command.trace_id)
            .ok_or_else(|| ControlError::new("seccomp_listener", "trace not found"))?;
        let target_known = trace
            .memberships
            .memberships()
            .any(|membership| membership.identity == target_identity);
        if !target_known {
            let inherited = self.process_seccomp.ensure_listener_target(
                trace_runtime,
                &self.process_registry,
                &self.identity_reader,
                command.trace_id,
                target_pid,
            )?;
            if let Some(identity) = inherited {
                let record = target_record
                    .or_else(|| self.process_registry.record(identity).cloned())
                    .ok_or_else(|| {
                        ControlError::new("process_registry", "listener process record is missing")
                    })?;
                if self.collector.active_binding_trace_count() > 0 {
                    self.collector
                        .seed_trace_memberships(command.trace_id, std::iter::once(record.clone()))
                        .map_err(|error| ControlError::new(error.stage, error.message))?;
                }
                self.storage
                    .upsert_process_record(record)
                    .map_err(|error| ControlError::new(error.stage, error.message))?;
                self.persist_memberships(
                    trace_runtime,
                    command.trace_id,
                    &BTreeSet::from([identity]),
                )?;
            }
        }
        self.seccomp_notify
            .register_listener(command.trace_id, command.listener_fd)
    }

    fn report_turn_lifecycle(
        &mut self,
        event: agent_lifecycle_contract::TurnLifecycleEvent,
    ) -> Result<(), ControlError> {
        self.report_turn_lifecycle_impl(event)
    }

    fn report_user_interaction(
        &mut self,
        event: agent_lifecycle_contract::UserInteractionEvent,
    ) -> Result<(), ControlError> {
        self.report_user_interaction_impl(event)
    }

    fn report_work_lifecycle(
        &mut self,
        event: agent_lifecycle_contract::WorkLifecycleEvent,
    ) -> Result<(), ControlError> {
        self.report_work_lifecycle_impl(event)
    }

    fn report_session_closed(
        &mut self,
        event: agent_lifecycle_contract::SessionClosedEvent,
    ) -> Result<(), ControlError> {
        self.report_session_closed_impl(event)
    }

    fn plugin_statuses(&self) -> Vec<PluginInstanceStatus> {
        self.plugin_statuses_impl()
    }

    fn load_plugin(
        &mut self,
        command: control_contract::command::PluginLoadCommand,
    ) -> Result<PluginInstanceStatus, ControlError> {
        self.load_plugin_impl(command)
    }

    fn unload_plugin(&mut self, instance_id: &str) -> Result<PluginInstanceStatus, ControlError> {
        self.unload_plugin_impl(instance_id)
    }

    fn handle_plugin_command(
        &mut self,
        command: control_contract::command::PluginCommandCommand,
    ) -> Result<PluginCommandReply, ControlError> {
        if command.instance_id.trim().is_empty() {
            return Err(ControlError::new(
                "plugin_command",
                "plugin instance id must not be empty",
            ));
        }
        if command.argv.is_empty() {
            return Err(ControlError::new(
                "plugin_command",
                "plugin command argv must not be empty",
            ));
        }
        let response = self
            .control_plugins
            .handle_command(
                &command.instance_id,
                plugin_system::PluginCommandRequest { argv: command.argv },
                plugin_system::PluginCommandBudget::default(),
            )
            .map_err(|error| ControlError::new(error.code, error.message))?;
        Ok(PluginCommandReply {
            instance_id: command.instance_id,
            exit_code: response.exit_code,
            stdout: response.stdout,
            stderr: response.stderr,
        })
    }

    fn plugin_config(
        &self,
        instance_id: &str,
    ) -> Result<control_contract::reply::PluginConfigReply, ControlError> {
        self.plugin_config_impl(instance_id)
    }

    fn validate_plugin_config(
        &self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<control_contract::reply::PluginConfigValidationReply, ControlError> {
        self.validate_plugin_config_impl(instance_id, config_json)
    }

    fn update_plugin_config(
        &mut self,
        instance_id: &str,
        config_json: &str,
    ) -> Result<control_contract::reply::PluginConfigReply, ControlError> {
        self.update_plugin_config_impl(instance_id, config_json)
    }
}

fn permission_mode(mode: DeploymentPermissionMode) -> PermissionMode {
    match mode {
        DeploymentPermissionMode::Auto => PermissionMode::Auto,
        DeploymentPermissionMode::Required => PermissionMode::Required,
        DeploymentPermissionMode::Disabled => PermissionMode::Disabled,
    }
}

fn contract_permission_mode(mode: PermissionMode) -> DeploymentPermissionMode {
    match mode {
        PermissionMode::Auto => DeploymentPermissionMode::Auto,
        PermissionMode::Required => DeploymentPermissionMode::Required,
        PermissionMode::Disabled => DeploymentPermissionMode::Disabled,
    }
}

fn min_optional_timeout(left: Option<Duration>, right: Option<Duration>) -> Option<Duration> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(timeout), None) | (None, Some(timeout)) => Some(timeout),
        (None, None) => None,
    }
}

fn persist_admitted_trace(
    storage: &mut dyn storage_core::StorageBackend,
    trace: model_core::trace::TraceRecord,
    memberships: Vec<model_core::process::ProcessMembership>,
    processes: Vec<ProcessRecord>,
    binding: Option<model_core::external_cgroup::ExternalCgroupBinding>,
) -> Result<(), storage_core::StorageError> {
    let transaction = storage.begin()?;
    let result = (|| {
        for record in processes {
            storage.upsert_process_record(record)?;
        }
        storage.create_trace(trace)?;
        for membership in memberships {
            storage.upsert_membership(membership)?;
        }
        if let Some(binding) = binding {
            storage.create_external_cgroup_binding(binding)?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => transaction.commit(),
        Err(error) => {
            transaction.rollback()?;
            Err(error)
        }
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use model_core::container::{ContainerRuntime, NormalizedContainerId};
    use model_core::external_cgroup::{ExternalCgroupBinding, HostBootId};
    use model_core::ids::{OtelTraceId, ProfileName, TraceId, TraceName};
    use model_core::process::{HostProcessCoordinates, ProcessMembership};
    use model_core::trace::{TraceAlertToken, TraceRecord};

    #[test]
    fn binding_failure_rolls_back_trace_processes_and_memberships_then_retry_commits() {
        let temp = tempfile::tempdir().unwrap();
        let mut storage = storage_factory::open_storage_backend(
            &storage_factory::StorageConfig::sqlite_path(temp.path().join("admission.sqlite")),
            storage_core::StorageOpenMode::ReadWrite,
        )
        .unwrap();
        let id = TraceId::new(1);
        let process = ProcessIdentity::new(1);
        let trace = TraceRecord::new(
            id,
            OtelTraceId::from_bytes([1; 16]).unwrap(),
            TraceAlertToken::new([0; 32]),
            process,
            TraceName::new("atomic"),
            ProfileName::new("test"),
            SystemTime::now(),
        );
        let record = ProcessRecord::new(
            process,
            ProcessObservation::host(HostProcessCoordinates::new(42, 900)),
        );
        let member = ProcessMembership::root(id, process, SystemTime::now());
        let mut binding = ExternalCgroupBinding::active(
            id,
            ContainerRuntime::Unknown,
            NormalizedContainerId::from_lower_hex(&"ab".repeat(32)).unwrap(),
            format!("docker/{}", "ab".repeat(32)),
            1,
            2,
            HostBootId::from_bytes([1; 16]),
            SystemTime::now(),
        );
        assert!(
            persist_admitted_trace(
                storage.as_mut(),
                trace.clone(),
                vec![member.clone()],
                vec![record.clone()],
                Some(binding.clone())
            )
            .is_err()
        );
        assert!(storage.get_trace(id).unwrap().is_none());
        assert!(storage.get_process_record(process).unwrap().is_none());
        assert!(storage.trace_memberships(id).unwrap().is_empty());
        assert!(storage.get_external_cgroup_binding(id).unwrap().is_none());
        binding.runtime = ContainerRuntime::Docker;
        persist_admitted_trace(
            storage.as_mut(),
            trace,
            vec![member],
            vec![record],
            Some(binding),
        )
        .unwrap();
        assert!(storage.get_trace(id).unwrap().is_some());
        assert!(storage.get_external_cgroup_binding(id).unwrap().is_some());
        storage.discard_orphan_external_binding(id).unwrap();
        assert!(storage.get_external_cgroup_binding(id).unwrap().is_some());
    }
}
