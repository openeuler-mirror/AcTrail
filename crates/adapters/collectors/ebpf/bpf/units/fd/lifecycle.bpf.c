/* Programs owned by the fd/lifecycle loading node. */
#include "../definitions.h"

SEC("tracepoint/sched/sched_process_exec")
int handle_fd_sched_process_exec(struct sched_process_exec_ctx *ctx) {
    fd_process_exec_cleanup(current_kernel_tgid(), 0, ctx);
    return 0;
}

SEC("raw_tracepoint/sched_process_fork")
int handle_fd_sched_process_fork(struct bpf_raw_tracepoint_args *ctx) {
    struct task_struct *parent_task = (struct task_struct *)ctx->args[0];
    struct task_struct *child_task = (struct task_struct *)ctx->args[1];
    __u32 parent_host_pid = 0;
    __u32 child_host_pid = 0;

    if (!parent_task || !child_task || !fd_tracking_enabled()) {
        return 0;
    }
    if (ACTRAIL_CORE_READ(&parent_host_pid, parent_task, tgid) != 0 ||
        ACTRAIL_CORE_READ(&child_host_pid, child_task, tgid) != 0 ||
        !parent_host_pid || !child_host_pid || parent_host_pid == child_host_pid) {
        return 0;
    }
    if (!process_observation_child_is_detailed(parent_host_pid)) {
        return 0;
    }
    fd_fork_seed(parent_host_pid, child_host_pid);
    return 0;
}

SEC("tracepoint/syscalls/sys_enter_close")
int handle_sys_enter_close(struct trace_event_raw_sys_enter *ctx) {
    fd_close_dispatch_enter(ctx);
    if (suppressed_fd_close_enter(ctx)) {
        return 0;
    }
    return capture_file_close_enter(ctx);
}

SEC("tracepoint/syscalls/sys_enter_close_range")
int handle_sys_enter_close_range(struct trace_event_raw_sys_enter *ctx) {
    fd_close_range_dispatch_enter(ctx);
    return emit_file_close_range_enter(ctx);
}

SEC("tracepoint/syscalls/sys_enter_dup")
int handle_sys_enter_dup(struct trace_event_raw_sys_enter *ctx) {
    fd_dup_enter(ctx, (__u32)ctx->args[0], 0, ACTRAIL_FD_DUP_RET_FD);
    if (suppressed_fd_dup_enter(
            (__u32)ctx->args[0],
            0,
            0,
            ACTRAIL_SUPPRESSED_FD_DUP_RET_FD
        )) {
        socket_payload_dup_enter(
            (__u32)ctx->args[0],
            0,
            ACTRAIL_SOCKET_DUP_RET_FD
        );
        return 0;
    }
    socket_payload_dup_enter(
        (__u32)ctx->args[0],
        0,
        ACTRAIL_SOCKET_DUP_RET_FD
    );
    return emit_file_dup_enter(ctx);
}

SEC("tracepoint/syscalls/sys_enter_dup2")
int handle_sys_enter_dup2(struct trace_event_raw_sys_enter *ctx) {
    fd_dup_enter(
        ctx,
        (__u32)ctx->args[0],
        (__u32)ctx->args[1],
        ACTRAIL_FD_DUP_TARGET_FD
    );
    if (suppressed_fd_dup_enter(
            (__u32)ctx->args[0],
            (__u32)ctx->args[1],
            1,
            ACTRAIL_SUPPRESSED_FD_DUP_TARGET_FD
        )) {
        socket_payload_dup_enter(
            (__u32)ctx->args[0],
            (__u32)ctx->args[1],
            ACTRAIL_SOCKET_DUP_TARGET_FD
        );
        return 0;
    }
    socket_payload_dup_enter(
        (__u32)ctx->args[0],
        (__u32)ctx->args[1],
        ACTRAIL_SOCKET_DUP_TARGET_FD
    );
    return emit_file_dup2_enter(ctx);
}

SEC("tracepoint/syscalls/sys_enter_dup3")
int handle_sys_enter_dup3(struct trace_event_raw_sys_enter *ctx) {
    fd_dup_enter(
        ctx,
        (__u32)ctx->args[0],
        (__u32)ctx->args[1],
        ACTRAIL_FD_DUP_TARGET_FD
    );
    if (suppressed_fd_dup_enter(
            (__u32)ctx->args[0],
            (__u32)ctx->args[1],
            1,
            ACTRAIL_SUPPRESSED_FD_DUP_TARGET_FD
        )) {
        socket_payload_dup_enter(
            (__u32)ctx->args[0],
            (__u32)ctx->args[1],
            ACTRAIL_SOCKET_DUP_TARGET_FD
        );
        return 0;
    }
    socket_payload_dup_enter(
        (__u32)ctx->args[0],
        (__u32)ctx->args[1],
        ACTRAIL_SOCKET_DUP_TARGET_FD
    );
    return emit_file_dup3_enter(ctx);
}

SEC("tracepoint/syscalls/sys_enter_fcntl")
int handle_sys_enter_fcntl(struct trace_event_raw_sys_enter *ctx) {
    __u32 command = (__u32)ctx->args[1];
    if (command == F_DUPFD || command == F_DUPFD_CLOEXEC) {
        fd_dup_enter(
            ctx,
            (__u32)ctx->args[0],
            0,
            ACTRAIL_FD_DUP_RET_FD
        );
    }
    if (suppressed_fd_fcntl_enter(ctx)) {
        socket_payload_fcntl_enter(ctx);
        return 0;
    }
    socket_payload_fcntl_enter(ctx);
    return capture_file_fcntl_enter(ctx);
}

SEC("tracepoint/syscalls/sys_enter_ioctl")
int handle_sys_enter_ioctl(struct trace_event_raw_sys_enter *ctx) {
    if (!(file_capture_flags() & ACTRAIL_CAPTURE_MCP_STDIO)) {
        return 0;
    }
    __u32 context_kind = fd_ioctl_flag_enter(ctx);

    return emit_file_ioctl_flag_enter(ctx, context_kind);
}

SEC("tracepoint/syscalls/sys_enter_pipe")
int handle_sys_enter_pipe(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_ipc_fd_pair_op(
        ACTRAIL_FILE_IPC_FD_PIPE,
        (__u64)ctx->args[0],
        0,
        0
    );
}

SEC("tracepoint/syscalls/sys_enter_pipe2")
int handle_sys_enter_pipe2(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_ipc_fd_pair_op(
        ACTRAIL_FILE_IPC_FD_PIPE,
        (__u64)ctx->args[0],
        0,
        (__u32)ctx->args[1]
    );
}

SEC("tracepoint/syscalls/sys_enter_socketpair")
int handle_sys_enter_socketpair(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_ipc_fd_pair_op(
        ACTRAIL_FILE_IPC_FD_UNIX_SOCKET,
        (__u64)ctx->args[3],
        (__u32)ctx->args[0],
        (__u32)ctx->args[1]
    );
}

SEC("tracepoint/syscalls/sys_exit_close")
int handle_sys_exit_close(struct trace_event_raw_sys_exit *ctx) {
    fd_close_dispatch_exit(ctx);
    return emit_file_completion_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_CLOSE);
}

SEC("tracepoint/syscalls/sys_exit_close_range")
int handle_sys_exit_close_range(struct trace_event_raw_sys_exit *ctx) {
    fd_close_dispatch_exit(ctx);
    return emit_file_completion_exit(
        ctx,
        ACTRAIL_FILE_CONTEXT,
        ACTRAIL_FILE_SYSCALL_CLOSE_RANGE
    );
}

SEC("tracepoint/syscalls/sys_exit_dup")
int handle_sys_exit_dup(struct trace_event_raw_sys_exit *ctx) {
    fd_dup_exit(ctx);
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    return emit_file_completion_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_DUP);
}

SEC("tracepoint/syscalls/sys_exit_dup2")
int handle_sys_exit_dup2(struct trace_event_raw_sys_exit *ctx) {
    fd_dup_exit(ctx);
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    return emit_file_completion_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_DUP2);
}

SEC("tracepoint/syscalls/sys_exit_dup3")
int handle_sys_exit_dup3(struct trace_event_raw_sys_exit *ctx) {
    fd_dup_exit(ctx);
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    return emit_file_completion_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_DUP3);
}

SEC("tracepoint/syscalls/sys_exit_fcntl")
int handle_sys_exit_fcntl(struct trace_event_raw_sys_exit *ctx) {
    fd_dup_exit(ctx);
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    return emit_file_completion_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_FCNTL);
}

SEC("tracepoint/syscalls/sys_exit_ioctl")
int handle_sys_exit_ioctl(struct trace_event_raw_sys_exit *ctx) {
    __u32 syscall_id = file_ioctl_flag_syscall_id(fd_flag_exit(ctx));

    return syscall_id
        ? emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, syscall_id)
        : 0;
}

SEC("tracepoint/syscalls/sys_exit_pipe")
int handle_sys_exit_pipe(struct trace_event_raw_sys_exit *ctx) {
    return emit_ipc_fd_pair_exit(ctx, ACTRAIL_FILE_SYSCALL_PIPE);
}

SEC("tracepoint/syscalls/sys_exit_pipe2")
int handle_sys_exit_pipe2(struct trace_event_raw_sys_exit *ctx) {
    return emit_ipc_fd_pair_exit(ctx, ACTRAIL_FILE_SYSCALL_PIPE2);
}

SEC("tracepoint/syscalls/sys_exit_socketpair")
int handle_sys_exit_socketpair(struct trace_event_raw_sys_exit *ctx) {
    return emit_ipc_fd_pair_exit(ctx, ACTRAIL_FILE_SYSCALL_SOCKETPAIR);
}

#ifndef ACTRAIL_COMBINED_OBJECT
char LICENSE[] SEC("license") = "GPL";
#endif
