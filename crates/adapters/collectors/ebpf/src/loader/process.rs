//! Process exec-context eBPF map configuration.

use std::ffi::OsStr;
use std::os::fd::{AsFd, AsRawFd};

use config_core::daemon::ProcessSeccompConfig;
use libbpf_rs::{MapCore, MapFlags, MapHandle, Object};

use crate::loader::LoaderError;

const PROCESS_EXEC_CONFIG_KEY: u32 = 0;
pub(super) const PROCESS_EXEC_ARGV_CONTINUE_PROGRAM: &str = "handle_process_exec_argv_continue";
const PROCESS_EXEC_ARGV_TAIL_CALLS_MAP: &str = "process_exec_argv_tail_calls";
const PROCESS_EXEC_ARG_MAX: u32 = 128;
const PROCESS_EXEC_ARGV_COPY_MAX_BYTES: u32 = 4_095;
const PROCESS_EXEC_CONFIG_FIELDS: usize = 4;
const PROCESS_EXEC_CONFIG_VALUE_SIZE: usize =
    PROCESS_EXEC_CONFIG_FIELDS * std::mem::size_of::<u32>();

pub(super) fn validate_config(config: &ProcessSeccompConfig) -> Result<(), LoaderError> {
    if config.max_args > PROCESS_EXEC_ARG_MAX {
        return Err(LoaderError::new(
            "process_exec_config",
            format!(
                "process_seccomp.max_args {} exceeds compiled eBPF maximum {}",
                config.max_args, PROCESS_EXEC_ARG_MAX
            ),
        ));
    }
    for (name, value) in [
        ("max_arg_bytes", config.max_arg_bytes),
        ("max_total_arg_bytes", config.max_total_arg_bytes),
    ] {
        if value > PROCESS_EXEC_ARGV_COPY_MAX_BYTES {
            return Err(LoaderError::new(
                "process_exec_config",
                format!(
                    "process_seccomp.{name} {value} exceeds compiled eBPF maximum {PROCESS_EXEC_ARGV_COPY_MAX_BYTES}"
                ),
            ));
        }
    }
    Ok(())
}

pub(super) fn configure_map(
    object: &Object,
    config: &ProcessSeccompConfig,
    executable_identity_enabled: bool,
) -> Result<(), LoaderError> {
    let map = object
        .maps()
        .find(|map| map.name() == OsStr::new("process_exec_config"))
        .ok_or_else(|| LoaderError::new("process_exec_config", "map is missing"))
        .and_then(|map| {
            MapHandle::try_from(&map)
                .map_err(|error| LoaderError::new("process_exec_config", error.to_string()))
        })?;
    let fields = [
        config.max_args,
        config.max_arg_bytes,
        config.max_total_arg_bytes,
        executable_identity_enabled as u32,
    ];
    let mut value = [0_u8; PROCESS_EXEC_CONFIG_VALUE_SIZE];
    for (index, field) in fields.into_iter().enumerate() {
        let start = index * std::mem::size_of::<u32>();
        value[start..start + std::mem::size_of::<u32>()].copy_from_slice(&field.to_ne_bytes());
    }
    map.update(
        &PROCESS_EXEC_CONFIG_KEY.to_ne_bytes(),
        &value,
        MapFlags::ANY,
    )
    .map_err(|error| LoaderError::new("process_exec_config", error.to_string()))
}

pub(super) fn configure_argv_tail_calls(object: &Object) -> Result<(), LoaderError> {
    let Some(map) = object
        .maps()
        .find(|map| map.name() == OsStr::new(PROCESS_EXEC_ARGV_TAIL_CALLS_MAP))
    else {
        // Kernels with bpf_loop use the in-program callback path and do not
        // compile the tail-call fallback map or continuation program.
        return Ok(());
    };
    let program = object
        .progs()
        .find(|program| program.name() == OsStr::new(PROCESS_EXEC_ARGV_CONTINUE_PROGRAM))
        .ok_or_else(|| {
            LoaderError::new(
                "process_exec_argv_tail_calls",
                format!("BPF program {PROCESS_EXEC_ARGV_CONTINUE_PROGRAM} is missing"),
            )
        })?;
    let map = MapHandle::try_from(&map)
        .map_err(|error| LoaderError::new("process_exec_argv_tail_calls", error.to_string()))?;
    let program_fd = u32::try_from(program.as_fd().as_raw_fd()).map_err(|error| {
        LoaderError::new(
            "process_exec_argv_tail_calls",
            format!("invalid continuation program fd: {error}"),
        )
    })?;
    map.update(
        &0u32.to_ne_bytes(),
        &program_fd.to_ne_bytes(),
        MapFlags::ANY,
    )
    .map_err(|error| LoaderError::new("process_exec_argv_tail_calls", error.to_string()))
}
