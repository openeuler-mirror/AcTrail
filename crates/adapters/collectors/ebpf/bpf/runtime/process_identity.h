#ifndef ACTRAIL_RUNTIME_PROCESS_IDENTITY_H
#define ACTRAIL_RUNTIME_PROCESS_IDENTITY_H

#include "../common/kernel_types.h"

#define ACTRAIL_MAX_PID_NAMESPACE_LEVEL 32

struct actrail_observer_pid_namespace {
    __u64 dev;
    __u64 ino;
    __u64 level_plus_one;
};

enum actrail_observer_pid_diagnostic_counter {
    ACTRAIL_OBSERVER_PID_FAST_PATH = 0,
    ACTRAIL_OBSERVER_PID_LEVEL_DISCOVERY = 1,
    ACTRAIL_OBSERVER_PID_LEVEL_MISMATCH = 2,
    ACTRAIL_OBSERVER_PID_RESOLUTION_FAIL = 3,
    ACTRAIL_OBSERVER_PID_INDEX_PUBLISH_FAIL = 4,
    ACTRAIL_OBSERVER_PID_DIAG_COUNTER_COUNT = 5,
};

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_observer_pid_namespace);
} observer_pid_namespace SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, ACTRAIL_OBSERVER_PID_DIAG_COUNTER_COUNT);
    __type(key, __u32);
    __type(value, __u64);
} observer_pid_diagnostics SEC(".maps");

static __always_inline void observer_pid_diag_inc(__u32 counter_id) {
    __u64 *counter = bpf_map_lookup_elem(&observer_pid_diagnostics, &counter_id);

    if (counter_id == ACTRAIL_OBSERVER_PID_FAST_PATH) {
        return;
    }
    if (counter) {
        __sync_fetch_and_add(counter, 1);
    }
}

static __always_inline __u64 current_kernel_pid_tgid(void) {
    return bpf_get_current_pid_tgid();
}

static __always_inline __u32 current_kernel_tgid(void) {
    return current_kernel_pid_tgid() >> 32;
}

static __always_inline __u64 current_pid_tgid(void) {
    return current_kernel_pid_tgid();
}

static __always_inline __u32 current_tgid(void) {
    return current_pid_tgid() >> 32;
}

static __always_inline int observer_pid_at_level(
    struct pid *pid,
    __u32 pid_level,
    __u32 observer_level,
    __u64 observer_inode,
    __u32 *observer_pid
) {
    struct upid *numbers;
    struct upid candidate = {};
    struct pid_namespace *namespace = 0;
    unsigned int namespace_inode = 0;

    if (!pid || observer_level > pid_level || observer_level > ACTRAIL_MAX_PID_NAMESPACE_LEVEL) {
        return 0;
    }
    numbers = __builtin_preserve_access_index(&pid->numbers[0]);
    if (bpf_probe_read_kernel(&candidate, sizeof(candidate), numbers + observer_level) != 0 ||
        candidate.nr <= 0 || !candidate.ns) {
        return 0;
    }
    namespace = candidate.ns;
    if (ACTRAIL_CORE_READ(&namespace_inode, namespace, ns.inum) != 0 ||
        (__u64)namespace_inode != observer_inode) {
        return 0;
    }
    *observer_pid = (__u32)candidate.nr;
    return 1;
}

static __always_inline __u32 observer_tgid_for_task(struct task_struct *task) {
    const __u32 config_key = 0;
    struct actrail_observer_pid_namespace *observer =
        bpf_map_lookup_elem(&observer_pid_namespace, &config_key);
    struct task_struct *leader = 0;
    struct pid *thread_pid = 0;
    unsigned int pid_level = 0;
    __u64 cached_level_plus_one;
    __u32 observer_pid = 0;

    if (!task || !observer || !observer->ino ||
        ACTRAIL_CORE_READ(&leader, task, group_leader) != 0 || !leader ||
        ACTRAIL_CORE_READ(&thread_pid, leader, thread_pid) != 0 || !thread_pid ||
        ACTRAIL_CORE_READ(&pid_level, thread_pid, level) != 0 ||
        pid_level > ACTRAIL_MAX_PID_NAMESPACE_LEVEL) {
        observer_pid_diag_inc(ACTRAIL_OBSERVER_PID_RESOLUTION_FAIL);
        return 0;
    }

    cached_level_plus_one = observer->level_plus_one;
    if (cached_level_plus_one) {
        __u32 cached_level = (__u32)(cached_level_plus_one - 1);

        if (observer_pid_at_level(
                thread_pid,
                pid_level,
                cached_level,
                observer->ino,
                &observer_pid)) {
            observer_pid_diag_inc(ACTRAIL_OBSERVER_PID_FAST_PATH);
            return observer_pid;
        }
        observer_pid_diag_inc(ACTRAIL_OBSERVER_PID_LEVEL_MISMATCH);
        observer_pid_diag_inc(ACTRAIL_OBSERVER_PID_RESOLUTION_FAIL);
        return 0;
    }

#pragma unroll
    for (__u32 level = 0; level <= ACTRAIL_MAX_PID_NAMESPACE_LEVEL; level++) {
        if (level > pid_level) {
            break;
        }
        if (observer_pid_at_level(
                thread_pid,
                pid_level,
                level,
                observer->ino,
                &observer_pid)) {
            __sync_val_compare_and_swap(&observer->level_plus_one, 0, level + 1);
            observer_pid_diag_inc(ACTRAIL_OBSERVER_PID_LEVEL_DISCOVERY);
            return observer_pid;
        }
    }

    observer_pid_diag_inc(ACTRAIL_OBSERVER_PID_RESOLUTION_FAIL);
    return 0;
}

static __always_inline int current_process_group_dead(void) {
    struct task_struct *task = actrail_bpf_get_current_task();
    struct signal_struct *signal = 0;
    int live_threads = -1;

    if (!task || ACTRAIL_CORE_READ(&signal, task, signal) != 0 || !signal ||
        ACTRAIL_CORE_READ(&live_threads, signal, live.counter) != 0) {
        return 0;
    }
    return live_threads == 0;
}

#endif
