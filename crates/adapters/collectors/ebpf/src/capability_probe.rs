//! Kernel, BTF, and collector capability probing.

use std::path::Path;

use collector_capability::CollectorDescriptor;
use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};
use model_core::ids::CollectorName;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EbpfEnvironment {
    pub kernel_release: String,
    pub btf_present: bool,
    pub admin_runtime: bool,
    pub tracefs_control_writable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TracefsControlState {
    pub writable: bool,
    pub reason_unavailable: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EbpfProbeResult {
    pub environment: EbpfEnvironment,
    pub descriptor: CollectorDescriptor,
    pub reason_unavailable: Option<String>,
}

pub fn probe() -> EbpfProbeResult {
    let kernel_release = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let btf_present = Path::new("/sys/kernel/btf/vmlinux").exists();
    let admin_runtime = is_admin_runtime();
    let tracefs_control = tracefs_control_state();
    let reason_unavailable = if !btf_present {
        Some("kernel BTF is missing".to_string())
    } else if !admin_runtime {
        Some("collector requires administrator privileges".to_string())
    } else if !tracefs_control.writable {
        tracefs_control.reason_unavailable.clone()
    } else {
        None
    };

    EbpfProbeResult {
        environment: EbpfEnvironment {
            kernel_release,
            btf_present,
            admin_runtime,
            tracefs_control_writable: tracefs_control.writable,
        },
        descriptor: CollectorDescriptor {
            name: CollectorName::new("ebpf"),
            capabilities: vec![
                CapabilityDescriptor::new(
                    Capability::ProcLifecycle,
                    vec![
                        CapabilityField::new(
                            "fork_exec_exit",
                            GuaranteeClass::GuaranteedByTransportCollector,
                        ),
                        CapabilityField::new(
                            "signals_session_process_group",
                            GuaranteeClass::GuaranteedByTransportCollector,
                        ),
                    ],
                ),
                CapabilityDescriptor::new(
                    Capability::NetTransport,
                    vec![
                        CapabilityField::new(
                            "endpoint_direction",
                            GuaranteeClass::GuaranteedByTransportCollector,
                        ),
                        CapabilityField::new(
                            "size_result_endpoint",
                            GuaranteeClass::GuaranteedByTransportCollector,
                        ),
                    ],
                ),
                CapabilityDescriptor::new(
                    Capability::IpcPipeFifo,
                    vec![CapabilityField::new(
                        "pipe_fifo_fd_io",
                        GuaranteeClass::AvailableWhenMetadataObservable,
                    )],
                ),
                CapabilityDescriptor::new(
                    Capability::IpcUnixSocket,
                    vec![CapabilityField::new(
                        "unix_socket_fd_io",
                        GuaranteeClass::AvailableWhenMetadataObservable,
                    )],
                ),
                CapabilityDescriptor::new(
                    Capability::FsAccessBasic,
                    vec![
                        CapabilityField::new(
                            "regular_file_fd_io",
                            GuaranteeClass::AvailableWhenMetadataObservable,
                        ),
                        CapabilityField::new(
                            "file_path_mutation_syscalls",
                            GuaranteeClass::GuaranteedByTransportCollector,
                        ),
                    ],
                ),
                CapabilityDescriptor::new(
                    Capability::FsMmap,
                    vec![CapabilityField::new(
                        "mmap_shared_file_access",
                        GuaranteeClass::GuaranteedByTransportCollector,
                    )],
                ),
            ],
            supports_attach_coverage_guard: false,
            supports_existing_pid_attach: true,
        },
        reason_unavailable,
    }
}

pub(crate) fn tracefs_control_state() -> TracefsControlState {
    let Ok(mountinfo) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return TracefsControlState {
            writable: false,
            reason_unavailable: Some("cannot read /proc/self/mountinfo".to_string()),
        };
    };
    let tracefs_mounts = mountinfo
        .lines()
        .filter_map(parse_tracefs_mount)
        .collect::<Vec<_>>();
    if tracefs_mounts.is_empty() {
        return TracefsControlState {
            writable: false,
            reason_unavailable: Some("tracefs mount is missing".to_string()),
        };
    }
    if tracefs_mounts
        .iter()
        .any(|mount| mount.control_path && mount.mount_options.iter().any(|option| option == "rw"))
    {
        return TracefsControlState {
            writable: true,
            reason_unavailable: None,
        };
    }
    let mount_descriptions = tracefs_mounts
        .iter()
        .map(|mount| format!("{}({})", mount.mount_point, mount.mount_options.join(",")))
        .collect::<Vec<_>>()
        .join(", ");
    TracefsControlState {
        writable: false,
        reason_unavailable: Some(format!(
            "tracefs control mount is not writable: {mount_descriptions}"
        )),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TracefsMount {
    mount_point: String,
    mount_options: Vec<String>,
    control_path: bool,
}

fn parse_tracefs_mount(line: &str) -> Option<TracefsMount> {
    let (mount_fields, fs_fields) = line.split_once(" - ")?;
    let mut fields = mount_fields.split_whitespace();
    let _mount_id = fields.next()?;
    let _parent_id = fields.next()?;
    let _device = fields.next()?;
    let _root = fields.next()?;
    let mount_point = fields.next()?.to_string();
    let mount_options = fields
        .next()?
        .split(',')
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let fs_type = fs_fields.split_whitespace().next()?;
    if fs_type != "tracefs" {
        return None;
    }
    let control_path =
        mount_point == "/sys/kernel/tracing" || mount_point == "/sys/kernel/debug/tracing";
    Some(TracefsMount {
        mount_point,
        mount_options,
        control_path,
    })
}

fn is_admin_runtime() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("Uid:")
                    .and_then(|rest| rest.split_whitespace().next())
                    .and_then(|value| value.parse::<u32>().ok())
            })
        })
        .map(|uid| uid == 0)
        .unwrap_or(false)
}
