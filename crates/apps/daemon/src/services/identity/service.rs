//! Shared trace process identity resolution for daemon services.

use std::collections::BTreeMap;
use std::time::SystemTime;

use collector_event::{RawCollectorEvent, RawObservationPayload};
use control_contract::reply::ControlError;
use ingest_runtime::IngestMatch;
use model_core::ids::TraceId;
use model_core::process::{
    ExitObservationSource, ExitStatus, MembershipState, ProcessIdentity, ProcessMembership,
};
use plugin_system::ControlActorProcessIdentity;
use process_identity::ProcessIdentityManager;
use process_identity::{
    IdentityLookupError, ProcessIdentityError, ProcessIdentityReader, ProcessResolution,
};
use storage_core::StorageBackend;
use trace_runtime::registry::{RegistryError, TraceRuntime};

pub(crate) const PROCESS_METADATA_PARENT_PID: &str = "ppid";
pub(crate) const PROCESS_METADATA_SECCOMP_OBSERVED: &str = "seccomp_observed";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedTraceProcess {
    pub(crate) trace_id: TraceId,
    pub(crate) process: ProcessIdentity,
    capture_enabled: bool,
    state: MembershipState,
}

impl ResolvedTraceProcess {
    pub(crate) fn into_ingest_match(self) -> IngestMatch {
        IngestMatch {
            trace_id: self.trace_id,
            process: self.process,
            parent: None,
        }
    }

    pub(crate) fn is_capturable(&self) -> bool {
        self.capture_enabled
            && matches!(
                self.state,
                MembershipState::Starting | MembershipState::Active
            )
    }
}

#[derive(Debug)]
pub(crate) struct SeccompIdentityPreparation {
    pub(crate) resolved: ResolvedTraceProcess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SeccompTaskStatus {
    process_pid: u32,
    parent_pid: u32,
}

pub(crate) struct SeccompNotificationIdentityRegistrar<'a> {
    process_manager: &'a mut ProcessIdentityManager,
    identity_reader: &'a dyn ProcessIdentityReader,
    storage: &'a mut dyn StorageBackend,
    process_id_block_size: u64,
}

impl<'a> SeccompNotificationIdentityRegistrar<'a> {
    pub(crate) fn new(
        process_manager: &'a mut ProcessIdentityManager,
        identity_reader: &'a dyn ProcessIdentityReader,
        storage: &'a mut dyn StorageBackend,
        process_id_block_size: u64,
    ) -> Self {
        Self {
            process_manager,
            identity_reader,
            storage,
            process_id_block_size,
        }
    }

    pub(crate) fn ensure(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        trace_id: TraceId,
        task_id: u32,
    ) -> Result<SeccompIdentityPreparation, ControlError> {
        let task_status = self.read_task_status(task_id)?;
        let process_pid = task_status.process_pid;
        let process = match self.verified_active_process(process_pid)? {
            Some(process) => process,
            None => {
                let observation = self.read_identity(process_pid, "command_control_identity")?;
                let resolution = self.resolve_or_create(observation)?;
                if resolution.created || resolution.enriched {
                    self.persist_process_record(resolution.identity)?;
                }
                resolution.identity
            }
        };
        if let Some(resolved) = TraceIdentityResolver::new(trace_runtime, self.process_manager)
            .match_process_in_trace(trace_id, process)
        {
            return Self::capturable_preparation(resolved);
        }
        if let Some((other_trace_id, _)) = trace_runtime.find_membership(&process) {
            return Err(ControlError::new(
                "command_control_identity",
                format!(
                    "listener trace {trace_id} received task {task_id} from process {process_pid} owned by trace {other_trace_id}"
                ),
            ));
        }
        let parent_pid = task_status.parent_pid;
        let parent_observation =
            self.read_identity(parent_pid, "command_control_parent_identity")?;
        let parent = self
            .process_manager
            .lookup(&parent_observation)
            .map_err(|error| {
                ControlError::new("command_control_parent_identity", format!("{error:?}"))
            })?
            .ok_or_else(|| {
                ControlError::new(
                    "command_control_parent_identity",
                    format!("cannot confirm process generation for parent pid {parent_pid}"),
                )
            })?;
        let parent_membership = trace_runtime
            .find_membership_in_trace(trace_id, &parent)
            .ok_or_else(|| {
                ControlError::new(
                    "command_control_parent_identity",
                    format!("parent pid {parent_pid} is not part of listener trace {trace_id}"),
                )
            })?;
        if !parent_membership.capture_enabled
            || !parent_membership.propagation_enabled
            || matches!(parent_membership.state, MembershipState::IdentityStale)
        {
            return Err(ControlError::new(
                "command_control_parent_identity",
                format!("parent pid {parent_pid} cannot propagate trace membership"),
            ));
        }
        trace_runtime
            .inherit_process(trace_id, &parent, process, SystemTime::now())
            .map_err(|error| {
                ControlError::new("command_control_identity_inherit", format!("{error:?}"))
            })?;
        let membership = trace_runtime
            .find_membership_in_trace(trace_id, &process)
            .ok_or_else(|| {
                ControlError::new(
                    "command_control_identity_inherit",
                    "inherited membership is missing",
                )
            })?;
        self.storage
            .upsert_membership(membership)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        let resolved = TraceIdentityResolver::new(trace_runtime, self.process_manager)
            .match_process_in_trace(trace_id, process)
            .ok_or_else(|| {
                ControlError::new(
                    "command_control_identity_inherit",
                    "cannot resolve inherited membership",
                )
            })?;
        Self::capturable_preparation(resolved)
    }

    fn resolve_or_create(
        &mut self,
        observation: model_core::process::ProcessObservation,
    ) -> Result<ProcessResolution, ControlError> {
        loop {
            match self.process_manager.resolve_or_create(observation.clone()) {
                Ok(resolution) => return Ok(resolution),
                Err(ProcessIdentityError::IdBlockExhausted) => {
                    let (block_start, block_end) = self
                        .storage
                        .reserve_process_id_block(self.process_id_block_size)
                        .map_err(|error| ControlError::new(error.stage, error.message))?;
                    self.process_manager
                        .install_reserved_block(block_start, block_end)
                        .map_err(|error| {
                            ControlError::new(
                                "command_control_identity_block",
                                format!("{error:?}"),
                            )
                        })?;
                }
                Err(error) => {
                    return Err(ControlError::new(
                        "command_control_identity",
                        format!("{error:?}"),
                    ));
                }
            }
        }
    }

    fn persist_process_record(&mut self, process: ProcessIdentity) -> Result<(), ControlError> {
        let record = self
            .process_manager
            .record(process)
            .cloned()
            .ok_or_else(|| {
                ControlError::new(
                    "command_control_identity",
                    format!("process record {} is missing", process.get()),
                )
            })?;
        self.storage
            .upsert_process_record(record)
            .map_err(|error| ControlError::new(error.stage, error.message))
    }

    fn read_identity(
        &self,
        pid: u32,
        stage: &'static str,
    ) -> Result<model_core::process::ProcessObservation, ControlError> {
        self.identity_reader.read_identity(pid).map_err(|error| {
            ControlError::new(
                stage,
                format!("cannot read identity for pid {pid}: {error:?}"),
            )
        })
    }

    fn verified_active_process(
        &self,
        process_pid: u32,
    ) -> Result<Option<ProcessIdentity>, ControlError> {
        let Some(process) = self.process_manager.active_host_pid(process_pid) else {
            return Ok(None);
        };
        let Some(host) = self
            .process_manager
            .record(process)
            .and_then(|record| record.host.as_ref())
        else {
            return Ok(None);
        };
        if host.pid != process_pid || host.start_time_ticks == 0 {
            return Ok(None);
        }
        let raw =
            std::fs::read_to_string(format!("/proc/{process_pid}/stat")).map_err(|error| {
                ControlError::new(
                    "command_control_identity",
                    format!("cannot read stat for process {process_pid}: {error}"),
                )
            })?;
        let start_time_ticks = raw
            .rfind(')')
            .and_then(|close_paren| raw.get(close_paren + 2..))
            .and_then(|remainder| remainder.split_whitespace().nth(19))
            .ok_or_else(|| {
                ControlError::new(
                    "command_control_identity",
                    format!("missing start time for process {process_pid}"),
                )
            })?
            .parse::<u64>()
            .map_err(|error| {
                ControlError::new(
                    "command_control_identity",
                    format!("parse start time for process {process_pid}: {error}"),
                )
            })?;
        Ok((host.start_time_ticks == start_time_ticks).then_some(process))
    }

    fn read_task_status(&self, task_id: u32) -> Result<SeccompTaskStatus, ControlError> {
        let raw = std::fs::read_to_string(format!("/proc/{task_id}/status")).map_err(|error| {
            ControlError::new(
                "command_control_identity",
                format!("cannot read status for task {task_id}: {error}"),
            )
        })?;
        let mut process_pid = None;
        let mut parent_pid = None;
        for line in raw.lines() {
            if process_pid.is_none()
                && let Some(value) = line.strip_prefix("Tgid:")
            {
                process_pid = Some(value.trim().parse::<u32>().map_err(|error| {
                    ControlError::new(
                        "command_control_identity",
                        format!("parse Tgid for task {task_id}: {error}"),
                    )
                })?);
            } else if parent_pid.is_none()
                && let Some(value) = line.strip_prefix("PPid:")
            {
                parent_pid = Some(value.trim().parse::<u32>().map_err(|error| {
                    ControlError::new(
                        "command_control_parent_identity",
                        format!("parse PPid for task {task_id}: {error}"),
                    )
                })?);
            }
            if process_pid.is_some() && parent_pid.is_some() {
                break;
            }
        }
        Ok(SeccompTaskStatus {
            process_pid: process_pid.ok_or_else(|| {
                ControlError::new(
                    "command_control_identity",
                    format!("missing Tgid for task {task_id}"),
                )
            })?,
            parent_pid: parent_pid.ok_or_else(|| {
                ControlError::new(
                    "command_control_parent_identity",
                    format!("missing PPid for task {task_id}"),
                )
            })?,
        })
    }

    fn capturable_preparation(
        resolved: ResolvedTraceProcess,
    ) -> Result<SeccompIdentityPreparation, ControlError> {
        if !resolved.is_capturable() {
            return Err(ControlError::new(
                "command_control_identity",
                format!(
                    "process {} is not capturable in trace {}",
                    resolved.process.get(),
                    resolved.trace_id
                ),
            ));
        }
        Ok(SeccompIdentityPreparation { resolved })
    }
}

pub(crate) struct TraceIdentityResolver<'a> {
    trace_runtime: &'a TraceRuntime,
    process_registry: &'a ProcessIdentityManager,
}

impl<'a> TraceIdentityResolver<'a> {
    pub(crate) fn new(
        trace_runtime: &'a TraceRuntime,
        process_registry: &'a ProcessIdentityManager,
    ) -> Self {
        Self {
            trace_runtime,
            process_registry,
        }
    }

    pub(crate) fn match_process(&self, process: ProcessIdentity) -> Option<ResolvedTraceProcess> {
        self.trace_runtime
            .find_membership(&process)
            .map(Self::resolved_trace_process)
    }

    pub(crate) fn match_process_in_trace(
        &self,
        trace_id: TraceId,
        process: ProcessIdentity,
    ) -> Option<ResolvedTraceProcess> {
        self.trace_runtime
            .find_membership_in_trace(trace_id, &process)
            .map(|membership| Self::resolved_trace_process((trace_id, membership)))
    }

    pub(crate) fn read_and_match_pid(
        &self,
        identity_reader: &impl ProcessIdentityReader,
        pid: u32,
        error_stage: &'static str,
    ) -> Result<Option<ResolvedTraceProcess>, ControlError> {
        let Some(process) = self.read_pid_process(identity_reader, pid, error_stage)? else {
            return Ok(None);
        };
        Ok(self.match_process(process))
    }

    pub(crate) fn runtime_or_read_pid_identity(
        &self,
        identity_reader: &impl ProcessIdentityReader,
        pid: u32,
        error_stage: &'static str,
    ) -> Result<Option<ProcessIdentity>, ControlError> {
        if let Some(process) = self.process_registry.active_host_pid(pid) {
            return Ok(Some(process));
        }
        self.read_pid_process(identity_reader, pid, error_stage)
    }

    fn read_pid_process(
        &self,
        identity_reader: &impl ProcessIdentityReader,
        pid: u32,
        error_stage: &'static str,
    ) -> Result<Option<ProcessIdentity>, ControlError> {
        let observation = match identity_reader.read_identity(pid) {
            Ok(observation) => observation,
            Err(IdentityLookupError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(ControlError::new(error_stage, format!("{error:?}"))),
        };
        self.process_registry
            .lookup(&observation)
            .map_err(|error| ControlError::new(error_stage, format!("{error:?}")))
    }

    fn resolved_trace_process(
        (trace_id, membership): (TraceId, ProcessMembership),
    ) -> ResolvedTraceProcess {
        ResolvedTraceProcess {
            trace_id,
            process: membership.identity,
            capture_enabled: membership.capture_enabled,
            state: membership.state,
        }
    }
}

pub(crate) struct RuntimeProcessEventApplier<'a> {
    trace_runtime: &'a mut TraceRuntime,
    process_manager: &'a mut ProcessIdentityManager,
}

impl<'a> RuntimeProcessEventApplier<'a> {
    pub(crate) fn new(
        trace_runtime: &'a mut TraceRuntime,
        process_manager: &'a mut ProcessIdentityManager,
    ) -> Self {
        Self {
            trace_runtime,
            process_manager,
        }
    }

    pub(crate) fn apply(
        &mut self,
        raw_event: &RawCollectorEvent,
        process: ProcessIdentity,
        parent: Option<ProcessIdentity>,
    ) -> Result<Option<IngestMatch>, ControlError> {
        match &raw_event.payload {
            RawObservationPayload::Process { operation, .. } if operation == "fork" => {
                self.apply_fork(raw_event, process, parent)
            }
            RawObservationPayload::Process {
                operation,
                metadata,
                ..
            } if operation == "exec" || operation == "command_control" => {
                self.apply_exec(raw_event, process, parent, metadata)
            }
            RawObservationPayload::Process {
                operation,
                metadata,
                ..
            } if operation == "exit" => self.apply_exit(raw_event, process, metadata),
            _ => Ok(self
                .match_process_for_event(raw_event, process)
                .map(ResolvedTraceProcess::into_ingest_match)),
        }
    }

    fn apply_fork(
        &mut self,
        raw_event: &RawCollectorEvent,
        child: ProcessIdentity,
        parent: Option<ProcessIdentity>,
    ) -> Result<Option<IngestMatch>, ControlError> {
        let Some(parent) = parent else {
            return Ok(None);
        };
        let Some(matched_parent) = self.match_process_for_event(raw_event, parent) else {
            return Ok(None);
        };
        self.insert_child(
            matched_parent.trace_id,
            parent,
            child,
            raw_event.envelope.observed_at,
        )
    }

    fn apply_exec(
        &mut self,
        raw_event: &RawCollectorEvent,
        process: ProcessIdentity,
        parent: Option<ProcessIdentity>,
        metadata: &BTreeMap<String, String>,
    ) -> Result<Option<IngestMatch>, ControlError> {
        if let Some(matched) = self.match_process_for_event(raw_event, process) {
            return Ok(Some(matched.into_ingest_match()));
        }
        let parent = parent.or_else(|| {
            metadata
                .get(PROCESS_METADATA_PARENT_PID)
                .and_then(|value| value.parse::<u32>().ok())
                .and_then(|pid| self.process_manager.active_host_pid(pid))
        });
        let Some(parent) = parent else {
            return Ok(None);
        };
        let Some(matched_parent) = self.match_process_for_event(raw_event, parent) else {
            return Ok(None);
        };
        self.insert_child(
            matched_parent.trace_id,
            parent,
            process,
            raw_event.envelope.observed_at,
        )
    }

    fn apply_exit(
        &mut self,
        raw_event: &RawCollectorEvent,
        process: ProcessIdentity,
        metadata: &BTreeMap<String, String>,
    ) -> Result<Option<IngestMatch>, ControlError> {
        let Some(matched) = self.match_process_for_event(raw_event, process) else {
            return Ok(None);
        };
        let trace_id = matched.trace_id;
        self.trace_runtime
            .mark_process_exited(
                trace_id,
                &process,
                ExitStatus {
                    code: Self::exit_code(metadata)?,
                    observed_at: raw_event.envelope.observed_at,
                    source: Some(ExitObservationSource::Event),
                },
            )
            .map_err(|error| ControlError::new("mark_process_exited", format!("{error:?}")))?;
        self.process_manager.mark_exited(process);
        Ok(Some(matched.into_ingest_match()))
    }

    fn match_process_for_event(
        &self,
        raw_event: &RawCollectorEvent,
        process: ProcessIdentity,
    ) -> Option<ResolvedTraceProcess> {
        let resolver = TraceIdentityResolver::new(self.trace_runtime, self.process_manager);
        match raw_event.envelope.trace_id {
            Some(trace_id) => resolver.match_process_in_trace(trace_id, process),
            None => resolver.match_process(process),
        }
    }

    fn insert_child(
        &mut self,
        trace_id: TraceId,
        parent: ProcessIdentity,
        child: ProcessIdentity,
        observed_at: SystemTime,
    ) -> Result<Option<IngestMatch>, ControlError> {
        match self
            .trace_runtime
            .insert_observed_child(trace_id, &parent, child, observed_at)
        {
            Ok(()) => {}
            Err(RegistryError::PropagationDisabled(_)) => return Ok(None),
            Err(error) => {
                return Err(ControlError::new(
                    "insert_observed_child",
                    format!("{error:?}"),
                ));
            }
        }
        Ok(Some(IngestMatch {
            trace_id,
            process: child,
            parent: Some(parent),
        }))
    }

    fn exit_code(metadata: &BTreeMap<String, String>) -> Result<Option<i32>, ControlError> {
        metadata
            .get("exit_code")
            .map(|value| {
                value
                    .parse::<i32>()
                    .map_err(|error| ControlError::new("exit_code", error.to_string()))
            })
            .transpose()
    }
}

pub(crate) struct ControlActorIdentityResolver<'a> {
    process_manager: &'a ProcessIdentityManager,
}

impl<'a> ControlActorIdentityResolver<'a> {
    pub(crate) fn new(process_manager: &'a ProcessIdentityManager) -> Self {
        Self { process_manager }
    }

    pub(crate) fn resolve(
        &self,
        process: ProcessIdentity,
    ) -> Result<ControlActorProcessIdentity, ControlError> {
        let record = self.process_manager.record(process).ok_or_else(|| {
            ControlError::new(
                "control_process_identity",
                format!("process record {} is missing", process.get()),
            )
        })?;
        let host = record.host.as_ref().ok_or_else(|| {
            ControlError::new(
                "control_process_identity",
                format!("process {} has no host coordinates", process.get()),
            )
        })?;
        Ok(ControlActorProcessIdentity {
            pid: host.pid,
            task_id: host.task_id,
            generation: host.start_boottime_ns.unwrap_or(host.start_time_ticks),
            namespace: record
                .namespaces
                .iter()
                .next()
                .map(|value| value.pid_namespace.as_str().to_string()),
        })
    }
}
