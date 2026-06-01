//! Clone syscall flag decoding for process-level observation.

use config_core::daemon::ProcessSeccompSyscall;
use control_contract::reply::ControlError;

use crate::services::seccomp_notify::read_process_bytes;

const CLONE3_FLAGS_OFFSET: u64 = 0;
const CLONE3_FLAGS_SIZE: usize = std::mem::size_of::<u64>();

pub(super) fn clone_flags(
    notification: &libc::seccomp_notif,
    syscall: ProcessSeccompSyscall,
) -> Result<Option<u64>, ControlError> {
    match syscall {
        ProcessSeccompSyscall::Clone => Ok(Some(notification.data.args[0])),
        ProcessSeccompSyscall::Clone3 => read_clone3_flags(notification),
        _ => Ok(None),
    }
}

pub(super) fn is_thread_clone(flags: Option<u64>) -> bool {
    flags
        .map(|flags| flags & libc::CLONE_THREAD as u64 != 0)
        .unwrap_or(false)
}

fn read_clone3_flags(notification: &libc::seccomp_notif) -> Result<Option<u64>, ControlError> {
    let args_ptr = notification.data.args[0];
    let args_size = notification.data.args[1];
    if args_ptr == 0 || args_size < CLONE3_FLAGS_SIZE as u64 {
        return Ok(None);
    }
    let flags_addr = args_ptr
        .checked_add(CLONE3_FLAGS_OFFSET)
        .ok_or_else(|| ControlError::new("process_seccomp_clone3", "clone3 flags overflow"))?;
    let bytes = read_process_bytes(notification.pid, flags_addr, CLONE3_FLAGS_SIZE)?;
    if bytes.len() != CLONE3_FLAGS_SIZE {
        return Err(ControlError::new(
            "process_seccomp_clone3",
            "short clone3 flags read",
        ));
    }
    Ok(Some(u64::from_ne_bytes(
        bytes.try_into().expect("clone3 flags width"),
    )))
}
