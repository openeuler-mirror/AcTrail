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

struct actrail_launch_binding_adapter_lookup {
    struct task_struct *task;
    struct actrail_pending_exec_binding *binding;
};

static __always_inline int actrail_launch_binding_adapter_lookup_current(
    __u32 current_host_tgid,
    struct actrail_launch_binding_adapter_lookup *lookup
) {
    (void)current_host_tgid;
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
    return lookup->binding != 0;
}

static __always_inline int actrail_launch_binding_adapter_match_current(
    const struct actrail_launch_binding_adapter_lookup *lookup
) {
    (void)lookup;
    return ACTRAIL_LAUNCH_BINDING_MATCH;
}

static __always_inline int actrail_launch_binding_adapter_delete(
    const struct actrail_launch_binding_adapter_lookup *lookup
) {
    return bpf_task_storage_delete(&pending_exec_bindings, lookup->task) == 0
        ? ACTRAIL_LAUNCH_BINDING_DELETED
        : ACTRAIL_LAUNCH_BINDING_DELETE_FAILED;
}

#endif
