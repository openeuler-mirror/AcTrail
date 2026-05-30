//! Remote syscall argument decoding for exec observations.

use control_contract::reply::ControlError;

use crate::services::seccomp_notify::read_process_bytes;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExecArgs {
    pub path: Option<String>,
    pub argv: Vec<String>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExecPath {
    pub path: Option<String>,
    pub truncated: bool,
}

pub(super) fn read_execve_path(
    pid: u32,
    path_ptr: u64,
    max_arg_bytes: u32,
) -> Result<ExecPath, ControlError> {
    read_exec_path(pid, path_ptr, max_arg_bytes)
}

pub(super) fn read_execveat_path(
    pid: u32,
    path_ptr: u64,
    max_arg_bytes: u32,
) -> Result<ExecPath, ControlError> {
    read_exec_path(pid, path_ptr, max_arg_bytes)
}

pub(super) fn read_execve_args(
    pid: u32,
    path: ExecPath,
    argv_ptr: u64,
    max_args: u32,
    max_arg_bytes: u32,
) -> Result<ExecArgs, ControlError> {
    read_exec_args(pid, path, argv_ptr, max_args, max_arg_bytes)
}

pub(super) fn read_execveat_args(
    pid: u32,
    path: ExecPath,
    argv_ptr: u64,
    max_args: u32,
    max_arg_bytes: u32,
) -> Result<ExecArgs, ControlError> {
    read_exec_args(pid, path, argv_ptr, max_args, max_arg_bytes)
}

fn read_exec_path(pid: u32, path_ptr: u64, max_arg_bytes: u32) -> Result<ExecPath, ControlError> {
    let mut truncated = false;
    let path = read_c_string(pid, path_ptr, max_arg_bytes, &mut truncated)?;
    Ok(ExecPath { path, truncated })
}

fn read_exec_args(
    pid: u32,
    path: ExecPath,
    argv_ptr: u64,
    max_args: u32,
    max_arg_bytes: u32,
) -> Result<ExecArgs, ControlError> {
    let mut truncated = path.truncated;
    let argv = read_argv(pid, argv_ptr, max_args, max_arg_bytes, &mut truncated)?;
    Ok(ExecArgs {
        path: path.path,
        argv,
        truncated,
    })
}

fn read_argv(
    pid: u32,
    argv_ptr: u64,
    max_args: u32,
    max_arg_bytes: u32,
    truncated: &mut bool,
) -> Result<Vec<String>, ControlError> {
    if argv_ptr == 0 {
        return Ok(Vec::new());
    }
    let pointer_size = std::mem::size_of::<usize>();
    let max_args = usize::try_from(max_args).map_err(|error| {
        ControlError::new(
            "process_seccomp_args",
            format!("max args overflow: {error}"),
        )
    })?;
    let table_size = max_args
        .checked_mul(pointer_size)
        .ok_or_else(|| ControlError::new("process_seccomp_args", "argv table size overflow"))?;
    let pointer_table = read_process_bytes(pid, argv_ptr, table_size)?;
    if pointer_table.len() != table_size {
        return Err(ControlError::new(
            "process_seccomp_args",
            "short argv pointer table read",
        ));
    }
    let mut argv = Vec::new();
    for pointer_bytes in pointer_table.chunks_exact(pointer_size) {
        let pointer = usize::from_ne_bytes(pointer_bytes.try_into().expect("pointer width")) as u64;
        if pointer == 0 {
            return Ok(argv);
        }
        if let Some(arg) = read_c_string(pid, pointer, max_arg_bytes, truncated)? {
            argv.push(arg);
        }
    }
    *truncated = true;
    Ok(argv)
}

fn read_c_string(
    pid: u32,
    remote_addr: u64,
    max_bytes: u32,
    truncated: &mut bool,
) -> Result<Option<String>, ControlError> {
    if remote_addr == 0 {
        return Ok(None);
    }
    let max_bytes = usize::try_from(max_bytes).map_err(|error| {
        ControlError::new(
            "process_seccomp_args",
            format!("max arg bytes overflow: {error}"),
        )
    })?;
    let bytes = read_process_bytes(pid, remote_addr, max_bytes)?;
    let end = bytes.iter().position(|byte| *byte == 0);
    if end.is_none() && bytes.len() == max_bytes {
        *truncated = true;
    }
    let value = match end {
        Some(end) => &bytes[..end],
        None => bytes.as_slice(),
    };
    Ok(Some(String::from_utf8_lossy(value).into_owned()))
}
