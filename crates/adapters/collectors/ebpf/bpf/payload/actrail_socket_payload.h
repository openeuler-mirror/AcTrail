#ifndef ACTRAIL_SOCKET_PAYLOAD_H
#define ACTRAIL_SOCKET_PAYLOAD_H

#include "actrail_socket_payload_types.h"

#define ACTRAIL_LINUX_EINPROGRESS 115
#define ACTRAIL_SOCKET_PAYLOAD_DIRECT_CHUNK_MAX 16

static __always_inline struct actrail_socket_payload_config *socket_payload_config(void) {
    __u32 key = 0;
    return bpf_map_lookup_elem(&payload_socket_config, &key);
}

static __always_inline __u32 payload_socket_capture_limit(void) {
    struct actrail_socket_payload_config *config = socket_payload_config();
    __u32 limit;

    if (!config || !config->enabled) {
        return 0;
    }
    limit = config->max_segment_bytes;
    if (limit > ACTRAIL_SOCKET_PAYLOAD_COPY_MAX_BYTES) {
        return ACTRAIL_SOCKET_PAYLOAD_COPY_MAX_BYTES;
    }
    return limit;
}

static __always_inline __u32 payload_socket_user_read_enabled(void) {
    struct actrail_socket_payload_config *config = socket_payload_config();

    return config && config->enabled && config->user_read_enabled;
}

static __always_inline struct actrail_socket_payload_fd_key socket_payload_fd_key(
    __u32 pid,
    __u32 fd
) {
    struct actrail_socket_payload_fd_key key = {};
    key.pid = pid;
    key.fd = fd;
    return key;
}

static __always_inline __u32 socket_payload_fd_generation(__u32 pid, __u32 fd) {
    struct actrail_socket_payload_fd_key key = socket_payload_fd_key(pid, fd);
    __u32 *generation = bpf_map_lookup_elem(&payload_socket_fds, &key);
    return generation ? *generation : 0;
}

static __always_inline __u32 socket_payload_next_generation(__u32 pid) {
    __u32 initial = 1;
    __u32 next;
    __u32 *current = bpf_map_lookup_elem(&payload_socket_process_generations, &pid);

    if (!current) {
        bpf_map_update_elem(&payload_socket_process_generations, &pid, &initial, BPF_ANY);
        return initial;
    }

    next = *current + 1;
    if (!next) {
        next = 1;
    }
    bpf_map_update_elem(&payload_socket_process_generations, &pid, &next, BPF_ANY);
    return next;
}

static __always_inline void socket_payload_set_fd_generation(
    __u32 pid,
    __u32 fd,
    __u32 generation
) {
    struct actrail_socket_payload_fd_key key;

    if (!generation) {
        return;
    }
    key = socket_payload_fd_key(pid, fd);
    bpf_map_update_elem(&payload_socket_fds, &key, &generation, BPF_ANY);
}

static __always_inline void socket_payload_track_fd(__u32 pid, __u32 fd) {
    struct actrail_socket_payload_config *config = socket_payload_config();
    __u32 generation;

    if (!config || !config->enabled) {
        return;
    }
    generation = socket_payload_next_generation(pid);
    socket_payload_set_fd_generation(pid, fd, generation);
}

static __always_inline void socket_payload_delete_fd(__u32 pid, __u32 fd) {
    struct actrail_socket_payload_fd_key key = socket_payload_fd_key(pid, fd);
    bpf_map_delete_elem(&payload_socket_fds, &key);
}

static __always_inline __u64 next_socket_payload_sequence(
    __u32 pid,
    __u32 direction,
    __u32 fd,
    __u32 fd_generation
) {
    struct actrail_socket_payload_sequence_key key = {};
    __u64 initial = 1;
    __u64 next;
    __u64 *current;

    key.pid = pid;
    key.direction = direction;
    key.fd = fd;
    key.fd_generation = fd_generation;
    current = bpf_map_lookup_elem(&payload_socket_stream_sequences, &key);
    if (!current) {
        bpf_map_update_elem(&payload_socket_stream_sequences, &key, &initial, BPF_ANY);
        return initial;
    }

    next = *current + 1;
    bpf_map_update_elem(&payload_socket_stream_sequences, &key, &next, BPF_ANY);
    return next;
}

static __always_inline int store_socket_payload_op(
    struct trace_event_raw_sys_enter *ctx,
    __u32 direction,
    __u32 syscall,
    __u32 fd_arg,
    __u32 buffer_arg,
    __u32 size_arg,
    __u32 require_tracked_fd
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_socket_payload_config *config = socket_payload_config();
    struct actrail_pending_socket_payload_op op = {};
    __u32 fd = (__u32)ctx->args[fd_arg];
    __u32 fd_generation = 0;

    if (!tgid || !trace_id || !config || !config->enabled || !ctx->args[buffer_arg] || !ctx->args[size_arg]) {
        return 0;
    }
    if (is_suppressed_fd(tgid, fd)) {
        return 0;
    }
    fd_generation = socket_payload_fd_generation(tgid, fd);
    if (require_tracked_fd && !fd_generation) {
        return 0;
    }

    op.trace_id = *trace_id;
    op.buffer_ptr = (__u64)ctx->args[buffer_arg];
    op.requested_size = (__u64)ctx->args[size_arg];
    op.pid_generation = current_process_start_time(tgid);
    op.fd = fd;
    op.fd_generation = fd_generation;
    op.direction = direction;
    op.syscall = syscall;
    bpf_map_update_elem(&pending_socket_payload_ops, &pid_tgid, &op, BPF_ANY);
    return 0;
}

static __always_inline int emit_socket_payload_completion(
    void *ctx,
    struct actrail_pending_socket_payload_op *op,
    __u32 tgid,
    __u32 tid,
    __u64 completed_size
) {
    struct actrail_socket_payload_completion_event *event;

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        return 0;
    }

    event->kind = ACTRAIL_SOCKET_PAYLOAD_COMPLETION;
    event->pid = tgid;
    event->tid = tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->sequence =
        next_socket_payload_sequence(tgid, op->direction, op->fd, op->fd_generation);
    event->completed_size = completed_size;
    event->requested_size = op->requested_size;
    event->buffer_ptr = op->buffer_ptr;
    event->pid_generation = op->pid_generation;
    event->fd = op->fd;
    event->flags = 0;
    event->syscall = op->syscall;
    event->fd_generation = op->fd_generation;

    actrail_event_submit(ctx, event);
    return 0;
}

struct actrail_socket_payload_chunk {
    __u64 offset;
    __u64 original_size;
    __u32 capture_size;
    __u32 flags;
    __u32 tgid;
    __u32 tid;
};

static __noinline int emit_socket_payload_direct_chunk(
    void *ctx,
    struct actrail_pending_socket_payload_op *op,
    const struct actrail_socket_payload_chunk *chunk
) {
    struct actrail_socket_payload_event *event;
    __u32 capture_size = chunk->capture_size & ACTRAIL_SOCKET_PAYLOAD_COPY_MAX_BYTES;

    if (!capture_size) {
        return 0;
    }

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        return 0;
    }

    event->kind = ACTRAIL_SOCKET_PAYLOAD;
    event->pid = chunk->tgid;
    event->tid = chunk->tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->sequence =
        next_socket_payload_sequence(chunk->tgid, op->direction, op->fd, op->fd_generation);
    event->fd = op->fd;
    event->original_size = (__u32)chunk->original_size;
    event->captured_size = capture_size;
    event->flags = chunk->flags;
    event->syscall = op->syscall;
    event->fd_generation = op->fd_generation;
    event->pid_generation = op->pid_generation;
    if (bpf_probe_read_user(
            event->bytes,
            capture_size,
            (void *)(unsigned long)(op->buffer_ptr + chunk->offset)
        ) != 0) {
        actrail_event_discard(event);
        return 0;
    }

    actrail_event_submit(ctx, event);
    return 0;
}

static __always_inline int emit_socket_payload_direct_chunks(
    void *ctx,
    struct actrail_pending_socket_payload_op *op,
    __u32 tgid,
    __u32 tid,
    __u64 original_size,
    __u32 limit
) {
    __u64 offset = 0;

#pragma unroll
    for (__u32 index = 0; index < ACTRAIL_SOCKET_PAYLOAD_DIRECT_CHUNK_MAX; index++) {
        __u64 remaining;
        __u64 bounded_size;
        __u64 segment_original_size;
        __u32 capture_size;
        __u32 flags = 0;
        struct actrail_socket_payload_chunk chunk = {};

        if (offset >= original_size) {
            break;
        }
        remaining = original_size - offset;
        bounded_size = remaining;
        if (bounded_size > limit) {
            bounded_size = limit;
        }
        actrail_barrier_var(bounded_size);
        bounded_size &= ACTRAIL_SOCKET_PAYLOAD_COPY_MAX_BYTES;
        capture_size = (__u32)bounded_size;
        if (!capture_size) {
            break;
        }

        segment_original_size = bounded_size;
        if (remaining > bounded_size && index == ACTRAIL_SOCKET_PAYLOAD_DIRECT_CHUNK_MAX - 1) {
            flags = ACTRAIL_SOCKET_PAYLOAD_TRUNCATED;
            segment_original_size = remaining;
        }
        chunk.offset = offset;
        chunk.original_size = segment_original_size;
        chunk.capture_size = capture_size;
        chunk.flags = flags;
        chunk.tgid = tgid;
        chunk.tid = tid;
        emit_socket_payload_direct_chunk(ctx, op, &chunk);
        offset += bounded_size;
    }
    return 0;
}

static __always_inline int emit_socket_payload_op(struct trace_event_raw_sys_exit *ctx) {
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

    if ((op->syscall == ACTRAIL_SOCKET_SYSCALL_WRITEV
            || op->syscall == ACTRAIL_SOCKET_SYSCALL_SENDMSG)
        && payload_socket_user_read_enabled()) {
        emit_socket_payload_completion(ctx, op, tgid, tid, original_size);
        bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
        return 0;
    }

    if (op->direction == ACTRAIL_SOCKET_PAYLOAD_OUTBOUND
        && payload_socket_user_read_enabled()
        && original_size > limit) {
        emit_socket_payload_completion(ctx, op, tgid, tid, original_size);
        bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
        return 0;
    }

    emit_socket_payload_direct_chunks(ctx, op, tgid, tid, original_size, limit);
    bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
    return 0;
}

static __always_inline int store_socket_payload_sendto_op(
    struct trace_event_raw_sys_enter *ctx
) {
    return store_socket_payload_op(
        ctx,
        ACTRAIL_SOCKET_PAYLOAD_OUTBOUND,
        ACTRAIL_SOCKET_SYSCALL_SENDTO,
        0,
        1,
        2,
        0
    );
}

static __always_inline int store_socket_payload_writev_op(
    struct trace_event_raw_sys_enter *ctx
) {
    return store_socket_payload_op(
        ctx,
        ACTRAIL_SOCKET_PAYLOAD_OUTBOUND,
        ACTRAIL_SOCKET_SYSCALL_WRITEV,
        0,
        1,
        2,
        1
    );
}

static __always_inline int store_socket_payload_sendmsg_op(
    struct trace_event_raw_sys_enter *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_socket_payload_config *config = socket_payload_config();
    struct actrail_pending_socket_payload_op op = {};
    __u32 fd = (__u32)ctx->args[0];

    if (!tgid || !trace_id || !config || !config->enabled || !ctx->args[1]) {
        return 0;
    }
    if (is_suppressed_fd(tgid, fd)) {
        return 0;
    }

    op.trace_id = *trace_id;
    op.buffer_ptr = (__u64)ctx->args[1];
    op.requested_size = 0;
    op.pid_generation = current_process_start_time(tgid);
    op.fd = fd;
    op.fd_generation = socket_payload_fd_generation(tgid, fd);
    op.direction = ACTRAIL_SOCKET_PAYLOAD_OUTBOUND;
    op.syscall = ACTRAIL_SOCKET_SYSCALL_SENDMSG;
    bpf_map_update_elem(&pending_socket_payload_ops, &pid_tgid, &op, BPF_ANY);
    return 0;
}

static __always_inline int store_socket_payload_recvfrom_op(
    struct trace_event_raw_sys_enter *ctx
) {
    return store_socket_payload_op(
        ctx,
        ACTRAIL_SOCKET_PAYLOAD_INBOUND,
        ACTRAIL_SOCKET_SYSCALL_RECVFROM,
        0,
        1,
        2,
        0
    );
}

static __always_inline int store_socket_payload_write_op(
    struct trace_event_raw_sys_enter *ctx
) {
    return store_socket_payload_op(
        ctx,
        ACTRAIL_SOCKET_PAYLOAD_OUTBOUND,
        ACTRAIL_SOCKET_SYSCALL_WRITE,
        0,
        1,
        2,
        1
    );
}

static __always_inline int store_socket_payload_read_op(
    struct trace_event_raw_sys_enter *ctx
) {
    return store_socket_payload_op(
        ctx,
        ACTRAIL_SOCKET_PAYLOAD_INBOUND,
        ACTRAIL_SOCKET_SYSCALL_READ,
        0,
        1,
        2,
        1
    );
}

static __always_inline void socket_payload_track_connect_exit(
    struct trace_event_raw_sys_exit *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    struct actrail_pending_net_op *op = bpf_map_lookup_elem(&pending_net_ops, &pid_tgid);

    if (!tgid || !op || op->kind != ACTRAIL_NET_CONNECT) {
        return;
    }
    if (ctx->ret != 0 && ctx->ret != -ACTRAIL_LINUX_EINPROGRESS) {
        return;
    }
    socket_payload_track_fd(tgid, op->fd);
}

static __always_inline void socket_payload_track_accept_exit(
    struct trace_event_raw_sys_exit *ctx
) {
    __u32 tgid = current_tgid();

    if (!tgid || ctx->ret < 0) {
        return;
    }
    socket_payload_track_fd(tgid, (__u32)ctx->ret);
}

static __always_inline void socket_payload_close_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    __u32 tgid = current_tgid();

    if (!tgid) {
        return;
    }
    socket_payload_delete_fd(tgid, (__u32)ctx->args[0]);
}

static __always_inline void socket_payload_dup_enter(
    struct trace_event_raw_sys_enter *ctx,
    __u32 source_fd_arg,
    __u32 target_fd_arg,
    __u32 mode
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    struct actrail_socket_payload_config *config = socket_payload_config();
    struct actrail_pending_socket_dup_op op = {};
    __u32 source_fd = (__u32)ctx->args[source_fd_arg];
    __u32 target_fd = target_fd_arg < ACTRAIL_SYSCALL_ARG_MISSING
        ? (__u32)ctx->args[target_fd_arg]
        : 0;

    if (!tgid || !config || !config->enabled) {
        return;
    }
    op.source_fd = source_fd;
    op.target_fd = target_fd;
    op.source_generation = socket_payload_fd_generation(tgid, source_fd);
    op.target_generation = target_fd_arg < ACTRAIL_SYSCALL_ARG_MISSING
        ? socket_payload_fd_generation(tgid, target_fd)
        : 0;
    op.mode = mode;
    if (!op.source_generation && !op.target_generation) {
        return;
    }
    bpf_map_update_elem(&pending_socket_dup_ops, &pid_tgid, &op, BPF_ANY);
}

static __always_inline void socket_payload_fcntl_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    __u32 command = (__u32)ctx->args[1];

    if (command != F_DUPFD && command != F_DUPFD_CLOEXEC) {
        return;
    }
    socket_payload_dup_enter(
        ctx,
        0,
        ACTRAIL_SYSCALL_ARG_MISSING,
        ACTRAIL_SOCKET_DUP_RET_FD
    );
}

static __always_inline void socket_payload_dup_exit(
    struct trace_event_raw_sys_exit *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    struct actrail_pending_socket_dup_op *op =
        bpf_map_lookup_elem(&pending_socket_dup_ops, &pid_tgid);
    __u32 new_fd;

    if (!tgid || !op) {
        return;
    }
    if (ctx->ret < 0) {
        bpf_map_delete_elem(&pending_socket_dup_ops, &pid_tgid);
        return;
    }
    new_fd = op->mode == ACTRAIL_SOCKET_DUP_RET_FD ? (__u32)ctx->ret : op->target_fd;
    if (op->source_generation) {
        socket_payload_set_fd_generation(tgid, new_fd, op->source_generation);
    } else if (op->target_generation) {
        socket_payload_delete_fd(tgid, new_fd);
    }
    bpf_map_delete_elem(&pending_socket_dup_ops, &pid_tgid);
}

#endif
