//! Programs grouped by their collection consumers.

pub(super) const PROC_LIFECYCLE_PROGRAMS: &[&str] = &[
    "handle_sched_process_fork",
    "handle_sched_process_exec",
    "handle_sched_process_exit",
    "handle_sys_enter_exit",
    "handle_sys_enter_exit_group",
];

pub(super) const PROCESS_SIGNAL_DIAGNOSTIC_PROGRAMS: &[&str] = &["handle_signal_generate"];

pub(super) const PROCESS_CONTEXT_PROGRAMS: &[&str] = &[
    "handle_sched_process_fork",
    "handle_sched_process_exec",
    "handle_sched_process_exit",
];

pub(super) const PROC_EXEC_CONTEXT_PROGRAMS: &[&str] = &[
    "handle_sched_process_fork",
    "handle_sched_process_exec",
    "handle_sys_enter_execve",
    "handle_sys_exit_execve",
    "handle_sys_enter_execveat",
    "handle_sys_exit_execveat",
    "handle_sys_enter_fork",
    "handle_sys_exit_fork",
    "handle_sys_enter_vfork",
    "handle_sys_exit_vfork",
    "handle_sys_enter_clone",
    "handle_sys_exit_clone",
    "handle_sys_enter_clone3",
    "handle_sys_exit_clone3",
];

pub(super) const TRACKING_REGISTRATION_PROGRAMS: &[&str] =
    &["handle_sched_process_exec", "handle_sched_process_exit"];

pub(super) const ON_DEMAND_PROGRAMS: &[&str] = &["resolve_process_identities"];

pub(super) const FD_PROCESS_LIFECYCLE_PROGRAMS: &[&str] = &[
    "handle_fd_sched_process_fork",
    "handle_fd_sched_process_exec",
];

pub(super) const NET_TRANSPORT_PROGRAMS: &[&str] = &[
    "handle_sched_process_fork",
    "handle_sched_process_exec",
    "handle_sched_process_exit",
    "handle_fd_sched_process_fork",
    "handle_fd_sched_process_exec",
    // fd birth: socket() registers the fd in the unified fd table.
    "handle_sys_enter_socket",
    "handle_sys_exit_socket",
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
    "handle_sys_enter_recvmsg",
    "handle_sys_exit_recvmsg",
    "handle_sys_enter_bind",
    "handle_sys_exit_bind",
    "handle_sys_enter_listen",
    "handle_sys_exit_listen",
    "handle_sys_enter_shutdown",
    "handle_sys_exit_shutdown",
    "handle_sys_enter_write",
    "handle_sys_exit_write",
    "handle_sys_enter_read",
    "handle_sys_exit_read",
    // fd lifecycle: close drives connection termination, dup maintains refcount.
    "handle_sys_enter_close",
    "handle_sys_exit_close",
    "handle_sys_enter_close_range",
    "handle_sys_exit_close_range",
    "handle_sys_enter_dup",
    "handle_sys_exit_dup",
    "handle_sys_enter_dup2",
    "handle_sys_exit_dup2",
    "handle_sys_enter_dup3",
    "handle_sys_exit_dup3",
    "handle_sys_enter_fcntl",
    "handle_sys_exit_fcntl",
];

pub(super) const FS_ACCESS_BASIC_FD_PROGRAMS: &[&str] = &[
    "handle_sys_enter_writev",
    "handle_sys_exit_writev",
    "handle_sys_enter_write",
    "handle_sys_exit_write",
    "handle_sys_enter_read",
    "handle_sys_exit_read",
];

pub(super) const FILE_FD_MUTATION_PROGRAMS: &[&str] =
    &["handle_sys_enter_ftruncate", "handle_sys_exit_ftruncate"];

pub(super) const FS_ACCESS_BASIC_PATH_PROGRAMS: &[&str] = &[
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
    "handle_sys_enter_unlink",
    "handle_sys_exit_unlink",
    "handle_sys_enter_rename",
    "handle_sys_exit_rename",
    "handle_sys_enter_renameat2",
    "handle_sys_exit_renameat2",
    "handle_sys_enter_mkdir",
    "handle_sys_exit_mkdir",
    "handle_sys_enter_rmdir",
    "handle_sys_exit_rmdir",
    "handle_sys_enter_truncate",
    "handle_sys_exit_truncate",
];

pub(super) const FILE_OPEN_PROGRAMS: &[&str] = &[
    "handle_sys_enter_open",
    "handle_sys_exit_open",
    "handle_sys_enter_openat",
    "handle_sys_exit_openat",
    "handle_sys_enter_openat2",
    "handle_sys_exit_openat2",
    "handle_sys_enter_creat",
    "handle_sys_exit_creat",
];

pub(super) const FILE_CONTEXT_PROGRAMS: &[&str] = &[
    "handle_sys_enter_close",
    "handle_sys_exit_close",
    "handle_sys_enter_close_range",
    "handle_sys_exit_close_range",
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

pub(super) const PLATFORM_OPTIONAL_TRACEPOINT_PROGRAMS: &[&str] = &[
    // arm64 exposes process creation through clone/clone3 and has no separate
    // fork/vfork syscall tracepoints. clone3 is likewise absent on older
    // kernels, while clone remains the portable required fallback.
    "handle_sys_enter_fork",
    "handle_sys_exit_fork",
    "handle_sys_enter_vfork",
    "handle_sys_exit_vfork",
    "handle_sys_enter_clone3",
    "handle_sys_exit_clone3",
    "handle_sys_enter_close_range",
    "handle_sys_exit_close_range",
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
    "handle_sys_enter_unlink",
    "handle_sys_exit_unlink",
    "handle_sys_enter_rename",
    "handle_sys_exit_rename",
    "handle_sys_enter_mkdir",
    "handle_sys_exit_mkdir",
    "handle_sys_enter_rmdir",
    "handle_sys_exit_rmdir",
];

/// Programs that carry stdio chunk payloads: read/write on any fd. A trace
/// that attaches only these (and not the pipe/socketpair programs) satisfies
/// `StdioChunk` without satisfying `IpcPipeFifo`/`IpcUnixSocket`.
pub(super) const STDIO_PROGRAMS: &[&str] = &[
    "handle_sys_enter_write",
    "handle_sys_exit_write",
    "handle_sys_enter_read",
    "handle_sys_exit_read",
];

pub(super) const IPC_PIPE_FIFO_PROGRAMS: &[&str] = &[
    "handle_sys_enter_pipe",
    "handle_sys_exit_pipe",
    "handle_sys_enter_pipe2",
    "handle_sys_exit_pipe2",
    "handle_sys_enter_writev",
    "handle_sys_exit_writev",
    "handle_sys_enter_write",
    "handle_sys_exit_write",
    "handle_sys_enter_read",
    "handle_sys_exit_read",
    "handle_sys_enter_close",
    "handle_sys_exit_close",
    "handle_sys_enter_close_range",
    "handle_sys_exit_close_range",
    "handle_sys_enter_dup",
    "handle_sys_exit_dup",
    "handle_sys_enter_dup2",
    "handle_sys_exit_dup2",
    "handle_sys_enter_dup3",
    "handle_sys_exit_dup3",
    "handle_sys_enter_fcntl",
    "handle_sys_exit_fcntl",
    "handle_sched_process_fork",
    "handle_sched_process_exec",
    "handle_sched_process_exit",
    "handle_fd_sched_process_fork",
    "handle_fd_sched_process_exec",
];

pub(super) const IPC_UNIX_SOCKET_PROGRAMS: &[&str] = &[
    "handle_sys_enter_socket",
    "handle_sys_exit_socket",
    "handle_sys_enter_socketpair",
    "handle_sys_exit_socketpair",
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
    "handle_sys_enter_recvmsg",
    "handle_sys_exit_recvmsg",
    "handle_sys_enter_write",
    "handle_sys_exit_write",
    "handle_sys_enter_read",
    "handle_sys_exit_read",
    "handle_sys_enter_close",
    "handle_sys_exit_close",
    "handle_sys_enter_close_range",
    "handle_sys_exit_close_range",
    "handle_sys_enter_dup",
    "handle_sys_exit_dup",
    "handle_sys_enter_dup2",
    "handle_sys_exit_dup2",
    "handle_sys_enter_dup3",
    "handle_sys_exit_dup3",
    "handle_sys_enter_fcntl",
    "handle_sys_exit_fcntl",
    "handle_sched_process_fork",
    "handle_sched_process_exec",
    "handle_sched_process_exit",
    "handle_fd_sched_process_fork",
    "handle_fd_sched_process_exec",
];

pub(super) const SOCKET_PAYLOAD_PROGRAMS: &[&str] = &[
    "handle_sys_enter_socket",
    "handle_sys_exit_socket",
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
    "handle_sys_enter_close_range",
    "handle_sys_exit_close_range",
    "handle_sys_enter_dup",
    "handle_sys_exit_dup",
    "handle_sys_enter_dup2",
    "handle_sys_exit_dup2",
    "handle_sys_enter_dup3",
    "handle_sys_exit_dup3",
    "handle_sys_enter_fcntl",
    "handle_sys_exit_fcntl",
    "handle_fd_sched_process_fork",
    "handle_fd_sched_process_exec",
    "handle_sched_process_exit",
];

pub(super) const FS_MMAP_PROGRAMS: &[&str] = &["handle_sys_enter_mmap", "handle_sys_exit_mmap"];

// Dedicated to MCP stdio endpoint association. Ordinary IPC selects its own
// programs independently; removing MCP removes this additional requirement.
pub(super) const MCP_STDIO_CONTEXT_PROGRAMS: &[&str] = &[
    "handle_sys_enter_pipe",
    "handle_sys_exit_pipe",
    "handle_sys_enter_pipe2",
    "handle_sys_exit_pipe2",
    "handle_sys_enter_socketpair",
    "handle_sys_exit_socketpair",
    "handle_sys_enter_close",
    "handle_sys_exit_close",
    "handle_sys_enter_close_range",
    "handle_sys_exit_close_range",
    "handle_sys_enter_dup",
    "handle_sys_exit_dup",
    "handle_sys_enter_dup2",
    "handle_sys_exit_dup2",
    "handle_sys_enter_dup3",
    "handle_sys_exit_dup3",
    "handle_sys_enter_fcntl",
    "handle_sys_exit_fcntl",
    "handle_sys_enter_ioctl",
    "handle_sys_exit_ioctl",
];

pub(super) const FILE_IO_SUMMARY_PROGRAMS: &[&str] = &[
    "handle_file_io_read",
    "handle_file_io_readv",
    "handle_file_io_write",
    "handle_file_io_writev",
    "handle_file_io_permission",
    "handle_file_io_free",
];
