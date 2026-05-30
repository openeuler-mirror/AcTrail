//! Linux process-control seccomp syscall mapping.

use config_core::daemon::ProcessSeccompSyscall;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum KernelProcessSyscall {
    Execve,
    Execveat,
    Fork,
    Vfork,
    Clone,
    Clone3,
}

impl KernelProcessSyscall {
    pub fn number(self) -> Option<libc::c_long> {
        match self {
            Self::Execve => Some(libc::SYS_execve),
            Self::Execveat => Some(libc::SYS_execveat),
            Self::Fork => fork_syscall_number(),
            Self::Vfork => vfork_syscall_number(),
            Self::Clone => Some(libc::SYS_clone),
            Self::Clone3 => Some(libc::SYS_clone3),
        }
    }

    pub fn from_number(number: i32) -> Option<Self> {
        [
            Self::Execve,
            Self::Execveat,
            Self::Fork,
            Self::Vfork,
            Self::Clone,
            Self::Clone3,
        ]
        .into_iter()
        .find(|candidate| {
            candidate
                .number()
                .and_then(|raw| i32::try_from(raw).ok())
                .is_some_and(|raw| raw == number)
        })
    }

    pub fn as_configured_syscall(self) -> ProcessSeccompSyscall {
        match self {
            Self::Execve => ProcessSeccompSyscall::Execve,
            Self::Execveat => ProcessSeccompSyscall::Execveat,
            Self::Fork => ProcessSeccompSyscall::Fork,
            Self::Vfork => ProcessSeccompSyscall::Vfork,
            Self::Clone => ProcessSeccompSyscall::Clone,
            Self::Clone3 => ProcessSeccompSyscall::Clone3,
        }
    }
}

pub fn effective_kernel_syscalls(syscall: ProcessSeccompSyscall) -> Vec<KernelProcessSyscall> {
    let candidates = match syscall {
        ProcessSeccompSyscall::Execve => vec![KernelProcessSyscall::Execve],
        ProcessSeccompSyscall::Execveat => vec![KernelProcessSyscall::Execveat],
        ProcessSeccompSyscall::Fork => process_creation_family(KernelProcessSyscall::Fork),
        ProcessSeccompSyscall::Vfork => process_creation_family(KernelProcessSyscall::Vfork),
        ProcessSeccompSyscall::Clone => vec![KernelProcessSyscall::Clone],
        ProcessSeccompSyscall::Clone3 => vec![KernelProcessSyscall::Clone3],
    };
    candidates
        .into_iter()
        .filter(|candidate| candidate.number().is_some())
        .collect()
}

pub fn syscall_name(syscall: ProcessSeccompSyscall) -> &'static str {
    match syscall {
        ProcessSeccompSyscall::Execve => "execve",
        ProcessSeccompSyscall::Execveat => "execveat",
        ProcessSeccompSyscall::Fork => "fork",
        ProcessSeccompSyscall::Vfork => "vfork",
        ProcessSeccompSyscall::Clone => "clone",
        ProcessSeccompSyscall::Clone3 => "clone3",
    }
}

fn process_creation_family(preferred: KernelProcessSyscall) -> Vec<KernelProcessSyscall> {
    if preferred.number().is_some() {
        return vec![preferred];
    }
    vec![KernelProcessSyscall::Clone, KernelProcessSyscall::Clone3]
}

#[cfg(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "arm",
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "s390x"
))]
fn fork_syscall_number() -> Option<libc::c_long> {
    Some(libc::SYS_fork)
}

#[cfg(not(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "arm",
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "s390x"
)))]
fn fork_syscall_number() -> Option<libc::c_long> {
    None
}

#[cfg(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "arm",
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "s390x"
))]
fn vfork_syscall_number() -> Option<libc::c_long> {
    Some(libc::SYS_vfork)
}

#[cfg(not(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "arm",
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "s390x"
)))]
fn vfork_syscall_number() -> Option<libc::c_long> {
    None
}
