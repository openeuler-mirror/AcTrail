/* Shared domain definitions; BPF entry programs reside in units/. */
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

#endif
