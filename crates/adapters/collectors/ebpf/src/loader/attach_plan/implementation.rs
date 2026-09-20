//! Capability-scoped eBPF program autoload planning.

use std::collections::BTreeSet;

use config_core::daemon::{EbpfCollectorConfig, FileCollectionConfig, PayloadConfig};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};

use super::LoaderError;
use super::tls;

#[path = "programs.rs"]
mod programs;
use programs::*;

const FILE_ATTACH_PRIORITY: u8 = 0;
const SHARED_FD_ATTACH_PRIORITY: u8 = 1;
const PROCESS_ATTACH_PRIORITY: u8 = 2;
const DEFAULT_ATTACH_PRIORITY: u8 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachPlan {
    capabilities: BTreeSet<Capability>,
    file_path_capture_enabled: bool,
    dynamic_go_tls_enabled: bool,
    mcp_stdio_enabled: bool,
    file_collection: FileCollectionConfig,
    file_directory_observation: bool,
    file_tty_observation: bool,
}

impl AttachPlan {
    pub fn baseline() -> Self {
        Self {
            capabilities: BTreeSet::new(),
            file_path_capture_enabled: false,
            dynamic_go_tls_enabled: false,
            mcp_stdio_enabled: false,
            file_collection: FileCollectionConfig::default(),
            file_directory_observation: false,
            file_tty_observation: false,
        }
    }

    pub fn from_requests(
        requests: &[CapabilityRequest],
        config: &EbpfCollectorConfig,
        payload: &PayloadConfig,
        semantic_projection_enabled: bool,
        file_collection: &FileCollectionConfig,
        file_directory_observation: bool,
        file_tty_observation: bool,
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
        plan.file_collection = *file_collection;
        plan.file_directory_observation = file_directory_observation;
        plan.file_tty_observation = file_tty_observation;
        plan.dynamic_go_tls_enabled = payload.tls.enabled && payload.tls.capture_backend.is_sync();
        plan.mcp_stdio_enabled = semantic_projection_enabled
            && payload.mcp.enabled
            && payload.stdio.enabled
            && payload.stdio.capture_stdin
            && plan.contains(&Capability::StdioChunk);
        plan
    }

    pub(crate) fn mcp_stdio_enabled(&self) -> bool {
        self.mcp_stdio_enabled
    }

    pub(crate) fn file_capture_enabled(&self) -> bool {
        self.file_path_capture_enabled
            && (self.contains(&Capability::FsAccessBasic) || self.contains(&Capability::FsMmap))
            && (self.file_directory_observation
                || self.file_collection.writable_open
                || self.file_collection.path_mutations
                || self.file_collection.fd_mutations
                || self.file_collection.read.enabled()
                || self.file_collection.write.enabled())
    }

    pub(crate) fn file_collection(&self) -> &FileCollectionConfig {
        &self.file_collection
    }

    pub(crate) fn file_directory_observation(&self) -> bool {
        self.file_directory_observation
    }

    pub(crate) fn file_fd_capture_enabled(&self) -> bool {
        self.file_capture_enabled() && self.file_collection.fd_mutations
    }

    pub(crate) fn file_tty_observation(&self) -> bool {
        self.file_tty_observation
    }

    pub(crate) fn file_io_summary_enabled(&self) -> bool {
        self.file_capture_enabled()
            && self.contains(&Capability::FsAccessBasic)
            && (self.file_collection.read.enabled() || self.file_collection.write.enabled())
    }

    pub fn contains(&self, capability: &Capability) -> bool {
        self.capabilities.contains(capability)
    }

    pub fn should_load_program(&self, program_name: &str) -> Result<bool, LoaderError> {
        if tls::is_payload_tls_program(program_name) {
            return Ok(self.contains(&Capability::TlsPlaintextPayload)
                || (self.dynamic_go_tls_enabled && tls::is_dynamic_tls_program(program_name)));
        }
        if FILE_IO_SUMMARY_PROGRAMS.contains(&program_name) {
            return Ok(self.file_io_summary_enabled());
        }
        if ON_DEMAND_PROGRAMS.contains(&program_name) {
            return Ok(true);
        }
        if capability_programs(program_name).is_none() {
            return Err(LoaderError::new(
                "attach_plan",
                format!("BPF program {program_name} has no capability mapping"),
            ));
        }
        if TRACKING_REGISTRATION_PROGRAMS.contains(&program_name) {
            return Ok(true);
        }
        if self.mcp_stdio_enabled
            && (FILE_OPEN_PROGRAMS.contains(&program_name)
                || MCP_STDIO_CONTEXT_PROGRAMS.contains(&program_name)
                || FD_PROCESS_LIFECYCLE_PROGRAMS.contains(&program_name)
                || PROCESS_CONTEXT_PROGRAMS.contains(&program_name))
        {
            return Ok(true);
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
            || FILE_OPEN_PROGRAMS.contains(&program_name)
            || FILE_CONTEXT_PROGRAMS.contains(&program_name)
            || FS_MMAP_PROGRAMS.contains(&program_name)
        {
            return FILE_ATTACH_PRIORITY;
        }
        if FS_ACCESS_BASIC_FD_PROGRAMS.contains(&program_name)
            || FD_PROCESS_LIFECYCLE_PROGRAMS.contains(&program_name)
        {
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
                self.file_capture_enabled()
                    && (((self.file_collection.read.enabled()
                        || self.file_collection.write.enabled())
                        && FILE_IO_SUMMARY_PROGRAMS.contains(&program_name))
                        || (self.file_collection.fd_mutations
                            && FILE_FD_MUTATION_PROGRAMS.contains(&program_name))
                        || FILE_OPEN_PROGRAMS.contains(&program_name)
                        || FILE_CONTEXT_PROGRAMS.contains(&program_name)
                        || PROCESS_CONTEXT_PROGRAMS.contains(&program_name)
                        || FD_PROCESS_LIFECYCLE_PROGRAMS.contains(&program_name)
                        || (self.file_collection.path_mutations
                            && FS_ACCESS_BASIC_PATH_PROGRAMS.contains(&program_name)))
            }
            Capability::FsMmap => {
                self.file_collection.fd_mutations
                    && (FS_MMAP_PROGRAMS.contains(&program_name)
                        || PROCESS_CONTEXT_PROGRAMS.contains(&program_name)
                        || (self.file_path_capture_enabled
                            && (FILE_OPEN_PROGRAMS.contains(&program_name)
                                || FILE_CONTEXT_PROGRAMS.contains(&program_name)
                                || FD_PROCESS_LIFECYCLE_PROGRAMS.contains(&program_name))))
            }
            Capability::IpcPipeFifo | Capability::IpcUnixSocket => {
                FILE_OPEN_PROGRAMS.contains(&program_name)
                    || capability_required_programs(capability)
                        .is_some_and(|programs| programs.contains(&program_name))
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
        if matches!(
            capability,
            Capability::IpcPipeFifo | Capability::IpcUnixSocket
        ) {
            return required_programs_attached(FILE_OPEN_PROGRAMS, attached_programs)
                && capability_required_programs(capability).is_some_and(|programs| {
                    required_programs_attached(programs, attached_programs)
                });
        }
        if matches!(capability, Capability::TlsPlaintextPayload) {
            return attached_programs
                .iter()
                .any(|program| tls::is_payload_tls_program(program));
        }
        if matches!(capability, Capability::FsAccessBasic) {
            if !self.file_capture_enabled() {
                return true;
            }
            return (!(self.file_collection.read.enabled()
                || self.file_collection.write.enabled())
                || programs_attached(FILE_IO_SUMMARY_PROGRAMS, attached_programs))
                && (!self.file_collection.fd_mutations
                    || required_programs_attached(FILE_FD_MUTATION_PROGRAMS, attached_programs))
                && required_programs_attached(FILE_OPEN_PROGRAMS, attached_programs)
                && required_programs_attached(FILE_CONTEXT_PROGRAMS, attached_programs)
                && programs_attached(PROCESS_CONTEXT_PROGRAMS, attached_programs)
                && programs_attached(FD_PROCESS_LIFECYCLE_PROGRAMS, attached_programs)
                && (!self.file_collection.path_mutations
                    || required_programs_attached(
                        FS_ACCESS_BASIC_PATH_PROGRAMS,
                        attached_programs,
                    ));
        }
        if matches!(capability, Capability::FsMmap) {
            if !self.file_collection.fd_mutations {
                return true;
            }
            return programs_attached(FS_MMAP_PROGRAMS, attached_programs)
                && programs_attached(PROCESS_CONTEXT_PROGRAMS, attached_programs)
                && (!self.file_path_capture_enabled
                    || (required_programs_attached(FILE_OPEN_PROGRAMS, attached_programs)
                        && required_programs_attached(FILE_CONTEXT_PROGRAMS, attached_programs)
                        && programs_attached(FD_PROCESS_LIFECYCLE_PROGRAMS, attached_programs)));
        }
        capability_required_programs(capability)
            .is_some_and(|programs| required_programs_attached(programs, attached_programs))
    }
}

pub(super) fn configure_program_autoload(
    open_object: &mut libbpf_rs::OpenObject,
    attach_plan: &AttachPlan,
    payload: &PayloadConfig,
) -> Result<(), LoaderError> {
    for mut program in open_object.progs_mut() {
        let program_name = program.name().to_string_lossy().into_owned();
        let autoload = if program_name == "handle_tls_mapping" {
            payload.tls.enabled
                && payload.tls.direct_dynamic_discovery_enabled
                && payload.tls.capture_backend
                    == config_core::daemon::PayloadTlsCaptureBackend::BpfCopy
        } else {
            attach_plan.should_load_program(&program_name)?
        };
        program.set_autoload(autoload);
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
        | Capability::ProcExecContext
        | Capability::NetTransport
        | Capability::FsAccessBasic
        | Capability::FsMmap => true,
        Capability::IpcPipeFifo | Capability::IpcUnixSocket => true,
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
        Capability::ProcExecContext => Some(PROC_EXEC_CONTEXT_PROGRAMS),
        Capability::NetTransport => Some(NET_TRANSPORT_PROGRAMS),
        Capability::FsAccessBasic => Some(FS_ACCESS_BASIC_FD_PROGRAMS),
        Capability::FsMmap => Some(FS_MMAP_PROGRAMS),
        Capability::StdioChunk => Some(STDIO_PROGRAMS),
        Capability::IpcPipeFifo => Some(IPC_PIPE_FIFO_PROGRAMS),
        Capability::IpcUnixSocket => Some(IPC_UNIX_SOCKET_PROGRAMS),
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
        FILE_FD_MUTATION_PROGRAMS,
        FILE_OPEN_PROGRAMS,
        FS_ACCESS_BASIC_PATH_PROGRAMS,
        FILE_CONTEXT_PROGRAMS,
        PROCESS_CONTEXT_PROGRAMS,
        PROC_EXEC_CONTEXT_PROGRAMS,
        FD_PROCESS_LIFECYCLE_PROGRAMS,
        PROCESS_SIGNAL_DIAGNOSTIC_PROGRAMS,
        STDIO_PROGRAMS,
        MCP_STDIO_CONTEXT_PROGRAMS,
        IPC_PIPE_FIFO_PROGRAMS,
        IPC_UNIX_SOCKET_PROGRAMS,
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
