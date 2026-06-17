#ifndef ACTRAIL_PROC_H
#define ACTRAIL_PROC_H

#include "actrail_runtime.h"

enum actrail_proc_coord_syscall_id {
    ACTRAIL_PROC_COORD_TRACEPOINT_SIGNAL_GENERATE = 1,
};

static __always_inline int emit_pending_child_proc_op(void) {
    __u32 child_global_pid = current_tgid();
    __u32 child_pid = current_namespace_tgid();
    struct actrail_pending_proc_op *op =
        bpf_map_lookup_elem(&pending_child_proc_ops, &child_global_pid);
    struct actrail_event event;

    if (!op) {
        return 0;
    }
    if (!child_pid) {
        bpf_map_delete_elem(&pending_child_proc_ops, &child_global_pid);
        return 0;
    }

    bpf_map_update_elem(&tracked_traces, &child_pid, &op->trace_id, BPF_ANY);
    set_process_generation(child_pid, op->child_generation);
    inherit_suppressed_fds_for_child(
        op->parent_pid,
        op->parent_generation,
        child_pid,
        op->child_generation
    );

    init_event(&event, ACTRAIL_PROC_FORK, op->parent_pid, op->trace_id);
    event.aux = child_pid;
    event.pid_generation = op->parent_generation;
    event.aux_generation = op->child_generation;
    emit_event(&event);
    bpf_map_delete_elem(&pending_child_proc_ops, &child_global_pid);
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

    event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
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

    bpf_ringbuf_submit(event, 0);
    return 0;
}

static __always_inline int store_pending_exit_op(struct trace_event_raw_sys_enter *ctx) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 pid = current_namespace_tgid();
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &pid);
    struct actrail_pending_exit_op op = {};

    if (!pid) {
        return 0;
    }
    if (!trace_id) {
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
