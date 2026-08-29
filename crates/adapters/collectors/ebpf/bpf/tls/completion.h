#ifndef ACTRAIL_TLS_COMPLETION_H
#define ACTRAIL_TLS_COMPLETION_H

#include "capture.h"
#include "diagnostics.h"


static __always_inline void capture_tls_payload_after_completion(
    void *ctx,
    const struct actrail_pending_tls_payload_op *op,
    __u32 tgid,
    __u32 tid,
    __u64 completed_size,
    __u32 flags
) {
    __u32 backend;

    if ((flags & ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED) != 0 ||
        completed_size == 0 ||
        op->direction != ACTRAIL_TLS_PAYLOAD_INBOUND) {
        return;
    }

    backend = payload_tls_capture_backend();
    if ((backend == ACTRAIL_TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK ||
         backend == ACTRAIL_TLS_BACKEND_BPF_COPY_ONLY) &&
        emit_tls_direct_capture(ctx, op, tgid, tid, completed_size) == 1) {
        return;
    }
    if (backend == ACTRAIL_TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK ||
        backend == ACTRAIL_TLS_BACKEND_SECCOMP_USER_READ) {
        emit_tls_capture_request(ctx, op, tgid, tid, completed_size);
    }
}

static __always_inline int emit_tls_payload_completion(
    void *ctx,
    __u64 completed_size,
    __u32 flags
) {
    __u64 host_pid_tgid = current_pid_tgid();
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    __u64 namespace_pid_tgid = host_pid_tgid;
    __u32 tgid = host_pid_tgid >> 32;
    __u32 tid = (__u32)host_pid_tgid;
    struct actrail_pending_tls_payload_op *op =
        bpf_map_lookup_elem(&pending_tls_payload_ops, &host_pid_tgid);
    struct actrail_tls_completion_event *event;

    tls_diag_inc(ACTRAIL_TLS_DIAG_COMPLETION_TOTAL);
    if (!op) {
        __u32 lookup_flags = 0;
        __u64 *trace_id = lookup_current_trace(&tgid, &tid, &lookup_flags);

        if (trace_id) {
            namespace_pid_tgid = current_trace_pid_tgid(*trace_id);
        }
        tls_diag_inc(ACTRAIL_TLS_DIAG_COMPLETION_MISSING_PENDING);
        emit_tls_payload_diagnostic_event(
            ctx,
            ACTRAIL_TLS_DIAG_EVENT_COMPLETION_MISSING_PENDING,
            host_pid_tgid,
            namespace_pid_tgid,
            0,
            0,
            completed_size,
            0
        );
        bpf_map_delete_elem(&pending_tls_payload_ops, &host_pid_tgid);
        bpf_map_delete_elem(&tls_pending_ns, &namespace_pid_tgid);
        return 0;
    }
    namespace_pid_tgid = current_trace_pid_tgid(op->trace_id);
    if (!namespace_pid_tgid) {
        namespace_pid_tgid = host_pid_tgid;
    }

    capture_tls_payload_after_completion(ctx, op, tgid, tid, completed_size, flags);

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_COMPLETION_RESERVE_FAIL);
        bpf_map_delete_elem(&pending_tls_payload_ops, &host_pid_tgid);
        bpf_map_delete_elem(&tls_pending_ns, &namespace_pid_tgid);
        return 0;
    }

    event->kind = ACTRAIL_TLS_PAYLOAD_COMPLETION;
    event->pid = tgid;
    event->tid = tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->stream_key = op->stream_key;
    event->operation_id = op->operation_id;
    event->completed_size = completed_size > 0xffffffffULL ? 0xffffffffU : (__u32)completed_size;
    event->flags = flags;
    event->symbol = op->symbol;
    event->library = op->library;
    event->pid_generation = op->pid_generation;
    event->buffer_ptr = op->buffer_ptr;
    event->host_pid = kernel_pid_tgid >> 32;
    event->host_tid = (__u32)kernel_pid_tgid;
    actrail_event_submit(ctx, event);
    tls_diag_inc(ACTRAIL_TLS_DIAG_COMPLETION_SUBMIT_OK);
    bpf_map_delete_elem(&pending_tls_payload_ops, &host_pid_tgid);
    bpf_map_delete_elem(&tls_pending_ns, &namespace_pid_tgid);
    return 0;
}

static __always_inline int emit_tls_payload_completion_from_return(struct pt_regs *ctx) {
    int result = (int)ACTRAIL_UPROBE_RET(ctx);

    if (result <= 0) {
        return emit_tls_payload_completion(ctx, 0, ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED);
    }
    return emit_tls_payload_completion(ctx, (__u64)result, 0);
}

static __always_inline int emit_tls_payload_completion_from_isize_return(struct pt_regs *ctx) {
    long result = (long)ACTRAIL_UPROBE_RET(ctx);

    if (result <= 0) {
        return emit_tls_payload_completion(ctx, 0, ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED);
    }
    return emit_tls_payload_completion(ctx, (__u64)result, 0);
}

static __always_inline int emit_tls_payload_completion_from_size_ptr(struct pt_regs *ctx) {
    __u64 pid_tgid = current_pid_tgid();
    struct actrail_pending_tls_payload_op *op =
        bpf_map_lookup_elem(&pending_tls_payload_ops, &pid_tgid);
    __u64 written = 0;
    long result = (long)ACTRAIL_UPROBE_RET(ctx);

    if (result != 1 || !op || !op->size_ptr) {
        return emit_tls_payload_completion(ctx, 0, ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED);
    }
    if (bpf_probe_read_user(&written, sizeof(written), (void *)(unsigned long)op->size_ptr) != 0) {
        return emit_tls_payload_completion(ctx, 0, ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED);
    }
    return emit_tls_payload_completion(ctx, written, 0);
}

static __always_inline int emit_tls_payload_completion_from_rust_result_usize(struct pt_regs *ctx) {
    __u64 result_tag = ACTRAIL_UPROBE_RET(ctx);

    if (result_tag != 0) {
        return emit_tls_payload_completion(ctx, 0, ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED);
    }
    return emit_tls_payload_completion(ctx, ACTRAIL_UPROBE_RET2(ctx), 0);
}

static __always_inline int emit_tls_immediate_payload_args(
    void *ctx,
    const struct actrail_tls_immediate_payload_args *args
) {
    __u32 direction = args->direction;
    __u32 symbol = args->symbol;
    __u32 library = args->library;
    __u64 stream_key = args->stream_key;
    __u64 buffer_ptr = args->buffer_ptr;
    __u64 requested_size = args->requested_size;
    __u64 host_pid_tgid = current_pid_tgid();
    __u32 tgid = host_pid_tgid >> 32;
    __u32 tid = (__u32)host_pid_tgid;
    struct actrail_pending_tls_payload_op *op;
    int stored = store_tls_payload_op(
        ctx,
        tls_op_metadata(direction, symbol),
        stream_key,
        buffer_ptr,
        requested_size,
        0
    );

    if (stored != 1) {
        return 0;
    }
    op = bpf_map_lookup_elem(&pending_tls_payload_ops, &host_pid_tgid);
    if (!op) {
        return 0;
    }
    op->library = library;
    if (direction == ACTRAIL_TLS_PAYLOAD_INBOUND &&
        payload_tls_capture_backend() != ACTRAIL_TLS_BACKEND_BPF_COPY_ONLY) {
        emit_tls_capture_request(ctx, op, tgid, tid, requested_size);
    }
    return emit_tls_payload_completion(ctx, requested_size, 0);
}

#define emit_tls_immediate_payload(ctx_arg, direction_arg, symbol_arg, library_arg, stream_key_arg, buffer_ptr_arg, requested_size_arg) ({ \
    struct actrail_tls_immediate_payload_args immediate_args = {}; \
    immediate_args.direction = (direction_arg); \
    immediate_args.symbol = (symbol_arg); \
    immediate_args.library = (library_arg); \
    immediate_args.stream_key = (stream_key_arg); \
    immediate_args.buffer_ptr = (buffer_ptr_arg); \
    immediate_args.requested_size = (requested_size_arg); \
    emit_tls_immediate_payload_args((ctx_arg), &immediate_args); \
})


#endif
