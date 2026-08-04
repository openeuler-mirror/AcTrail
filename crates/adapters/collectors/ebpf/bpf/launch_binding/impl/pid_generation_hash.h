#ifndef ACTRAIL_LAUNCH_BINDING_PID_GENERATION_HASH_H
#define ACTRAIL_LAUNCH_BINDING_PID_GENERATION_HASH_H

static struct task_struct *(*bpf_get_current_task)(void) =
    (void *)BPF_FUNC_get_current_task;

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(map_flags, BPF_F_NO_PREALLOC);
    __uint(max_entries, 1);
    __type(key, struct actrail_launch_binding_key);
    __type(value, struct actrail_pending_exec_binding);
} pending_exec_bindings SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(map_flags, BPF_F_NO_PREALLOC);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_launch_binding_key);
} pending_exec_pid_index SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} pending_exec_generation_tick_ns SEC(".maps");

struct actrail_launch_binding_adapter_lookup {
    struct actrail_launch_binding_key key;
    struct actrail_pending_exec_binding *binding;
};

static __always_inline int actrail_launch_binding_adapter_lookup_current(
    __u32 current_host_tgid,
    struct actrail_launch_binding_adapter_lookup *lookup
) {
    struct actrail_launch_binding_key *key;

    key = bpf_map_lookup_elem(&pending_exec_pid_index, &current_host_tgid);
    if (!key) {
        return 0;
    }
    __builtin_memcpy(&lookup->key, key, sizeof(lookup->key));
    lookup->binding = bpf_map_lookup_elem(&pending_exec_bindings, &lookup->key);
    return lookup->binding != 0;
}

static __always_inline int actrail_launch_binding_adapter_match_current(
    const struct actrail_launch_binding_adapter_lookup *lookup
) {
    struct task_struct *task = (struct task_struct *)bpf_get_current_task();
    __u64 start_boottime_ns = 0;
    __u64 *generation_tick_ns;
    __u64 current_generation;
    __u32 config_key = 0;

    if (!lookup->binding ||
        lookup->binding->trace_id != lookup->key.trace_id ||
        lookup->binding->generation != lookup->key.generation) {
        return ACTRAIL_LAUNCH_BINDING_STALE;
    }
    generation_tick_ns = bpf_map_lookup_elem(
        &pending_exec_generation_tick_ns,
        &config_key
    );
    if (!task || !generation_tick_ns || !*generation_tick_ns ||
        ACTRAIL_CORE_READ(&start_boottime_ns, task, start_boottime) != 0 ||
        !start_boottime_ns) {
        return ACTRAIL_LAUNCH_BINDING_UNAVAILABLE;
    }
    current_generation = start_boottime_ns / *generation_tick_ns;
    return current_generation == lookup->key.generation
        ? ACTRAIL_LAUNCH_BINDING_MATCH
        : ACTRAIL_LAUNCH_BINDING_STALE;
}

static __always_inline int actrail_launch_binding_adapter_delete(
    const struct actrail_launch_binding_adapter_lookup *lookup
) {
    if (bpf_map_delete_elem(&pending_exec_bindings, &lookup->key) != 0) {
        return ACTRAIL_LAUNCH_BINDING_DELETE_FAILED;
    }
    if (bpf_map_delete_elem(
            &pending_exec_pid_index,
            &lookup->key.host_pid) != 0) {
        return ACTRAIL_LAUNCH_BINDING_DELETED_WITH_CLEANUP_FAILURE;
    }
    return ACTRAIL_LAUNCH_BINDING_DELETED;
}

#endif
