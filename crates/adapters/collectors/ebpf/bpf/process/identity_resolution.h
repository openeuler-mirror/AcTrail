#ifndef ACTRAIL_PROCESS_IDENTITY_RESOLUTION_H
#define ACTRAIL_PROCESS_IDENTITY_RESOLUTION_H

#include "../runtime/process_generation.h"

struct actrail_process_identity_resolution_key {
    __u64 start_time_ticks;
    __u32 observer_namespace_tgid;
    __u32 reserved;
};

struct actrail_process_identity_resolution {
    __u64 start_boottime_ns;
    __u32 kernel_tgid;
    __u32 reserved;
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(map_flags, BPF_F_NO_PREALLOC);
    __uint(max_entries, 1);
    __type(key, struct actrail_process_identity_resolution_key);
    __type(value, struct actrail_process_identity_resolution);
} process_identity_resolutions SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} process_identity_resolution_tick_ns SEC(".maps");

SEC("iter/task")
int resolve_process_identities(struct bpf_iter__task *ctx) {
    const __u32 config_key = 0;
    struct task_struct *task = ctx->task;
    struct actrail_process_identity_resolution_key key = {};
    struct actrail_process_identity_resolution *resolution;
    __u64 *tick_ns;
    __u64 start_boottime_ns = 0;
    __u32 kernel_pid = 0;
    __u32 kernel_tgid = 0;

    if (!task ||
        ACTRAIL_CORE_READ(&kernel_pid, task, pid) != 0 ||
        ACTRAIL_CORE_READ(&kernel_tgid, task, tgid) != 0 ||
        !kernel_tgid || kernel_pid != kernel_tgid ||
        ACTRAIL_CORE_READ(&start_boottime_ns, task, start_boottime) != 0 ||
        !start_boottime_ns) {
        return 0;
    }
    tick_ns = bpf_map_lookup_elem(
        &process_identity_resolution_tick_ns,
        &config_key
    );
    if (!tick_ns || !*tick_ns) {
        return 0;
    }
    key.start_time_ticks = start_boottime_ns / *tick_ns;
    key.observer_namespace_tgid = observer_tgid_for_task(task);
    if (!key.start_time_ticks || !key.observer_namespace_tgid) {
        return 0;
    }
    resolution = bpf_map_lookup_elem(&process_identity_resolutions, &key);
    if (!resolution) {
        return 0;
    }
    resolution->start_boottime_ns = start_boottime_ns;
    resolution->kernel_tgid = kernel_tgid;
    return 0;
}

#endif
