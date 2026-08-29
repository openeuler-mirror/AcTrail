#ifndef ACTRAIL_PAYLOAD_SOCKET_STATE_H
#define ACTRAIL_PAYLOAD_SOCKET_STATE_H

static __always_inline struct actrail_socket_payload_fd_key socket_payload_fd_key(
    __u32 pid,
    __u32 fd
) {
    struct actrail_socket_payload_fd_key key = {};
    key.pid = pid;
    key.fd = fd;
    return key;
}

static __always_inline struct actrail_socket_payload_fd_state *socket_payload_fd_state(
    __u32 pid,
    __u32 fd
) {
    struct actrail_socket_payload_fd_key key = socket_payload_fd_key(pid, fd);
    return bpf_map_lookup_elem(&payload_socket_fds, &key);
}

static __always_inline __u32 socket_payload_fd_generation(__u32 pid, __u32 fd) {
    struct actrail_socket_payload_fd_state *state = socket_payload_fd_state(pid, fd);
    return state ? state->generation : 0;
}

static __always_inline int socket_payload_mark_fd_tls_owned(
    __u32 pid,
    __u32 fd,
    __u32 expected_generation
) {
    struct actrail_socket_payload_fd_state *state = socket_payload_fd_state(pid, fd);

    if (!state || !expected_generation || state->generation != expected_generation) {
        return 0;
    }
    state->flags |= ACTRAIL_SOCKET_FD_TLS_OWNED;
    return 1;
}

static __always_inline __u32 socket_payload_next_generation(__u32 pid) {
    __u32 initial = 1;
    __u32 next;
    __u32 *current = bpf_map_lookup_elem(&payload_socket_process_generations, &pid);

    if (!current) {
        if (bpf_map_update_elem(
                &payload_socket_process_generations,
                &pid,
                &initial,
                BPF_NOEXIST
            ) != 0) {
                current = bpf_map_lookup_elem(&payload_socket_process_generations, &pid);
                if (current) {
                    next = *current + 1;
                    if (next && bpf_map_update_elem(
                            &payload_socket_process_generations,
                            &pid,
                            &next,
                            BPF_EXIST
                        ) == 0) {
                        return next;
                    }
                }
                event_transport_diag_inc(ACTRAIL_SOCKET_STATE_UPDATE_FAIL);
                return 0;
        }
        return initial;
    }

    next = *current + 1;
    if (!next || bpf_map_update_elem(
            &payload_socket_process_generations,
            &pid,
            &next,
            BPF_EXIST
        ) != 0) {
        event_transport_diag_inc(ACTRAIL_SOCKET_STATE_UPDATE_FAIL);
        return 0;
    }
    return next;
}

static __always_inline int socket_payload_set_fd_state(
    __u32 pid,
    __u32 fd,
    __u32 generation,
    __u32 flags
) {
    struct actrail_socket_payload_fd_key key;
    struct actrail_socket_payload_fd_state state = {};

    if (!generation) {
        return 0;
    }
    key = socket_payload_fd_key(pid, fd);
    state.generation = generation;
    state.flags = flags;
    if (bpf_map_update_elem(&payload_socket_fds, &key, &state, BPF_ANY) != 0) {
        event_transport_diag_inc(ACTRAIL_SOCKET_STATE_UPDATE_FAIL);
        return 0;
    }
    return 1;
}

static __always_inline void socket_payload_delete_sequences(
    __u32 pid,
    __u32 fd,
    __u32 generation
) {
    struct actrail_socket_payload_sequence_key key = {};

    key.pid = pid;
    key.fd = fd;
    key.fd_generation = generation;
    key.direction = ACTRAIL_SOCKET_PAYLOAD_INBOUND;
    bpf_map_delete_elem(&payload_socket_stream_sequences, &key);
    key.direction = ACTRAIL_SOCKET_PAYLOAD_OUTBOUND;
    bpf_map_delete_elem(&payload_socket_stream_sequences, &key);
}

static __always_inline void socket_payload_release_fd(__u32 pid, __u32 fd) {
    struct actrail_socket_payload_config *config = socket_payload_config();
    struct actrail_socket_payload_fd_key key;
    struct actrail_socket_payload_fd_state *state;
    __u32 generation;

    if (!config || !config->enabled) {
        return;
    }
    key = socket_payload_fd_key(pid, fd);
    state = bpf_map_lookup_elem(&payload_socket_fds, &key);
    generation = state ? state->generation : 0;
    if (generation) {
        socket_payload_delete_sequences(pid, fd, generation);
    }
    socket_payload_delete_sequences(pid, fd, 0);
    bpf_map_delete_elem(&payload_socket_fds, &key);
}

static __always_inline void socket_payload_release_process(__u32 pid) {
    struct actrail_socket_payload_config *config = socket_payload_config();

    if (!config || !config->enabled) {
        return;
    }
    bpf_map_delete_elem(&payload_socket_process_generations, &pid);
}

static __always_inline void socket_payload_track_fd(__u32 pid, __u32 fd) {
    struct actrail_socket_payload_config *config = socket_payload_config();
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &pid);
    __u32 generation;

    if (!config || !config->enabled || !trace_id) {
        return;
    }
    socket_payload_release_fd(pid, fd);
    generation = socket_payload_next_generation(pid);
    socket_payload_set_fd_state(pid, fd, generation, 0);
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
    struct actrail_socket_payload_fd_state *source_state;

    if (!tgid || !config || !config->enabled) {
        return;
    }
    op.source_fd = source_fd;
    op.target_fd = target_fd;
    source_state = socket_payload_fd_state(tgid, source_fd);
    op.source_generation = source_state ? source_state->generation : 0;
    op.source_flags = source_state ? source_state->flags : 0;
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
        if (new_fd != op->source_fd) {
            socket_payload_release_fd(tgid, new_fd);
        }
        socket_payload_set_fd_state(
            tgid,
            new_fd,
            op->source_generation,
            op->source_flags
        );
    } else if (op->target_generation) {
        socket_payload_release_fd(tgid, new_fd);
    }
    bpf_map_delete_elem(&pending_socket_dup_ops, &pid_tgid);
}

#endif
