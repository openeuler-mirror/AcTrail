#ifndef ACTRAIL_PROCESS_OBSERVE_H
#define ACTRAIL_PROCESS_OBSERVE_H

#include "../runtime/event_transport.h"
#include "../fd/suppressed.h"
#include "../launch_binding/binding.h"
#include "state.h"

enum actrail_proc_coord_syscall_id {
    ACTRAIL_PROC_COORD_TRACEPOINT_SIGNAL_GENERATE = 1,
};

static __always_inline int finalize_fork_trace_binding(__u32 child_kernel_pid) {
    __u32 child_pid = 0;
    struct actrail_fork_trace_binding *binding =
        bpf_map_lookup_elem(&fork_trace_bindings, &child_kernel_pid);
    struct actrail_process_identity *identity;
    __u64 *explicit_trace_id;
    int tracked_trace_updated;

    if (!binding) {
        return 0;
    }
    child_pid = current_tgid();
    if (!child_pid) {
        return 0;
    }
    explicit_trace_id = bpf_map_lookup_elem(&tracked_traces, &child_pid);
    if (explicit_trace_id) {
        bpf_map_delete_elem(&fork_trace_bindings, &child_kernel_pid);
        return 0;
    }
    /* Equal host and namespace PIDs still require promotion from the
     * fork-only binding into the normal lifecycle maps. */
    tracked_trace_updated = bpf_map_update_elem(
        &tracked_traces,
        &child_pid,
        &binding->trace_id,
        BPF_ANY
    );
    if (tracked_trace_updated != 0) {
        return 0;
    }

    identity = lookup_process_identity(child_kernel_pid);
    if (!identity || identity->start_boottime_ns != binding->child_generation) {
        bpf_map_delete_elem(&tracked_traces, &child_pid);
        return 0;
    }
    inherit_suppressed_fds_for_child(
        binding->parent_pid,
        binding->parent_generation,
        child_pid,
        binding->child_generation
    );
    return 0;
}

static __always_inline int emit_exec_proc_event(
    struct sched_process_exec_ctx *ctx,
    __u32 pid,
    __u64 trace_id
) {
    struct actrail_process_exec_event *event;
    __u32 filename_offset;
    __u32 filename_data_size;
    long filename_size;

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        return -1;
    }

    __builtin_memset(event, 0, sizeof(*event));
    init_current_event_header(
        &event->header,
        ACTRAIL_PROC_EXEC,
        sizeof(*event),
        trace_id
    );
    filename_offset = ctx->filename_loc & 0xffff;
    filename_data_size = ctx->filename_loc >> 16;
    if (filename_offset) {
        const void *filename = (const void *)ctx + filename_offset;

        filename_size = bpf_probe_read_kernel_str(
            event->filename,
            sizeof(event->filename),
            filename
        );
        if (filename_size > 0) {
            event->filename_size = (__u32)(filename_size - 1);
            if (filename_size == sizeof(event->filename) ||
                filename_data_size > sizeof(event->filename)) {
                event->filename_flags |= ACTRAIL_EXEC_FILENAME_FLAG_TRUNCATED;
            }
        }
    }

    actrail_event_submit(ctx, event);
    return 0;
}

static __noinline int store_pending_exit_op(
    struct trace_event_raw_sys_enter *ctx,
    __u32 group_exit
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u64 *trace_id = 0;
    struct actrail_pending_exit_op op = {};

    if (pid) {
        trace_id = bpf_map_lookup_elem(&tracked_traces, &pid);
    }
    if (!trace_id) {
        __u64 kernel_pid_tgid = current_kernel_pid_tgid();
        __u32 kernel_pid = kernel_pid_tgid >> 32;

        if (kernel_pid_tgid && kernel_pid_tgid != pid_tgid) {
            trace_id = bpf_map_lookup_elem(&tracked_traces, &kernel_pid);
            if (trace_id) {
                pid_tgid = kernel_pid_tgid;
                pid = kernel_pid;
            }
        }
    }
    if (!pid || !trace_id) {
        return 0;
    }

    op.code = (__s32)ctx->args[0];
    op.group_exit = group_exit;
    if (group_exit) {
        pid_tgid = ((__u64)pid << 32) | pid;
    }
    bpf_map_update_elem(&pending_exit_ops, &pid_tgid, &op, BPF_ANY);
    return 0;
}

static __always_inline void attach_exit_code(
    struct actrail_process_exit_event *event,
    __u64 pid_tgid
) {
    __u64 group_key = (pid_tgid & 0xffffffff00000000ULL) | (pid_tgid >> 32);
    struct actrail_pending_exit_op *op = bpf_map_lookup_elem(&pending_exit_ops, &pid_tgid);

    if (!op && group_key != pid_tgid) {
        op = bpf_map_lookup_elem(&pending_exit_ops, &group_key);
    }
    if (op) {
        event->exit_code = op->code;
        event->exit_flags |= ACTRAIL_PROCESS_EXIT_CODE_VALID;
    }
    bpf_map_delete_elem(&pending_exit_ops, &pid_tgid);
    if (group_key != pid_tgid) {
        bpf_map_delete_elem(&pending_exit_ops, &group_key);
    }
}

static __always_inline void discard_process_exit_codes(__u64 pid_tgid) {
    __u64 group_key = (pid_tgid & 0xffffffff00000000ULL) | (pid_tgid >> 32);

    bpf_map_delete_elem(&pending_exit_ops, &pid_tgid);
    if (group_key != pid_tgid) {
        bpf_map_delete_elem(&pending_exit_ops, &group_key);
    }
}

static __always_inline void discard_thread_exit_code(__u64 pid_tgid) {
    struct actrail_pending_exit_op *op = bpf_map_lookup_elem(&pending_exit_ops, &pid_tgid);

    if (op && !op->group_exit) {
        bpf_map_delete_elem(&pending_exit_ops, &pid_tgid);
    }
}

#endif
