//! Process-control syscall notification decoding.

use config_core::daemon::ProcessSeccompSyscall;
use control_contract::reply::ControlError;
use linux_platform::process_seccomp::{
    KernelProcessSyscall, effective_kernel_syscalls, syscall_name as configured_syscall_name,
};

pub(super) fn effective_syscalls(
    syscalls: impl IntoIterator<Item = ProcessSeccompSyscall>,
) -> Result<std::collections::BTreeSet<KernelProcessSyscall>, ControlError> {
    let mut effective = std::collections::BTreeSet::new();
    for syscall in syscalls {
        let kernel_syscalls = effective_kernel_syscalls(syscall);
        if kernel_syscalls.is_empty() {
            return Err(ControlError::new(
                "process_seccomp_syscall",
                format!("process seccomp syscall {syscall:?} has no kernel syscall on this target"),
            ));
        }
        effective.extend(kernel_syscalls);
    }
    Ok(effective)
}

pub(super) fn syscall_from_notification(
    notification: &libc::seccomp_notif,
) -> Result<Option<KernelProcessSyscall>, ControlError> {
    Ok(KernelProcessSyscall::from_number(notification.data.nr))
}

pub(super) fn syscall_name(syscall: ProcessSeccompSyscall) -> &'static str {
    configured_syscall_name(syscall)
}
