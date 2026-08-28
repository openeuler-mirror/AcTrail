#ifndef ACTRAIL_FD_PROGRAMS_H
#define ACTRAIL_FD_PROGRAMS_H

#include "lifecycle.h"
#include "../file/observe.h"
#include "../file/bulk_read.h"
#include "../payload/socket_capture.h"

SEC("tracepoint/syscalls/sys_enter_close")
int handle_sys_enter_close(struct trace_event_raw_sys_enter *ctx) {
    store_file_bulk_read_fast_close_op(ctx);
    fd_close_dispatch_enter(ctx);
    if (suppressed_fd_close_enter(ctx)) {
        return 0;
    }
    return emit_file_close_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_close")
int handle_sys_exit_close(struct trace_event_raw_sys_exit *ctx) {
    fd_close_dispatch_exit(ctx);
    emit_file_bulk_read_fast_close_op(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_CLOSE);
}

SEC("tracepoint/syscalls/sys_enter_close_range")
int handle_sys_enter_close_range(struct trace_event_raw_sys_enter *ctx) {
    fd_close_range_dispatch_enter(ctx);
    return emit_file_close_range_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_close_range")
int handle_sys_exit_close_range(struct trace_event_raw_sys_exit *ctx) {
    fd_close_dispatch_exit(ctx);
    return emit_file_exit(
        ctx,
        ACTRAIL_FILE_CONTEXT,
        ACTRAIL_FILE_SYSCALL_CLOSE_RANGE
    );
}

SEC("tracepoint/syscalls/sys_enter_dup")
int handle_sys_enter_dup(struct trace_event_raw_sys_enter *ctx) {
    fd_dup_enter(ctx, (__u32)ctx->args[0], 0, ACTRAIL_FD_DUP_RET_FD, 0);
    if (suppressed_fd_dup_enter(
            (__u32)ctx->args[0],
            0,
            0,
            ACTRAIL_SUPPRESSED_FD_DUP_RET_FD
        )) {
        socket_payload_dup_enter(
            ctx,
            0,
            ACTRAIL_SYSCALL_ARG_MISSING,
            ACTRAIL_SOCKET_DUP_RET_FD
        );
        return 0;
    }
    socket_payload_dup_enter(
        ctx,
        0,
        ACTRAIL_SYSCALL_ARG_MISSING,
        ACTRAIL_SOCKET_DUP_RET_FD
    );
    store_file_bulk_read_fast_dup_op(
        (__u32)ctx->args[0],
        0,
        0,
        ACTRAIL_FILE_BULK_READ_FAST_DUP_RET_FD
    );
    return emit_file_dup_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_dup")
int handle_sys_exit_dup(struct trace_event_raw_sys_exit *ctx) {
    fd_dup_exit(ctx);
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    emit_file_bulk_read_fast_dup_op(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_DUP);
}

SEC("tracepoint/syscalls/sys_enter_dup2")
int handle_sys_enter_dup2(struct trace_event_raw_sys_enter *ctx) {
    fd_dup_enter(
        ctx,
        (__u32)ctx->args[0],
        (__u32)ctx->args[1],
        ACTRAIL_FD_DUP_TARGET_FD,
        0
    );
    if (suppressed_fd_dup_enter(
            (__u32)ctx->args[0],
            (__u32)ctx->args[1],
            1,
            ACTRAIL_SUPPRESSED_FD_DUP_TARGET_FD
        )) {
        socket_payload_dup_enter(ctx, 0, 1, ACTRAIL_SOCKET_DUP_TARGET_FD);
        return 0;
    }
    socket_payload_dup_enter(ctx, 0, 1, ACTRAIL_SOCKET_DUP_TARGET_FD);
    store_file_bulk_read_fast_dup_op(
        (__u32)ctx->args[0],
        (__u32)ctx->args[1],
        1,
        ACTRAIL_FILE_BULK_READ_FAST_DUP_TARGET_FD
    );
    return emit_file_dup2_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_dup2")
int handle_sys_exit_dup2(struct trace_event_raw_sys_exit *ctx) {
    fd_dup_exit(ctx);
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    emit_file_bulk_read_fast_dup_op(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_DUP2);
}

SEC("tracepoint/syscalls/sys_enter_dup3")
int handle_sys_enter_dup3(struct trace_event_raw_sys_enter *ctx) {
    fd_dup_enter(
        ctx,
        (__u32)ctx->args[0],
        (__u32)ctx->args[1],
        ACTRAIL_FD_DUP_TARGET_FD,
        fd_creation_flags((__u64)ctx->args[2])
    );
    if (suppressed_fd_dup_enter(
            (__u32)ctx->args[0],
            (__u32)ctx->args[1],
            1,
            ACTRAIL_SUPPRESSED_FD_DUP_TARGET_FD
        )) {
        socket_payload_dup_enter(ctx, 0, 1, ACTRAIL_SOCKET_DUP_TARGET_FD);
        return 0;
    }
    socket_payload_dup_enter(ctx, 0, 1, ACTRAIL_SOCKET_DUP_TARGET_FD);
    store_file_bulk_read_fast_dup_op(
        (__u32)ctx->args[0],
        (__u32)ctx->args[1],
        1,
        ACTRAIL_FILE_BULK_READ_FAST_DUP_TARGET_FD
    );
    return emit_file_dup3_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_dup3")
int handle_sys_exit_dup3(struct trace_event_raw_sys_exit *ctx) {
    fd_dup_exit(ctx);
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    emit_file_bulk_read_fast_dup_op(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_DUP3);
}

SEC("tracepoint/syscalls/sys_enter_fcntl")
int handle_sys_enter_fcntl(struct trace_event_raw_sys_enter *ctx) {
    __u32 command = (__u32)ctx->args[1];
    if (command == F_DUPFD || command == F_DUPFD_CLOEXEC) {
        fd_dup_enter(
            ctx,
            (__u32)ctx->args[0],
            0,
            ACTRAIL_FD_DUP_RET_FD,
            command == F_DUPFD_CLOEXEC ? ACTRAIL_FD_FLAG_CLOEXEC : 0
        );
    }
    fd_fcntl_flag_enter(ctx);
    if (suppressed_fd_fcntl_enter(ctx)) {
        socket_payload_fcntl_enter(ctx);
        return 0;
    }
    socket_payload_fcntl_enter(ctx);
    store_file_bulk_read_fast_fcntl_op(ctx);
    return emit_file_fcntl_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_fcntl")
int handle_sys_exit_fcntl(struct trace_event_raw_sys_exit *ctx) {
    fd_dup_exit(ctx);
    fd_flag_exit(ctx);
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    emit_file_bulk_read_fast_dup_op(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_FCNTL);
}

SEC("tracepoint/syscalls/sys_enter_ioctl")
int handle_sys_enter_ioctl(struct trace_event_raw_sys_enter *ctx) {
    __u32 context_kind = fd_ioctl_flag_enter(ctx);

    return emit_file_ioctl_flag_enter(ctx, context_kind);
}

SEC("tracepoint/syscalls/sys_exit_ioctl")
int handle_sys_exit_ioctl(struct trace_event_raw_sys_exit *ctx) {
    __u32 syscall_id = file_ioctl_flag_syscall_id(fd_flag_exit(ctx));

    return syscall_id
        ? emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, syscall_id)
        : 0;
}


#endif
