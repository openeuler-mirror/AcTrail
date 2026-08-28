#ifndef ACTRAIL_FILE_PROGRAMS_H
#define ACTRAIL_FILE_PROGRAMS_H

#include "observe.h"
#include "open.h"

SEC("tracepoint/syscalls/sys_enter_pipe")
int handle_sys_enter_pipe(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_ipc_fd_pair_op(
        ctx,
        ACTRAIL_FILE_IPC_FD_PIPE,
        0,
        0,
        0
    );
}

SEC("tracepoint/syscalls/sys_exit_pipe")
int handle_sys_exit_pipe(struct trace_event_raw_sys_exit *ctx) {
    return emit_ipc_fd_pair_exit(ctx, ACTRAIL_FILE_SYSCALL_PIPE);
}

SEC("tracepoint/syscalls/sys_enter_pipe2")
int handle_sys_enter_pipe2(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_ipc_fd_pair_op(
        ctx,
        ACTRAIL_FILE_IPC_FD_PIPE,
        0,
        0,
        (__u32)ctx->args[1]
    );
}

SEC("tracepoint/syscalls/sys_exit_pipe2")
int handle_sys_exit_pipe2(struct trace_event_raw_sys_exit *ctx) {
    return emit_ipc_fd_pair_exit(ctx, ACTRAIL_FILE_SYSCALL_PIPE2);
}

SEC("tracepoint/syscalls/sys_enter_socketpair")
int handle_sys_enter_socketpair(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_ipc_fd_pair_op(
        ctx,
        ACTRAIL_FILE_IPC_FD_UNIX_SOCKET,
        3,
        (__u32)ctx->args[0],
        (__u32)ctx->args[1]
    );
}

SEC("tracepoint/syscalls/sys_exit_socketpair")
int handle_sys_exit_socketpair(struct trace_event_raw_sys_exit *ctx) {
    return emit_ipc_fd_pair_exit(ctx, ACTRAIL_FILE_SYSCALL_SOCKETPAIR);
}

SEC("tracepoint/syscalls/sys_enter_open")
int handle_sys_enter_open(struct trace_event_raw_sys_enter *ctx) {
    fd_open_enter(ctx, (__u64)ctx->args[1]);
    return emit_file_open_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_open")
int handle_sys_exit_open(struct trace_event_raw_sys_exit *ctx) {
    fd_register_open_exit(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_OPEN, ACTRAIL_FILE_SYSCALL_OPEN);
}

SEC("tracepoint/syscalls/sys_enter_openat")
int handle_sys_enter_openat(struct trace_event_raw_sys_enter *ctx) {
    fd_open_enter(ctx, (__u64)ctx->args[2]);
    return emit_file_openat_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_openat")
int handle_sys_exit_openat(struct trace_event_raw_sys_exit *ctx) {
    fd_register_open_exit(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_OPEN, ACTRAIL_FILE_SYSCALL_OPENAT);
}

SEC("tracepoint/syscalls/sys_enter_openat2")
int handle_sys_enter_openat2(struct trace_event_raw_sys_enter *ctx) {
    struct actrail_open_how how = {};
    read_file_open_how(ctx, &how);
    fd_open_enter(ctx, how.flags);
    return emit_file_openat2_enter(ctx, &how);
}

SEC("tracepoint/syscalls/sys_exit_openat2")
int handle_sys_exit_openat2(struct trace_event_raw_sys_exit *ctx) {
    fd_register_open_exit(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_OPEN, ACTRAIL_FILE_SYSCALL_OPENAT2);
}

SEC("tracepoint/syscalls/sys_enter_creat")
int handle_sys_enter_creat(struct trace_event_raw_sys_enter *ctx) {
    fd_open_enter(ctx, 0);
    return emit_file_creat_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_creat")
int handle_sys_exit_creat(struct trace_event_raw_sys_exit *ctx) {
    fd_register_open_exit(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_OPEN, ACTRAIL_FILE_SYSCALL_CREAT);
}

SEC("tracepoint/syscalls/sys_enter_unlinkat")
int handle_sys_enter_unlinkat(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_unlinkat_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_unlinkat")
int handle_sys_exit_unlinkat(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_UNLINK, ACTRAIL_FILE_SYSCALL_UNLINKAT);
}

SEC("tracepoint/syscalls/sys_enter_renameat")
int handle_sys_enter_renameat(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_renameat_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_renameat")
int handle_sys_exit_renameat(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_RENAME, ACTRAIL_FILE_SYSCALL_RENAMEAT);
}

SEC("tracepoint/syscalls/sys_enter_mkdirat")
int handle_sys_enter_mkdirat(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_mkdirat_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_mkdirat")
int handle_sys_exit_mkdirat(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_MKDIR, ACTRAIL_FILE_SYSCALL_MKDIRAT);
}

SEC("tracepoint/syscalls/sys_enter_mmap")
int handle_sys_enter_mmap(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_mmap_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_mmap")
int handle_sys_exit_mmap(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_MMAP, ACTRAIL_FILE_SYSCALL_MMAP);
}

SEC("tracepoint/syscalls/sys_enter_chdir")
int handle_sys_enter_chdir(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_chdir_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_chdir")
int handle_sys_exit_chdir(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_CHDIR);
}

SEC("tracepoint/syscalls/sys_enter_fchdir")
int handle_sys_enter_fchdir(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_fchdir_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_fchdir")
int handle_sys_exit_fchdir(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_FCHDIR);
}


#endif
