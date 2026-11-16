/* Process exec observation helpers. */
#ifndef ACTRAIL_PROCESS_OBSERVATION_EXEC_H
#define ACTRAIL_PROCESS_OBSERVATION_EXEC_H

#include "argv.h"

static __always_inline int store_process_exec_attempt(
    struct trace_event_raw_sys_enter *ctx,
    __u32 syscall,
    __u64 path_ptr,
    __u64 argv_ptr,
    __u64 execveat_metadata
) {
    __s32 execveat_dirfd = (__s32)(execveat_metadata >> 32);
    __u32 execveat_flags = (__u32)execveat_metadata;
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    __u32 host_tgid = kernel_pid_tgid >> 32;
    __u32 host_tid = (__u32)kernel_pid_tgid;
    __u32 pid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id = lookup_current_trace(&pid, &tid, &lookup_flags);
    __u64 current_trace_id = 0;
    __u64 launch_trace_id = 0;
    __u64 launch_generation = 0;
    struct actrail_pending_process_exec pending = {};
    __u64 *previous_key;

    if (trace_id) {
        current_trace_id = *trace_id;
    }
    if (!trace_id && actrail_launch_binding_observe_current(
            host_tgid,
            &launch_trace_id,
            &launch_generation)) {
        pid = host_tgid;
        tid = host_tid;
    }
    if (!kernel_pid_tgid || !host_tgid ||
        (!current_trace_id && !launch_trace_id)) {
        return 0;
    }
    pending.trace_id = current_trace_id ? current_trace_id : launch_trace_id;
    pending.attempt_id = next_process_exec_attempt_id(kernel_pid_tgid);
    if (!pending.attempt_id) {
        return 0;
    }
    pending.observed_ktime_ns = bpf_ktime_get_ns();
    pending.pid_generation = launch_generation
        ? launch_generation
        : current_process_start_time(pid);
    pending.pid = pid;
    pending.tid = tid;
    pending.host_pid = host_tgid;
    pending.host_tid = host_tid;
    pending.syscall = syscall;

    previous_key = bpf_map_lookup_elem(&pending_process_exec_tgid_index, &host_tgid);
    if (previous_key && *previous_key != kernel_pid_tgid) {
        bpf_map_delete_elem(&pending_process_exec_ops, previous_key);
    }
    if (bpf_map_update_elem(
            &pending_process_exec_ops,
            &kernel_pid_tgid,
            &pending,
            BPF_ANY) != 0 ||
        bpf_map_update_elem(
            &pending_process_exec_tgid_index,
            &host_tgid,
            &kernel_pid_tgid,
            BPF_ANY) != 0) {
        bpf_map_delete_elem(&pending_process_exec_ops, &kernel_pid_tgid);
        return 0;
    }
    if (!emit_process_exec_attempt(
            ctx,
            &pending,
            path_ptr,
            execveat_dirfd,
            execveat_flags)) {
        return 0;
    }
    emit_process_exec_args(ctx, &pending, argv_ptr);
    return 0;
}

static __always_inline int emit_process_exec_failure(
    struct trace_event_raw_sys_exit *ctx
) {
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    __u32 host_tgid = kernel_pid_tgid >> 32;
    struct actrail_pending_process_exec *pending =
        bpf_map_lookup_elem(&pending_process_exec_ops, &kernel_pid_tgid);
    struct actrail_process_exec_result_event *event;

    if (!pending) {
        return 0;
    }
    if (ctx->ret >= 0) {
        delete_pending_process_exec(host_tgid, kernel_pid_tgid);
        return 0;
    }
    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        delete_pending_process_exec(host_tgid, kernel_pid_tgid);
        return 0;
    }
    event->kind = ACTRAIL_PROC_EXEC_RESULT;
    event->pid = pending->pid;
    event->tid = pending->tid;
    event->syscall = pending->syscall;
    event->trace_id = pending->trace_id;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->attempt_id = pending->attempt_id;
    event->pid_generation = pending->pid_generation;
    event->result = ctx->ret;
    event->host_pid = pending->host_pid;
    event->host_tid = pending->host_tid;
    actrail_event_submit(ctx, event);
    delete_pending_process_exec(host_tgid, kernel_pid_tgid);
    return 0;
}

#endif
