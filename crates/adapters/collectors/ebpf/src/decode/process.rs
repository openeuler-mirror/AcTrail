//! Process observations projected from decoded kernel events.

use super::{DecodeError, clock, resolve_event_observation, resolve_typed_event_observation};
use crate::loader::{
    KernelEventIdentity, KernelExecPayload, KernelExitPayload, KernelForkPayload,
    KernelObservationCommon, KernelObservationEvent, KernelObservationPayload,
    KernelProcessExecAttemptEvent, KernelProcessExecResultEvent, KernelProcessForkAttemptEvent,
    KernelProcessForkResultEvent, KernelSignalPayload,
};
use crate::maps::BindingStateMap;
use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use model_core::ids::{CollectorName, TraceId};
use model_core::process::{HostProcessCoordinates, ProcessObservation};
use std::collections::BTreeMap;

pub(super) fn decode_fork(
    common: &KernelObservationCommon,
    payload: &KernelForkPayload,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let parent = resolve_fork_observation(common.trace_id, &payload.parent, bindings)
        .map_err(|error| DecodeError::new("parent_identity", error))?;
    let child = resolve_fork_observation(common.trace_id, &common.subject, bindings)
        .map_err(|error| DecodeError::new("fork_identity", error))?;
    bindings.track_with_kernel_tgid(
        common.trace_id,
        child.clone(),
        common.subject.binding_tgid(),
        common.subject.start_boottime_ns,
    );

    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            trace_id: Some(common.trace_id),
            observed_at: clock::wall_from_ktime(common.observed_ktime_ns),
            process: child,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::Process {
            exec_file_identity: None,
            operation: "fork".to_string(),
            parent: Some(parent),
            metadata: BTreeMap::new(),
        },
    }))
}

pub(crate) fn fork_parent_observation(
    event: &KernelObservationEvent,
    bindings: &BindingStateMap,
) -> Result<ProcessObservation, DecodeError> {
    let KernelObservationPayload::Fork(payload) = &event.payload else {
        return Err(DecodeError::new("parent_identity", "event is not fork"));
    };
    resolve_fork_observation(event.common.trace_id, &payload.parent, bindings)
        .map_err(|error| DecodeError::new("parent_identity", error))
}

pub(crate) fn fork_child_observation(
    event: &KernelObservationEvent,
    bindings: &BindingStateMap,
) -> Result<ProcessObservation, DecodeError> {
    let KernelObservationPayload::Fork(_) = &event.payload else {
        return Err(DecodeError::new("fork_identity", "event is not fork"));
    };
    resolve_fork_observation(event.common.trace_id, &event.common.subject, bindings)
        .map_err(|error| DecodeError::new("fork_identity", error))
}

fn resolve_fork_observation(
    trace_id: TraceId,
    identity: &KernelEventIdentity,
    bindings: &BindingStateMap,
) -> Result<ProcessObservation, String> {
    if identity.observer_namespace_tgid == 0
        || identity.kernel_tgid == 0
        || identity.start_boottime_ns == 0
    {
        return Err(
            "fork event requires observer-namespace TGID, kernel TGID, and start boottime"
                .to_string(),
        );
    }
    if let Some(observation) = bindings
        .tracked_event_observation(
            trace_id,
            identity.binding_tgid(),
            identity.start_boottime_ns,
        )
        .cloned()
    {
        return Ok(observation);
    }
    Ok(ProcessObservation::host(
        HostProcessCoordinates::new(identity.observer_namespace_tgid, 0)
            .with_start_boottime_ns(identity.start_boottime_ns),
    ))
}

pub(super) fn decode_exec(
    common: &KernelObservationCommon,
    payload: &KernelExecPayload,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let observation = resolve_typed_event_observation(common, bindings)
        .map_err(|error| DecodeError::new("exec_identity", error))?;
    bindings.track_with_kernel_tgid(
        common.trace_id,
        observation.clone(),
        common.subject.binding_tgid(),
        common.subject.start_boottime_ns,
    );
    let mut metadata = BTreeMap::new();
    if let Some(exec_filename) = &payload.filename {
        metadata.insert("executable".to_string(), exec_filename.path.clone());
        metadata.insert("exec_filename".to_string(), exec_filename.path.clone());
        metadata.insert(
            "exec_filename_source".to_string(),
            "sched_process_exec".to_string(),
        );
        if exec_filename.truncated {
            metadata.insert("exec_filename_truncated".to_string(), "true".to_string());
        }
    }
    if let Some(attempt) = &payload.exec_attempt {
        append_exec_attempt_metadata(&mut metadata, attempt);
        metadata.insert("exec.result".to_string(), "success".to_string());
        metadata.insert("result".to_string(), "0".to_string());
    }

    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            trace_id: Some(common.trace_id),
            observed_at: clock::wall_from_ktime(common.observed_ktime_ns),
            process: observation,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::Process {
            exec_file_identity: payload.exec_file_identity.clone(),
            operation: "exec".to_string(),
            parent: None,
            metadata,
        },
    }))
}

pub(crate) fn decode_process_exec_failure(
    event: KernelProcessExecResultEvent,
    attempt: Option<KernelProcessExecAttemptEvent>,
    bindings: &BindingStateMap,
) -> Result<RawCollectorEvent, DecodeError> {
    let observation = resolve_event_observation(
        event.trace_id,
        event.pid,
        event.host_pid,
        event.pid_generation,
        bindings,
    )
    .map_err(|error| DecodeError::new("exec_identity", error))?;
    let mut metadata = BTreeMap::new();
    metadata.insert("exec.attempt_id".to_string(), event.attempt_id.to_string());
    metadata.insert("exec.result".to_string(), "failed".to_string());
    metadata.insert("result".to_string(), event.result.to_string());
    metadata.insert(
        "errno".to_string(),
        event.result.saturating_neg().to_string(),
    );
    metadata.insert(
        "syscall".to_string(),
        exec_syscall_name(event.syscall).to_string(),
    );
    if let Some(attempt) = attempt {
        append_exec_attempt_metadata(&mut metadata, &attempt);
    }
    Ok(RawCollectorEvent {
        envelope: RawEventEnvelope {
            trace_id: Some(event.trace_id),
            observed_at: clock::wall_from_ktime(event.observed_ktime_ns),
            process: observation,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::Process {
            exec_file_identity: None,
            operation: "exec".to_string(),
            parent: None,
            metadata,
        },
    })
}

pub(crate) fn decode_process_fork_result(
    event: KernelProcessForkResultEvent,
    attempt: Option<KernelProcessForkAttemptEvent>,
    bindings: &BindingStateMap,
) -> Result<RawCollectorEvent, DecodeError> {
    let observation = resolve_event_observation(
        event.trace_id,
        event.pid,
        event.host_pid,
        event.pid_generation,
        bindings,
    )
    .map_err(|error| DecodeError::new("fork_identity", error))?;
    let mut metadata = BTreeMap::new();
    metadata.insert("fork.attempt_id".to_string(), event.attempt_id.to_string());
    metadata.insert(
        "syscall".to_string(),
        fork_syscall_name(event.syscall).to_string(),
    );
    metadata.insert("result".to_string(), event.result.to_string());
    if event.result < 0 {
        metadata.insert(
            "errno".to_string(),
            event.result.saturating_neg().to_string(),
        );
    }
    if let Some(attempt) = attempt {
        metadata.insert("clone.flags".to_string(), attempt.flags.to_string());
        metadata.insert("clone.thread".to_string(), "false".to_string());
        if attempt.syscall == 6 {
            metadata.insert(
                "clone3.args_ptr".to_string(),
                attempt.clone3_args_ptr.to_string(),
            );
            metadata.insert(
                "clone3.args_size".to_string(),
                attempt.clone3_args_size.to_string(),
            );
        }
        if attempt.capture_flags != 0 {
            metadata.insert(
                "fork.capture_flags".to_string(),
                attempt.capture_flags.to_string(),
            );
        }
    } else {
        metadata.insert(
            "fork.capture_flags".to_string(),
            "attempt_missing".to_string(),
        );
    }
    Ok(RawCollectorEvent {
        envelope: RawEventEnvelope {
            trace_id: Some(event.trace_id),
            observed_at: clock::wall_from_ktime(event.observed_ktime_ns),
            process: observation,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::Process {
            exec_file_identity: None,
            operation: "fork_attempt".to_string(),
            parent: None,
            metadata,
        },
    })
}

fn append_exec_attempt_metadata(
    metadata: &mut BTreeMap<String, String>,
    attempt: &KernelProcessExecAttemptEvent,
) {
    metadata.insert(
        "exec.attempt_id".to_string(),
        attempt.attempt_id.to_string(),
    );
    metadata.insert(
        "syscall".to_string(),
        exec_syscall_name(attempt.syscall).to_string(),
    );
    if !attempt.path.is_empty() {
        metadata.insert("executable".to_string(), attempt.path.clone());
        metadata.insert("exec.path".to_string(), attempt.path.clone());
    }
    if !attempt.argv.is_empty() {
        metadata.insert("argv".to_string(), attempt.argv.join("\n"));
        metadata.insert("argv_count".to_string(), attempt.argv.len().to_string());
        metadata.insert("command_line".to_string(), attempt.argv.join(" "));
    }
    if attempt.syscall == 2 {
        metadata.insert(
            "execveat.dirfd".to_string(),
            attempt.execveat_dirfd.to_string(),
        );
        metadata.insert(
            "execveat.flags".to_string(),
            attempt.execveat_flags.to_string(),
        );
    }
    metadata.insert("env_captured".to_string(), "false".to_string());
    metadata.insert(
        "args_truncated".to_string(),
        (attempt.capture_flags != 0).to_string(),
    );
    if attempt.capture_flags != 0 {
        metadata.insert(
            "exec.capture_flags".to_string(),
            attempt.capture_flags.to_string(),
        );
    }
}

fn exec_syscall_name(syscall: u32) -> &'static str {
    match syscall {
        1 => "execve",
        2 => "execveat",
        _ => "unknown",
    }
}

fn fork_syscall_name(syscall: u32) -> &'static str {
    match syscall {
        3 => "fork",
        4 => "vfork",
        5 => "clone",
        6 => "clone3",
        _ => "unknown",
    }
}

pub(super) fn decode_exit(
    common: &KernelObservationCommon,
    payload: &KernelExitPayload,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let observation = resolve_typed_event_observation(common, bindings)
        .map_err(|error| DecodeError::new("exit_identity", error))?;

    let mut metadata = BTreeMap::new();
    if let Some(exit_code) = payload.exit_code {
        metadata.insert("exit_code".to_string(), exit_code.to_string());
    }

    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            trace_id: Some(common.trace_id),
            observed_at: clock::wall_from_ktime(common.observed_ktime_ns),
            process: observation,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::Process {
            exec_file_identity: None,
            operation: "exit".to_string(),
            parent: None,
            metadata,
        },
    }))
}

pub(super) fn decode_signal(
    common: &KernelObservationCommon,
    payload: &KernelSignalPayload,
    bindings: &mut BindingStateMap,
) -> Result<Option<RawCollectorEvent>, DecodeError> {
    let observation = resolve_typed_event_observation(common, bindings)
        .map_err(|error| DecodeError::new("process_coordination_identity", error))?;
    let mut metadata = BTreeMap::from([
        ("operation".to_string(), "signal".to_string()),
        ("result".to_string(), payload.signal_result.to_string()),
        ("syscall".to_string(), "signal_generate".to_string()),
    ]);
    metadata.insert(
        "target_kernel_tid".to_string(),
        payload.target_kernel_tid.to_string(),
    );
    metadata.insert("signal".to_string(), payload.signal.to_string());
    if payload.target_group != 0 {
        metadata.insert("target_group".to_string(), payload.target_group.to_string());
    }
    Ok(Some(RawCollectorEvent {
        envelope: RawEventEnvelope {
            trace_id: Some(common.trace_id),
            observed_at: clock::wall_from_ktime(common.observed_ktime_ns),
            process: observation,
            collector: CollectorName::new("ebpf"),
        },
        payload: RawObservationPayload::Process {
            exec_file_identity: None,
            operation: "signal".to_string(),
            parent: None,
            metadata,
        },
    }))
}
