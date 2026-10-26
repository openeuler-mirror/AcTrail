#ifndef ACTRAIL_FD_SWEEP_H
#define ACTRAIL_FD_SWEEP_H

#include "index.h"
#include "suppressed.h"

static __always_inline int fd_fork_object_acquire(
    __u32 child_pid,
    __u64 generation,
    const struct actrail_fd_object_snapshot *parent_object
) {
    struct actrail_fd_object_key key = { .pid = child_pid, .generation = generation };
    struct actrail_fd_object_state *existing = bpf_map_lookup_elem(&fd_objects, &key);
    struct actrail_fd_object_state object = {
        .refcount = 1,
        .category = parent_object->category,
        .trace_id = parent_object->trace_id,
        .remote = parent_object->remote,
    };

    if (existing) {
        if (existing->category != parent_object->category
            || existing->trace_id != parent_object->trace_id) {
            return 0;
        }
        __sync_fetch_and_add(&existing->refcount, 1);
        return 1;
    }
    return bpf_map_update_elem(&fd_objects, &key, &object, BPF_NOEXIST) == 0;
}

static __always_inline void fd_fork_object_release(__u32 child_pid, __u64 generation) {
    struct actrail_fd_object_key key = { .pid = child_pid, .generation = generation };
    struct actrail_fd_object_state *object = bpf_map_lookup_elem(&fd_objects, &key);

    if (!object || !object->refcount) {
        return;
    }
    if (object->refcount == 1) {
        bpf_map_delete_elem(&fd_objects, &key);
    } else {
        __sync_fetch_and_add(&object->refcount, (__u32)-1);
    }
}

static __noinline int fd_fork_seed_slot(
    __u32 parent_pid,
    __u32 child_pid,
    __u32 slot_index
) {
    struct actrail_fd_index_slot_key parent_slot_key = {
        .pid = parent_pid,
        .slot = slot_index,
    };
    struct actrail_fd_index_slot_key child_slot_key = {
        .pid = child_pid,
        .slot = slot_index,
    };
    struct actrail_fd_index_slot *parent_slot;
    struct actrail_fd_index_slot child_slot;
    struct actrail_fd_key parent_fd_key;
    struct actrail_fd_key child_fd_key;
    struct actrail_fd_state *parent_state;
    struct actrail_fd_state child_state;
    struct actrail_fd_object_snapshot parent_object = {};
    __u64 file_identity;
    __u32 *child_active_count;

    parent_slot = bpf_map_lookup_elem(&fd_index_slots, &parent_slot_key);
    if (!parent_slot) {
        return 0;
    }
    child_slot = *parent_slot;
    parent_fd_key.pid = parent_pid;
    parent_fd_key.fd = child_slot.fd;
    parent_state = bpf_map_lookup_elem(&fd_table, &parent_fd_key);
    if (!parent_state || parent_state->generation != child_slot.generation
        || parent_state->index_slot != slot_index) {
        return 0;
    }
    child_state = *parent_state;
    if (child_state.category == ACTRAIL_FD_CATEGORY_NET
        && !fd_object_snapshot(parent_pid, child_state.generation, &parent_object)) {
        return 0;
    }
    file_identity = child_state.file_identity;
    parent_slot = bpf_map_lookup_elem(&fd_index_slots, &parent_slot_key);
    parent_state = bpf_map_lookup_elem(&fd_table, &parent_fd_key);
    if (!parent_slot || parent_slot->fd != child_slot.fd
        || parent_slot->generation != child_slot.generation || !parent_state
        || parent_state->generation != child_slot.generation
        || parent_state->index_slot != slot_index
        || parent_state->file_identity != file_identity) {
        return 0;
    }

    if (bpf_map_update_elem(&fd_index_slots, &child_slot_key, &child_slot, BPF_NOEXIST) != 0) {
        return 0;
    }
    child_fd_key.pid = child_pid;
    child_fd_key.fd = child_slot.fd;
    if (bpf_map_update_elem(&fd_table, &child_fd_key, &child_state, BPF_NOEXIST) != 0) {
        bpf_map_delete_elem(&fd_index_slots, &child_slot_key);
        return 0;
    }
    if (child_state.category == ACTRAIL_FD_CATEGORY_NET && !fd_fork_object_acquire(
            child_pid,
            child_state.generation,
            &parent_object)) {
        bpf_map_delete_elem(&fd_table, &child_fd_key);
        bpf_map_delete_elem(&fd_index_slots, &child_slot_key);
        return 0;
    }
    child_active_count = bpf_map_lookup_elem(&fd_process_active_counts, &child_pid);
    if (!child_active_count) {
        if (child_state.category == ACTRAIL_FD_CATEGORY_NET) {
            fd_fork_object_release(child_pid, child_state.generation);
        }
        bpf_map_delete_elem(&fd_table, &child_fd_key);
        bpf_map_delete_elem(&fd_index_slots, &child_slot_key);
        return 0;
    }
    *child_active_count += 1;
    return 1;
}

#ifdef ACTRAIL_BPF_LOOP
struct actrail_fd_fork_seed_ctx {
    __u32 parent_pid;
    __u32 child_pid;
    __u32 max_slots;
    __u32 reserved;
};

static long fd_fork_seed_step(__u32 slot, void *opaque) {
    struct actrail_fd_fork_seed_ctx *ctx = opaque;

    if (slot >= ctx->max_slots) {
        return 1;
    }
    fd_fork_seed_slot(ctx->parent_pid, ctx->child_pid, slot);
    return 0;
}
#endif

static __always_inline void fd_fork_seed(__u32 parent_pid, __u32 child_pid) {
    __u32 max_slots = fd_slot_limit();
    __u32 active_count = fd_process_active_count(parent_pid);

    if (!parent_pid || !child_pid || !active_count || !fd_tracking_enabled() || !max_slots
        || max_slots > ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES) {
        return;
    }
    if (!fd_process_active_count_seed_empty(child_pid)) {
        return;
    }
#ifdef ACTRAIL_BPF_LOOP
    {
        struct actrail_fd_fork_seed_ctx ctx = {
            .parent_pid = parent_pid,
            .child_pid = child_pid,
            .max_slots = max_slots,
        };

        bpf_loop(ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES, fd_fork_seed_step, &ctx, 0);
    }
#else
    __u32 slot;

#pragma clang loop unroll(disable)
    for (slot = 0; slot < ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES; slot++) {
        if (slot >= max_slots) {
            break;
        }
        fd_fork_seed_slot(parent_pid, child_pid, slot);
    }
#endif
    {
        __u32 *child_active_count =
            bpf_map_lookup_elem(&fd_process_active_counts, &child_pid);

        if (child_active_count && !*child_active_count) {
            bpf_map_delete_elem(&fd_process_active_counts, &child_pid);
        }
    }
}

static __noinline void fd_exec_cleanup_slot(
    __u32 pid,
    __u32 slot_index,
    __u64 trace_id,
    void *ctx
) {
    struct actrail_fd_index_slot_key slot_key = { .pid = pid, .slot = slot_index };
    struct actrail_fd_index_slot *slot = bpf_map_lookup_elem(&fd_index_slots, &slot_key);
    struct actrail_fd_state *state;
    __u64 file_identity = 0;

    if (!slot) {
        return;
    }
    state = fd_lookup(pid, slot->fd);
    if (state && state->generation == slot->generation
        && state->index_slot == slot_index) {
        __u32 fd = slot->fd;
        __u64 generation = slot->generation;

        file_identity = fd_kernel_file_identity(fd);
        if (file_identity == ACTRAIL_FD_FILE_IDENTITY_READ_FAILED) {
            return;
        }
        if (!file_identity || state->file_identity != file_identity) {
            fd_release(pid, fd, generation, trace_id, ctx);
        }
    }
}

#ifdef ACTRAIL_BPF_LOOP
struct actrail_fd_exec_cleanup_ctx {
    __u32 pid;
    __u32 max_slots;
    __u64 trace_id;
    void *program_ctx;
};

static long fd_process_exec_cleanup_step(__u32 slot, void *opaque) {
    struct actrail_fd_exec_cleanup_ctx *ctx = opaque;

    if (slot >= ctx->max_slots) {
        return 1;
    }
    fd_exec_cleanup_slot(ctx->pid, slot, ctx->trace_id, ctx->program_ctx);
    return 0;
}
#endif

static __always_inline void fd_process_exec_cleanup(
    __u32 pid,
    __u64 trace_id,
    void *ctx
) {
    __u32 max_slots = fd_slot_limit();

    if (!pid || !fd_process_active_count(pid) || !fd_tracking_enabled() || !max_slots
        || max_slots > ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES) {
        return;
    }
#ifdef ACTRAIL_BPF_LOOP
    {
        struct actrail_fd_exec_cleanup_ctx loop_ctx = {
            .pid = pid,
            .max_slots = max_slots,
            .trace_id = trace_id,
            .program_ctx = ctx,
        };

        bpf_loop(
            ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES,
            fd_process_exec_cleanup_step,
            &loop_ctx,
            0
        );
    }
#else
    __u32 slot;

#pragma clang loop unroll(disable)
    for (slot = 0; slot < ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES; slot++) {
        if (slot >= max_slots) {
            break;
        }
        fd_exec_cleanup_slot(pid, slot, trace_id, ctx);
    }
#endif
}

static __noinline void fd_exit_cleanup_slot(
    __u32 pid,
    __u32 slot_index,
    __u64 trace_id,
    void *ctx
) {
    struct actrail_fd_index_slot_key slot_key = { .pid = pid, .slot = slot_index };
    struct actrail_fd_index_slot *slot = bpf_map_lookup_elem(&fd_index_slots, &slot_key);

    if (slot) {
        __u32 fd = slot->fd;
        __u64 generation = slot->generation;
        fd_release(pid, fd, generation, trace_id, ctx);
    }
}

#ifdef ACTRAIL_BPF_LOOP
struct actrail_fd_exit_cleanup_ctx {
    __u32 pid;
    __u32 max_slots;
    __u64 trace_id;
    void *program_ctx;
};

static long fd_process_exit_cleanup_step(__u32 slot, void *opaque) {
    struct actrail_fd_exit_cleanup_ctx *ctx = opaque;

    if (slot >= ctx->max_slots) {
        return 1;
    }
    fd_exit_cleanup_slot(ctx->pid, slot, ctx->trace_id, ctx->program_ctx);
    return 0;
}
#endif

static __always_inline void fd_process_exit_cleanup(
    __u32 pid,
    __u64 trace_id,
    void *ctx
) {
    __u32 max_slots = fd_slot_limit();

    if (!pid) {
        return;
    }
    socket_payload_release_process(pid);
    if (bpf_map_delete_elem(&fd_process_active_counts, &pid) != 0) {
        return;
    }
    if (!max_slots || max_slots > ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES) {
        return;
    }
#ifdef ACTRAIL_BPF_LOOP
    {
        struct actrail_fd_exit_cleanup_ctx loop_ctx = {
            .pid = pid,
            .max_slots = max_slots,
            .trace_id = trace_id,
            .program_ctx = ctx,
        };

        bpf_loop(
            ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES,
            fd_process_exit_cleanup_step,
            &loop_ctx,
            0
        );
    }
#else
    __u32 slot;

#pragma clang loop unroll(disable)
    for (slot = 0; slot < ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES; slot++) {
        if (slot >= max_slots) {
            break;
        }
        fd_exit_cleanup_slot(pid, slot, trace_id, ctx);
    }
#endif
}

struct actrail_fd_close_range_sweep {
    __u64 trace_id;
    void *program_ctx;
    __u32 pid;
    __u32 first;
    __u32 last;
    __u32 flags;
};

static __noinline void fd_close_range_slot(
    const struct actrail_fd_close_range_sweep *sweep,
    __u32 slot_index
) {
    struct actrail_fd_index_slot_key slot_key = {
        .pid = sweep->pid,
        .slot = slot_index,
    };
    struct actrail_fd_index_slot *slot = bpf_map_lookup_elem(&fd_index_slots, &slot_key);
    struct actrail_fd_state *state;
    __u32 fd;

    if (!slot) {
        return;
    }
    fd = slot->fd;
    if (fd < sweep->first || fd > sweep->last) {
        return;
    }
    state = fd_lookup(sweep->pid, fd);
    if (!state || state->generation != slot->generation || state->index_slot != slot_index) {
        return;
    }
    {
        __u64 generation = slot->generation;
        fd_release(
            sweep->pid,
            fd,
            generation,
            sweep->trace_id,
            sweep->program_ctx
        );
    }
}

#ifdef ACTRAIL_BPF_LOOP
struct actrail_fd_close_range_loop_ctx {
    struct actrail_fd_close_range_sweep sweep;
    __u32 max_slots;
};

static long fd_close_range_dispatch_step(__u32 slot, void *opaque) {
    struct actrail_fd_close_range_loop_ctx *ctx = opaque;

    if (slot >= ctx->max_slots) {
        return 1;
    }
    fd_close_range_slot(&ctx->sweep, slot);
    return 0;
}
#endif

static __always_inline void fd_close_range_dispatch(
    __u32 pid,
    __u32 first,
    __u32 last,
    __u32 flags,
    __u64 trace_id,
    void *ctx
) {
    __u32 max_slots = fd_slot_limit();

    if (!max_slots || max_slots > ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES) {
        return;
    }
#ifdef ACTRAIL_BPF_LOOP
    {
        struct actrail_fd_close_range_loop_ctx loop_ctx = {
            .sweep = {
                .trace_id = trace_id,
                .program_ctx = ctx,
                .pid = pid,
                .first = first,
                .last = last,
                .flags = flags,
            },
            .max_slots = max_slots,
        };

        bpf_loop(
            ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES,
            fd_close_range_dispatch_step,
            &loop_ctx,
            0
        );
    }
#else
    __u32 slot;
    struct actrail_fd_close_range_sweep sweep = {
        .trace_id = trace_id,
        .program_ctx = ctx,
        .pid = pid,
        .first = first,
        .last = last,
        .flags = flags,
    };

#pragma clang loop unroll(disable)
    for (slot = 0; slot < ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES; slot++) {
        if (slot >= max_slots) {
            break;
        }
        fd_close_range_slot(&sweep, slot);
    }
#endif
}

#endif
