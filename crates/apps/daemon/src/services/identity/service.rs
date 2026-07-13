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
use process_identity::{IdentityLookupError, ProcessIdentityReader};
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
            _ => Ok(
                TraceIdentityResolver::new(self.trace_runtime, self.process_manager)
                    .match_process(process)
                    .map(ResolvedTraceProcess::into_ingest_match),
            ),
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
        let Some((trace_id, _)) = self.trace_runtime.find_membership(&parent) else {
            return Ok(None);
        };
        self.insert_child(trace_id, parent, child, raw_event.envelope.observed_at)
    }

    fn apply_exec(
        &mut self,
        raw_event: &RawCollectorEvent,
        process: ProcessIdentity,
        parent: Option<ProcessIdentity>,
        metadata: &BTreeMap<String, String>,
    ) -> Result<Option<IngestMatch>, ControlError> {
        if let Some(matched) = TraceIdentityResolver::new(self.trace_runtime, self.process_manager)
            .match_process(process)
        {
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
        let Some((trace_id, _)) = self.trace_runtime.find_membership(&parent) else {
            return Ok(None);
        };
        self.insert_child(trace_id, parent, process, raw_event.envelope.observed_at)
    }

    fn apply_exit(
        &mut self,
        raw_event: &RawCollectorEvent,
        process: ProcessIdentity,
        metadata: &BTreeMap<String, String>,
    ) -> Result<Option<IngestMatch>, ControlError> {
        let Some((trace_id, membership)) = self.trace_runtime.find_membership(&process) else {
            return Ok(None);
        };
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
        Ok(Some(
            TraceIdentityResolver::resolved_trace_process((trace_id, membership))
                .into_ingest_match(),
        ))
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
