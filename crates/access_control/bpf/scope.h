#ifndef ACTRAIL_ACCESS_CONTROL_SCOPE_H
#define ACTRAIL_ACCESS_CONTROL_SCOPE_H

#include "abi.h"

/* Pending bindings bootstrap control scope once. Task storage then owns scope
 * across exec and synchronous fork inheritance, independently of observation.
 * Pending layouts follow the collector's launch_binding/binding.h. */
struct ac_observer_namespace {
    __u64 dev;
    __u64 ino;
    __u64 level_plus_one;
};
struct ac_pending_binding {
    __u64 trace_id;
    __u64 generation;
    __u32 observer_namespace_tgid;
    __u32 suppressed_fd_count;
    __u32 counted;
    __u32 reserved;
    __u8 suppressed_fds[512];
};
struct ac_pending_key {
    __u32 observer_namespace_tgid;
    __u32 reserved;
    __u64 generation;
    __u64 trace_id;
};

_Static_assert(sizeof(struct ac_observer_namespace) == 24, "observer namespace ABI");
_Static_assert(sizeof(struct ac_pending_binding) == 544, "pending binding ABI");
_Static_assert(sizeof(struct ac_pending_key) == 24, "pending key ABI");

#define AC_SHARED_MAP(NAME, TYPE, KEY, VALUE) \
    struct { \
        __uint(type, TYPE); \
        __uint(max_entries, 1); \
        __type(key, KEY); \
        __type(value, VALUE); \
    } NAME SEC(".maps")

AC_SHARED_MAP(observer_pid_namespace, BPF_MAP_TYPE_ARRAY, __u32, struct ac_observer_namespace);
AC_SHARED_MAP(pending_exec_generation_tick_ns, BPF_MAP_TYPE_ARRAY, __u32, __u64);
struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __uint(map_flags, BPF_F_MMAPABLE);
    __type(key, __u32);
    __type(value, __u64);
} pending_exec_count SEC(".maps");

#ifdef AC_PENDING_HASH
AC_SHARED_MAP(pending_exec_pid_index, BPF_MAP_TYPE_HASH, __u32, struct ac_pending_key);
AC_SHARED_MAP(pending_exec_bindings, BPF_MAP_TYPE_HASH, struct ac_pending_key, struct ac_pending_binding);
#else
AC_SHARED_MAP(pending_exec_observer_bindings, BPF_MAP_TYPE_HASH, __u32, struct ac_pending_binding);
#endif

struct {
    __uint(type, BPF_MAP_TYPE_TASK_STORAGE);
    __uint(map_flags, BPF_F_NO_PREALLOC);
    __type(key, int);
    __type(value, __u64);
} ac_task_scope SEC(".maps");

static __u64 *(*ac_task_storage_get)(
    void *map, struct task_struct *task, void *value, __u64 flags
) = (void *)156;
static struct task_struct *(*ac_current_task_btf)(void) = (void *)158;

static __always_inline __u32 ac_observer_at_level(
    struct pid *pid, __u32 level, __u64 inode
) {
    struct upid candidate = {};
    unsigned int namespace_inode = 0;
    if (level > 32) return 0;
    struct upid *numbers = __builtin_preserve_access_index(&pid->numbers[0]);
    if (bpf_probe_read_kernel(&candidate, sizeof(candidate), numbers + level) ||
        candidate.nr <= 0 || !candidate.ns ||
        ACTRAIL_CORE_READ(&namespace_inode, candidate.ns, ns.inum) ||
        namespace_inode != inode) return 0;
    return candidate.nr;
}

static __always_inline __u32 ac_observer_tgid(struct task_struct *task) {
    __u32 zero = 0, level = 0;
    struct ac_observer_namespace *observer =
        bpf_map_lookup_elem(&observer_pid_namespace, &zero);
    struct task_struct *leader = 0;
    struct pid *pid = 0;
    if (!observer || !observer->ino ||
        ACTRAIL_CORE_READ(&leader, task, group_leader) || !leader ||
        ACTRAIL_CORE_READ(&pid, leader, thread_pid) || !pid ||
        ACTRAIL_CORE_READ(&level, pid, level) || level > 32) return 0;
    if (observer->level_plus_one) {
        __u64 cached = observer->level_plus_one - 1;
        return cached <= level ? ac_observer_at_level(pid, cached, observer->ino) : 0;
    }
    for (__u32 i = 0; i <= 32; i++) {
        if (i > level) break;
        __u32 observed = ac_observer_at_level(pid, i, observer->ino);
        if (observed) return observed;
    }
    return 0;
}

static __always_inline int ac_current_trace(__u64 *trace) {
    struct task_struct *task = ac_current_task_btf();
    __u64 *stored = ac_task_storage_get(&ac_task_scope, task, 0, 0);
    if (stored) {
        *trace = *stored;
        return *trace ? 1 : -12;
    }
    __u32 zero = 0;
    __u64 *pending_count = bpf_map_lookup_elem(&pending_exec_count, &zero);
    if (!pending_count || !*pending_count) return 0;
    __u32 observer = ac_observer_tgid(task);
    if (!observer) return 0;
    struct ac_pending_binding *binding;
#ifdef AC_PENDING_HASH
    struct ac_pending_key *found = bpf_map_lookup_elem(&pending_exec_pid_index, &observer);
    if (!found) return 0;
    struct ac_pending_key key = *found;
    binding = bpf_map_lookup_elem(&pending_exec_bindings, &key);
    if (!binding || binding->trace_id != key.trace_id ||
        binding->generation != key.generation ||
        binding->observer_namespace_tgid != key.observer_namespace_tgid) return 0;
#else
    binding = bpf_map_lookup_elem(&pending_exec_observer_bindings, &observer);
#endif
    if (!binding || !binding->counted || !binding->trace_id || !binding->generation ||
        binding->observer_namespace_tgid != observer) return 0;
    __u64 *tick_ns = bpf_map_lookup_elem(&pending_exec_generation_tick_ns, &zero);
    __u64 started = 0;
    if (!tick_ns || !*tick_ns ||
        ACTRAIL_CORE_READ(&started, task, start_boottime) || !started ||
        started / *tick_ns != binding->generation) return 0;
    *trace = binding->trace_id;
    stored = ac_task_storage_get(&ac_task_scope, task, trace, 1);
    return stored && *stored == *trace ? 1 : -12;
}

#endif
