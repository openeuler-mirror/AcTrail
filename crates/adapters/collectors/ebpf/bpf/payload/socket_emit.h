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

struct actrail_socket_payload_loop {
    void *program_ctx;
    struct actrail_pending_socket_payload_op *op;
    struct actrail_socket_payload_chunk chunk;
    __u64 range_base;
    __u64 range_size;
    __u64 range_offset;
    __u32 range_index;
    __u32 segment_limit;
    __u32 stopped;
};

static long socket_payload_chunk_step(__u32 index, void *opaque) {
    struct actrail_socket_payload_loop *state = opaque;
    struct actrail_socket_payload_chunk *chunk = &state->chunk;
    __u64 remaining;

    /* Derive each coordinate from the callback index. A mutable offset carried
     * across callbacks makes verifier state grow with the captured byte count. */
    __u64 offset = (__u64)index * state->segment_limit;
    if (offset >= state->range_size) {
        return 1;
    }
    chunk->offset = state->range_offset + offset;
    chunk->buffer_ptr = state->range_base + offset;
    remaining = state->range_size - offset;
    chunk->capture_size = remaining > state->segment_limit
        ? state->segment_limit : (__u32)remaining;
    chunk->index = state->range_index + index;
    chunk->original_size = chunk->capture_size;
    if (emit_socket_payload_direct_chunk(state->program_ctx, state->op, chunk)) {
        state->stopped = 1;
        return 1;
    }
    return 0;
}

static __always_inline void emit_socket_payload_range(
    struct actrail_socket_payload_loop *state
) {
    __u32 count = (__u32)((state->range_size + state->segment_limit - 1) /
        state->segment_limit);

    if (!count || count > ACTRAIL_SOCKET_PAYLOAD_MAX_CHUNKS) {
        return;
    }
#ifdef ACTRAIL_BPF_LOOP
    bpf_loop(count, socket_payload_chunk_step, state, 0);
#else
#pragma clang loop unroll(disable)
    for (__u32 index = 0; index < ACTRAIL_SOCKET_PAYLOAD_MAX_CHUNKS; index++) {
        if (index >= count || socket_payload_chunk_step(index, state)) {
            break;
        }
    }
#endif
    state->range_index += count;
}

static long socket_payload_iovec_step(__u32 index, void *opaque) {
    struct actrail_socket_payload_loop *state = opaque;
    struct actrail_user_iovec iov = {};
    __u64 remaining;

    if (state->range_offset >= state->chunk.operation_captured_size) {
        return 1;
    }
    if (bpf_probe_read_user(
            &iov,
            sizeof(iov),
            (void *)(unsigned long)(state->op->iov_ptr + (__u64)index * sizeof(iov))
        ) != 0) {
        event_transport_diag_inc(ACTRAIL_SOCKET_READ_USER_FAIL);
        return 1;
    }
    if (!iov.len) {
        return 0;
    }
    remaining = state->chunk.operation_captured_size - state->range_offset;
    state->range_base = iov.base;
    state->range_size = iov.len > remaining ? remaining : iov.len;
    emit_socket_payload_range(state);
    state->range_offset += state->range_size;
    return state->stopped ? 1 : 0;
}

static __always_inline void emit_socket_payload_direct(
    void *ctx,
    struct actrail_pending_socket_payload_op *op,
    __u64 pid_tgid,
    __u64 original_size,
    __u32 segment_limit,
    __u32 iovec_payload
) {
    struct actrail_socket_payload_config *config = socket_payload_config();
    struct actrail_socket_payload_loop state = {
        .program_ctx = ctx,
        .op = op,
        .segment_limit = segment_limit,
    };
    __u64 captured = original_size;

    if (!config || !config->max_operation_bytes || !segment_limit) {
        return;
    }
    if (!iovec_payload && captured > op->requested_size) {
        captured = op->requested_size;
    }
    if (captured > config->max_operation_bytes) {
        captured = config->max_operation_bytes;
    }
    state.chunk.operation_original_size = original_size;
    state.chunk.operation_captured_size = captured;
    state.chunk.tgid = pid_tgid >> 32;
    state.chunk.tid = (__u32)pid_tgid;
    if (captured < original_size) {
        state.chunk.flags = ACTRAIL_SOCKET_PAYLOAD_TRUNCATED |
            ACTRAIL_SOCKET_PAYLOAD_TOTAL_LIMIT |
            ACTRAIL_SOCKET_PAYLOAD_POLICY_LIMITED;
    }
    if (!iovec_payload) {
        state.range_base = op->buffer_ptr;
        state.range_size = captured;
        emit_socket_payload_range(&state);
        return;
    }
    /* A successful writev/sendmsg has already passed the syscall's iovec count
     * checks. Keep the original vector coordinates for one logical operation. */
    __u32 count = (__u32)op->iov_count;
#ifdef ACTRAIL_BPF_LOOP
    bpf_loop(count, socket_payload_iovec_step, &state, 0);
#else
#pragma clang loop unroll(disable)
    for (__u32 index = 0; index < ACTRAIL_SOCKET_PAYLOAD_MAX_CHUNKS; index++) {
        if (index >= count || socket_payload_iovec_step(index, &state)) {
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

    if (op->direction == ACTRAIL_SOCKET_PAYLOAD_OUTBOUND
        && op->delegated
        && (iovec_payload || original_size > limit)) {
        emit_socket_payload_completion(ctx, op, tgid, tid, original_size);
        bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
        return 0;
    }
    emit_socket_payload_direct(ctx, op, pid_tgid, original_size, limit, iovec_payload);
    bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
    return 0;
}

#endif
