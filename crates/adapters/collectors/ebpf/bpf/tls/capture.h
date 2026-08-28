#ifndef ACTRAIL_TLS_CAPTURE_H
#define ACTRAIL_TLS_CAPTURE_H

#include "state.h"
#include "diagnostics.h"


struct actrail_tls_direct_capture_chunk {
    __u64 offset;
    __u32 capture_size;
    __u32 original_size;
};

static __noinline int emit_tls_direct_capture_chunk(
    void *ctx,
    const struct actrail_pending_tls_payload_op *op,
    __u32 tgid,
    __u32 tid,
    const struct actrail_tls_direct_capture_chunk *chunk
) {
#ifdef ACTRAIL_EVENT_TRANSPORT_PERF
    tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_COPY_TOO_LARGE);
    return 0;
#else
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    struct actrail_tls_direct_capture_event *event;
    __u32 capture_size =
        chunk->capture_size & ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES;

    if (!capture_size) {
        return 0;
    }

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_RESERVE_FAIL);
        return 0;
    }
    event->kind = ACTRAIL_TLS_PAYLOAD_DIRECT_CAPTURE;
    event->pid = tgid;
    event->tid = tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->stream_key = op->stream_key;
    event->operation_id = op->operation_id;
    event->original_size = chunk->original_size;
    event->captured_size = capture_size;
    event->flags = 0;
    event->symbol = op->symbol;
    event->library = op->library;
    event->operation_offset = (__u32)chunk->offset;
    event->pid_generation = op->pid_generation;
    event->host_pid = kernel_pid_tgid >> 32;
    event->host_tid = (__u32)kernel_pid_tgid;
    if (bpf_probe_read_user(
            event->bytes,
            capture_size,
            (void *)(unsigned long)(op->buffer_ptr + chunk->offset)
        ) != 0) {
        actrail_event_discard(event);
        tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_READ_FAIL);
        return 0;
    }
    actrail_event_submit(ctx, event);
    tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_SUBMIT_OK);
    return 1;
#endif
}

static __always_inline int emit_tls_direct_capture(
    void *ctx,
    const struct actrail_pending_tls_payload_op *op,
    __u32 tgid,
    __u32 tid,
    __u64 requested_size
) {
    __u32 copy_limit = payload_tls_direct_copy_limit();
    __u64 capture_limit =
        (__u64)copy_limit * ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_CHUNKS;
    __u64 capture_size = requested_size;
    __u64 offset = 0;
    int copied_full = 1;

    tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_COPY_ATTEMPT);
    if (!requested_size || !copy_limit) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_COPY_TOO_LARGE);
        return 0;
    }
    if (capture_size > capture_limit) {
        if (payload_tls_capture_backend() != ACTRAIL_TLS_BACKEND_BPF_COPY_ONLY) {
            tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_COPY_TOO_LARGE);
            return 0;
        }
        capture_size = capture_limit;
        copied_full = 0;
        tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_COPY_TOO_LARGE);
    }

#pragma unroll
    for (__u32 index = 0;
         index < ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_CHUNKS;
         index++) {
        __u64 remaining;
        __u64 bounded_size;
        struct actrail_tls_direct_capture_chunk chunk = {};

        if (offset >= capture_size) {
            break;
        }
        remaining = capture_size - offset;
        bounded_size = remaining > copy_limit ? copy_limit : remaining;
        actrail_barrier_var(bounded_size);
        bounded_size &= ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES;
        if (!bounded_size) {
            copied_full = 0;
            break;
        }
        chunk.offset = offset;
        chunk.capture_size = (__u32)bounded_size;
        chunk.original_size = (__u32)requested_size;
        if (emit_tls_direct_capture_chunk(ctx, op, tgid, tid, &chunk) != 1) {
            copied_full = 0;
            break;
        }
        offset += bounded_size;
    }
    return copied_full || payload_tls_capture_backend() == ACTRAIL_TLS_BACKEND_BPF_COPY_ONLY;
}

static __always_inline int emit_tls_capture_request(
    void *ctx,
    const struct actrail_pending_tls_payload_op *op,
    __u32 tgid,
    __u32 tid,
    __u64 requested_size
) {
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    struct actrail_tls_capture_request_event *event =
        actrail_event_reserve(sizeof(*event));
    if (!event) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_CAPTURE_REQUEST_RESERVE_FAIL);
        return 0;
    }
    event->kind = ACTRAIL_TLS_PAYLOAD_CAPTURE_REQUEST;
    event->pid = tgid;
    event->tid = tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->stream_key = op->stream_key;
    event->operation_id = op->operation_id;
    event->requested_size = requested_size;
    event->buffer_ptr = op->buffer_ptr;
    event->pid_generation = op->pid_generation;
    event->symbol = op->symbol;
    event->library = op->library;
    event->host_pid = kernel_pid_tgid >> 32;
    event->host_tid = (__u32)kernel_pid_tgid;
    if (bpf_send_signal(ACTRAIL_TLS_CAPTURE_SIGSTOP) == 0) {
        if (actrail_event_submit(ctx, event) == 0) {
            tls_diag_inc(ACTRAIL_TLS_DIAG_CAPTURE_REQUEST_SUBMIT_OK);
        } else if (bpf_send_signal(ACTRAIL_TLS_CAPTURE_SIGCONT) != 0) {
            tls_diag_inc(ACTRAIL_TLS_DIAG_CAPTURE_REQUEST_SIGNAL_FAIL);
        }
    } else {
        actrail_event_discard(event);
        tls_diag_inc(ACTRAIL_TLS_DIAG_CAPTURE_REQUEST_SIGNAL_FAIL);
    }
    return 0;
}

static __always_inline int store_tls_payload_op_args(
    void *ctx,
    const struct actrail_tls_payload_op_args *args
) {
    __u32 metadata = args->metadata;
    __u64 stream_key = args->stream_key;
    __u64 buffer_ptr = args->buffer_ptr;
    __u64 requested_size = args->requested_size;
    __u64 size_ptr = args->size_ptr;
    __u64 host_pid_tgid = current_pid_tgid();
    __u64 namespace_pid_tgid = host_pid_tgid;
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id = lookup_current_trace(&tgid, &tid, &lookup_flags);
    struct actrail_pending_tls_payload_op op = {};

    tls_diag_inc(ACTRAIL_TLS_DIAG_ENTER_TOTAL);
    if (lookup_flags & ACTRAIL_TRACE_LOOKUP_FLAG_HOST_FALLBACK) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_TRACE_LOOKUP_HOST_FALLBACK);
        emit_tls_payload_diagnostic_event(
            ctx,
            ACTRAIL_TLS_DIAG_EVENT_TRACE_LOOKUP_HOST_FALLBACK,
            host_pid_tgid,
            namespace_pid_tgid,
            metadata,
            lookup_flags,
            requested_size,
            buffer_ptr
        );
    }
    if (!trace_id) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_TRACE_LOOKUP_MISS);
        emit_tls_payload_diagnostic_event(
            ctx,
            ACTRAIL_TLS_DIAG_EVENT_TRACE_LOOKUP_MISS,
            host_pid_tgid,
            namespace_pid_tgid,
            metadata,
            lookup_flags,
            requested_size,
            buffer_ptr
        );
        return 0;
    }
    namespace_pid_tgid = current_trace_pid_tgid(*trace_id);
    if (!namespace_pid_tgid) {
        namespace_pid_tgid = host_pid_tgid;
        tls_diag_inc(ACTRAIL_TLS_DIAG_NAMESPACE_FALLBACK);
    }
    if (!buffer_ptr) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_EMPTY_BUFFER);
        emit_tls_payload_diagnostic_event(
            ctx,
            ACTRAIL_TLS_DIAG_EVENT_EMPTY_BUFFER,
            host_pid_tgid,
            namespace_pid_tgid,
            metadata,
            lookup_flags,
            requested_size,
            buffer_ptr
        );
        return 0;
    }

    op.trace_id = *trace_id;
    op.operation_id = bpf_ktime_get_ns() ^ host_pid_tgid;
    op.stream_key = stream_key;
    op.buffer_ptr = buffer_ptr;
    op.requested_size = requested_size;
    op.size_ptr = size_ptr;
    op.pid_generation = current_process_start_time(tgid);
    op.direction = metadata & 0xffff;
    op.symbol = metadata >> 16;
    op.library = payload_tls_library_for_symbol(op.symbol);
    op.capture_state = ACTRAIL_TLS_CAPTURE_STATE_NEEDS_SECCOMP;
    if (op.direction == ACTRAIL_TLS_PAYLOAD_OUTBOUND &&
        payload_tls_bpf_copy_enabled() &&
        emit_tls_direct_capture(ctx, &op, tgid, tid, op.requested_size) == 1) {
        op.capture_state = ACTRAIL_TLS_CAPTURE_STATE_BPF_COPIED_FULL;
    }
    if (bpf_map_update_elem(&pending_tls_payload_ops, &host_pid_tgid, &op, BPF_ANY) != 0) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_PENDING_UPDATE_FAIL);
        emit_tls_payload_diagnostic_event(
            ctx,
            ACTRAIL_TLS_DIAG_EVENT_PENDING_UPDATE_FAIL,
            host_pid_tgid,
            namespace_pid_tgid,
            metadata,
            lookup_flags,
            requested_size,
            buffer_ptr
        );
        return 0;
    }
    tls_diag_inc(ACTRAIL_TLS_DIAG_PENDING_UPDATE_OK);
    if (bpf_map_update_elem(&tls_pending_ns, &namespace_pid_tgid, &host_pid_tgid, BPF_ANY) != 0) {
        emit_tls_payload_diagnostic_event(
            ctx,
            ACTRAIL_TLS_DIAG_EVENT_PENDING_NAMESPACE_UPDATE_FAIL,
            host_pid_tgid,
            namespace_pid_tgid,
            metadata,
            lookup_flags,
            requested_size,
            buffer_ptr
        );
        bpf_map_delete_elem(&pending_tls_payload_ops, &host_pid_tgid);
        return 0;
    }
    if (op.direction == ACTRAIL_TLS_PAYLOAD_OUTBOUND &&
        payload_tls_capture_backend() == ACTRAIL_TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK &&
        op.capture_state == ACTRAIL_TLS_CAPTURE_STATE_NEEDS_SECCOMP) {
        emit_tls_capture_request(ctx, &op, tgid, tid, op.requested_size);
    }
    if (op.direction == ACTRAIL_TLS_PAYLOAD_OUTBOUND &&
        payload_tls_capture_backend() == ACTRAIL_TLS_BACKEND_SECCOMP_USER_READ) {
        emit_tls_capture_request(ctx, &op, tgid, tid, op.requested_size);
    }
    return 1;
}

#define store_tls_payload_op(ctx_arg, metadata_arg, stream_key_arg, buffer_ptr_arg, requested_size_arg, size_ptr_arg) ({ \
    struct actrail_tls_payload_op_args op_args = {}; \
    op_args.metadata = (metadata_arg); \
    op_args.stream_key = (stream_key_arg); \
    op_args.buffer_ptr = (buffer_ptr_arg); \
    op_args.requested_size = (requested_size_arg); \
    op_args.size_ptr = (size_ptr_arg); \
    store_tls_payload_op_args((ctx_arg), &op_args); \
})


#endif
