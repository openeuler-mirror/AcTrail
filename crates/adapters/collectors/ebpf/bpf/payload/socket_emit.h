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
    struct task_struct *task = actrail_bpf_get_current_task();
    __u32 observer_pid = observer_tgid_for_task(task);
    __u32 observer_tid = observer_tid_for_task(task);
    struct actrail_socket_payload_completion_event *event;
    __u64 sequence;

    /* seccomp notifications use the listener's PID namespace, not kernel IDs. */
    if (!observer_pid || !observer_tid) {
        return 0;
    }

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        event_transport_diag_inc(ACTRAIL_SOCKET_RESERVE_FAIL);
        return 0;
    }

    event->kind = ACTRAIL_SOCKET_PAYLOAD_COMPLETION;
    event->observer_tgid = observer_pid;
    event->observer_tid = observer_tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = op->started_ktime_ns;
    sequence = next_socket_payload_sequence(tgid, op->direction, op->fd, op->socket_generation);
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
    event->socket_generation = op->socket_generation;
    event->kernel_tgid = kernel_pid_tgid >> 32;
    event->kernel_tid = (__u32)kernel_pid_tgid;

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

    /* Reserve the sequence even when capture fails: the next delivered chunk
     * must expose the missing bytes to the existing stream gap handling. */
    sequence = next_socket_payload_sequence(
        chunk->tgid, op->direction, op->fd, op->socket_generation
    );
    if (!sequence) {
        return 1;
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
    event->socket_generation = op->socket_generation;
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
        return 1;
    }
    if (!chunk->offset
        && socket_payload_prefix_is_tls_hello(event->bytes, capture_size)
        && socket_payload_mark_fd_tls_owned(
            chunk->tgid,
            op->fd,
            op->socket_generation
        )) {
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

struct actrail_socket_payload_loop {
    void *program_ctx;
    struct actrail_pending_socket_payload_op *op;
    struct actrail_socket_payload_chunk chunk;
    __u32 segment_limit;
};

static long socket_payload_chunk_step(__u32 index, void *opaque) {
    struct actrail_socket_payload_loop *state = opaque;
    struct actrail_socket_payload_chunk *chunk = &state->chunk;
    __u64 remaining;

    /* Derive each coordinate from the callback index. A mutable offset carried
     * across callbacks makes verifier state grow with the captured byte count. */
    chunk->offset = (__u64)index * state->segment_limit;
    if (chunk->offset >= chunk->operation_captured_size) {
        return 1;
    }
    chunk->buffer_ptr = state->op->buffer_ptr + chunk->offset;
    remaining = chunk->operation_captured_size - chunk->offset;
    chunk->capture_size = remaining > state->segment_limit
        ? state->segment_limit : (__u32)remaining;
    chunk->index = index;
    chunk->original_size = chunk->capture_size;
    if (emit_socket_payload_direct_chunk(state->program_ctx, state->op, chunk)) {
        return 1;
    }
    return 0;
}

static __always_inline void emit_socket_payload_inbound(
    void *ctx,
    struct actrail_pending_socket_payload_op *op,
    __u64 pid_tgid,
    __u64 original_size,
    __u32 segment_limit
) {
    struct actrail_socket_payload_config *config = socket_payload_config();
    struct actrail_socket_payload_loop state = {
        .program_ctx = ctx,
        .op = op,
        .segment_limit = segment_limit,
    };
    __u64 captured = original_size;
    __u32 count;

    if (!config || !config->max_operation_bytes || !segment_limit) {
        return;
    }
    if (captured > op->requested_size) {
        captured = op->requested_size;
    }
    if (captured > config->max_operation_bytes) {
        captured = config->max_operation_bytes;
    }
    count = (__u32)((captured + segment_limit - 1) / segment_limit);
    if (!count || count > ACTRAIL_SOCKET_PAYLOAD_MAX_CHUNKS) {
        return;
    }
    state.chunk.buffer_ptr = op->buffer_ptr;
    state.chunk.operation_original_size = original_size;
    state.chunk.operation_captured_size = captured;
    state.chunk.tgid = pid_tgid >> 32;
    state.chunk.tid = (__u32)pid_tgid;
    if (captured < original_size) {
        state.chunk.flags = ACTRAIL_SOCKET_PAYLOAD_TRUNCATED |
            ACTRAIL_SOCKET_PAYLOAD_TOTAL_LIMIT |
            ACTRAIL_SOCKET_PAYLOAD_POLICY_LIMITED;
    }
#ifdef ACTRAIL_BPF_LOOP
    bpf_loop(count, socket_payload_chunk_step, &state, 0);
#else
#pragma clang loop unroll(disable)
    for (__u32 index = 0; index < ACTRAIL_SOCKET_PAYLOAD_MAX_CHUNKS; index++) {
        if (index >= count || socket_payload_chunk_step(index, &state)) {
            break;
        }
    }
#endif
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
    if (op->direction == ACTRAIL_SOCKET_PAYLOAD_INBOUND) {
        emit_socket_payload_inbound(ctx, op, pid_tgid, original_size, limit);
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
