#ifndef ACTRAIL_FD_INDEX_H
#define ACTRAIL_FD_INDEX_H

static __always_inline __u32 fd_category_for_domain(__u32 domain) {
    switch (domain) {
    case AF_INET:
    case AF_INET6:
        return ACTRAIL_FD_CATEGORY_NET;
    case AF_UNIX:
        return ACTRAIL_FD_CATEGORY_IPC_UNIX_SOCKET;
    default:
        return ACTRAIL_FD_CATEGORY_NONE;
    }
}

static __always_inline struct actrail_fd_config *fd_config(void) {
    __u32 key = 0;
    return bpf_map_lookup_elem(&fd_category_config, &key);
}

static __always_inline __u32 fd_category_config_flags(void) {
    struct actrail_fd_config *config = fd_config();
    return config ? config->category_flags : 0;
}

static __always_inline int fd_tracking_enabled(void) {
    return fd_category_config_flags() != 0;
}

static __always_inline int fd_category_enabled(__u32 category) {
    return (fd_category_config_flags() & (1u << category)) != 0;
}

static __always_inline __u32 fd_slot_limit(void) {
    struct actrail_fd_config *config = fd_config();
    return config ? config->max_slots_per_process : 0;
}

static __always_inline __u32 fd_process_active_count(__u32 pid) {
    __u32 *count = bpf_map_lookup_elem(&fd_process_active_counts, &pid);
    return count ? *count : 0;
}

static __always_inline int fd_process_active_count_seed_empty(__u32 pid) {
    __u32 count = 0;

    if (!pid) {
        return 0;
    }
    return bpf_map_update_elem(&fd_process_active_counts, &pid, &count, BPF_NOEXIST) == 0;
}

static __always_inline int fd_process_active_count_acquire(__u32 pid) {
    __u32 *count = bpf_map_lookup_elem(&fd_process_active_counts, &pid);
    __u32 initial = 1;

    if (count) {
        __sync_fetch_and_add(count, 1);
        return 1;
    }
    if (bpf_map_update_elem(&fd_process_active_counts, &pid, &initial, BPF_NOEXIST) == 0) {
        return 1;
    }
    count = bpf_map_lookup_elem(&fd_process_active_counts, &pid);
    if (!count) {
        return 0;
    }
    __sync_fetch_and_add(count, 1);
    return 1;
}

static __always_inline void fd_process_active_count_release(__u32 pid) {
    __u32 *count = bpf_map_lookup_elem(&fd_process_active_counts, &pid);

    if (count && *count) {
        __sync_fetch_and_add(count, (__u32)-1);
    }
}

static __always_inline struct actrail_fd_state *fd_lookup(__u32 pid, __u32 fd) {
    struct actrail_fd_key key = { .pid = pid, .fd = fd };
    return bpf_map_lookup_elem(&fd_table, &key);
}

static __always_inline __u64 fd_kernel_file_identity(__u32 fd) {
    struct task_struct *task = actrail_bpf_get_current_task();
    struct files_struct *files = 0;
    struct fdtable *table = 0;
    struct file **entries = 0;
    struct file *file = 0;
    __u32 max_fds = 0;
    __u64 entry_address;

    if (!task || ACTRAIL_CORE_READ(&files, task, files) != 0 || !files
        || ACTRAIL_CORE_READ(&table, files, fdt) != 0 || !table
        || ACTRAIL_CORE_READ(&max_fds, table, max_fds) != 0) {
        return ACTRAIL_FD_FILE_IDENTITY_READ_FAILED;
    }
    if (fd >= max_fds) {
        return 0;
    }
    if (ACTRAIL_CORE_READ(&entries, table, fd) != 0 || !entries) {
        return ACTRAIL_FD_FILE_IDENTITY_READ_FAILED;
    }
    entry_address = (__u64)entries + ((__u64)fd * sizeof(*entries));
    if (bpf_probe_read_kernel(
            &file,
            sizeof(file),
            (const void *)entry_address
        ) != 0) {
        return ACTRAIL_FD_FILE_IDENTITY_READ_FAILED;
    }
    return (__u64)file;
}

static __always_inline int fd_kernel_cloexec(__u32 fd) {
    struct task_struct *task = actrail_bpf_get_current_task();
    struct files_struct *files = 0;
    struct fdtable *table = 0;
    unsigned long *bitmap = 0;
    unsigned long word = 0;
    __u32 max_fds = 0;
    __u32 bits_per_word = sizeof(word) * 8;
    __u64 word_address;

    if (!task || ACTRAIL_CORE_READ(&files, task, files) != 0 || !files
        || ACTRAIL_CORE_READ(&table, files, fdt) != 0 || !table
        || ACTRAIL_CORE_READ(&max_fds, table, max_fds) != 0) {
        return -1;
    }
    if (fd >= max_fds) {
        return 0;
    }
    if (ACTRAIL_CORE_READ(&bitmap, table, close_on_exec) != 0 || !bitmap) {
        return -1;
    }
    word_address = (__u64)bitmap + ((__u64)(fd / bits_per_word) * sizeof(word));
    if (bpf_probe_read_kernel(
            &word,
            sizeof(word),
            (const void *)word_address
        ) != 0) {
        return -1;
    }
    return (word >> (fd % bits_per_word)) & 1;
}

static __always_inline struct actrail_fd_object_state *fd_object_lookup(
    __u32 pid,
    __u64 generation
) {
    struct actrail_fd_object_key key = { .pid = pid, .generation = generation };
    return bpf_map_lookup_elem(&fd_objects, &key);
}

static __always_inline int fd_object_snapshot(
    __u32 pid,
    __u64 generation,
    struct actrail_fd_object_snapshot *snapshot
) {
    struct actrail_fd_object_state *object = fd_object_lookup(pid, generation);

    if (!object) {
        return 0;
    }
    snapshot->refcount = object->refcount;
    snapshot->category = object->category;
    snapshot->trace_id = object->trace_id;
    snapshot->remote = object->remote;
    return 1;
}

static __always_inline __u64 fd_generation_next(void) {
    __u64 generation = bpf_ktime_get_ns();
    return generation ? generation : 1;
}

static __noinline int fd_slot_allocate(
    __u32 pid,
    __u32 fd,
    __u64 generation,
    __u32 *allocated_slot
) {
    __u32 max_slots = fd_slot_limit();
    __u32 start;
    __u32 offset;

    if (!max_slots || max_slots > ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES) {
        return 0;
    }
    start = fd % max_slots;
#pragma clang loop unroll(disable)
    for (offset = 0; offset < ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES; offset++) {
        struct actrail_fd_index_slot_key key;
        struct actrail_fd_index_slot value = { .fd = fd, .generation = generation };
        __u32 candidate;

        if (offset >= max_slots) {
            break;
        }
        candidate = start + offset;
        if (candidate >= max_slots) {
            candidate -= max_slots;
        }
        key.pid = pid;
        key.slot = candidate;
        if (bpf_map_update_elem(&fd_index_slots, &key, &value, BPF_NOEXIST) == 0) {
            *allocated_slot = candidate;
            return 1;
        }
    }
    return 0;
}

static __always_inline int fd_slot_release(
    __u32 pid,
    __u32 slot,
    __u32 fd,
    __u64 generation
) {
    struct actrail_fd_index_slot_key key = { .pid = pid, .slot = slot };
    struct actrail_fd_index_slot *indexed = bpf_map_lookup_elem(&fd_index_slots, &key);

    if (!indexed || indexed->fd != fd || indexed->generation != generation) {
        return 0;
    }
    return bpf_map_delete_elem(&fd_index_slots, &key) == 0;
}

static __always_inline int fd_object_create(
    __u32 pid,
    __u64 generation,
    __u32 category,
    __u64 trace_id,
    __u64 file_identity
) {
    struct actrail_fd_object_key key = { .pid = pid, .generation = generation };
    struct actrail_fd_object_state object = {
        .refcount = 1,
        .category = category,
        .trace_id = trace_id,
        .file_identity = file_identity,
    };

    return bpf_map_update_elem(&fd_objects, &key, &object, BPF_NOEXIST) == 0;
}

static __always_inline __u32 fd_object_ref_release(
    __u32 pid,
    __u64 generation,
    struct actrail_fd_object_snapshot *snapshot
) {
    struct actrail_fd_object_key key = { .pid = pid, .generation = generation };
    struct actrail_fd_object_state *object = bpf_map_lookup_elem(&fd_objects, &key);

    if (!object || !object->refcount) {
        return ACTRAIL_FD_REFCOUNT_MISSING;
    }
    snapshot->refcount = object->refcount;
    snapshot->category = object->category;
    snapshot->trace_id = object->trace_id;
    snapshot->remote = object->remote;
    __sync_fetch_and_add(&object->refcount, (__u32)-1);
    object = bpf_map_lookup_elem(&fd_objects, &key);
    if (!object || object->refcount != 0) {
        return ACTRAIL_FD_REFCOUNT_NOT_LAST;
    }
    if (bpf_map_delete_elem(&fd_objects, &key) != 0) {
        return ACTRAIL_FD_REFCOUNT_NOT_LAST;
    }
    return 0;
}

static __always_inline int fd_claim(
    __u32 pid,
    __u32 fd,
    __u64 expected_generation,
    struct actrail_fd_state *snapshot
) {
    struct actrail_fd_key key = { .pid = pid, .fd = fd };
    struct actrail_fd_state *state = bpf_map_lookup_elem(&fd_table, &key);

    if (!state || (expected_generation && state->generation != expected_generation)) {
        return 0;
    }
    *snapshot = *state;
    if (bpf_map_delete_elem(&fd_table, &key) != 0) {
        return 0;
    }
    fd_slot_release(pid, snapshot->index_slot, fd, snapshot->generation);
    fd_process_active_count_release(pid);
    return 1;
}

static __noinline int fd_release(
    __u32 pid,
    __u32 fd,
    __u64 expected_generation,
    __u64 trace_id,
    void *ctx
) {
    struct actrail_fd_state state = {};
    struct actrail_fd_object_snapshot object = {};
    __u32 remaining;
    __u64 event_trace_id;

    if (!fd_claim(pid, fd, expected_generation, &state)) {
        return 0;
    }
    remaining = fd_object_ref_release(pid, state.generation, &object);
    event_trace_id = trace_id ? trace_id : object.trace_id;
    if (state.category == ACTRAIL_FD_CATEGORY_NET && remaining == 0 && event_trace_id) {
        struct actrail_event *event = actrail_event_reserve(sizeof(*event));

        if (event) {
            init_event(event, ACTRAIL_NET_CLOSE, pid, event_trace_id);
            event->fd = fd;
            event->aux = ACTRAIL_SYSCALL_FAMILY_SOCKET;
            event->aux_generation = state.generation;
            event->remote = object.remote;
            actrail_event_submit(ctx, event);
        }
    }
    return 1;
}

static __always_inline void fd_reserved_ref_release(
    __u32 pid,
    __u32 fd,
    const struct actrail_fd_state *state,
    __u64 trace_id,
    void *ctx
) {
    struct actrail_fd_object_snapshot object = {};
    __u32 remaining;
    __u64 event_trace_id;

    remaining = fd_object_ref_release(pid, state->generation, &object);
    event_trace_id = trace_id ? trace_id : object.trace_id;
    if (state->category == ACTRAIL_FD_CATEGORY_NET && remaining == 0 && event_trace_id) {
        struct actrail_event *event = actrail_event_reserve(sizeof(*event));

        if (event) {
            init_event(event, ACTRAIL_NET_CLOSE, pid, event_trace_id);
            event->fd = fd;
            event->aux = ACTRAIL_SYSCALL_FAMILY_SOCKET;
            event->aux_generation = state->generation;
            event->remote = object.remote;
            actrail_event_submit(ctx, event);
        }
    }
}

static __always_inline int fd_object_ref_reserve(
    __u32 pid,
    __u32 fd,
    const struct actrail_fd_state *source,
    __u64 trace_id,
    void *ctx
) {
    struct actrail_fd_object_state *object = fd_object_lookup(pid, source->generation);
    struct actrail_fd_state *current;

    if (!object || !object->refcount
        || object->refcount >= ACTRAIL_FD_REFCOUNT_NOT_LAST) {
        return 0;
    }
    __sync_fetch_and_add(&object->refcount, 1);
    current = fd_lookup(pid, fd);
    object = fd_object_lookup(pid, source->generation);
    if (current && current->generation == source->generation && object) {
        return 1;
    }
    fd_reserved_ref_release(pid, fd, source, trace_id, ctx);
    return 0;
}

static __always_inline int fd_install_state(
    __u32 pid,
    __u32 fd,
    const struct actrail_fd_state *source,
    __u32 flags
) {
    struct actrail_fd_state state;
    struct actrail_fd_key key;
    __u32 slot = 0;

    if (!fd_object_lookup(pid, source->generation)) {
        return 0;
    }
    if (!fd_process_active_count_acquire(pid)) {
        return 0;
    }
    if (!fd_slot_allocate(pid, fd, source->generation, &slot)) {
        fd_process_active_count_release(pid);
        return 0;
    }
    state = *source;
    state.flags = flags;
    state.index_slot = slot;
    key.pid = pid;
    key.fd = fd;
    if (bpf_map_update_elem(&fd_table, &key, &state, BPF_NOEXIST) == 0) {
        return 1;
    }
    fd_slot_release(pid, slot, fd, state.generation);
    fd_process_active_count_release(pid);
    return 0;
}

static __always_inline int fd_register(const struct actrail_fd_registration *registration) {
    struct actrail_fd_state state = {};
    struct actrail_fd_state *existing;
    struct actrail_fd_object_state *existing_object;
    __u64 file_identity = 0;
    __u64 observed_identity = 0;
    __u64 generation;
    __u32 descriptor_flags = registration->flags;
    __u32 pid = registration->pid;
    __u32 fd = registration->fd;
    int close_on_exec;

    if (!pid || registration->category == ACTRAIL_FD_CATEGORY_NONE
        || !fd_category_enabled(registration->category)) {
        return 0;
    }
    file_identity = fd_kernel_file_identity(fd);
    if (file_identity <= ACTRAIL_FD_FILE_IDENTITY_READ_FAILED) {
        return 0;
    }
    close_on_exec = fd_kernel_cloexec(fd);
    if (close_on_exec >= 0) {
        descriptor_flags = close_on_exec ? ACTRAIL_FD_FLAG_CLOEXEC : 0;
    }
    existing = fd_lookup(pid, fd);
    if (existing) {
        existing_object = fd_object_lookup(pid, existing->generation);
        if (existing_object && existing_object->file_identity == file_identity) {
            if (close_on_exec >= 0) {
                existing->flags = descriptor_flags;
            }
            return 1;
        }
        __u64 stale_generation = existing->generation;

        fd_release(
            pid,
            fd,
            stale_generation,
            registration->trace_id,
            registration->program_ctx
        );
        if (fd_lookup(pid, fd)) {
            return 0;
        }
    }
    generation = fd_generation_next();
    state.category = registration->category;
    state.generation = generation;
    if (!fd_object_create(
            pid,
            generation,
            registration->category,
            registration->trace_id,
            file_identity)) {
        return 0;
    }
    if (!fd_install_state(pid, fd, &state, descriptor_flags)) {
        struct actrail_fd_object_key object_key = { .pid = pid, .generation = generation };
        bpf_map_delete_elem(&fd_objects, &object_key);
        return 0;
    }
    observed_identity = fd_kernel_file_identity(fd);
    if (observed_identity != ACTRAIL_FD_FILE_IDENTITY_READ_FAILED
        && observed_identity != file_identity) {
        fd_release(
            pid,
            fd,
            generation,
            registration->trace_id,
            registration->program_ctx
        );
        return 0;
    }
    return 1;
}

static __always_inline int fd_snapshot(
    __u32 pid,
    __u32 fd,
    struct actrail_fd_state *state_snapshot,
    struct actrail_fd_object_snapshot *object_snapshot
) {
    struct actrail_fd_state *state = fd_lookup(pid, fd);

    if (!state) {
        return 0;
    }
    *state_snapshot = *state;
    return fd_object_snapshot(pid, state_snapshot->generation, object_snapshot);
}

static __always_inline void fd_update_endpoint_expected(
    __u32 pid,
    __u32 fd,
    __u64 expected_generation,
    const struct actrail_endpoint *remote
) {
    struct actrail_fd_state *state = fd_lookup(pid, fd);
    struct actrail_fd_object_state *object;

    if (!state || state->category != ACTRAIL_FD_CATEGORY_NET
        || (expected_generation && state->generation != expected_generation)) {
        return;
    }
    object = fd_object_lookup(pid, state->generation);
    if (object) {
        object->remote = *remote;
    }
}

static __always_inline void fd_update_endpoint(
    __u32 pid,
    __u32 fd,
    const struct actrail_endpoint *remote
) {
    fd_update_endpoint_expected(pid, fd, 0, remote);
}

#endif
