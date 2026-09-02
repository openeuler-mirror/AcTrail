#ifndef ACTRAIL_PAYLOAD_SOCKET_CAPTURE_H
#define ACTRAIL_PAYLOAD_SOCKET_CAPTURE_H

#include "socket_types.h"
#include "socket_tls.h"
#include "../runtime/fetch_add_compat.h"

#define ACTRAIL_LINUX_EINPROGRESS 115
#define ACTRAIL_SOCKET_PAYLOAD_IOVEC_SCAN_MAX 4

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

static __always_inline struct actrail_pending_socket_payload_op *
socket_payload_op_scratch_value(void) {
    __u32 key = 0;

    return bpf_map_lookup_elem(&socket_payload_op_scratch, &key);
}

static __always_inline __u64 next_socket_payload_operation_id(void) {
    __u64 kernel_pid_tgid = bpf_get_current_pid_tgid();
    __u64 initial = 1;
    __u64 *sequence;
    __u64 count;

    if (!kernel_pid_tgid) {
        event_transport_diag_inc(ACTRAIL_SOCKET_SEQUENCE_UPDATE_FAIL);
        return 0;
    }
    sequence = bpf_map_lookup_elem(
        &payload_socket_operation_sequence,
        &kernel_pid_tgid
    );
    if (!sequence) {
        if (bpf_map_update_elem(
                &payload_socket_operation_sequence,
                &kernel_pid_tgid,
                &initial,
                BPF_NOEXIST
            ) == 0) {
            return actrail_thread_sequence_id(kernel_pid_tgid, initial);
        }
        sequence = bpf_map_lookup_elem(
            &payload_socket_operation_sequence,
            &kernel_pid_tgid
        );
        if (!sequence) {
            event_transport_diag_inc(ACTRAIL_SOCKET_SEQUENCE_UPDATE_FAIL);
            return 0;
        }
    }
    count = actrail_fetch_add_next(sequence);
    if (!count) {
        event_transport_diag_inc(ACTRAIL_SOCKET_SEQUENCE_UPDATE_FAIL);
        return 0;
    }
    return actrail_thread_sequence_id(kernel_pid_tgid, count);
}

#include "socket_state.h"

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
    struct actrail_socket_payload_fd_state *fd_state;

    if (fd_generation) {
        fd_state = socket_payload_fd_state(pid, fd);
        if (!fd_state || fd_state->generation != fd_generation) {
            event_transport_diag_inc(ACTRAIL_SOCKET_STATE_UPDATE_FAIL);
            return 0;
        }
    }

    key.pid = pid;
    key.direction = direction;
    key.fd = fd;
    key.fd_generation = fd_generation;
    current = bpf_map_lookup_elem(&payload_socket_stream_sequences, &key);
    if (!current) {
        if (bpf_map_update_elem(
                &payload_socket_stream_sequences,
                &key,
                &initial,
                BPF_NOEXIST
            ) == 0) {
            return initial;
        }
        current = bpf_map_lookup_elem(&payload_socket_stream_sequences, &key);
        if (!current) {
            event_transport_diag_inc(ACTRAIL_SOCKET_SEQUENCE_UPDATE_FAIL);
            return 0;
        }
    }

    next = *current + 1;
    if (!next || bpf_map_update_elem(
            &payload_socket_stream_sequences,
            &key,
            &next,
            BPF_EXIST
        ) != 0) {
        event_transport_diag_inc(ACTRAIL_SOCKET_SEQUENCE_UPDATE_FAIL);
        return 0;
    }
    return next;
}

static __always_inline int store_socket_payload_op(
    __u32 syscall,
    __u32 fd,
    __u64 buffer_ptr,
    __u64 requested_size,
    __u32 require_tracked_fd
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u32 direction = syscall == ACTRAIL_SOCKET_SYSCALL_READ ||
            syscall == ACTRAIL_SOCKET_SYSCALL_RECVFROM
        ? ACTRAIL_SOCKET_PAYLOAD_INBOUND
        : ACTRAIL_SOCKET_PAYLOAD_OUTBOUND;
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_socket_payload_config *config = socket_payload_config();
    struct actrail_pending_socket_payload_op *op;
    struct actrail_socket_payload_fd_state *fd_state;
    __u32 fd_generation = 0;

    if (!tgid || !trace_id || !process_observation_is_detailed(tgid) ||
        !config || !config->enabled || !buffer_ptr || !requested_size) {
        return 0;
    }
    if (is_suppressed_fd(tgid, fd)) {
        return 0;
    }
    fd_state = socket_payload_fd_state(tgid, fd);
    if (fd_state && (fd_state->flags & ACTRAIL_SOCKET_FD_TLS_OWNED)) {
        return 0;
    }
    fd_generation = fd_state ? fd_state->generation : 0;
    if (require_tracked_fd && !fd_generation) {
        return 0;
    }
    if (fd_state
        && direction == ACTRAIL_SOCKET_PAYLOAD_OUTBOUND
        && (syscall == ACTRAIL_SOCKET_SYSCALL_WRITE
            || syscall == ACTRAIL_SOCKET_SYSCALL_SENDTO)
        && socket_payload_read_tls_hello_prefix(buffer_ptr, requested_size)) {
        fd_state->flags |= ACTRAIL_SOCKET_FD_TLS_OWNED;
        return 0;
    }

    op = socket_payload_op_scratch_value();
    if (!op) {
        return 0;
    }
    op->trace_id = *trace_id;
    op->operation_id = next_socket_payload_operation_id();
    if (!op->operation_id) {
        return 0;
    }
    op->buffer_ptr = buffer_ptr;
    op->requested_size = requested_size;
    op->pid_generation = current_process_start_time(tgid);
    op->started_ktime_ns = bpf_ktime_get_ns();
    op->fd = fd;
    op->fd_generation = fd_generation;
    op->direction = direction;
    op->syscall = syscall;
    op->copy_base = buffer_ptr;
    op->copy_capacity = requested_size;
    op->delegated = config->user_read_enabled;
    op->truncation_flags = config->truncation_flags;
    bpf_map_update_elem(&pending_socket_payload_ops, &pid_tgid, op, BPF_ANY);
    return 0;
}

/*
 * Locate the first non-empty iovec of a writev/sendmsg scatter-gather buffer.
 * Pure bpf-copy mode captures at most one segment from this range, so the
 * search is a small compile-time-unrolled scan instead of a verifier loop.
 */
static __always_inline int socket_payload_first_iovec_range(
    __u64 iov_ptr,
    __u64 iov_count,
    __u64 *copy_base,
    __u64 *copy_capacity
) {
    __u64 bound = iov_count;
    struct actrail_user_iovec iov = {};

    *copy_base = 0;
    *copy_capacity = 0;
    if (bound > ACTRAIL_SOCKET_PAYLOAD_IOVEC_SCAN_MAX) {
        bound = ACTRAIL_SOCKET_PAYLOAD_IOVEC_SCAN_MAX;
    }
#pragma unroll
    for (__u32 index = 0;
         index < ACTRAIL_SOCKET_PAYLOAD_IOVEC_SCAN_MAX;
         index++) {
        if ((__u64)index >= bound) {
            break;
        }
        if (bpf_probe_read_user(
                &iov,
                sizeof(iov),
                (void *)(unsigned long)(
                    iov_ptr + ((__u64)index * sizeof(iov))
                )
            ) != 0) {
            event_transport_diag_inc(ACTRAIL_SOCKET_READ_USER_FAIL);
            return 0;
        }
        if (iov.base && iov.len) {
            *copy_base = iov.base;
            *copy_capacity = iov.len;
            return 1;
        }
    }
    return 0;
}

#include "socket_emit.h"

static __always_inline int store_socket_payload_sendto_op(
    struct trace_event_raw_sys_enter *ctx
) {
    return store_socket_payload_op(
        ACTRAIL_SOCKET_SYSCALL_SENDTO,
        (__u32)ctx->args[0],
        (__u64)ctx->args[1],
        (__u64)ctx->args[2],
        0
    );
}

static __always_inline int store_socket_payload_writev_op(
    struct trace_event_raw_sys_enter *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_socket_payload_config *config = socket_payload_config();
    struct actrail_pending_socket_payload_op *op;
    struct actrail_socket_payload_fd_state *fd_state;
    __u32 fd = (__u32)ctx->args[0];

    if (!tgid || !trace_id || !process_observation_is_detailed(tgid) ||
        !config || !config->enabled || !config->max_segment_bytes ||
        !ctx->args[1] || !ctx->args[2] || is_suppressed_fd(tgid, fd)) {
        return 0;
    }
    fd_state = socket_payload_fd_state(tgid, fd);
    if (!fd_state || (fd_state->flags & ACTRAIL_SOCKET_FD_TLS_OWNED)) {
        return 0;
    }
    op = socket_payload_op_scratch_value();
    if (!op) {
        return 0;
    }
    op->trace_id = *trace_id;
    op->operation_id = next_socket_payload_operation_id();
    if (!op->operation_id) {
        return 0;
    }
    op->buffer_ptr = (__u64)ctx->args[1];
    op->requested_size = (__u64)ctx->args[2];
    op->pid_generation = current_process_start_time(tgid);
    op->started_ktime_ns = bpf_ktime_get_ns();
    op->fd = fd;
    op->fd_generation = fd_state->generation;
    op->direction = ACTRAIL_SOCKET_PAYLOAD_OUTBOUND;
    op->syscall = ACTRAIL_SOCKET_SYSCALL_WRITEV;
    op->delegated = config->user_read_enabled;
    op->truncation_flags = config->truncation_flags;
    if (!op->delegated &&
        !socket_payload_first_iovec_range(
            op->buffer_ptr,
            op->requested_size,
            &op->copy_base,
            &op->copy_capacity
        )) {
        return 0;
    }
    if (op->delegated) {
        op->copy_base = 0;
        op->copy_capacity = 0;
    }
    bpf_map_update_elem(&pending_socket_payload_ops, &pid_tgid, op, BPF_ANY);
    return 0;
}

static __always_inline int store_socket_payload_sendmsg_op(
    struct trace_event_raw_sys_enter *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_socket_payload_config *config = socket_payload_config();
    struct actrail_pending_socket_payload_op *op;
    struct actrail_user_msghdr message = {};
    struct actrail_socket_payload_fd_state *fd_state;
    __u32 fd = (__u32)ctx->args[0];

    if (!tgid || !trace_id || !process_observation_is_detailed(tgid) ||
        !config || !config->enabled || !config->max_segment_bytes ||
        !ctx->args[1] ||
        bpf_probe_read_user(
            &message,
            sizeof(message),
            (void *)(unsigned long)ctx->args[1]
        ) != 0 || !message.iov || !message.iov_len) {
        return 0;
    }
    if (is_suppressed_fd(tgid, fd)) {
        return 0;
    }
    fd_state = socket_payload_fd_state(tgid, fd);
    if (fd_state && (fd_state->flags & ACTRAIL_SOCKET_FD_TLS_OWNED)) {
        return 0;
    }

    op = socket_payload_op_scratch_value();
    if (!op) {
        return 0;
    }
    op->trace_id = *trace_id;
    op->operation_id = next_socket_payload_operation_id();
    if (!op->operation_id) {
        return 0;
    }
    op->buffer_ptr = (__u64)ctx->args[1];
    op->requested_size = 0;
    op->pid_generation = current_process_start_time(tgid);
    op->started_ktime_ns = bpf_ktime_get_ns();
    op->fd = fd;
    op->fd_generation = fd_state ? fd_state->generation : 0;
    op->direction = ACTRAIL_SOCKET_PAYLOAD_OUTBOUND;
    op->syscall = ACTRAIL_SOCKET_SYSCALL_SENDMSG;
    op->delegated = config->user_read_enabled;
    op->truncation_flags = config->truncation_flags;
    if (!op->delegated &&
        !socket_payload_first_iovec_range(
            message.iov,
            message.iov_len,
            &op->copy_base,
            &op->copy_capacity
        )) {
        return 0;
    }
    if (op->delegated) {
        op->copy_base = 0;
        op->copy_capacity = 0;
    }
    bpf_map_update_elem(&pending_socket_payload_ops, &pid_tgid, op, BPF_ANY);
    return 0;
}

static __always_inline int store_socket_payload_recvfrom_op(
    struct trace_event_raw_sys_enter *ctx
) {
    return store_socket_payload_op(
        ACTRAIL_SOCKET_SYSCALL_RECVFROM,
        (__u32)ctx->args[0],
        (__u64)ctx->args[1],
        (__u64)ctx->args[2],
        0
    );
}

static __always_inline int store_socket_payload_write_op(
    struct trace_event_raw_sys_enter *ctx
) {
    return store_socket_payload_op(
        ACTRAIL_SOCKET_SYSCALL_WRITE,
        (__u32)ctx->args[0],
        (__u64)ctx->args[1],
        (__u64)ctx->args[2],
        1
    );
}

static __always_inline int store_socket_payload_read_op(
    struct trace_event_raw_sys_enter *ctx
) {
    return store_socket_payload_op(
        ACTRAIL_SOCKET_SYSCALL_READ,
        (__u32)ctx->args[0],
        (__u64)ctx->args[1],
        (__u64)ctx->args[2],
        1
    );
}

#endif
