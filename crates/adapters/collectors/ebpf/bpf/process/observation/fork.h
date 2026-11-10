/* Process fork observation helpers. */
#ifndef ACTRAIL_PROCESS_OBSERVATION_FORK_H
#define ACTRAIL_PROCESS_OBSERVATION_FORK_H

#include "../observe.h"

static __noinline int emit_process_fork_attempt(
    struct trace_event_raw_sys_enter *ctx,
    struct actrail_pending_process_fork *pending
) {
    struct actrail_process_fork_attempt_event *event =
        actrail_event_reserve(sizeof(*event));

    if (!event) {
        return 0;
    }
    event->kind = ACTRAIL_PROC_FORK_ATTEMPT;
    event->pid = pending->pid;
    event->tid = pending->tid;
    event->syscall = pending->syscall;
    event->trace_id = pending->trace_id;
    event->observed_ktime_ns = pending->observed_ktime_ns;
    event->attempt_id = pending->attempt_id;
    event->pid_generation = pending->pid_generation;
    event->flags = pending->flags;
    event->clone3_args_ptr = pending->clone3_args_ptr;
    event->clone3_args_size = pending->clone3_args_size;
    event->capture_flags = pending->capture_flags;
    event->host_pid = pending->host_pid;
    event->host_tid = pending->host_tid;
    event->reserved = 0;
    actrail_event_submit(ctx, event);
    return 0;
}

/*
 * Stage a pending fork attempt for sys_enter_fork/vfork/clone/clone3.
 * The argument list stays within the eBPF-to-eBPF call limit (5 scalars) so
 * the helper compiles even if inlining is skipped. clone3 capture metadata is
 * derived here from the raw user pointer/size instead of being passed by the
 * caller; `flags` is meaningful for fork/vfork/clone and is replaced by the
 * decoded prefix for clone3 (callers pass 0 there).
 */
static __always_inline int store_process_fork_attempt(
    struct trace_event_raw_sys_enter *ctx,
    __u32 syscall,
    __u64 flags,
    __u64 clone3_args_ptr,
    __u64 clone3_args_size
) {
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    __u32 host_tgid = kernel_pid_tgid >> 32;
    __u32 host_tid = (__u32)kernel_pid_tgid;
    __u32 pid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id = lookup_current_trace(&pid, &tid, &lookup_flags);
    struct actrail_pending_process_fork pending = {};
    __u32 capture_flags = 0;

    if (!kernel_pid_tgid || !host_tgid || !trace_id) {
        return 0;
    }
    if (syscall == ACTRAIL_PROCESS_CLONE3) {
        struct actrail_user_clone_args_prefix prefix = {};

        if (clone3_args_size < sizeof(prefix)) {
            capture_flags |= ACTRAIL_PROCESS_FORK_CLONE3_TRUNCATED;
        } else if (bpf_probe_read_user(
                &prefix,
                sizeof(prefix),
                (void *)(unsigned long)clone3_args_ptr
            ) != 0) {
            capture_flags |= ACTRAIL_PROCESS_FORK_CLONE3_READ_FAILED;
        } else {
            flags = prefix.flags;
        }
    }
    if (flags & ACTRAIL_PROCESS_CLONE_THREAD) {
        return 0;
    }
    pending.trace_id = *trace_id;
    pending.attempt_id = next_process_fork_attempt_id(kernel_pid_tgid);
    if (!pending.attempt_id) {
        return 0;
    }
    pending.observed_ktime_ns = bpf_ktime_get_ns();
    pending.pid_generation = current_process_start_time(pid);
    pending.flags = flags;
    pending.clone3_args_ptr = clone3_args_ptr;
    pending.clone3_args_size = clone3_args_size;
    pending.pid = pid;
    pending.tid = tid;
    pending.host_pid = host_tgid;
    pending.host_tid = host_tid;
    pending.syscall = syscall;
    pending.capture_flags = capture_flags;
    if (bpf_map_update_elem(
            &pending_process_fork_ops,
            &kernel_pid_tgid,
            &pending,
            BPF_ANY) != 0) {
        return 0;
    }
    return emit_process_fork_attempt(ctx, &pending);
}

static __always_inline int emit_process_fork_result(
    struct trace_event_raw_sys_exit *ctx
) {
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    struct actrail_pending_process_fork *pending =
        bpf_map_lookup_elem(&pending_process_fork_ops, &kernel_pid_tgid);
    struct actrail_process_fork_result_event *event;

    if (!pending) {
        return 0;
    }
    event = actrail_event_reserve(sizeof(*event));
    if (event) {
        event->kind = ACTRAIL_PROC_FORK_RESULT;
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
    }
    bpf_map_delete_elem(&pending_process_fork_ops, &kernel_pid_tgid);
    return 0;
}

#endif
