#ifndef ACTRAIL_LAUNCH_BINDING_H
#define ACTRAIL_LAUNCH_BINDING_H

#include "../runtime/event_transport.h"

enum actrail_launch_binding_match_result {
    ACTRAIL_LAUNCH_BINDING_UNAVAILABLE = 0,
    ACTRAIL_LAUNCH_BINDING_MATCH = 1,
    ACTRAIL_LAUNCH_BINDING_STALE = 2,
};

enum actrail_launch_binding_delete_result {
    ACTRAIL_LAUNCH_BINDING_DELETE_FAILED = 0,
    ACTRAIL_LAUNCH_BINDING_DELETED = 1,
    ACTRAIL_LAUNCH_BINDING_DELETED_WITH_CLEANUP_FAILURE = 2,
};

enum actrail_launch_binding_failure_status {
    ACTRAIL_LAUNCH_BINDING_IDENTITY_FAILURE = 1,
    ACTRAIL_LAUNCH_BINDING_PROMOTION_FAILURE = 2,
    ACTRAIL_LAUNCH_BINDING_CLEANUP_FAILURE = 3,
};

struct actrail_launch_binding_failure_event {
    __u32 kind;
    __u32 status;
    __u64 trace_id;
} __attribute__((packed));

struct actrail_pending_exec_suppressed_fd {
    __s32 fd;
    __u32 purpose;
};

struct actrail_pending_exec_binding {
    __u64 trace_id;
    __u64 generation;
    __u32 observer_namespace_tgid;
    __u32 suppressed_fd_count;
    __u32 counted;
    __u32 reserved;
    struct actrail_pending_exec_suppressed_fd
        suppressed_fds[ACTRAIL_SUPPRESSED_FD_INDEX_SLOT_MAX];
};

struct actrail_launch_binding_key {
    __u32 observer_namespace_tgid;
    __u32 reserved;
    __u64 generation;
    __u64 trace_id;
};

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(map_flags, BPF_F_MMAPABLE);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} pending_exec_count SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} pending_exec_generation_tick_ns SEC(".maps");

#if defined(ACTRAIL_LAUNCH_BINDING_TASK_STORAGE) && \
    defined(ACTRAIL_LAUNCH_BINDING_PID_GENERATION_HASH)
#error "exactly one launch binding Adapter must be selected"
#elif defined(ACTRAIL_LAUNCH_BINDING_TASK_STORAGE)
#include "impl/task_storage.h"
#elif defined(ACTRAIL_LAUNCH_BINDING_PID_GENERATION_HASH)
#include "impl/pid_generation_hash.h"
#else
#error "a launch binding Adapter must be selected"
#endif

static __always_inline __u64 *actrail_launch_binding_pending_count(void) {
    __u32 key = 0;

    return bpf_map_lookup_elem(&pending_exec_count, &key);
}

static __always_inline void actrail_launch_binding_report_failure(
    void *ctx,
    __u64 trace_id,
    __u32 status
) {
    struct actrail_launch_binding_failure_event *event;

    if (!trace_id) {
        return;
    }
    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        return;
    }
    event->kind = ACTRAIL_LAUNCH_BINDING_FAILURE;
    event->status = status;
    event->trace_id = trace_id;
    actrail_event_submit(ctx, event);
}

static __always_inline void actrail_launch_binding_decrement(__u64 *pending_count) {
    if (pending_count) {
        __sync_fetch_and_add(pending_count, (__u64)-1);
    }
}

static __always_inline int actrail_launch_binding_install_suppressed_fds(
    const struct actrail_pending_exec_binding *binding,
    __u32 pid
) {
    __u32 index;

    if (binding->suppressed_fd_count > ACTRAIL_SUPPRESSED_FD_INDEX_SLOT_MAX) {
        return 0;
    }
#pragma unroll
    for (index = 0; index < ACTRAIL_SUPPRESSED_FD_INDEX_SLOT_MAX; index++) {
        struct actrail_suppressed_fd_value value = {};
        __s32 fd;

        if (index >= binding->suppressed_fd_count) {
            break;
        }
        fd = binding->suppressed_fds[index].fd;
        value.trace_id = binding->trace_id;
        value.purpose = binding->suppressed_fds[index].purpose;
        if (fd < 0 ||
            value.purpose == ACTRAIL_SUPPRESSED_FD_PURPOSE_NONE ||
            !upsert_suppressed_fd_for_generation(
                pid,
                binding->generation,
                (__u32)fd,
                &value)) {
            return 0;
        }
    }
    return 1;
}

static __always_inline void actrail_launch_binding_delete_invalid(
    void *ctx,
    const struct actrail_launch_binding_adapter_lookup *lookup,
    __u64 *pending_count
) {
    __u32 counted = lookup->binding ? lookup->binding->counted : 0;
    __u64 trace_id = lookup->binding ? lookup->binding->trace_id : 0;
    int deleted = actrail_launch_binding_adapter_delete(lookup);

    if (deleted == ACTRAIL_LAUNCH_BINDING_DELETE_FAILED) {
        actrail_launch_binding_report_failure(
            ctx,
            trace_id,
            ACTRAIL_LAUNCH_BINDING_CLEANUP_FAILURE
        );
        return;
    }
    if (deleted == ACTRAIL_LAUNCH_BINDING_DELETED_WITH_CLEANUP_FAILURE) {
        actrail_launch_binding_report_failure(
            ctx,
            trace_id,
            ACTRAIL_LAUNCH_BINDING_CLEANUP_FAILURE
        );
    }
    if (counted) {
        actrail_launch_binding_decrement(pending_count);
    }
}

static __always_inline __u64 actrail_launch_binding_promote_current(
    void *ctx,
    __u32 current_kernel_tgid
) {
    struct actrail_launch_binding_adapter_lookup lookup = {};
    struct actrail_pending_exec_binding *binding;
    __u64 *pending_count = actrail_launch_binding_pending_count();
    __u64 trace_id;
    __u64 generation;
    int deleted;
    int match;

    if (!current_kernel_tgid || !pending_count || *pending_count == 0 ||
        !actrail_launch_binding_adapter_lookup_current(
            current_kernel_tgid,
            &lookup)) {
        return 0;
    }
    binding = lookup.binding;
    if (!binding) {
        return 0;
    }
    if (!binding->counted) {
        actrail_launch_binding_report_failure(
            ctx,
            binding->trace_id,
            ACTRAIL_LAUNCH_BINDING_IDENTITY_FAILURE
        );
        return 0;
    }
    if (!binding->trace_id || !binding->generation ||
        !binding->observer_namespace_tgid) {
        actrail_launch_binding_report_failure(
            ctx,
            binding->trace_id,
            ACTRAIL_LAUNCH_BINDING_IDENTITY_FAILURE
        );
        actrail_launch_binding_delete_invalid(ctx, &lookup, pending_count);
        return 0;
    }
    match = actrail_launch_binding_adapter_match_current(&lookup);
    if (match == ACTRAIL_LAUNCH_BINDING_STALE) {
        actrail_launch_binding_report_failure(
            ctx,
            binding->trace_id,
            ACTRAIL_LAUNCH_BINDING_IDENTITY_FAILURE
        );
        actrail_launch_binding_delete_invalid(ctx, &lookup, pending_count);
        return 0;
    }
    if (match != ACTRAIL_LAUNCH_BINDING_MATCH) {
        actrail_launch_binding_report_failure(
            ctx,
            binding->trace_id,
            ACTRAIL_LAUNCH_BINDING_IDENTITY_FAILURE
        );
        actrail_launch_binding_delete_invalid(ctx, &lookup, pending_count);
        return 0;
    }

    trace_id = binding->trace_id;
    generation = binding->generation;
    if (set_process_identity(
            current_kernel_tgid,
            generation,
            binding->observer_namespace_tgid) != 0) {
        actrail_launch_binding_report_failure(
            ctx,
            trace_id,
            ACTRAIL_LAUNCH_BINDING_PROMOTION_FAILURE
        );
        actrail_launch_binding_delete_invalid(ctx, &lookup, pending_count);
        return 0;
    }
    if (!actrail_launch_binding_install_suppressed_fds(
            binding,
            current_kernel_tgid)) {
        actrail_launch_binding_report_failure(
            ctx,
            trace_id,
            ACTRAIL_LAUNCH_BINDING_PROMOTION_FAILURE
        );
        cleanup_suppressed_fds_for_process(current_kernel_tgid, generation);
        if (bpf_map_delete_elem(&process_identities, &current_kernel_tgid) != 0) {
            event_transport_diag_inc(ACTRAIL_PROCESS_IDENTITY_CLEANUP_FAIL);
        }
        actrail_launch_binding_delete_invalid(ctx, &lookup, pending_count);
        return 0;
    }
    if (bpf_map_update_elem(
            &tracked_traces,
            &current_kernel_tgid,
            &trace_id,
            BPF_ANY) != 0) {
        actrail_launch_binding_report_failure(
            ctx,
            trace_id,
            ACTRAIL_LAUNCH_BINDING_PROMOTION_FAILURE
        );
        cleanup_suppressed_fds_for_process(current_kernel_tgid, generation);
        if (bpf_map_delete_elem(&process_identities, &current_kernel_tgid) != 0) {
            event_transport_diag_inc(ACTRAIL_PROCESS_IDENTITY_CLEANUP_FAIL);
        }
        actrail_launch_binding_delete_invalid(ctx, &lookup, pending_count);
        return 0;
    }
    deleted = actrail_launch_binding_adapter_delete(&lookup);
    if (deleted == ACTRAIL_LAUNCH_BINDING_DELETE_FAILED) {
        actrail_launch_binding_report_failure(
            ctx,
            trace_id,
            ACTRAIL_LAUNCH_BINDING_CLEANUP_FAILURE
        );
        bpf_map_delete_elem(&tracked_traces, &current_kernel_tgid);
        cleanup_suppressed_fds_for_process(current_kernel_tgid, generation);
        if (bpf_map_delete_elem(&process_identities, &current_kernel_tgid) != 0) {
            event_transport_diag_inc(ACTRAIL_PROCESS_IDENTITY_CLEANUP_FAIL);
        }
        return 0;
    }
    if (deleted == ACTRAIL_LAUNCH_BINDING_DELETED_WITH_CLEANUP_FAILURE) {
        actrail_launch_binding_report_failure(
            ctx,
            trace_id,
            ACTRAIL_LAUNCH_BINDING_CLEANUP_FAILURE
        );
    }
    actrail_launch_binding_decrement(pending_count);
    return trace_id;
}

static __always_inline void actrail_launch_binding_cleanup_current(
    void *ctx,
    __u32 current_kernel_tgid
) {
    struct actrail_launch_binding_adapter_lookup lookup = {};
    __u64 *pending_count = actrail_launch_binding_pending_count();
    int match;

    if (!current_kernel_tgid || !pending_count || *pending_count == 0 ||
        !actrail_launch_binding_adapter_lookup_current(
            current_kernel_tgid,
            &lookup) ||
        !lookup.binding) {
        return;
    }
    if (!lookup.binding->trace_id || !lookup.binding->generation) {
        actrail_launch_binding_report_failure(
            ctx,
            lookup.binding->trace_id,
            ACTRAIL_LAUNCH_BINDING_IDENTITY_FAILURE
        );
        actrail_launch_binding_delete_invalid(ctx, &lookup, pending_count);
        return;
    }
    match = actrail_launch_binding_adapter_match_current(&lookup);
    if (match == ACTRAIL_LAUNCH_BINDING_UNAVAILABLE) {
        actrail_launch_binding_report_failure(
            ctx,
            lookup.binding->trace_id,
            ACTRAIL_LAUNCH_BINDING_IDENTITY_FAILURE
        );
        actrail_launch_binding_delete_invalid(ctx, &lookup, pending_count);
        return;
    }
    if (match == ACTRAIL_LAUNCH_BINDING_STALE) {
        actrail_launch_binding_report_failure(
            ctx,
            lookup.binding->trace_id,
            ACTRAIL_LAUNCH_BINDING_IDENTITY_FAILURE
        );
    }
    actrail_launch_binding_delete_invalid(ctx, &lookup, pending_count);
}

#endif
