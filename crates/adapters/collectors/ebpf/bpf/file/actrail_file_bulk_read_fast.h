#ifndef ACTRAIL_FILE_BULK_READ_FAST_H
#define ACTRAIL_FILE_BULK_READ_FAST_H

struct actrail_file_bulk_read_fast_process_key {
    __u32 pid;
    __u64 generation;
} __attribute__((packed));

struct actrail_file_bulk_read_fast_process_value {
    __u64 trace_id;
} __attribute__((packed));

struct actrail_file_bulk_read_fast_fd_key {
    __u32 pid;
    __u32 fd;
    __u64 generation;
} __attribute__((packed));

struct actrail_file_bulk_read_fast_fd_stats {
    __u64 trace_id;
    __u64 read_count;
    __u64 bytes_read;
    __u64 error_count;
    __u64 first_ktime_ns;
    __u64 last_ktime_ns;
} __attribute__((packed));

struct actrail_pending_file_bulk_read_fast_op {
    __u32 pid;
    __u32 fd;
    __u32 source_fd;
    __u32 target_fd;
    __u32 mode;
    __u32 source_tracked;
    __u32 target_tracked;
    __u64 generation;
} __attribute__((packed));

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_file_bulk_read_fast_process_key);
    __type(value, struct actrail_file_bulk_read_fast_process_value);
} file_bulk_read_fast_processes SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_file_bulk_read_fast_fd_key);
    __type(value, struct actrail_file_bulk_read_fast_fd_stats);
} file_bulk_read_fast_fd_stats SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_file_bulk_read_fast_op);
} pending_file_bulk_read_fast_ops SEC(".maps");

static __always_inline void fill_file_bulk_read_fast_process_key(
    __u32 pid,
    __u64 generation,
    struct actrail_file_bulk_read_fast_process_key *key
) {
    key->pid = pid;
    key->generation = generation;
}

static __always_inline void fill_file_bulk_read_fast_fd_key(
    __u32 pid,
    __u32 fd,
    __u64 generation,
    struct actrail_file_bulk_read_fast_fd_key *key
) {
    key->pid = pid;
    key->fd = fd;
    key->generation = generation;
}

static __always_inline struct actrail_file_bulk_read_fast_process_value *
lookup_file_bulk_read_fast_process(__u32 pid, __u64 generation) {
    struct actrail_file_bulk_read_fast_process_key key = {};

    if (!pid || !generation) {
        return 0;
    }
    fill_file_bulk_read_fast_process_key(pid, generation, &key);
    return bpf_map_lookup_elem(&file_bulk_read_fast_processes, &key);
}

static __always_inline void delete_file_bulk_read_fast_process(
    __u32 pid,
    __u64 generation
) {
    struct actrail_file_bulk_read_fast_process_key key = {};

    if (!pid || !generation) {
        return;
    }
    fill_file_bulk_read_fast_process_key(pid, generation, &key);
    bpf_map_delete_elem(&file_bulk_read_fast_processes, &key);
}

static __always_inline struct actrail_file_bulk_read_fast_fd_stats *
lookup_file_bulk_read_fast_fd(__u32 pid, __u32 fd, __u64 generation) {
    struct actrail_file_bulk_read_fast_fd_key key = {};

    if (!pid || !generation || fd == ACTRAIL_FILE_FD_MISSING) {
        return 0;
    }
    fill_file_bulk_read_fast_fd_key(pid, fd, generation, &key);
    return bpf_map_lookup_elem(&file_bulk_read_fast_fd_stats, &key);
}

static __always_inline void delete_file_bulk_read_fast_fd(
    __u32 pid,
    __u32 fd,
    __u64 generation
) {
    struct actrail_file_bulk_read_fast_fd_key key = {};

    if (!pid || !generation || fd == ACTRAIL_FILE_FD_MISSING) {
        return;
    }
    fill_file_bulk_read_fast_fd_key(pid, fd, generation, &key);
    bpf_map_delete_elem(&file_bulk_read_fast_fd_stats, &key);
}

static __always_inline void insert_file_bulk_read_fast_fd(
    __u32 pid,
    __u32 fd,
    __u64 generation,
    __u64 trace_id
) {
    struct actrail_file_bulk_read_fast_fd_key key = {};
    struct actrail_file_bulk_read_fast_fd_stats stats = {};

    if (!pid || !generation || !trace_id || fd == ACTRAIL_FILE_FD_MISSING) {
        return;
    }
    fill_file_bulk_read_fast_fd_key(pid, fd, generation, &key);
    stats.trace_id = trace_id;
    bpf_map_update_elem(&file_bulk_read_fast_fd_stats, &key, &stats, BPF_ANY);
}

static __always_inline void upsert_file_bulk_read_fast_fd_stats(
    __u32 pid,
    __u32 fd,
    __u64 generation,
    const struct actrail_file_bulk_read_fast_fd_stats *stats
) {
    struct actrail_file_bulk_read_fast_fd_key key = {};

    if (!pid || !generation || !stats || fd == ACTRAIL_FILE_FD_MISSING) {
        return;
    }
    fill_file_bulk_read_fast_fd_key(pid, fd, generation, &key);
    bpf_map_update_elem(&file_bulk_read_fast_fd_stats, &key, stats, BPF_ANY);
}

static __always_inline int emit_file_bulk_read_fast_summary(
    void *ctx,
    __u32 pid,
    __u32 fd,
    __u64 generation,
    const struct actrail_file_bulk_read_fast_fd_stats *stats
) {
    struct actrail_file_event *event;

    if (!stats || !stats->trace_id || stats->read_count == 0) {
        return 0;
    }
    event = actrail_event_reserve(ACTRAIL_FILE_EVENT_HEADER_SIZE);
    if (!event) {
        return 0;
    }
    init_file_event_header(event, ACTRAIL_FILE_READ_SUMMARY);
    event->pid = pid;
    event->tid = (__u32)current_pid_tgid();
    event->pid_generation = generation;
    event->phase = ACTRAIL_FILE_PHASE_EXIT;
    event->result = 0;
    event->trace_id = stats->trace_id;
    event->fd = fd;
    event->aux = ACTRAIL_FILE_SYSCALL_READ_SUMMARY;
    event->arg0 = stats->read_count;
    event->arg1 = stats->bytes_read;
    event->arg2 = stats->error_count;
    event->arg3 = stats->first_ktime_ns;
    event->arg4 = stats->last_ktime_ns;
    actrail_event_submit(ctx, event);
    return 1;
}

static __always_inline void maybe_insert_file_bulk_read_fast_open_fd(
    __u32 pid,
    __u32 fd,
    __u64 generation,
    __u64 trace_id
) {
    struct actrail_file_bulk_read_fast_process_value *process;

    if (!pid || !generation || !trace_id) {
        return;
    }
    process = lookup_file_bulk_read_fast_process(pid, generation);
    if (!process || process->trace_id != trace_id) {
        return;
    }
    insert_file_bulk_read_fast_fd(pid, fd, generation, trace_id);
}

static __always_inline int store_file_bulk_read_fast_read_op(
    struct trace_event_raw_sys_enter *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u32 fd = (__u32)ctx->args[0];
    __u64 *generation = lookup_process_generation(pid);
    struct actrail_file_bulk_read_fast_fd_stats *stats;
    struct actrail_pending_file_bulk_read_fast_op op = {};

    if (!pid || !generation) {
        return 0;
    }
    stats = lookup_file_bulk_read_fast_fd(pid, fd, *generation);
    if (!stats) {
        return 0;
    }
    op.pid = pid;
    op.fd = fd;
    op.generation = *generation;
    bpf_map_update_elem(&pending_file_bulk_read_fast_ops, &pid_tgid, &op, BPF_ANY);
    return 1;
}

static __always_inline int emit_file_bulk_read_fast_read_op(
    struct trace_event_raw_sys_exit *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    struct actrail_pending_file_bulk_read_fast_op *op =
        bpf_map_lookup_elem(&pending_file_bulk_read_fast_ops, &pid_tgid);
    struct actrail_file_bulk_read_fast_fd_stats *stats;
    __u64 now = bpf_ktime_get_ns();

    if (!op) {
        return 0;
    }
    stats = lookup_file_bulk_read_fast_fd(op->pid, op->fd, op->generation);
    if (!stats) {
        bpf_map_delete_elem(&pending_file_bulk_read_fast_ops, &pid_tgid);
        return 1;
    }
    if (stats->read_count == 0) {
        stats->first_ktime_ns = now;
    }
    stats->last_ktime_ns = now;
    stats->read_count += 1;
    if (ctx->ret < 0) {
        stats->error_count += 1;
    } else {
        stats->bytes_read += (__u64)ctx->ret;
    }
    bpf_map_delete_elem(&pending_file_bulk_read_fast_ops, &pid_tgid);
    return 1;
}

static __always_inline int store_file_bulk_read_fast_close_op(
    struct trace_event_raw_sys_enter *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u32 fd = (__u32)ctx->args[0];
    __u64 *generation = lookup_process_generation(pid);
    struct actrail_file_bulk_read_fast_fd_stats *stats;
    struct actrail_pending_file_bulk_read_fast_op op = {};

    if (!pid || !generation) {
        return 0;
    }
    stats = lookup_file_bulk_read_fast_fd(pid, fd, *generation);
    if (!stats) {
        return 0;
    }
    op.pid = pid;
    op.fd = fd;
    op.generation = *generation;
    bpf_map_update_elem(&pending_file_bulk_read_fast_ops, &pid_tgid, &op, BPF_ANY);
    return 1;
}

static __always_inline void emit_file_bulk_read_fast_close_op(
    struct trace_event_raw_sys_exit *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    struct actrail_pending_file_bulk_read_fast_op *op =
        bpf_map_lookup_elem(&pending_file_bulk_read_fast_ops, &pid_tgid);
    struct actrail_file_bulk_read_fast_fd_stats *stats;

    if (!op) {
        return;
    }
    if (ctx->ret == 0) {
        stats = lookup_file_bulk_read_fast_fd(op->pid, op->fd, op->generation);
        emit_file_bulk_read_fast_summary(ctx, op->pid, op->fd, op->generation, stats);
        delete_file_bulk_read_fast_fd(op->pid, op->fd, op->generation);
    }
    bpf_map_delete_elem(&pending_file_bulk_read_fast_ops, &pid_tgid);
}

#define ACTRAIL_FILE_BULK_READ_FAST_DUP_RET_FD 1
#define ACTRAIL_FILE_BULK_READ_FAST_DUP_TARGET_FD 2

static __always_inline int store_file_bulk_read_fast_dup_op(
    __u32 source_fd,
    __u32 target_fd,
    __u32 has_target_fd,
    __u32 mode
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u64 *generation = lookup_process_generation(pid);
    struct actrail_pending_file_bulk_read_fast_op op = {};

    if (!pid || !generation) {
        return 0;
    }
    op.pid = pid;
    op.fd = source_fd;
    op.source_fd = source_fd;
    op.target_fd = has_target_fd ? target_fd : 0;
    op.mode = mode;
    op.generation = *generation;
    if (lookup_file_bulk_read_fast_fd(pid, source_fd, *generation)) {
        op.source_tracked = 1;
    }
    if (has_target_fd && lookup_file_bulk_read_fast_fd(pid, op.target_fd, *generation)) {
        op.target_tracked = 1;
    }
    if (!op.source_tracked && !op.target_tracked) {
        return 0;
    }
    bpf_map_update_elem(&pending_file_bulk_read_fast_ops, &pid_tgid, &op, BPF_ANY);
    return 1;
}

static __always_inline int store_file_bulk_read_fast_fcntl_op(
    struct trace_event_raw_sys_enter *ctx
) {
    __u32 command = (__u32)ctx->args[1];

    if (command != F_DUPFD && command != F_DUPFD_CLOEXEC) {
        return 0;
    }
    return store_file_bulk_read_fast_dup_op(
        (__u32)ctx->args[0],
        0,
        0,
        ACTRAIL_FILE_BULK_READ_FAST_DUP_RET_FD
    );
}

static __always_inline void emit_file_bulk_read_fast_dup_op(
    struct trace_event_raw_sys_exit *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    struct actrail_pending_file_bulk_read_fast_op *op =
        bpf_map_lookup_elem(&pending_file_bulk_read_fast_ops, &pid_tgid);
    struct actrail_file_bulk_read_fast_fd_stats *source_stats;
    struct actrail_file_bulk_read_fast_fd_stats *target_stats;
    struct actrail_file_bulk_read_fast_fd_stats stats_copy = {};
    __u32 new_fd;

    if (!op) {
        return;
    }
    if (ctx->ret < 0) {
        bpf_map_delete_elem(&pending_file_bulk_read_fast_ops, &pid_tgid);
        return;
    }
    new_fd = op->mode == ACTRAIL_FILE_BULK_READ_FAST_DUP_RET_FD
        ? (__u32)ctx->ret
        : op->target_fd;
    if (op->target_tracked && op->target_fd != op->source_fd) {
        target_stats = lookup_file_bulk_read_fast_fd(op->pid, op->target_fd, op->generation);
        emit_file_bulk_read_fast_summary(
            ctx,
            op->pid,
            op->target_fd,
            op->generation,
            target_stats
        );
        delete_file_bulk_read_fast_fd(op->pid, op->target_fd, op->generation);
    }
    if (op->source_tracked) {
        source_stats = lookup_file_bulk_read_fast_fd(op->pid, op->source_fd, op->generation);
        if (source_stats) {
            stats_copy = *source_stats;
            upsert_file_bulk_read_fast_fd_stats(op->pid, new_fd, op->generation, &stats_copy);
        }
    } else if (op->target_tracked) {
        delete_file_bulk_read_fast_fd(op->pid, new_fd, op->generation);
    }
    bpf_map_delete_elem(&pending_file_bulk_read_fast_ops, &pid_tgid);
}

#endif
