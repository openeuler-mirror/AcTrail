#ifndef ACTRAIL_PROC_H
#define ACTRAIL_PROC_H

#include "actrail_runtime.h"

enum actrail_proc_coord_syscall_id {
    ACTRAIL_PROC_COORD_TRACEPOINT_SIGNAL_GENERATE = 1,
};

static __always_inline int finalize_fork_trace_binding(__u32 child_kernel_pid) {
    __u32 child_pid = 0;
    struct actrail_fork_trace_binding *binding =
        bpf_map_lookup_elem(&fork_trace_bindings, &child_kernel_pid);
    int tracked_trace_updated;
    int process_generation_updated;

    if (!binding) {
        return 0;
    }
    child_pid = current_tgid();
    if (!child_pid) {
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

    process_generation_updated = bpf_map_update_elem(
        &process_start_times,
        &child_pid,
        &binding->child_generation,
        BPF_ANY
    );
    if (process_generation_updated != 0) {
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
    struct actrail_exec_event *event;
    __u32 filename_offset;
    __u32 filename_data_size;
    long filename_size;

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        return -1;
    }

    init_event(&event->event, ACTRAIL_PROC_EXEC, pid, trace_id);
    event->event.aux = (__u32)ctx->old_pid;
    event->filename_size = 0;
    event->filename_flags = 0;
    event->filename[0] = 0;

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

static __noinline int store_pending_exit_op(struct trace_event_raw_sys_enter *ctx) {
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
    bpf_map_update_elem(&pending_exit_ops, &pid_tgid, &op, BPF_ANY);
    return 0;
}

static __always_inline void attach_exit_code(
    struct actrail_event *event,
    __u64 pid_tgid
) {
    struct actrail_pending_exit_op *op = bpf_map_lookup_elem(&pending_exit_ops, &pid_tgid);

    if (!op) {
        return;
    }
    event->aux = (__u32)op->code;
    event->result = 1;
    bpf_map_delete_elem(&pending_exit_ops, &pid_tgid);
}

#endif
