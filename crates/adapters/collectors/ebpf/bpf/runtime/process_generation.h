#ifndef ACTRAIL_RUNTIME_PROCESS_GENERATION_H
#define ACTRAIL_RUNTIME_PROCESS_GENERATION_H

#include "trace_membership.h"

struct actrail_process_identity {
    __u64 start_boottime_ns;
    union {
        struct {
            __u32 observer_namespace_tgid;
            __u32 exit_claimed;
        };
        __u64 exit_state;
    };
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_process_identity);
} process_identities SEC(".maps");

struct actrail_trace_namespace_thread_identity {
    __u64 trace_id;
    __u64 start_boottime_ns;
    __u64 namespace_pid_tgid;
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(map_flags, BPF_F_NO_PREALLOC);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_trace_namespace_thread_identity);
} trace_namespace_thread_identities SEC(".maps");

static __always_inline struct actrail_process_identity *lookup_process_identity(
    __u32 kernel_tgid
) {
    if (!kernel_tgid) {
        return 0;
    }
    return bpf_map_lookup_elem(&process_identities, &kernel_tgid);
}

static __always_inline int claim_process_exit(
    struct actrail_process_identity *identity
) {
    __u64 observed;

    if (!identity) {
        return 0;
    }
    observed = identity->exit_state;
    if (observed >> 32) {
        return 0;
    }
    return __sync_val_compare_and_swap(
        &identity->exit_state,
        observed,
        observed | (1ULL << 32)
    ) == observed;
}

/*
 * Keep this lookup inlined. Older verifiers can reject nested calls from
 * large payload subprograms while backtracking scalar precision.
 */
static __always_inline __u64 current_process_start_time(__u32 pid) {
    struct actrail_process_identity *identity;

    if (!pid) {
        return 0;
    }
    identity = lookup_process_identity(pid);
    if (identity) {
        return identity->start_boottime_ns;
    }
    struct actrail_fork_trace_binding *binding =
        bpf_map_lookup_elem(&fork_trace_bindings, &pid);
    if (binding) {
        return binding->child_generation;
    }
    return 0;
}

static __always_inline __u64 current_trace_pid_tgid(__u64 trace_id) {
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    __u32 kernel_tgid = kernel_pid_tgid >> 32;
    __u64 generation = current_process_start_time(kernel_tgid);
    struct actrail_trace_namespace_thread_identity *cached;
    struct actrail_pid_namespace *namespace;
    struct actrail_bpf_pidns_info namespace_pid = {};
    struct actrail_trace_namespace_thread_identity identity = {};

    if (!trace_id || !kernel_pid_tgid || !generation) {
        return 0;
    }
    cached = bpf_map_lookup_elem(
        &trace_namespace_thread_identities,
        &kernel_pid_tgid
    );
    if (cached && cached->trace_id == trace_id &&
        cached->start_boottime_ns == generation) {
        return cached->namespace_pid_tgid;
    }
    namespace = bpf_map_lookup_elem(&trace_pid_namespaces, &trace_id);
    if (!namespace ||
        bpf_get_ns_current_pid_tgid(
            namespace->dev,
            namespace->ino,
            &namespace_pid,
            sizeof(namespace_pid)) != 0 ||
        !namespace_pid.tgid || !namespace_pid.pid) {
        return 0;
    }
    identity.trace_id = trace_id;
    identity.start_boottime_ns = generation;
    identity.namespace_pid_tgid =
        ((__u64)namespace_pid.tgid << 32) | namespace_pid.pid;
    if (bpf_map_update_elem(
            &trace_namespace_thread_identities,
            &kernel_pid_tgid,
            &identity,
            BPF_ANY) != 0) {
        return 0;
    }
    return identity.namespace_pid_tgid;
}

static __always_inline void delete_trace_namespace_thread_identity(
    __u64 kernel_pid_tgid
) {
    if (kernel_pid_tgid) {
        bpf_map_delete_elem(
            &trace_namespace_thread_identities,
            &kernel_pid_tgid
        );
    }
}

static __always_inline int set_process_identity(
    __u32 kernel_tgid,
    __u64 start_boottime_ns,
    __u32 observer_namespace_tgid
) {
    struct actrail_process_identity identity = {
        .start_boottime_ns = start_boottime_ns,
        .observer_namespace_tgid = observer_namespace_tgid,
    };

    if (!kernel_tgid || !start_boottime_ns || !observer_namespace_tgid) {
        return -1;
    }
    return bpf_map_update_elem(
        &process_identities,
        &kernel_tgid,
        &identity,
        BPF_ANY
    );
}

static __always_inline void set_process_start_time(__u32 pid, __u64 start_time) {
    struct actrail_process_identity *identity;

    if (!pid || !start_time) {
        return;
    }
    identity = lookup_process_identity(pid);
    if (identity) {
        identity->start_boottime_ns = start_time;
    }
}

static __always_inline void delete_process_start_time(__u32 pid) {
    if (!pid) {
        return;
    }
    bpf_map_delete_elem(&process_identities, &pid);
}

static __always_inline __u64 *lookup_process_start_time(__u32 pid) {
    struct actrail_process_identity *identity = lookup_process_identity(pid);

    return identity ? &identity->start_boottime_ns : 0;
}

#endif
