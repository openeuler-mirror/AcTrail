//! Capability-scoped eBPF program autoload planning.

use std::collections::BTreeSet;

use config_core::daemon::{EbpfCollectorConfig, PayloadConfig};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};

use super::LoaderError;
use super::tls;

const PROC_LIFECYCLE_PROGRAMS: &[&str] = &[
    "handle_sched_process_fork",
    "handle_sched_process_exec",
    "handle_sched_process_exit",
    "handle_sys_enter_exit",
    "handle_sys_enter_exit_group",
];

const PROCESS_SIGNAL_DIAGNOSTIC_PROGRAMS: &[&str] = &["handle_signal_generate"];

const PROCESS_CONTEXT_PROGRAMS: &[&str] = &[
    "handle_sched_process_fork",
    "handle_sched_process_exec",
    "handle_sched_process_exit",
];

const NET_TRANSPORT_PROGRAMS: &[&str] = &[
    "handle_sys_enter_connect",
    "handle_sys_exit_connect",
    "handle_sys_enter_accept",
    "handle_sys_enter_accept4",
    "handle_sys_exit_accept",
    "handle_sys_exit_accept4",
    "handle_sys_enter_sendto",
    "handle_sys_exit_sendto",
    "handle_sys_enter_writev",
    "handle_sys_exit_writev",
    "handle_sys_enter_sendmsg",
    "handle_sys_exit_sendmsg",
    "handle_sys_enter_recvfrom",
    "handle_sys_exit_recvfrom",
    "handle_sys_enter_bind",
    "handle_sys_exit_bind",
    "handle_sys_enter_listen",
    "handle_sys_exit_listen",
    "handle_sys_enter_write",
    "handle_sys_exit_write",
    "handle_sys_enter_read",
    "handle_sys_exit_read",
];

const FS_ACCESS_BASIC_FD_PROGRAMS: &[&str] = &[
    "handle_sys_enter_writev",
    "handle_sys_exit_writev",
    "handle_sys_enter_write",
    "handle_sys_exit_write",
    "handle_sys_enter_read",
    "handle_sys_exit_read",
];

const FS_ACCESS_BASIC_PATH_PROGRAMS: &[&str] = &[
    "handle_sys_enter_open",
    "handle_sys_exit_open",
    "handle_sys_enter_openat",
    "handle_sys_exit_openat",
    "handle_sys_enter_openat2",
    "handle_sys_exit_openat2",
    "handle_sys_enter_creat",
    "handle_sys_exit_creat",
    "handle_sys_enter_unlinkat",
    "handle_sys_exit_unlinkat",
    "handle_sys_enter_renameat",
    "handle_sys_exit_renameat",
    "handle_sys_enter_mkdirat",
    "handle_sys_exit_mkdirat",
];

const FS_ACCESS_BASIC_CONTEXT_PROGRAMS: &[&str] = &[
    "handle_sys_enter_close",
    "handle_sys_exit_close",
    "handle_sys_enter_dup",
    "handle_sys_exit_dup",
    "handle_sys_enter_dup2",
    "handle_sys_exit_dup2",
    "handle_sys_enter_dup3",
    "handle_sys_exit_dup3",
    "handle_sys_enter_fcntl",
    "handle_sys_exit_fcntl",
    "handle_sys_enter_chdir",
    "handle_sys_exit_chdir",
    "handle_sys_enter_fchdir",
    "handle_sys_exit_fchdir",
];

const PLATFORM_OPTIONAL_TRACEPOINT_PROGRAMS: &[&str] = &[
    "handle_sys_enter_dup2",
    "handle_sys_exit_dup2",
    "handle_sys_enter_dup3",
    "handle_sys_exit_dup3",
    "handle_sys_enter_open",
    "handle_sys_exit_open",
    "handle_sys_enter_openat2",
    "handle_sys_exit_openat2",
    "handle_sys_enter_creat",
    "handle_sys_exit_creat",
    "handle_sys_enter_pipe",
    "handle_sys_exit_pipe",
];

/// Programs that carry stdio chunk payloads: read/write on any fd. A trace
/// that attaches only these (and not the pipe/socketpair programs) satisfies
/// `StdioChunk` without satisfying `IpcPipeFifo`/`IpcUnixSocket`.
const STDIO_PROGRAMS: &[&str] = &[
    "handle_sys_enter_write",
    "handle_sys_exit_write",
    "handle_sys_enter_read",
    "handle_sys_exit_read",
];

/// Programs that create IPC channels (pipe/pipe2/socketpair). Required for
/// `IpcPipeFifo` and `IpcUnixSocket` to be considered satisfied.
const IPC_PROGRAMS: &[&str] = &[
    "handle_sys_enter_pipe",
    "handle_sys_exit_pipe",
    "handle_sys_enter_pipe2",
    "handle_sys_exit_pipe2",
    "handle_sys_enter_socketpair",
    "handle_sys_exit_socketpair",
];

const SOCKET_PAYLOAD_PROGRAMS: &[&str] = &[
    "handle_sys_enter_connect",
    "handle_sys_exit_connect",
    "handle_sys_enter_accept",
    "handle_sys_enter_accept4",
    "handle_sys_exit_accept",
    "handle_sys_exit_accept4",
    "handle_sys_enter_sendto",
    "handle_sys_exit_sendto",
    "handle_sys_enter_writev",
    "handle_sys_exit_writev",
    "handle_sys_enter_sendmsg",
    "handle_sys_exit_sendmsg",
    "handle_sys_enter_recvfrom",
    "handle_sys_exit_recvfrom",
    "handle_sys_enter_write",
    "handle_sys_exit_write",
    "handle_sys_enter_read",
    "handle_sys_exit_read",
    "handle_sys_enter_close",
    "handle_sys_exit_close",
    "handle_sys_enter_dup",
    "handle_sys_exit_dup",
    "handle_sys_enter_dup2",
    "handle_sys_exit_dup2",
    "handle_sys_enter_dup3",
    "handle_sys_exit_dup3",
    "handle_sys_enter_fcntl",
    "handle_sys_exit_fcntl",
];

const FS_MMAP_PROGRAMS: &[&str] = &["handle_sys_enter_mmap", "handle_sys_exit_mmap"];

const FILE_ATTACH_PRIORITY: u8 = 0;
const SHARED_FD_ATTACH_PRIORITY: u8 = 1;
const PROCESS_ATTACH_PRIORITY: u8 = 2;
const DEFAULT_ATTACH_PRIORITY: u8 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachPlan {
    capabilities: BTreeSet<Capability>,
    file_path_capture_enabled: bool,
    dynamic_go_tls_enabled: bool,
}

impl AttachPlan {
    pub fn baseline() -> Self {
        Self {
            capabilities: BTreeSet::new(),
            file_path_capture_enabled: false,
            dynamic_go_tls_enabled: false,
        }
    }

    pub fn from_requests(
        requests: &[CapabilityRequest],
        config: &EbpfCollectorConfig,
        payload: &PayloadConfig,
    ) -> Self {
        let mut plan = Self::baseline();
        for request in requests {
            if request.mode == RequestMode::Disabled {
                continue;
            }
            if capability_configured_for_attach(&request.capability, payload) {
                plan.capabilities.insert(request.capability.clone());
            }
        }
        plan.file_path_capture_enabled = config.file_path_capture_enabled;
        plan.dynamic_go_tls_enabled = payload.tls.enabled && payload.tls.capture_backend.is_sync();
        plan
    }

    pub fn contains(&self, capability: &Capability) -> bool {
        self.capabilities.contains(capability)
    }

    pub fn should_load_program(&self, program_name: &str) -> Result<bool, LoaderError> {
        if tls::is_payload_tls_program(program_name) {
            return Ok(self.contains(&Capability::TlsPlaintextPayload)
                || (self.dynamic_go_tls_enabled && tls::is_go_tls_program(program_name)));
        }
        if capability_programs(program_name).is_none() {
            return Err(LoaderError::new(
                "attach_plan",
                format!("BPF program {program_name} has no capability mapping"),
            ));
        }
        Ok(self
            .capabilities
            .iter()
            .any(|capability| self.capability_loads_program(capability, &program_name)))
    }

    pub fn attached_capabilities(&self, attached_programs: &[String]) -> BTreeSet<Capability> {
        self.capabilities
            .iter()
            .filter(|capability| self.capability_satisfied_by(capability, attached_programs))
            .cloned()
            .collect()
    }

    pub(crate) fn is_satisfied_by(&self, attached_capabilities: &BTreeSet<Capability>) -> bool {
        self.capabilities.is_subset(attached_capabilities)
    }

    pub(crate) fn attach_priority(&self, program_name: &str) -> u8 {
        if FS_ACCESS_BASIC_PATH_PROGRAMS.contains(&program_name)
            || FS_ACCESS_BASIC_CONTEXT_PROGRAMS.contains(&program_name)
            || FS_MMAP_PROGRAMS.contains(&program_name)
        {
            return FILE_ATTACH_PRIORITY;
        }
        if FS_ACCESS_BASIC_FD_PROGRAMS.contains(&program_name) {
            return SHARED_FD_ATTACH_PRIORITY;
        }
        if PROC_LIFECYCLE_PROGRAMS.contains(&program_name) {
            return PROCESS_ATTACH_PRIORITY;
        }
        DEFAULT_ATTACH_PRIORITY
    }

    pub(crate) fn allows_missing_tracepoint(&self, program_name: &str) -> bool {
        PLATFORM_OPTIONAL_TRACEPOINT_PROGRAMS.contains(&program_name)
    }

    pub(crate) fn dynamic_go_tls_enabled(&self) -> bool {
        self.dynamic_go_tls_enabled
    }

    fn capability_loads_program(&self, capability: &Capability, program_name: &str) -> bool {
        match capability {
            Capability::FsAccessBasic => {
                FS_ACCESS_BASIC_FD_PROGRAMS.contains(&program_name)
                    || (self.file_path_capture_enabled
                        && (FS_ACCESS_BASIC_PATH_PROGRAMS.contains(&program_name)
                            || FS_ACCESS_BASIC_CONTEXT_PROGRAMS.contains(&program_name)
                            || PROCESS_CONTEXT_PROGRAMS.contains(&program_name)))
            }
            Capability::FsMmap => {
                FS_MMAP_PROGRAMS.contains(&program_name)
                    || PROCESS_CONTEXT_PROGRAMS.contains(&program_name)
                    || (self.file_path_capture_enabled
                        && FS_ACCESS_BASIC_CONTEXT_PROGRAMS.contains(&program_name))
            }
            _ => capability_required_programs(capability)
                .is_some_and(|programs| programs.contains(&program_name)),
        }
    }

    fn capability_satisfied_by(
        &self,
        capability: &Capability,
        attached_programs: &[String],
    ) -> bool {
        if matches!(capability, Capability::TlsPlaintextPayload) {
            return attached_programs
                .iter()
                .any(|program| tls::is_payload_tls_program(program));
        }
        if matches!(capability, Capability::FsAccessBasic) {
            return programs_attached(FS_ACCESS_BASIC_FD_PROGRAMS, attached_programs)
                && (!self.file_path_capture_enabled
                    || (required_programs_attached(
                        FS_ACCESS_BASIC_PATH_PROGRAMS,
                        attached_programs,
                    ) && required_programs_attached(
                        FS_ACCESS_BASIC_CONTEXT_PROGRAMS,
                        attached_programs,
                    ) && programs_attached(PROCESS_CONTEXT_PROGRAMS, attached_programs)));
        }
        if matches!(capability, Capability::FsMmap) {
            return programs_attached(FS_MMAP_PROGRAMS, attached_programs)
                && programs_attached(PROCESS_CONTEXT_PROGRAMS, attached_programs);
        }
        capability_required_programs(capability)
            .is_some_and(|programs| required_programs_attached(programs, attached_programs))
    }
}

pub(super) fn configure_program_autoload(
    open_object: &mut libbpf_rs::OpenObject,
    attach_plan: &AttachPlan,
) -> Result<(), LoaderError> {
    for mut program in open_object.progs_mut() {
        let program_name = program.name().to_string_lossy().into_owned();
        program.set_autoload(attach_plan.should_load_program(&program_name)?);
    }
    Ok(())
}

pub(super) fn effective_config_for_attach_plan(
    payload: &PayloadConfig,
    attach_plan: &AttachPlan,
) -> PayloadConfig {
    let mut effective = payload.clone();
    if !attach_plan.contains(&Capability::TlsPlaintextPayload)
        && !attach_plan.dynamic_go_tls_enabled()
    {
        effective.tls.enabled = false;
    }
    if !attach_plan.contains(&Capability::StdioChunk) {
        effective.stdio.enabled = false;
    }
    if !attach_plan.contains(&Capability::SocketPlaintextPayload) {
        effective.socket.enabled = false;
    }
    effective
}

fn capability_configured_for_attach(capability: &Capability, payload: &PayloadConfig) -> bool {
    match capability {
        Capability::ProcLifecycle
        | Capability::NetTransport
        | Capability::FsAccessBasic
        | Capability::FsMmap
        | Capability::IpcPipeFifo
        | Capability::IpcUnixSocket => true,
        Capability::TlsPlaintextPayload => {
            payload.tls.enabled && !payload.tls.capture_backend.is_sync()
        }
        Capability::SocketPlaintextPayload => payload.socket.enabled,
        Capability::StdioChunk => {
            payload.stdio.enabled
                && (payload.stdio.capture_stdin
                    || payload.stdio.capture_stdout
                    || payload.stdio.capture_stderr)
        }
        _ => false,
    }
}

fn capability_required_programs(capability: &Capability) -> Option<&'static [&'static str]> {
    match capability {
        Capability::ProcLifecycle => Some(PROC_LIFECYCLE_PROGRAMS),
        Capability::NetTransport => Some(NET_TRANSPORT_PROGRAMS),
        Capability::FsAccessBasic => Some(FS_ACCESS_BASIC_FD_PROGRAMS),
        Capability::FsMmap => Some(FS_MMAP_PROGRAMS),
        Capability::StdioChunk => Some(STDIO_PROGRAMS),
        Capability::IpcPipeFifo | Capability::IpcUnixSocket => Some(IPC_PROGRAMS),
        Capability::SocketPlaintextPayload => Some(SOCKET_PAYLOAD_PROGRAMS),
        Capability::TlsPlaintextPayload => Some(&[]),
        _ => None,
    }
}

fn capability_programs(program_name: &str) -> Option<()> {
    [
        PROC_LIFECYCLE_PROGRAMS,
        NET_TRANSPORT_PROGRAMS,
        FS_ACCESS_BASIC_FD_PROGRAMS,
        FS_ACCESS_BASIC_PATH_PROGRAMS,
        FS_ACCESS_BASIC_CONTEXT_PROGRAMS,
        PROCESS_CONTEXT_PROGRAMS,
        PROCESS_SIGNAL_DIAGNOSTIC_PROGRAMS,
        STDIO_PROGRAMS,
        IPC_PROGRAMS,
        SOCKET_PAYLOAD_PROGRAMS,
        FS_MMAP_PROGRAMS,
    ]
    .iter()
    .any(|programs| programs.contains(&program_name))
    .then_some(())
}

fn programs_attached(programs: &[&str], attached_programs: &[String]) -> bool {
    programs
        .iter()
        .all(|program| attached_programs.iter().any(|attached| attached == program))
}

fn required_programs_attached(programs: &[&str], attached_programs: &[String]) -> bool {
    programs
        .iter()
        .filter(|program| !PLATFORM_OPTIONAL_TRACEPOINT_PROGRAMS.contains(program))
        .all(|program| attached_programs.iter().any(|attached| attached == program))
}

#[cfg(test)]
#[path = "attach_plan/tests.rs"]
mod tests;
