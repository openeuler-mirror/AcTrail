#ifndef ACTRAIL_NET_H
#define ACTRAIL_NET_H

#include "actrail_runtime.h"

static __always_inline __u64 net_descriptor(__u32 kind, __u32 syscall_family) {
    return ((__u64)syscall_family << 32) | kind;
}

static __always_inline int store_pending_net_op_resolved(
    __u64 descriptor,
    __u32 fd,
    __u64 requested_size,
    __u64 sockaddr_ptr
) {
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id = lookup_current_trace(&tgid, &tid, &lookup_flags);
    __u64 pid_tgid = ((__u64)tgid << 32) | tid;
    struct actrail_pending_net_op op = {};

    if (!tgid) {
        return 0;
    }
    if (!trace_id) {
        return 0;
    }

    op.trace_id = *trace_id;
    op.kind = (__u32)descriptor;
    op.fd = fd;
    if (is_suppressed_fd(tgid, op.fd)) {
        return 0;
    }
    op.syscall_family = (__u32)(descriptor >> 32);
    op.requested_size = requested_size;
    op.sockaddr_ptr = sockaddr_ptr;
    bpf_map_update_elem(&pending_net_ops, &pid_tgid, &op, BPF_ANY);
    return 0;
}

static __always_inline int emit_pending_net_op(struct trace_event_raw_sys_exit *ctx) {
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id = lookup_current_trace(&tgid, &tid, &lookup_flags);
    __u64 pid_tgid = ((__u64)tgid << 32) | tid;
    struct actrail_pending_net_op *op = bpf_map_lookup_elem(&pending_net_ops, &pid_tgid);
    struct actrail_event event;
    struct actrail_endpoint remote = {};

    if (!tgid || !trace_id) {
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
    emit_event(ctx, &event);
    bpf_map_delete_elem(&pending_net_ops, &pid_tgid);
    return 0;
}

#endif
