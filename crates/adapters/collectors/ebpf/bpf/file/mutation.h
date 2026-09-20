#ifndef ACTRAIL_FILE_MUTATION_H
#define ACTRAIL_FILE_MUTATION_H

#include "observe.h"

SEC("tracepoint/syscalls/sys_enter_unlink")
int handle_sys_enter_unlink(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_enter(ctx,
        file_enter_descriptor(ACTRAIL_FILE_UNLINK, ACTRAIL_FILE_SYSCALL_UNLINK, 1),
        ACTRAIL_FILE_FD_MISSING, (__u64)ctx->args[0], 0);
}

SEC("tracepoint/syscalls/sys_exit_unlink")
int handle_sys_exit_unlink(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_UNLINK, ACTRAIL_FILE_SYSCALL_UNLINK);
}

SEC("tracepoint/syscalls/sys_enter_rename")
int handle_sys_enter_rename(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_enter(ctx,
        file_enter_descriptor(ACTRAIL_FILE_RENAME, ACTRAIL_FILE_SYSCALL_RENAME, 2),
        ACTRAIL_FILE_FD_MISSING, (__u64)ctx->args[0], (__u64)ctx->args[1]);
}

SEC("tracepoint/syscalls/sys_exit_rename")
int handle_sys_exit_rename(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_RENAME, ACTRAIL_FILE_SYSCALL_RENAME);
}

SEC("tracepoint/syscalls/sys_enter_renameat2")
int handle_sys_enter_renameat2(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_enter(ctx,
        file_enter_descriptor(ACTRAIL_FILE_RENAME, ACTRAIL_FILE_SYSCALL_RENAMEAT2, 5),
        ACTRAIL_FILE_FD_MISSING, (__u64)ctx->args[1], (__u64)ctx->args[3]);
}

SEC("tracepoint/syscalls/sys_exit_renameat2")
int handle_sys_exit_renameat2(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_RENAME, ACTRAIL_FILE_SYSCALL_RENAMEAT2);
}

SEC("tracepoint/syscalls/sys_enter_mkdir")
int handle_sys_enter_mkdir(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_enter(ctx,
        file_enter_descriptor(ACTRAIL_FILE_MKDIR, ACTRAIL_FILE_SYSCALL_MKDIR, 2),
        ACTRAIL_FILE_FD_MISSING, (__u64)ctx->args[0], 0);
}

SEC("tracepoint/syscalls/sys_exit_mkdir")
int handle_sys_exit_mkdir(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_MKDIR, ACTRAIL_FILE_SYSCALL_MKDIR);
}

SEC("tracepoint/syscalls/sys_enter_rmdir")
int handle_sys_enter_rmdir(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_enter(ctx,
        file_enter_descriptor(ACTRAIL_FILE_RMDIR, ACTRAIL_FILE_SYSCALL_RMDIR, 1),
        ACTRAIL_FILE_FD_MISSING, (__u64)ctx->args[0], 0);
}

SEC("tracepoint/syscalls/sys_exit_rmdir")
int handle_sys_exit_rmdir(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_RMDIR, ACTRAIL_FILE_SYSCALL_RMDIR);
}

SEC("tracepoint/syscalls/sys_enter_truncate")
int handle_sys_enter_truncate(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_enter(ctx,
        file_enter_descriptor(ACTRAIL_FILE_TRUNCATE, ACTRAIL_FILE_SYSCALL_TRUNCATE, 2),
        ACTRAIL_FILE_FD_MISSING, (__u64)ctx->args[0], 0);
}

SEC("tracepoint/syscalls/sys_exit_truncate")
int handle_sys_exit_truncate(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_TRUNCATE, ACTRAIL_FILE_SYSCALL_TRUNCATE);
}

SEC("tracepoint/syscalls/sys_enter_ftruncate")
int handle_sys_enter_ftruncate(struct trace_event_raw_sys_enter *ctx) {
    return capture_file_completion_enter(ctx, ACTRAIL_FILE_SYSCALL_FTRUNCATE,
        (__u32)ctx->args[0], 2);
}

SEC("tracepoint/syscalls/sys_exit_ftruncate")
int handle_sys_exit_ftruncate(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_completion_exit(ctx, ACTRAIL_FILE_TRUNCATE, ACTRAIL_FILE_SYSCALL_FTRUNCATE);
}

#endif
