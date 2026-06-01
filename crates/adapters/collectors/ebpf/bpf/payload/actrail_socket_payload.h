#ifndef ACTRAIL_SOCKET_PAYLOAD_H
#define ACTRAIL_SOCKET_PAYLOAD_H

#include "actrail_socket_payload_types.h"

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
    __u32 tgid = current_namespace_tgid();
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_socket_payload_config *config = socket_payload_config();
    struct actrail_pending_socket_payload_op op = {};
    __u32 fd = (__u32)ctx->args[fd_arg];
    __u32 fd_generation = 0;

    if (!tgid || !trace_id || !config || !config->enabled || !ctx->args[buffer_arg] || !ctx->args[size_arg]) {
        return 0;
    }
    fd_generation = socket_payload_fd_generation(tgid, fd);
    if (require_tracked_fd && !fd_generation) {
        return 0;
    }

    op.trace_id = *trace_id;
    op.buffer_ptr = (__u64)ctx->args[buffer_arg];
    op.requested_size = (__u64)ctx->args[size_arg];
    op.fd = fd;
    op.fd_generation = fd_generation;
    op.direction = direction;
    op.syscall = syscall;
    bpf_map_update_elem(&pending_socket_payload_ops, &pid_tgid, &op, BPF_ANY);
    return 0;
}

static __always_inline int emit_socket_payload_completion(
    struct actrail_pending_socket_payload_op *op,
    __u32 tgid,
    __u32 tid,
    __u64 completed_size
) {
    struct actrail_socket_payload_completion_event *event;

    event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
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
    event->pid_generation = ensure_process_generation(tgid);
    event->fd = op->fd;
    event->flags = 0;
    event->syscall = op->syscall;
    event->fd_generation = op->fd_generation;

    bpf_ringbuf_submit(event, 0);
    return 0;
}

static __always_inline int emit_socket_payload_op(struct trace_event_raw_sys_exit *ctx) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = current_namespace_tgid();
    __u32 tid = (__u32)pid_tgid;
    struct actrail_pending_socket_payload_op *op =
        bpf_map_lookup_elem(&pending_socket_payload_ops, &pid_tgid);
    struct actrail_socket_payload_event *event;
    __u64 original_size = (__u64)ctx->ret;
    __u64 bounded_size;
    __u32 capture_size;
    __u32 limit = payload_socket_capture_limit();

    if (!tgid || !op || ctx->ret <= 0 || !limit) {
        bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
        return 0;
    }

    if (op->direction == ACTRAIL_SOCKET_PAYLOAD_OUTBOUND
        && payload_socket_user_read_enabled()
        && original_size > limit) {
        emit_socket_payload_completion(op, tgid, tid, original_size);
        bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
        return 0;
    }

    bounded_size = original_size & ACTRAIL_SOCKET_PAYLOAD_COPY_MAX_BYTES;
    if (original_size > ACTRAIL_SOCKET_PAYLOAD_COPY_MAX_BYTES) {
        bounded_size = ACTRAIL_SOCKET_PAYLOAD_COPY_MAX_BYTES;
    }
    if (bounded_size > limit) {
        bounded_size = limit;
    }
    actrail_barrier_var(bounded_size);
    bounded_size &= ACTRAIL_SOCKET_PAYLOAD_COPY_MAX_BYTES;
    capture_size = (__u32)bounded_size;
    if (!capture_size) {
        bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
        return 0;
    }

    event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event) {
        bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
        return 0;
    }

    event->kind = ACTRAIL_SOCKET_PAYLOAD;
    event->pid = tgid;
    event->tid = tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->sequence =
        next_socket_payload_sequence(tgid, op->direction, op->fd, op->fd_generation);
    event->fd = op->fd;
    event->original_size = (__u32)original_size;
    event->captured_size = capture_size;
    event->flags = original_size > bounded_size ? ACTRAIL_SOCKET_PAYLOAD_TRUNCATED : 0;
    event->syscall = op->syscall;
    event->fd_generation = op->fd_generation;
    event->pid_generation = ensure_process_generation(tgid);
    if (bpf_probe_read_user(event->bytes, bounded_size, (void *)(unsigned long)op->buffer_ptr) != 0) {
        bpf_ringbuf_discard(event, 0);
        bpf_map_delete_elem(&pending_socket_payload_ops, &pid_tgid);
        return 0;
    }

    bpf_ringbuf_submit(event, 0);
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
    __u32 tgid = current_namespace_tgid();
    struct actrail_pending_net_op *op = bpf_map_lookup_elem(&pending_net_ops, &pid_tgid);

    if (!tgid || !op || ctx->ret != 0 || op->kind != ACTRAIL_NET_CONNECT) {
        return;
    }
    socket_payload_track_fd(tgid, op->fd);
}

static __always_inline void socket_payload_track_accept_exit(
    struct trace_event_raw_sys_exit *ctx
) {
    __u32 tgid = current_namespace_tgid();

    if (!tgid || ctx->ret < 0) {
        return;
    }
    socket_payload_track_fd(tgid, (__u32)ctx->ret);
}

static __always_inline void socket_payload_close_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    __u32 tgid = current_namespace_tgid();

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
    __u32 tgid = current_namespace_tgid();
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
    __u32 tgid = current_namespace_tgid();
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
