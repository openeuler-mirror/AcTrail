#ifndef ACTRAIL_SUPPRESSED_FD_H
#define ACTRAIL_SUPPRESSED_FD_H

#define ACTRAIL_SUPPRESSED_FD_PURPOSE_NONE 0
#define ACTRAIL_SUPPRESSED_FD_INDEX_SLOT_MAX 64

struct actrail_suppressed_fd_config {
    __u32 index_slots_per_process;
};

struct actrail_suppressed_fd_key {
    __u32 pid;
    __u32 fd;
    __u64 generation;
};

struct actrail_suppressed_fd_value {
    __u64 trace_id;
    __u32 purpose;
};

struct actrail_suppressed_fd_index_key {
    __u32 pid;
    __u32 slot;
    __u64 generation;
};

struct actrail_suppressed_fd_index_value {
    __u64 trace_id;
    __u32 fd;
    __u32 purpose;
};

struct actrail_pending_suppressed_fd_dup_op {
    __u32 source_fd;
    __u32 target_fd;
    __u32 mode;
    __u32 source_suppressed;
    __u32 target_suppressed;
    struct actrail_suppressed_fd_value source_value;
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_suppressed_fd_key);
    __type(value, struct actrail_suppressed_fd_value);
} suppressed_fds SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_suppressed_fd_index_key);
    __type(value, struct actrail_suppressed_fd_index_value);
} suppressed_fd_index SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_suppressed_fd_config);
} suppressed_fd_config SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_suppressed_fd_dup_op);
} pending_suppressed_fd_dup_ops SEC(".maps");

static __always_inline __u32 suppressed_fd_index_slots(void) {
    __u32 key = 0;
    struct actrail_suppressed_fd_config *config =
        bpf_map_lookup_elem(&suppressed_fd_config, &key);

    if (!config) {
        return 0;
    }
    return config->index_slots_per_process;
}

static __always_inline void fill_suppressed_fd_key(
    __u32 pid,
    __u32 fd,
    __u64 generation,
    struct actrail_suppressed_fd_key *key
) {
    key->pid = pid;
    key->fd = fd;
    key->generation = generation;
}

static __always_inline void fill_suppressed_fd_index_key(
    __u32 pid,
    __u64 generation,
    __u32 slot,
    struct actrail_suppressed_fd_index_key *key
) {
    key->pid = pid;
    key->slot = slot;
    key->generation = generation;
}

static __always_inline void suppressed_fd_index_value_from_fd(
    __u32 fd,
    const struct actrail_suppressed_fd_value *value,
    struct actrail_suppressed_fd_index_value *index_value
) {
    index_value->trace_id = value->trace_id;
    index_value->fd = fd;
    index_value->purpose = value->purpose;
}

static __always_inline void suppressed_fd_value_from_index(
    const struct actrail_suppressed_fd_index_value *index_value,
    struct actrail_suppressed_fd_value *value
) {
    value->trace_id = index_value->trace_id;
    value->purpose = index_value->purpose;
}

static __always_inline struct actrail_suppressed_fd_value *lookup_suppressed_fd(
    __u32 pid,
    __u32 fd
) {
    __u64 *generation = lookup_process_generation(pid);
    struct actrail_suppressed_fd_key key = {};

    if (!generation) {
        return 0;
    }
    fill_suppressed_fd_key(pid, fd, *generation, &key);
    return bpf_map_lookup_elem(&suppressed_fds, &key);
}

static __always_inline int is_suppressed_fd(__u32 pid, __u32 fd) {
    return lookup_suppressed_fd(pid, fd) != 0;
}

static __always_inline int upsert_suppressed_fd_index(
    __u32 pid,
    __u64 generation,
    __u32 fd,
    const struct actrail_suppressed_fd_value *value
) {
    __u32 slots = suppressed_fd_index_slots();
    struct actrail_suppressed_fd_index_key key = {};
    struct actrail_suppressed_fd_index_value index_value = {};
    __u32 slot;

    if (!pid || !generation || !slots || value->purpose == ACTRAIL_SUPPRESSED_FD_PURPOSE_NONE) {
        return 0;
    }
    suppressed_fd_index_value_from_fd(fd, value, &index_value);
#pragma unroll
    for (slot = 0; slot < ACTRAIL_SUPPRESSED_FD_INDEX_SLOT_MAX; slot++) {
        struct actrail_suppressed_fd_index_value *existing;

        if (slot >= slots) {
            break;
        }
        fill_suppressed_fd_index_key(pid, generation, slot, &key);
        existing = bpf_map_lookup_elem(&suppressed_fd_index, &key);
        if (existing) {
            if (existing->fd == fd) {
                return bpf_map_update_elem(&suppressed_fd_index, &key, &index_value, BPF_ANY) == 0;
            }
            continue;
        }
        if (bpf_map_update_elem(&suppressed_fd_index, &key, &index_value, BPF_ANY) == 0) {
            return 1;
        }
    }
    return 0;
}

static __always_inline void delete_suppressed_fd_index(
    __u32 pid,
    __u64 generation,
    __u32 fd
) {
    __u32 slots = suppressed_fd_index_slots();
    struct actrail_suppressed_fd_index_key key = {};
    __u32 slot;

    if (!pid || !generation || !slots) {
        return;
    }
#pragma unroll
    for (slot = 0; slot < ACTRAIL_SUPPRESSED_FD_INDEX_SLOT_MAX; slot++) {
        struct actrail_suppressed_fd_index_value *existing;

        if (slot >= slots) {
            break;
        }
        fill_suppressed_fd_index_key(pid, generation, slot, &key);
        existing = bpf_map_lookup_elem(&suppressed_fd_index, &key);
        if (existing && existing->fd == fd) {
            bpf_map_delete_elem(&suppressed_fd_index, &key);
            return;
        }
    }
}

static __always_inline void delete_suppressed_fd_for_generation(
    __u32 pid,
    __u64 generation,
    __u32 fd
) {
    struct actrail_suppressed_fd_key key = {};

    if (!pid || !generation) {
        return;
    }
    fill_suppressed_fd_key(pid, fd, generation, &key);
    bpf_map_delete_elem(&suppressed_fds, &key);
    delete_suppressed_fd_index(pid, generation, fd);
}

static __always_inline int upsert_suppressed_fd_for_generation(
    __u32 pid,
    __u64 generation,
    __u32 fd,
    const struct actrail_suppressed_fd_value *value
) {
    struct actrail_suppressed_fd_key key = {};

    if (!pid || !generation) {
        return 0;
    }
    fill_suppressed_fd_key(pid, fd, generation, &key);
    if (bpf_map_update_elem(&suppressed_fds, &key, value, BPF_ANY) != 0) {
        return 0;
    }
    upsert_suppressed_fd_index(pid, generation, fd, value);
    return 1;
}

static __always_inline void delete_suppressed_fd(__u32 pid, __u32 fd) {
    __u64 *generation = lookup_process_generation(pid);

    if (!generation) {
        return;
    }
    delete_suppressed_fd_for_generation(pid, *generation, fd);
}

static __always_inline void inherit_suppressed_fds_for_child(
    __u32 parent_pid,
    __u64 parent_generation,
    __u32 child_pid,
    __u64 child_generation
) {
    __u32 slots = suppressed_fd_index_slots();
    struct actrail_suppressed_fd_index_key parent_index_key = {};
    struct actrail_suppressed_fd_index_key child_index_key = {};
    struct actrail_suppressed_fd_key child_fd_key = {};
    __u32 slot;

    if (!parent_pid || !parent_generation || !child_pid || !child_generation || !slots) {
        return;
    }
#pragma unroll
    for (slot = 0; slot < ACTRAIL_SUPPRESSED_FD_INDEX_SLOT_MAX; slot++) {
        struct actrail_suppressed_fd_index_value *parent_index_value;
        struct actrail_suppressed_fd_value value = {};

        if (slot >= slots) {
            break;
        }
        fill_suppressed_fd_index_key(parent_pid, parent_generation, slot, &parent_index_key);
        parent_index_value = bpf_map_lookup_elem(&suppressed_fd_index, &parent_index_key);
        if (!parent_index_value) {
            continue;
        }
        suppressed_fd_value_from_index(parent_index_value, &value);
        fill_suppressed_fd_key(child_pid, parent_index_value->fd, child_generation, &child_fd_key);
        if (bpf_map_update_elem(&suppressed_fds, &child_fd_key, &value, BPF_ANY) != 0) {
            continue;
        }
        fill_suppressed_fd_index_key(child_pid, child_generation, slot, &child_index_key);
        bpf_map_update_elem(&suppressed_fd_index, &child_index_key, parent_index_value, BPF_ANY);
    }
}

static __always_inline void cleanup_suppressed_fds_for_process(
    __u32 pid,
    __u64 generation
) {
    __u32 slots = suppressed_fd_index_slots();
    struct actrail_suppressed_fd_index_key index_key = {};
    struct actrail_suppressed_fd_key fd_key = {};
    __u32 slot;

    if (!pid || !generation || !slots) {
        return;
    }
#pragma unroll
    for (slot = 0; slot < ACTRAIL_SUPPRESSED_FD_INDEX_SLOT_MAX; slot++) {
        struct actrail_suppressed_fd_index_value *index_value;

        if (slot >= slots) {
            break;
        }
        fill_suppressed_fd_index_key(pid, generation, slot, &index_key);
        index_value = bpf_map_lookup_elem(&suppressed_fd_index, &index_key);
        if (!index_value) {
            continue;
        }
        fill_suppressed_fd_key(pid, index_value->fd, generation, &fd_key);
        bpf_map_delete_elem(&suppressed_fds, &fd_key);
        bpf_map_delete_elem(&suppressed_fd_index, &index_key);
    }
}

static __always_inline int suppressed_fd_close_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    __u32 pid = current_tgid();
    __u32 fd = (__u32)ctx->args[0];
    int suppressed;

    if (!pid) {
        return 0;
    }
    suppressed = is_suppressed_fd(pid, fd);
    if (suppressed) {
        delete_suppressed_fd(pid, fd);
    }
    return suppressed;
}

#define ACTRAIL_SUPPRESSED_FD_DUP_RET_FD 1
#define ACTRAIL_SUPPRESSED_FD_DUP_TARGET_FD 2

static __always_inline int suppressed_fd_dup_enter(
    __u32 source_fd,
    __u32 target_fd,
    __u32 has_target_fd,
    __u32 mode
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    struct actrail_suppressed_fd_value *source;
    struct actrail_suppressed_fd_value *target;
    struct actrail_pending_suppressed_fd_dup_op op = {};

    if (!pid) {
        return 0;
    }
    op.source_fd = source_fd;
    op.target_fd = has_target_fd ? target_fd : 0;
    op.mode = mode;
    source = lookup_suppressed_fd(pid, op.source_fd);
    target = has_target_fd ? lookup_suppressed_fd(pid, op.target_fd) : 0;
    if (source) {
        op.source_suppressed = 1;
        op.source_value = *source;
    }
    if (target) {
        op.target_suppressed = 1;
    }
    if (!op.source_suppressed && !op.target_suppressed) {
        return 0;
    }
    bpf_map_update_elem(&pending_suppressed_fd_dup_ops, &pid_tgid, &op, BPF_ANY);
    return 1;
}

static __always_inline int suppressed_fd_fcntl_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    __u32 command = (__u32)ctx->args[1];

    if (command != F_DUPFD && command != F_DUPFD_CLOEXEC) {
        return 0;
    }
    return suppressed_fd_dup_enter(
        (__u32)ctx->args[0],
        0,
        0,
        ACTRAIL_SUPPRESSED_FD_DUP_RET_FD
    );
}

static __always_inline void suppressed_fd_dup_exit(
    struct trace_event_raw_sys_exit *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u64 *generation = lookup_process_generation(pid);
    struct actrail_pending_suppressed_fd_dup_op *op =
        bpf_map_lookup_elem(&pending_suppressed_fd_dup_ops, &pid_tgid);
    __u32 new_fd;

    if (!pid || !generation || !op) {
        return;
    }
    if (ctx->ret < 0) {
        bpf_map_delete_elem(&pending_suppressed_fd_dup_ops, &pid_tgid);
        return;
    }
    new_fd = op->mode == ACTRAIL_SUPPRESSED_FD_DUP_RET_FD ? (__u32)ctx->ret : op->target_fd;
    if (op->source_suppressed) {
        upsert_suppressed_fd_for_generation(pid, *generation, new_fd, &op->source_value);
    } else if (op->target_suppressed) {
        delete_suppressed_fd_for_generation(pid, *generation, new_fd);
    }
    bpf_map_delete_elem(&pending_suppressed_fd_dup_ops, &pid_tgid);
}

#endif
