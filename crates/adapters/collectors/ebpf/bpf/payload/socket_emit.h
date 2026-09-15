#ifndef ACTRAIL_PAYLOAD_SOCKET_EMIT_H
#define ACTRAIL_PAYLOAD_SOCKET_EMIT_H

/*
 * Socket payload exit-side emitters. Included from socket_capture.h after the
 * sequence runtime helpers are defined.
 */

static __always_inline int emit_socket_payload_completion(
    void *ctx,
    struct actrail_pending_socket_payload_op *op,
    __u32 tgid,
    __u32 tid,
    __u64 completed_size
) {
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    struct actrail_socket_payload_completion_event *event;
    __u64 sequence;

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        event_transport_diag_inc(ACTRAIL_SOCKET_RESERVE_FAIL);
        return 0;
    }

    event->kind = ACTRAIL_SOCKET_PAYLOAD_COMPLETION;
    event->pid = tgid;
    event->tid = tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = op->started_ktime_ns;
    sequence = next_socket_payload_sequence(tgid, op->direction, op->fd, op->fd_generation);
    if (!sequence) {
        actrail_event_discard(event);
        return 0;
    }
    event->sequence = sequence;
    event->operation_id = op->operation_id;
    event->completed_size = completed_size;
    event->requested_size = op->requested_size;
    event->buffer_ptr = op->buffer_ptr;
    event->pid_generation = op->pid_generation;
    event->fd = op->fd;
    event->flags = 0;
    event->syscall = op->syscall;
    event->fd_generation = op->fd_generation;
    event->host_pid = kernel_pid_tgid >> 32;
    event->host_tid = (__u32)kernel_pid_tgid;

    actrail_event_submit(ctx, event);
    return 0;
}

struct actrail_socket_payload_chunk {
    __u64 buffer_ptr;
    __u64 offset;
    __u64 original_size;
    __u64 operation_original_size;
    __u64 operation_captured_size;
    __u32 capture_size;
    __u32 flags;
    __u32 tgid;
    __u32 tid;
    __u32 index;
};

static __noinline int emit_socket_payload_direct_chunk(
    void *ctx,
    struct actrail_pending_socket_payload_op *op,
    const struct actrail_socket_payload_chunk *chunk
) {
    struct actrail_socket_payload_event *event;
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    __u32 capture_size = chunk->capture_size & ACTRAIL_SOCKET_PAYLOAD_COPY_MAX_BYTES;
    __u64 sequence;

    if (!capture_size) {
        return 0;
    }

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        event_transport_diag_inc(ACTRAIL_SOCKET_RESERVE_FAIL);
        return 1;
    }

    event->kind = ACTRAIL_SOCKET_PAYLOAD;
    event->pid = chunk->tgid;
    event->tid = chunk->tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = op->direction == ACTRAIL_SOCKET_PAYLOAD_OUTBOUND
        ? op->started_ktime_ns
        : bpf_ktime_get_ns();
    event->operation_id = op->operation_id;
    event->operation_offset = chunk->offset;
    event->operation_original_size = chunk->operation_original_size;
    event->operation_captured_size = chunk->operation_captured_size;
    event->fd = op->fd;
    event->original_size = (__u32)chunk->original_size;
    event->captured_size = capture_size;
    event->flags = chunk->flags;
    event->syscall = op->syscall;
    event->fd_generation = op->fd_generation;
    event->pid_generation = op->pid_generation;
    event->host_pid = kernel_pid_tgid >> 32;
    event->host_tid = (__u32)kernel_pid_tgid;
    event->operation_chunk_index = chunk->index;
    event->reserved = 0;
    if (bpf_probe_read_user(
            event->bytes,
            capture_size,
            (void *)(unsigned long)chunk->buffer_ptr
        ) != 0) {
        event_transport_diag_inc(ACTRAIL_SOCKET_READ_USER_FAIL);
        actrail_event_discard(event);
        return 0;
    }
    if (!chunk->offset
        && socket_payload_prefix_is_tls_hello(event->bytes, capture_size)
        && socket_payload_mark_fd_tls_owned(
            chunk->tgid,
            op->fd,
            op->fd_generation
        )) {
        actrail_event_discard(event);
        return 1;
    }
    sequence = next_socket_payload_sequence(
        chunk->tgid,
        op->direction,
        op->fd,
        op->fd_generation
    );
    if (!sequence) {
        actrail_event_discard(event);
        return 1;
    }
    event->sequence = sequence;

    actrail_event_submit(ctx, event);
    return 0;
}

static __always_inline int emit_socket_payload_prefix_chunk(
    void *ctx,
    struct actrail_pending_socket_payload_op *op,
    __u64 pid_tgid,
    __u64 source_ptr,
    __u64 op_original_size,
    __u64 capture_capacity,
    __u32 limit
) {
    __u64 bounded_size = op_original_size;
    __u32 capture_size;
    __u32 flags = 0;
    struct actrail_socket_payload_chunk chunk = {};

    if (bounded_size > capture_capacity) {
        bounded_size = capture_capacity;
    }
    if (bounded_size > limit) {
        bounded_size = limit;
    }
    actrail_barrier_var(bounded_size);
    bounded_size &= ACTRAIL_SOCKET_PAYLOAD_COPY_MAX_BYTES;
    capture_size = (__u32)bounded_size;
    if (!capture_size) {
        return 0;
    }
    if (op_original_size > bounded_size) {
        flags = ACTRAIL_SOCKET_PAYLOAD_TRUNCATED |
            ACTRAIL_SOCKET_PAYLOAD_CHUNK_LIMIT |
            op->truncation_flags;
    }
    chunk.buffer_ptr = source_ptr;
    chunk.offset = 0;
    chunk.original_size = op_original_size;
    chunk.operation_original_size = op_original_size;
    chunk.operation_captured_size = bounded_size;
    chunk.capture_size = capture_size;
    chunk.flags = flags;
    chunk.tgid = pid_tgid >> 32;
    chunk.tid = (__u32)pid_tgid;
    chunk.index = 0;
    emit_socket_payload_direct_chunk(ctx, op, &chunk);
    return 0;
}

static __always_inline int emit_socket_payload_op(
    struct trace_event_raw_sys_exit *ctx,
    __u32 iovec_payload
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u32 tid = (__u32)pid_tgid;
    struct actrail_pending_socket_payload_op *op =
        bpf_map_lookup_elem(&pending_socket_payload_ops, &pid_tgid);
    __u64 original_size = (__u64)ctx->ret;
    __u32 limit = payload_socket_capture_limit();

    if (!tgid || !op || ctx->ret <= 0 || !limit) {
        bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
        return 0;
    }

    if (iovec_payload) {
        if (op->delegated) {
            emit_socket_payload_completion(ctx, op, tgid, tid, original_size);
            bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
            return 0;
        }
        if (!op->copy_base || !op->copy_capacity) {
            bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
            return 0;
        }
        emit_socket_payload_prefix_chunk(
            ctx,
            op,
            pid_tgid,
            op->copy_base,
            original_size,
            op->copy_capacity,
            limit
        );
        bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
        return 0;
    }
    if (op->direction == ACTRAIL_SOCKET_PAYLOAD_OUTBOUND
        && op->delegated
        && original_size > limit) {
        emit_socket_payload_completion(ctx, op, tgid, tid, original_size);
        bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
        return 0;
    }
    emit_socket_payload_prefix_chunk(
        ctx,
        op,
        pid_tgid,
        op->buffer_ptr,
        original_size,
        op->requested_size,
        limit
    );
    bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
    return 0;
}

#endif
