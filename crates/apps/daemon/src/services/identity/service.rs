//! Shared trace process identity resolution for daemon services.

use std::collections::BTreeMap;
use std::time::SystemTime;

use collector_event::{RawCollectorEvent, RawObservationPayload};
use control_contract::reply::ControlError;
use ingest_runtime::IngestMatch;
use model_core::ids::TraceId;
use model_core::payload::PayloadSourceBoundary;
use model_core::process::{
    ExitObservationSource, ExitStatus, MembershipState, ProcessIdentity, ProcessMembership,
};
use payload_event::RawPayloadSegment;
use process_identity_contract::lookup::{IdentityLookupError, ProcessIdentityReader};
use trace_runtime::registry::TraceRuntime;

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
}

impl<'a> TraceIdentityResolver<'a> {
    pub(crate) fn new(trace_runtime: &'a TraceRuntime) -> Self {
        Self { trace_runtime }
    }

    pub(crate) fn match_observed_process(
        &self,
        observed: &ProcessIdentity,
    ) -> Option<ResolvedTraceProcess> {
        self.trace_runtime
            .find_membership(observed)
            .map(resolved_trace_process)
    }

    pub(crate) fn match_pid(&self, pid: u32) -> Option<ResolvedTraceProcess> {
        self.trace_runtime
            .find_membership_by_pid(pid)
            .map(resolved_trace_process)
    }

    pub(crate) fn read_and_match_pid(
        &self,
        identity_reader: &impl ProcessIdentityReader,
        pid: u32,
        error_stage: &'static str,
    ) -> Result<Option<ResolvedTraceProcess>, ControlError> {
        let Some(identity) = self.read_pid_identity(identity_reader, pid, error_stage)? else {
            return Ok(None);
        };
        Ok(self.match_observed_process(&identity))
    }

    pub(crate) fn payload_process(&self, raw: &RawPayloadSegment) -> Option<ResolvedTraceProcess> {
        self.match_observed_process(&raw.process)
            .or_else(|| self.tls_userspace_pid_match(raw))
            .or_else(|| self.direct_trace_scoped_tls_sync_process(raw))
    }

    pub(crate) fn runtime_or_read_pid_identity(
        &self,
        identity_reader: &impl ProcessIdentityReader,
        pid: u32,
        error_stage: &'static str,
    ) -> Result<Option<ProcessIdentity>, ControlError> {
        if let Some(resolved) = self.match_pid(pid) {
            return Ok(Some(resolved.process));
        }
        self.read_pid_identity(identity_reader, pid, error_stage)
    }

    fn tls_userspace_pid_match(&self, raw: &RawPayloadSegment) -> Option<ResolvedTraceProcess> {
        if raw.source_boundary != PayloadSourceBoundary::TlsUserSpace {
            return None;
        }
        self.match_pid(raw.process.pid)
    }

    fn direct_trace_scoped_tls_sync_process(
        &self,
        raw: &RawPayloadSegment,
    ) -> Option<ResolvedTraceProcess> {
        if raw.source_boundary != PayloadSourceBoundary::TlsUserSpace {
            return None;
        }
        self.trace_runtime
            .get_trace(raw.trace_id)
            .map(|_| ResolvedTraceProcess {
                trace_id: raw.trace_id,
                process: raw.process.clone(),
                capture_enabled: true,
                state: MembershipState::Active,
            })
    }

    fn read_pid_identity(
        &self,
        identity_reader: &impl ProcessIdentityReader,
        pid: u32,
        error_stage: &'static str,
    ) -> Result<Option<ProcessIdentity>, ControlError> {
        match identity_reader.read_identity(pid) {
            Ok(identity) => Ok(Some(identity)),
            Err(IdentityLookupError::NotFound { .. }) => Ok(None),
            Err(error) => Err(ControlError::new(error_stage, format!("{error:?}"))),
        }
    }
}

impl TraceIdentityResolver<'_> {
    pub(crate) fn apply_runtime_effects(
        trace_runtime: &mut TraceRuntime,
        raw_event: &RawCollectorEvent,
    ) -> Result<Option<IngestMatch>, ControlError> {
        match &raw_event.payload {
            RawObservationPayload::Process {
                operation, parent, ..
            } if operation == "fork" => fork_effect(trace_runtime, raw_event, parent.as_ref()),
            RawObservationPayload::Process {
                operation,
                parent,
                metadata,
            } if operation == "exec" || operation == "command_control" => {
                exec_effect(trace_runtime, raw_event, parent.as_ref(), metadata)
            }
            RawObservationPayload::Process {
                operation,
                metadata,
                ..
            } if operation == "exit" => exit_effect(trace_runtime, raw_event, metadata),
            _ => Ok(TraceIdentityResolver::new(trace_runtime)
                .match_observed_process(&raw_event.envelope.process)
                .map(ResolvedTraceProcess::into_ingest_match)),
        }
    }
}

fn fork_effect(
    trace_runtime: &mut TraceRuntime,
    raw_event: &RawCollectorEvent,
    parent: Option<&ProcessIdentity>,
) -> Result<Option<IngestMatch>, ControlError> {
    let Some(parent_identity) = parent else {
        return Ok(None);
    };
    let Some(parent_match) =
        TraceIdentityResolver::new(trace_runtime).match_observed_process(parent_identity)
    else {
        return Ok(None);
    };
    insert_observed_child_match(
        trace_runtime,
        parent_match.trace_id,
        parent_identity,
        raw_event.envelope.process.clone(),
        raw_event.envelope.observed_at,
    )
    .map(Some)
}

fn exec_effect(
    trace_runtime: &mut TraceRuntime,
    raw_event: &RawCollectorEvent,
    parent: Option<&ProcessIdentity>,
    metadata: &BTreeMap<String, String>,
) -> Result<Option<IngestMatch>, ControlError> {
    let identity = raw_event.envelope.process.clone();
    if metadata
        .get(PROCESS_METADATA_SECCOMP_OBSERVED)
        .is_some_and(|value| value == "true")
    {
        if let Some(matched) = TraceIdentityResolver::new(trace_runtime).match_pid(identity.pid) {
            return Ok(Some(matched.into_ingest_match()));
        }
    }
    if let Some(matched) =
        TraceIdentityResolver::new(trace_runtime).match_observed_process(&identity)
    {
        return Ok(Some(matched.into_ingest_match()));
    }
    if let Some(parent_identity) = parent {
        if let Some(parent_match) =
            TraceIdentityResolver::new(trace_runtime).match_pid(parent_identity.pid)
        {
            return insert_observed_child_match(
                trace_runtime,
                parent_match.trace_id,
                &parent_match.process,
                identity,
                raw_event.envelope.observed_at,
            )
            .map(Some);
        }
    }
    if let Some(parent_pid) = metadata
        .get(PROCESS_METADATA_PARENT_PID)
        .and_then(|value| value.parse().ok())
    {
        if let Some(parent_match) = TraceIdentityResolver::new(trace_runtime).match_pid(parent_pid)
        {
            return insert_observed_child_match(
                trace_runtime,
                parent_match.trace_id,
                &parent_match.process,
                identity,
                raw_event.envelope.observed_at,
            )
            .map(Some);
        }
    }
    Ok(trace_runtime
        .refresh_process_identity(identity)
        .map(|(trace_id, process)| IngestMatch { trace_id, process }))
}

fn exit_effect(
    trace_runtime: &mut TraceRuntime,
    raw_event: &RawCollectorEvent,
    metadata: &BTreeMap<String, String>,
) -> Result<Option<IngestMatch>, ControlError> {
    let Some(matched) = TraceIdentityResolver::new(trace_runtime)
        .match_observed_process(&raw_event.envelope.process)
    else {
        return Ok(None);
    };
    trace_runtime
        .mark_process_exited(
            matched.trace_id,
            &matched.process,
            ExitStatus {
                code: exit_code(metadata)?,
                observed_at: raw_event.envelope.observed_at,
                source: Some(ExitObservationSource::Event),
            },
        )
        .map_err(|error| ControlError::new("mark_process_exited", format!("{error:?}")))?;
    Ok(Some(matched.into_ingest_match()))
}

fn insert_observed_child_match(
    trace_runtime: &mut TraceRuntime,
    trace_id: TraceId,
    parent: &ProcessIdentity,
    child: ProcessIdentity,
    observed_at: SystemTime,
) -> Result<IngestMatch, ControlError> {
    trace_runtime
        .insert_observed_child(trace_id, parent, child.clone(), observed_at)
        .map_err(|error| ControlError::new("insert_observed_child", format!("{error:?}")))?;
    Ok(IngestMatch {
        trace_id,
        process: child,
    })
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
