#ifndef ACTRAIL_NET_H
#define ACTRAIL_NET_H

#include "actrail_runtime.h"

static __always_inline int store_pending_net_op(
    struct trace_event_raw_sys_enter *ctx,
    __u32 kind,
    __u32 fd_arg,
    __u32 size_arg,
    __u32 sockaddr_arg,
    __u32 syscall_family
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = current_namespace_tgid();
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_pending_net_op op = {};

    if (!tgid) {
        return 0;
    }
    if (!trace_id) {
        return 0;
    }

    op.trace_id = *trace_id;
    op.kind = kind;
    op.fd = (__u32)ctx->args[fd_arg];
    op.syscall_family = syscall_family;
    op.requested_size =
        size_arg < ACTRAIL_SYSCALL_ARG_MISSING ? (__u64)ctx->args[size_arg] : 0;
    op.sockaddr_ptr =
        sockaddr_arg < ACTRAIL_SYSCALL_ARG_MISSING ? (__u64)ctx->args[sockaddr_arg] : 0;
    bpf_map_update_elem(&pending_net_ops, &pid_tgid, &op, BPF_ANY);
    return 0;
}

static __always_inline int emit_pending_net_op(struct trace_event_raw_sys_exit *ctx) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = current_namespace_tgid();
    struct actrail_pending_net_op *op = bpf_map_lookup_elem(&pending_net_ops, &pid_tgid);
    struct actrail_event event;
    struct actrail_endpoint remote = {};

    if (!tgid) {
        return 0;
    }
    if (!op) {
        return 0;
    }

    if (op->kind == ACTRAIL_NET_ACCEPT && ctx->ret < 0) {
        bpf_map_delete_elem(&pending_net_ops, &pid_tgid);
        return 0;
    }

    init_event(&event, op->kind, tgid, op->trace_id);
    event.aux = op->syscall_family;
    event.result = (__s32)ctx->ret;
    event.fd = op->fd;
    event.requested_size = op->requested_size;
    if (op->kind == ACTRAIL_NET_ACCEPT && ctx->ret >= 0) {
        event.fd = (__u32)ctx->ret;
    }
    read_endpoint(op->sockaddr_ptr, &remote);
    if (op->kind == ACTRAIL_NET_BIND) {
        event.local = remote;
    } else {
        event.remote = remote;
    }
    emit_event(&event);
    bpf_map_delete_elem(&pending_net_ops, &pid_tgid);
    return 0;
}

#endif
