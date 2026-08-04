use super::*;

pub(super) fn open_requests_creation(event: &KernelFilePathEvent) -> bool {
    match event.aux {
        FILE_SYSCALL_CREAT => true,
        FILE_SYSCALL_OPEN => event.arg1 & libc::O_CREAT as u64 != 0,
        FILE_SYSCALL_OPENAT | FILE_SYSCALL_OPENAT2 => event.arg2 & libc::O_CREAT as u64 != 0,
        _ => false,
    }
}

pub(super) fn pending_key(event: &KernelFilePathEvent) -> PendingKey {
    PendingKey {
        trace_id: event.trace_id,
        tid: event.tid,
        syscall: event.aux,
    }
}

pub(super) fn path_string(bytes: &[u8], flags: u32) -> Option<String> {
    if flags & PATH_FLAG_CAPTURED == 0 {
        return None;
    }
    Some(String::from_utf8_lossy(bytes).into_owned())
}

pub(super) fn primary_dirfd(event: &KernelFilePathEvent) -> Option<u32> {
    match event.aux {
        FILE_SYSCALL_OPENAT
        | FILE_SYSCALL_OPENAT2
        | FILE_SYSCALL_UNLINKAT
        | FILE_SYSCALL_RENAMEAT
        | FILE_SYSCALL_RENAMEAT2
        | FILE_SYSCALL_MKDIRAT => Some(event.arg0 as u32),
        _ => None,
    }
}

pub(super) fn secondary_dirfd(event: &KernelFilePathEvent) -> Option<u32> {
    match event.aux {
        FILE_SYSCALL_RENAMEAT | FILE_SYSCALL_RENAMEAT2 => Some(event.arg2 as u32),
        _ => None,
    }
}

pub(in crate::decode::file_path) fn dup_target_fd(
    event: &KernelFilePathEvent,
    result: i64,
) -> Option<u32> {
    match event.aux {
        FILE_SYSCALL_DUP => u32::try_from(result).ok(),
        FILE_SYSCALL_FCNTL if fcntl_duplicates_fd(event) => u32::try_from(result).ok(),
        FILE_SYSCALL_DUP2 | FILE_SYSCALL_DUP3 => Some(event.arg1 as u32),
        _ => None,
    }
}

pub(in crate::decode::file_path) fn fcntl_duplicates_fd(event: &KernelFilePathEvent) -> bool {
    i32::try_from(event.arg1)
        .is_ok_and(|command| matches!(command, libc::F_DUPFD | libc::F_DUPFD_CLOEXEC))
}

pub(super) fn fcntl_sets_fd_flags(event: &KernelFilePathEvent) -> bool {
    i32::try_from(event.arg1).is_ok_and(|command| command == libc::F_SETFD)
}

pub(super) fn duplicated_fd_close_on_exec(event: &KernelFilePathEvent) -> bool {
    match event.aux {
        FILE_SYSCALL_DUP3 => event.arg2 & libc::O_CLOEXEC as u64 != 0,
        FILE_SYSCALL_FCNTL => {
            i32::try_from(event.arg1).is_ok_and(|command| command == libc::F_DUPFD_CLOEXEC)
        }
        FILE_SYSCALL_DUP | FILE_SYSCALL_DUP2 => false,
        _ => false,
    }
}

pub(super) fn ipc_pair_close_on_exec(event: &KernelFilePathEvent) -> bool {
    match event.aux {
        FILE_SYSCALL_PIPE2 => event.arg2 & libc::O_CLOEXEC as u64 != 0,
        FILE_SYSCALL_SOCKETPAIR => event.arg2 & libc::SOCK_CLOEXEC as u64 != 0,
        FILE_SYSCALL_PIPE => false,
        _ => false,
    }
}

pub(super) fn ipc_kind_from_event(event: &KernelFilePathEvent) -> Option<FdIpcKind> {
    match (event.aux, event.arg1) {
        (FILE_SYSCALL_PIPE | FILE_SYSCALL_PIPE2, FILE_IPC_KIND_PIPE) => Some(FdIpcKind::Pipe),
        (FILE_SYSCALL_SOCKETPAIR, FILE_IPC_KIND_UNIX_SOCKET) => Some(FdIpcKind::UnixSocket),
        _ => None,
    }
}
