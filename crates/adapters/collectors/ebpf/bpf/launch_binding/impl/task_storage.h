#ifndef ACTRAIL_LAUNCH_BINDING_TASK_STORAGE_H
#define ACTRAIL_LAUNCH_BINDING_TASK_STORAGE_H

#define ACTRAIL_BPF_MAP_TYPE_TASK_STORAGE 29
#define ACTRAIL_BPF_FUNC_TASK_STORAGE_GET 156
#define ACTRAIL_BPF_FUNC_TASK_STORAGE_DELETE 157
#define ACTRAIL_BPF_FUNC_GET_CURRENT_TASK_BTF 158

/* Stable Linux UAPI values are kept local so an explicitly selected
 * task-storage build does not depend on the build host's older headers. */

static void *(*bpf_task_storage_get)(
    void *map,
    struct task_struct *task,
    void *value,
    __u64 flags
) = (void *)ACTRAIL_BPF_FUNC_TASK_STORAGE_GET;
static long (*bpf_task_storage_delete)(
    void *map,
    struct task_struct *task
) = (void *)ACTRAIL_BPF_FUNC_TASK_STORAGE_DELETE;
static struct task_struct *(*bpf_get_current_task_btf)(void) =
    (void *)ACTRAIL_BPF_FUNC_GET_CURRENT_TASK_BTF;

struct {
    __uint(type, ACTRAIL_BPF_MAP_TYPE_TASK_STORAGE);
    __uint(map_flags, BPF_F_NO_PREALLOC);
    __type(key, int);
    __type(value, struct actrail_pending_exec_binding);
} pending_exec_bindings SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(map_flags, BPF_F_NO_PREALLOC);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_pending_exec_binding);
} pending_exec_observer_bindings SEC(".maps");

struct actrail_launch_binding_adapter_lookup {
    struct task_struct *task;
    struct actrail_pending_exec_binding *binding;
    __u32 observer_namespace_tgid;
    __u32 from_observer_fallback;
};

static __always_inline int actrail_launch_binding_adapter_lookup_current(
    __u32 current_kernel_tgid,
    struct actrail_launch_binding_adapter_lookup *lookup
) {
    (void)current_kernel_tgid;
    lookup->task = bpf_get_current_task_btf();
    if (!lookup->task) {
        return 0;
    }
    lookup->binding = bpf_task_storage_get(
        &pending_exec_bindings,
        lookup->task,
        0,
        0
    );
    if (lookup->binding) {
        lookup->observer_namespace_tgid =
            lookup->binding->observer_namespace_tgid;
        return 1;
    }
    lookup->observer_namespace_tgid = observer_tgid_for_task(lookup->task);
    if (!lookup->observer_namespace_tgid) {
        return 0;
    }
    lookup->binding = bpf_map_lookup_elem(
        &pending_exec_observer_bindings,
        &lookup->observer_namespace_tgid
    );
    if (lookup->binding) {
        lookup->from_observer_fallback = 1;
    }
    return lookup->binding != 0;
}

static __always_inline int actrail_launch_binding_adapter_match_current(
    const struct actrail_launch_binding_adapter_lookup *lookup
) {
    const __u32 config_key = 0;
    struct task_struct *task = bpf_get_current_task_btf();
    __u64 *generation_tick_ns;
    __u64 start_boottime_ns = 0;

    generation_tick_ns = bpf_map_lookup_elem(
        &pending_exec_generation_tick_ns,
        &config_key
    );
    if (!task || !lookup->binding || !generation_tick_ns ||
        !*generation_tick_ns ||
        ACTRAIL_CORE_READ(
            &start_boottime_ns,
            task,
            start_boottime) != 0 ||
        !start_boottime_ns) {
        return ACTRAIL_LAUNCH_BINDING_UNAVAILABLE;
    }
    return start_boottime_ns / *generation_tick_ns == lookup->binding->generation
        ? ACTRAIL_LAUNCH_BINDING_MATCH
        : ACTRAIL_LAUNCH_BINDING_STALE;
}

static __always_inline int actrail_launch_binding_adapter_delete(
    const struct actrail_launch_binding_adapter_lookup *lookup
) {
    int primary_deleted;
    int observer_deleted;

    if (lookup->from_observer_fallback) {
        return bpf_map_delete_elem(
            &pending_exec_observer_bindings,
            &lookup->observer_namespace_tgid) == 0
            ? ACTRAIL_LAUNCH_BINDING_DELETED
            : ACTRAIL_LAUNCH_BINDING_DELETE_FAILED;
    }
    primary_deleted =
        bpf_task_storage_delete(&pending_exec_bindings, lookup->task) == 0;
    observer_deleted = bpf_map_delete_elem(
        &pending_exec_observer_bindings,
        &lookup->observer_namespace_tgid) == 0;
    if (!primary_deleted) {
        return ACTRAIL_LAUNCH_BINDING_DELETE_FAILED;
    }
    return observer_deleted
        ? ACTRAIL_LAUNCH_BINDING_DELETED
        : ACTRAIL_LAUNCH_BINDING_DELETED_WITH_CLEANUP_FAILURE;
}

#endif
