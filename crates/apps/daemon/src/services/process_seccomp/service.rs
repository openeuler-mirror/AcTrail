//! Process-control seccomp observation and launch-child registration.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use config_core::daemon::{ProcessSeccompConfig, ProcessSeccompSyscall};
use control_contract::reply::ControlError;
use linux_platform::process_seccomp::KernelProcessSyscall;
use model_core::ids::{CollectorName, TraceId};
use model_core::process::ProcessIdentity;
use process_identity_contract::lookup::ProcessIdentityReader;
use trace_runtime::registry::TraceRuntime;

use super::clone_flags::{clone_flags, is_thread_clone};
use super::procfs::{absolute_exec_path_missing, parent_pid};
use super::remote_args::{
    ExecArgs, ExecPath, read_execve_args, read_execve_path, read_execveat_args, read_execveat_path,
};
use super::syscall::{effective_syscalls, syscall_from_notification, syscall_name};
use crate::services::identity::{
    PROCESS_METADATA_PARENT_PID, PROCESS_METADATA_SECCOMP_OBSERVED, TraceIdentityResolver,
};
use crate::services::seccomp_notify::NotificationContinuation;

pub(crate) const PROCESS_SECCOMP_COLLECTOR_NAME: &str = "process-seccomp";

#[derive(Debug)]
pub(crate) struct ProcessSeccompService {
    enabled: bool,
    syscalls: BTreeSet<KernelProcessSyscall>,
    max_args: u32,
    max_arg_bytes: u32,
    pending_max_entries: u32,
}

impl ProcessSeccompService {
    pub(crate) fn new(config: &ProcessSeccompConfig) -> Self {
        Self {
            enabled: config.enabled,
            syscalls: effective_syscalls(config.syscalls.iter().copied())
                .unwrap_or_else(|error| panic!("build process seccomp syscall map: {error:?}")),
            max_args: config.max_args,
            max_arg_bytes: config.max_arg_bytes,
            pending_max_entries: config.pending_max_entries,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn ensure_pending_observation_capacity(
        &self,
        pending_len: usize,
    ) -> Result<(), ControlError> {
        let limit = usize::try_from(self.pending_max_entries).map_err(|error| {
            ControlError::new(
                "process_seccomp_pending",
                format!("pending observation limit overflow: {error}"),
            )
        })?;
        if pending_len > limit {
            return Err(ControlError::new(
                "process_seccomp_pending",
                format!(
                    "pending process seccomp observations {pending_len} exceed configured limit {limit}"
                ),
            ));
        }
        Ok(())
    }

    pub(crate) fn ensure_listener_target(
        &self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        identity_reader: &impl ProcessIdentityReader,
        trace_id: TraceId,
        target_pid: u32,
    ) -> Result<Option<ProcessIdentity>, ControlError> {
        if trace_runtime
            .find_membership_by_pid(target_pid)
            .is_some_and(|(found_trace_id, _)| found_trace_id == trace_id)
        {
            return Ok(None);
        }
        let Some(target_identity) = identity(trace_runtime, identity_reader, target_pid)? else {
            return Ok(None);
        };
        let Some(ppid) = parent_pid(target_pid)? else {
            return Ok(None);
        };
        let Some((parent_trace_id, parent)) = trace_runtime.find_membership_by_pid(ppid) else {
            return Err(ControlError::new(
                "seccomp_listener",
                "seccomp listener target parent is not part of the trace",
            ));
        };
        if parent_trace_id != trace_id {
            return Err(ControlError::new(
                "seccomp_listener",
                "seccomp listener target parent belongs to a different trace",
            ));
        }
        trace_runtime
            .inherit_process(
                trace_id,
                &parent.identity,
                target_identity.clone(),
                SystemTime::now(),
            )
            .map_err(|error| ControlError::new("process_seccomp_inherit", format!("{error:?}")))?;
        Ok(Some(target_identity))
    }

    pub(crate) fn handle_notification(
        &self,
        trace_runtime: &TraceRuntime,
        identity_reader: &impl ProcessIdentityReader,
        notification: &libc::seccomp_notif,
        continuation: &mut NotificationContinuation,
    ) -> Result<Vec<ProcessSeccompObservation>, ControlError> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        let Some(syscall) = syscall_from_notification(notification)? else {
            return Ok(Vec::new());
        };
        if !self.syscalls.contains(&syscall) {
            return Ok(Vec::new());
        }
        let syscall = syscall.as_configured_syscall();
        match syscall {
            ProcessSeccompSyscall::Execve => {
                let observed_at = SystemTime::now();
                let path = read_execve_path(
                    notification.pid,
                    notification.data.args[0],
                    self.max_arg_bytes,
                )?;
                if skip_missing_exec_candidate(notification.pid, &path) {
                    continuation.continue_now()?;
                    return Ok(Vec::new());
                }
                let args = read_execve_args(
                    notification.pid,
                    path,
                    notification.data.args[1],
                    self.max_args,
                    self.max_arg_bytes,
                )?;
                let Some(process) = identity(trace_runtime, identity_reader, notification.pid)?
                else {
                    continuation.continue_now()?;
                    return Ok(Vec::new());
                };
                let parent_pid = parent_pid(notification.pid)?;
                continuation.continue_now()?;
                Ok(vec![ProcessSeccompObservation {
                    observed_at,
                    process,
                    parent_pid,
                    syscall,
                    details: ProcessSeccompObservationDetails::Exec {
                        args,
                        execveat_dirfd: None,
                        execveat_flags: None,
                    },
                }])
            }
            ProcessSeccompSyscall::Execveat => {
                let observed_at = SystemTime::now();
                let path = read_execveat_path(
                    notification.pid,
                    notification.data.args[1],
                    self.max_arg_bytes,
                )?;
                if skip_missing_exec_candidate(notification.pid, &path) {
                    continuation.continue_now()?;
                    return Ok(Vec::new());
                }
                let args = read_execveat_args(
                    notification.pid,
                    path,
                    notification.data.args[2],
                    self.max_args,
                    self.max_arg_bytes,
                )?;
                let Some(process) = identity(trace_runtime, identity_reader, notification.pid)?
                else {
                    continuation.continue_now()?;
                    return Ok(Vec::new());
                };
                let parent_pid = parent_pid(notification.pid)?;
                continuation.continue_now()?;
                Ok(vec![ProcessSeccompObservation {
                    observed_at,
                    process,
                    parent_pid,
                    syscall,
                    details: ProcessSeccompObservationDetails::Exec {
                        args,
                        execveat_dirfd: Some(notification.data.args[0]),
                        execveat_flags: Some(notification.data.args[4]),
                    },
                }])
            }
            ProcessSeccompSyscall::Fork
            | ProcessSeccompSyscall::Vfork
            | ProcessSeccompSyscall::Clone
            | ProcessSeccompSyscall::Clone3 => {
                let observed_at = SystemTime::now();
                let flags = clone_flags(notification, syscall)?;
                if is_thread_clone(flags) {
                    continuation.continue_now()?;
                    return Ok(Vec::new());
                }
                let Some(process) = identity(trace_runtime, identity_reader, notification.pid)?
                else {
                    continuation.continue_now()?;
                    return Ok(Vec::new());
                };
                continuation.continue_now()?;
                Ok(vec![ProcessSeccompObservation {
                    observed_at,
                    process,
                    parent_pid: None,
                    syscall,
                    details: ProcessSeccompObservationDetails::ForkAttempt {
                        flags,
                        clone3_args_ptr: matches!(syscall, ProcessSeccompSyscall::Clone3)
                            .then_some(notification.data.args[0]),
                        clone3_args_size: matches!(syscall, ProcessSeccompSyscall::Clone3)
                            .then_some(notification.data.args[1]),
                    },
                }])
            }
        }
    }

    pub(crate) fn materialize_observation(
        &self,
        trace_runtime: &TraceRuntime,
        observation: ProcessSeccompObservation,
    ) -> RawCollectorEvent {
        let process = trace_runtime
            .find_membership_by_pid(observation.process.pid)
            .map(|(_, membership)| membership.identity)
            .unwrap_or_else(|| observation.process.clone());
        match observation.details {
            ProcessSeccompObservationDetails::Exec {
                args,
                execveat_dirfd,
                execveat_flags,
            } => {
                let parent = observation
                    .parent_pid
                    .and_then(|pid| trace_runtime.find_membership_by_pid(pid))
                    .map(|(_, membership)| membership.identity);
                self.exec_event(
                    observation.observed_at,
                    process,
                    observation.parent_pid,
                    parent,
                    observation.syscall,
                    args,
                    execveat_dirfd,
                    execveat_flags,
                )
            }
            ProcessSeccompObservationDetails::ForkAttempt {
                flags,
                clone3_args_ptr,
                clone3_args_size,
            } => self.fork_attempt_event(
                observation.observed_at,
                process,
                observation.syscall,
                flags,
                clone3_args_ptr,
                clone3_args_size,
            ),
        }
    }

    fn exec_event(
        &self,
        observed_at: SystemTime,
        process: ProcessIdentity,
        parent_pid: Option<u32>,
        parent: Option<ProcessIdentity>,
        syscall: ProcessSeccompSyscall,
        args: ExecArgs,
        execveat_dirfd: Option<u64>,
        execveat_flags: Option<u64>,
    ) -> RawCollectorEvent {
        let mut metadata = common_metadata(syscall);
        if let Some(parent_pid) = parent_pid {
            metadata.insert(
                PROCESS_METADATA_PARENT_PID.to_string(),
                parent_pid.to_string(),
            );
        }
        if let Some(path) = args.path.filter(|value| !value.is_empty()) {
            metadata.insert("executable".to_string(), path.clone());
            metadata.insert("exec.path".to_string(), path);
        }
        if !args.argv.is_empty() {
            metadata.insert("argv".to_string(), args.argv.join("\n"));
            metadata.insert("argv_count".to_string(), args.argv.len().to_string());
            metadata.insert("command_line".to_string(), args.argv.join(" "));
        }
        if let Some(dirfd) = execveat_dirfd {
            metadata.insert("execveat.dirfd".to_string(), dirfd.to_string());
        }
        if let Some(flags) = execveat_flags {
            metadata.insert("execveat.flags".to_string(), flags.to_string());
        }
        metadata.insert("env_captured".to_string(), "false".to_string());
        metadata.insert("args_truncated".to_string(), args.truncated.to_string());
        process_event(observed_at, process, "exec", parent, metadata)
    }

    fn fork_attempt_event(
        &self,
        observed_at: SystemTime,
        process: ProcessIdentity,
        syscall: ProcessSeccompSyscall,
        flags: Option<u64>,
        clone3_args_ptr: Option<u64>,
        clone3_args_size: Option<u64>,
    ) -> RawCollectorEvent {
        let mut metadata = common_metadata(syscall);
        if let Some(flags) = flags {
            metadata.insert("clone.flags".to_string(), flags.to_string());
            metadata.insert(
                "clone.thread".to_string(),
                ((flags & libc::CLONE_THREAD as u64) != 0).to_string(),
            );
        }
        if let Some(args_ptr) = clone3_args_ptr {
            metadata.insert("clone3.args_ptr".to_string(), args_ptr.to_string());
        }
        if let Some(args_size) = clone3_args_size {
            metadata.insert("clone3.args_size".to_string(), args_size.to_string());
        }
        process_event(observed_at, process, "fork_attempt", None, metadata)
    }
}

#[derive(Debug)]
pub(crate) struct ProcessSeccompObservation {
    observed_at: SystemTime,
    process: ProcessIdentity,
    parent_pid: Option<u32>,
    syscall: ProcessSeccompSyscall,
    details: ProcessSeccompObservationDetails,
}

#[derive(Debug)]
enum ProcessSeccompObservationDetails {
    Exec {
        args: ExecArgs,
        execveat_dirfd: Option<u64>,
        execveat_flags: Option<u64>,
    },
    ForkAttempt {
        flags: Option<u64>,
        clone3_args_ptr: Option<u64>,
        clone3_args_size: Option<u64>,
    },
}

fn identity(
    trace_runtime: &TraceRuntime,
    identity_reader: &impl ProcessIdentityReader,
    pid: u32,
) -> Result<Option<ProcessIdentity>, ControlError> {
    TraceIdentityResolver::new(trace_runtime).runtime_or_read_pid_identity(
        identity_reader,
        pid,
        "process_seccomp_identity",
    )
}

fn common_metadata(syscall: ProcessSeccompSyscall) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            PROCESS_METADATA_SECCOMP_OBSERVED.to_string(),
            "true".to_string(),
        ),
        ("syscall".to_string(), syscall_name(syscall).to_string()),
    ])
}

fn skip_missing_exec_candidate(pid: u32, path: &ExecPath) -> bool {
    if path.truncated {
        return false;
    }
    path.path
        .as_deref()
        .is_some_and(|value| absolute_exec_path_missing(pid, value))
}

fn process_event(
    observed_at: SystemTime,
    process: ProcessIdentity,
    operation: &str,
    parent: Option<ProcessIdentity>,
    metadata: BTreeMap<String, String>,
) -> RawCollectorEvent {
    RawCollectorEvent {
        envelope: RawEventEnvelope {
            observed_at,
            process,
            collector: CollectorName::new(PROCESS_SECCOMP_COLLECTOR_NAME),
        },
        payload: RawObservationPayload::Process {
            operation: operation.to_string(),
            parent,
            metadata,
        },
    }
}
