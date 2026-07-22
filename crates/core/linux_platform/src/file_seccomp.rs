//! Linux file-mutation seccomp syscall mapping.

use config_core::daemon::EnforcementSeccompSyscall;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum KernelFileSeccompSyscall {
    Mkdir,
    MkdirAt,
    Rmdir,
    UnlinkAtRemovedir,
}

impl KernelFileSeccompSyscall {
    pub fn number(self) -> Option<libc::c_long> {
        match self {
            Self::Mkdir => mkdir_syscall_number(),
            Self::MkdirAt => Some(libc::SYS_mkdirat),
            Self::Rmdir => rmdir_syscall_number(),
            Self::UnlinkAtRemovedir => Some(libc::SYS_unlinkat),
        }
    }

    pub fn configured(self) -> EnforcementSeccompSyscall {
        match self {
            Self::Mkdir | Self::MkdirAt => EnforcementSeccompSyscall::Mkdir,
            Self::Rmdir | Self::UnlinkAtRemovedir => EnforcementSeccompSyscall::Rmdir,
        }
    }

    pub fn path_argument(self) -> usize {
        match self {
            Self::Mkdir | Self::Rmdir => 0,
            Self::MkdirAt | Self::UnlinkAtRemovedir => 1,
        }
    }

    pub fn dirfd_argument(self) -> Option<usize> {
        match self {
            Self::Mkdir | Self::Rmdir => None,
            Self::MkdirAt | Self::UnlinkAtRemovedir => Some(0),
        }
    }

    pub fn from_seccomp(data: &libc::seccomp_data) -> Option<Self> {
        effective_kernel_syscalls(EnforcementSeccompSyscall::Mkdir)
            .into_iter()
            .chain(effective_kernel_syscalls(EnforcementSeccompSyscall::Rmdir))
            .find(|candidate| {
                candidate
                    .number()
                    .and_then(|number| i32::try_from(number).ok())
                    .is_some_and(|number| number == data.nr)
                    && (!matches!(candidate, Self::UnlinkAtRemovedir)
                        || data.args[2] & libc::AT_REMOVEDIR as u64 != 0)
            })
    }
}

pub fn effective_kernel_syscalls(
    syscall: EnforcementSeccompSyscall,
) -> Vec<KernelFileSeccompSyscall> {
    let candidates = match syscall {
        EnforcementSeccompSyscall::Mkdir => vec![
            KernelFileSeccompSyscall::Mkdir,
            KernelFileSeccompSyscall::MkdirAt,
        ],
        EnforcementSeccompSyscall::Rmdir => vec![
            KernelFileSeccompSyscall::Rmdir,
            KernelFileSeccompSyscall::UnlinkAtRemovedir,
        ],
    };
    candidates
        .into_iter()
        .filter(|candidate| candidate.number().is_some())
        .collect()
}

#[cfg(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "arm",
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "s390x"
))]
fn mkdir_syscall_number() -> Option<libc::c_long> {
    Some(libc::SYS_mkdir)
}

#[cfg(not(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "arm",
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "s390x"
)))]
fn mkdir_syscall_number() -> Option<libc::c_long> {
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
fn rmdir_syscall_number() -> Option<libc::c_long> {
    Some(libc::SYS_rmdir)
}

#[cfg(not(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "arm",
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "s390x"
)))]
fn rmdir_syscall_number() -> Option<libc::c_long> {
    None
}
