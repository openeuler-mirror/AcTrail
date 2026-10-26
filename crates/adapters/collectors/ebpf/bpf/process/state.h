#ifndef ACTRAIL_PROCESS_STATE_H
#define ACTRAIL_PROCESS_STATE_H

#include "../runtime/trace_membership.h"

struct actrail_observer_fork_binding {
    struct actrail_fork_trace_binding binding;
    __u32 kernel_tgid;
    __u32 reserved;
};

struct actrail_pending_exit_op {
    __s32 code;
    __u32 group_exit;
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_observer_fork_binding);
} observer_fork_trace_bindings SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_exit_op);
} pending_exit_ops SEC(".maps");

#endif
