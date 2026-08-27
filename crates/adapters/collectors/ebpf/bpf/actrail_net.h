#ifndef ACTRAIL_NET_H
#define ACTRAIL_NET_H

#include "actrail_runtime.h"
#include "actrail_fd.h"

#define ACTRAIL_NET_EINPROGRESS 115

static __always_inline __u64 net_descriptor(__u32 kind, __u32 syscall_family) {
    return ((__u64)syscall_family << 32) | kind;
}

static __always_inline int store_pending_net_op_with_flags(
    __u64 descriptor,
    __u32 fd,
    __u64 requested_size,
    __u64 sockaddr_ptr,
    __u32 flags
) {
    __u64 operation_key = current_kernel_pid_tgid();
    __u32 kernel_pid = operation_key >> 32;
    __u32 kind = (__u32)descriptor;
    struct actrail_fd_state fd_state = {};
    struct actrail_fd_object_snapshot fd_object = {};
    struct actrail_pending_net_op op = {};
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id;

    if (!operation_key) {
        return 0;
    }
    if (fd_snapshot(kernel_pid, fd, &fd_state, &fd_object)) {
        op.generation = fd_state.generation;
        op.category = fd_state.category;
        op.remote = fd_object.remote;
    } else if (kind == ACTRAIL_FD_IO_SEND || kind == ACTRAIL_FD_IO_RECV
        || kind == ACTRAIL_NET_SHUTDOWN || kind == ACTRAIL_NET_ACCEPT) {
        return 0;
    }

    trace_id = lookup_current_trace(&tgid, &tid, &lookup_flags);
    if (!tgid || !trace_id) {
        return 0;
    }
    if (is_suppressed_fd(tgid, fd)) {
        return 0;
    }
    op.trace_id = *trace_id;
    op.pid = tgid;
    op.kind = kind;
    op.fd = fd;
    op.syscall_family = (__u32)(descriptor >> 32);
    op.flags = flags;
    op.requested_size = requested_size;
    op.sockaddr_ptr = sockaddr_ptr;
    bpf_map_update_elem(&pending_net_ops, &operation_key, &op, BPF_ANY);
    return 0;
}

static __always_inline int store_pending_net_op_resolved(
    __u64 descriptor,
    __u32 fd,
    __u64 requested_size,
    __u64 sockaddr_ptr
) {
    return store_pending_net_op_with_flags(
        descriptor,
        fd,
        requested_size,
        sockaddr_ptr,
        0
    );
}

static __noinline void emit_resolved_net_event(
    struct trace_event_raw_sys_exit *ctx,
    const struct actrail_pending_net_op *op,
    const struct actrail_endpoint *remote
) {
    struct actrail_event event;

    init_event(&event, op->kind, op->pid, op->trace_id);
    event.aux = op->syscall_family;
    event.result = (__s32)ctx->ret;
    event.fd = op->kind == ACTRAIL_NET_ACCEPT && ctx->ret >= 0
        ? (__u32)ctx->ret
        : op->fd;
    event.requested_size = op->requested_size;
    event.aux_generation = op->generation;
    if (op->kind == ACTRAIL_NET_SHUTDOWN) {
        event.reserved = op->flags;
    } else if (op->kind == ACTRAIL_FD_IO_SEND || op->kind == ACTRAIL_FD_IO_RECV) {
        event.reserved = op->category;
    }
    if (op->kind == ACTRAIL_NET_BIND) {
        event.local = *remote;
    } else {
        event.remote = *remote;
    }
    emit_event(ctx, &event);
}

static __always_inline void fd_accept_exit(struct trace_event_raw_sys_exit *ctx) {
    __u64 operation_key = current_kernel_pid_tgid();
    struct actrail_pending_net_op *op =
        bpf_map_lookup_elem(&pending_net_ops, &operation_key);
    struct actrail_fd_registration registration = {};

    if (!op || op->kind != ACTRAIL_NET_ACCEPT || ctx->ret < 0
        || op->category == ACTRAIL_FD_CATEGORY_NONE) {
        return;
    }
    registration.trace_id = op->trace_id;
    registration.program_ctx = ctx;
    registration.pid = op->pid;
    registration.fd = (__u32)ctx->ret;
    registration.category = op->category;
    registration.flags = fd_creation_flags(op->flags);
    fd_register(&registration);
}

static __always_inline int emit_pending_net_op(struct trace_event_raw_sys_exit *ctx) {
    __u64 operation_key = current_kernel_pid_tgid();
    struct actrail_pending_net_op *op =
        bpf_map_lookup_elem(&pending_net_ops, &operation_key);
    struct actrail_endpoint remote = {};

    if (!op || op->kind == ACTRAIL_FD_SOCKET_PENDING_KIND) {
        return 0;
    }
    if (op->kind == ACTRAIL_NET_ACCEPT && ctx->ret < 0) {
        bpf_map_delete_elem(&pending_net_ops, &operation_key);
        return 0;
    }
    read_endpoint(op->sockaddr_ptr, &remote);
    if (remote.family == 0) {
        remote = op->remote;
    }
    if (op->kind == ACTRAIL_NET_CONNECT
        && (ctx->ret == 0 || ctx->ret == -ACTRAIL_NET_EINPROGRESS)) {
        fd_update_endpoint_expected(op->pid, op->fd, op->generation, &remote);
    }
    if (op->kind == ACTRAIL_NET_ACCEPT && ctx->ret >= 0
        && op->category != ACTRAIL_FD_CATEGORY_NONE) {
        fd_update_endpoint(op->pid, (__u32)ctx->ret, &remote);
    }
    emit_resolved_net_event(ctx, op, &remote);
    bpf_map_delete_elem(&pending_net_ops, &operation_key);
    return 0;
}

#endif
