#ifndef ACTRAIL_FILE_COMPLETION_H
#define ACTRAIL_FILE_COMPLETION_H

// Scalar-only file operations keep one pending observation per thread.
// Kernel FD classification and reference maintenance remain independent.
static __always_inline int file_fd_has_context(__u32 pid, __u32 fd) {
    struct actrail_fd_state *state = fd_lookup(pid, fd);
    return state && (state->category == ACTRAIL_FD_CATEGORY_DIRECTORY
        || state->category == ACTRAIL_FD_CATEGORY_FILE);
}

static __always_inline int file_completion_needed(
    struct trace_event_raw_sys_enter *ctx,
    __u32 syscall_id,
    __u32 fd
) {
    __u32 flags = file_capture_flags();
    __u32 pid = (__u32)(current_kernel_pid_tgid() >> 32);
    if (syscall_id == ACTRAIL_FILE_SYSCALL_MMAP
        || syscall_id == ACTRAIL_FILE_SYSCALL_FTRUNCATE) {
        return flags & ACTRAIL_CAPTURE_FILE_FD_MUTATIONS;
    }
    if (flags & ACTRAIL_CAPTURE_MCP_STDIO) {
        return 1;
    }
    if (syscall_id == ACTRAIL_FILE_SYSCALL_CLOSE_RANGE) {
        return fd_process_active_count(pid) != 0;
    }
    if (file_fd_has_context(pid, fd)) {
        return 1;
    }
    return (syscall_id == ACTRAIL_FILE_SYSCALL_DUP2
        || syscall_id == ACTRAIL_FILE_SYSCALL_DUP3)
        && file_fd_has_context(pid, (__u32)ctx->args[1]);
}

static __always_inline int capture_file_completion_enter(
    struct trace_event_raw_sys_enter *ctx,
    __u32 syscall_id,
    __u32 fd,
    __u32 arg_count
) {
    __u64 operation_key = current_kernel_pid_tgid();
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id;
    struct actrail_pending_file_completion op = {};

    if (!operation_key || !file_context_capture_enabled(syscall_id)) {
        return 0;
    }
    bpf_map_delete_elem(&pending_file_completion_ops, &operation_key);
    if (!file_completion_needed(ctx, syscall_id, fd)) {
        return 0;
    }
    trace_id = lookup_current_detailed_trace(&tgid, &tid, &lookup_flags);
    if (!tgid || !trace_id) {
        return 0;
    }
    op.trace_id = *trace_id;
    op.pid_generation = current_process_start_time(tgid);
    op.pid = tgid;
    op.tid = tid;
    op.fd = fd;
    op.syscall_id = syscall_id;
#pragma unroll
    for (int index = 0; index < 6; index++) {
        if (index < arg_count) {
            op.args[index] = ctx->args[index];
        }
    }
    if (bpf_map_update_elem(&pending_file_completion_ops, &operation_key, &op, BPF_ANY)) {
        event_transport_diag_inc(ACTRAIL_FILE_PENDING_UPDATE_FAIL);
    }
    return 0;
}

static __always_inline int capture_file_fcntl_enter(struct trace_event_raw_sys_enter *ctx) {
    __u32 command = (__u32)ctx->args[1];
    __u32 flags = file_capture_flags();

    if (command != F_DUPFD && command != F_DUPFD_CLOEXEC
        && !(command == F_SETFD && (flags & ACTRAIL_CAPTURE_MCP_STDIO))) {
        return 0;
    }
    return capture_file_completion_enter(
        ctx, ACTRAIL_FILE_SYSCALL_FCNTL, (__u32)ctx->args[0], 3
    );
}

static __always_inline int emit_file_completion_exit(
    struct trace_event_raw_sys_exit *ctx,
    __u32 kind,
    __u32 syscall_id
) {
    __u64 operation_key = current_kernel_pid_tgid();
    struct actrail_pending_file_completion *pending;
    struct actrail_pending_file_completion op;
    struct actrail_file_event *event;

    if (!operation_key || !file_context_capture_enabled(syscall_id)) {
        return 0;
    }
    pending = bpf_map_lookup_elem(&pending_file_completion_ops, &operation_key);
    if (!pending || pending->syscall_id != syscall_id) {
        return 0;
    }
    op = *pending;
    bpf_map_delete_elem(&pending_file_completion_ops, &operation_key);
    if (syscall_id == ACTRAIL_FILE_SYSCALL_MMAP && ctx->ret < 0) {
        return 0;
    }
    event = actrail_event_reserve(ACTRAIL_FILE_EVENT_HEADER_SIZE);
    if (!event) {
        return 0;
    }
    init_file_event_header(event, kind);
    event->pid = op.pid;
    event->tid = op.tid;
    event->pid_generation = op.pid_generation;
    event->trace_id = op.trace_id;
    event->phase = ACTRAIL_FILE_PHASE_EXIT;
    event->aux = syscall_id;
    event->fd = op.fd;
    event->arg0 = op.args[0];
    event->arg1 = op.args[1];
    event->arg2 = op.args[2];
    event->arg3 = op.args[3];
    event->arg4 = op.args[4];
    event->arg5 = op.args[5];
    event->result = ctx->ret;
    if (kind == ACTRAIL_FILE_CONTEXT
        && !(file_capture_flags() & (ACTRAIL_CAPTURE_FILE_READ | ACTRAIL_CAPTURE_FILE_WRITE
            | ACTRAIL_CAPTURE_FILE_FD_MUTATIONS | ACTRAIL_CAPTURE_FILE_ENUMERATE))) {
        event->path_flags |= ACTRAIL_FILE_PATH_INTERNAL_CONTEXT;
    }
    actrail_event_submit(ctx, event);
    return 0;
}

#endif
