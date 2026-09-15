#ifndef ACTRAIL_RUNTIME_TRACE_MEMBERSHIP_H
#define ACTRAIL_RUNTIME_TRACE_MEMBERSHIP_H

#include "process_identity.h"

enum actrail_trace_lookup_flag {
    ACTRAIL_TRACE_LOOKUP_FLAG_HOST_FALLBACK = 1,
    ACTRAIL_TRACE_LOOKUP_FLAG_CONTEXT_PID_FALLBACK = 2,
};

struct actrail_pid_namespace {
    __u64 dev;
    __u64 ino;
};

struct actrail_fork_trace_binding {
    __u64 trace_id;
    __u64 parent_generation;
    __u64 child_generation;
    __u32 parent_pid;
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} tracked_traces SEC(".maps");

enum actrail_process_observation_depth {
    ACTRAIL_OBSERVATION_DEPTH_UNLIMITED = -1,
    ACTRAIL_OBSERVATION_DEPTH_LIFECYCLE_ONLY = -2,
};

struct actrail_process_observation_scope {
    __u64 start_boottime_ns;
    __s32 remaining_depth;
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_process_observation_scope);
} process_observation_depths SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pid_namespace);
} trace_pid_namespaces SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_fork_trace_binding);
} fork_trace_bindings SEC(".maps");

static __always_inline __u64 *lookup_current_trace(
    __u32 *tgid,
    __u32 *tid,
    __u32 *flags
) {
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    __u32 kernel_tgid = kernel_pid_tgid >> 32;
    __u64 *trace_id = 0;

    *flags = 0;
    if (kernel_pid_tgid) {
        trace_id = bpf_map_lookup_elem(&tracked_traces, &kernel_tgid);
        if (trace_id) {
            *tgid = kernel_tgid;
            *tid = (__u32)kernel_pid_tgid;
            return trace_id;
        }
    }

    if (kernel_tgid) {
        struct actrail_fork_trace_binding *binding =
            bpf_map_lookup_elem(&fork_trace_bindings, &kernel_tgid);
        if (binding) {
            *tgid = kernel_tgid;
            *tid = (__u32)kernel_pid_tgid;
            *flags = ACTRAIL_TRACE_LOOKUP_FLAG_HOST_FALLBACK;
            return &binding->trace_id;
        }
    }

    *tgid = kernel_tgid;
    *tid = (__u32)kernel_pid_tgid;
    return 0;
}

static __always_inline struct actrail_process_observation_scope *
lookup_process_observation_scope(__u32 kernel_tgid) {
    struct actrail_process_observation_scope *scope =
        bpf_map_lookup_elem(&process_observation_depths, &kernel_tgid);
    struct actrail_process_identity *identity;

    if (!scope) {
        return 0;
    }
    identity = lookup_process_identity(kernel_tgid);
    if (!identity || !identity->start_boottime_ns ||
        identity->start_boottime_ns != scope->start_boottime_ns) {
        return 0;
    }
    return scope;
}

static __always_inline int process_observation_is_detailed(__u32 kernel_tgid) {
    struct actrail_process_observation_scope *scope =
        lookup_process_observation_scope(kernel_tgid);

    return !scope ||
        scope->remaining_depth != ACTRAIL_OBSERVATION_DEPTH_LIFECYCLE_ONLY;
}

static __always_inline int process_observation_child_is_detailed(__u32 parent_kernel_tgid) {
    struct actrail_process_observation_scope *scope =
        lookup_process_observation_scope(parent_kernel_tgid);

    return !scope || scope->remaining_depth > 0;
}

static __always_inline __u64 *lookup_current_detailed_trace(
    __u32 *tgid,
    __u32 *tid,
    __u32 *flags
) {
    __u64 *trace_id = lookup_current_trace(tgid, tid, flags);

    if (!trace_id || !process_observation_is_detailed(current_kernel_tgid())) {
        return 0;
    }
    return trace_id;
}

static __always_inline __u64 *lookup_trace_for_context_pid(
    __u32 context_pid,
    __u32 *tgid,
    __u32 *tid,
    __u32 *flags
) {
    __u64 *trace_id = lookup_current_trace(tgid, tid, flags);

    if (trace_id || !context_pid) {
        return trace_id;
    }

    trace_id = bpf_map_lookup_elem(&tracked_traces, &context_pid);
    if (trace_id) {
        *tgid = context_pid;
        *tid = context_pid;
        *flags = ACTRAIL_TRACE_LOOKUP_FLAG_HOST_FALLBACK |
            ACTRAIL_TRACE_LOOKUP_FLAG_CONTEXT_PID_FALLBACK;
    }
    return trace_id;
}

#endif
