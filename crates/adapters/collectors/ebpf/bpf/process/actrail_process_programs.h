#ifndef ACTRAIL_PROCESS_PROGRAMS_H
#define ACTRAIL_PROCESS_PROGRAMS_H

SEC("raw_tracepoint/sched_process_fork")
int handle_sched_process_fork(struct bpf_raw_tracepoint_args *ctx) {
    struct task_struct *parent_task = (struct task_struct *)ctx->args[0];
    struct task_struct *child_task = (struct task_struct *)ctx->args[1];
    __u32 parent_pid = 0;
    __u32 parent_tid = 0;
    __u32 lookup_flags = 0;
    __u32 context_parent_pid = 0;
    __u32 parent_host_pid = 0;
    __u32 child_host_pid = 0;
    __u64 child_start_boottime_ns = 0;
    __u64 inherited_trace_id = 0;

    if (!parent_task || !child_task) {
        return 0;
    }
    if (ACTRAIL_CORE_READ(&context_parent_pid, parent_task, pid) != 0 ||
        ACTRAIL_CORE_READ(&parent_host_pid, parent_task, tgid) != 0 ||
        ACTRAIL_CORE_READ(&child_host_pid, child_task, tgid) != 0 ||
        ACTRAIL_CORE_READ(&child_start_boottime_ns, child_task, start_boottime) != 0) {
        return 0;
    }
    __u64 *trace_id = lookup_trace_for_context_pid(
        context_parent_pid,
        &parent_pid,
        &parent_tid,
        &lookup_flags
    );
    struct actrail_fork_trace_binding binding = {};
    struct actrail_event event = {};
    __u32 child_kernel_pid = child_host_pid;

    if (trace_id) {
        inherited_trace_id = *trace_id;
    } else {
        struct actrail_fork_trace_binding *parent_binding =
            bpf_map_lookup_elem(&fork_trace_bindings, &parent_host_pid);

        if (parent_binding) {
            inherited_trace_id = parent_binding->trace_id;
            parent_pid = parent_host_pid;
            lookup_flags = ACTRAIL_TRACE_LOOKUP_FLAG_HOST_FALLBACK;
        }
    }

    if (!parent_pid || !inherited_trace_id) {
        return 0;
    }
    if (!child_kernel_pid || !child_start_boottime_ns) {
        return 0;
    }
    if (child_host_pid == parent_host_pid) {
        return 0;
    }

    binding.trace_id = inherited_trace_id;
    binding.parent_generation = current_process_start_time(parent_pid);
    binding.child_generation = child_start_boottime_ns;
    binding.parent_pid = parent_pid;

    /* sched_process_fork runs before wake_up_new_task().  Publish the child
     * binding here so its first post-fork syscall is already controlled. */
    if (bpf_map_update_elem(
            &fork_trace_bindings,
            &child_kernel_pid,
            &binding,
            BPF_ANY) != 0) {
        event_transport_diag_inc(ACTRAIL_FORK_IDENTITY_PUBLISH_FAIL);
        return 0;
    }
    if (bpf_map_update_elem(
            &process_start_times,
            &child_kernel_pid,
            &child_start_boottime_ns,
            BPF_ANY) != 0) {
        bpf_map_delete_elem(&fork_trace_bindings, &child_kernel_pid);
        event_transport_diag_inc(ACTRAIL_FORK_IDENTITY_PUBLISH_FAIL);
        return 0;
    }
    init_event(&event, ACTRAIL_PROC_FORK, parent_pid, inherited_trace_id);
    event.aux = 0;
    event.reserved = ACTRAIL_PROC_FORK_CHILD_HOST_ONLY;
    if (lookup_flags & ACTRAIL_TRACE_LOOKUP_FLAG_HOST_FALLBACK) {
        event.reserved |= ACTRAIL_PROC_FORK_PARENT_HOST_ONLY;
    }
    event.host_pid = parent_host_pid;
    event.aux_host_pid = child_host_pid;
    event.pid_generation = binding.parent_generation;
    event.aux_generation = child_start_boottime_ns;
    return emit_event(ctx, &event);
}

SEC("tracepoint/sched/sched_process_exec")
int handle_sched_process_exec(struct sched_process_exec_ctx *ctx) {
    __u32 pid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u32 context_pid = (__u32)ctx->old_pid;
    __u64 *trace_id = lookup_trace_for_context_pid(context_pid, &pid, &tid, &lookup_flags);
    __u64 exec_trace_id = 0;

    if (!pid) {
        return 0;
    }
    finalize_fork_trace_binding(current_kernel_tgid());
    trace_id = lookup_trace_for_context_pid(context_pid, &pid, &tid, &lookup_flags);
    if (!trace_id && actrail_launch_binding_promote_current(ctx, pid)) {
        trace_id = lookup_trace_for_context_pid(context_pid, &pid, &tid, &lookup_flags);
    }
    if (trace_id) {
        exec_trace_id = *trace_id;
    }
    if (!exec_trace_id) {
        return 0;
    }

    return emit_exec_proc_event(ctx, pid, exec_trace_id);
}

SEC("tracepoint/sched/sched_process_exit")
int handle_sched_process_exit(struct sched_process_exit_ctx *ctx) {
    __u32 pid = 0;
    __u32 tid = 0;
    __u32 state_pid;
    __u32 lookup_flags = 0;
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    __u64 *trace_id;
    __u64 *start_time;
    __u64 exit_trace_id = 0;
    __u64 exit_generation;
    __u32 context_pid = (__u32)ctx->pid;
    __u32 host_pid = kernel_pid_tgid >> 32;
    __u32 host_tid = (__u32)kernel_pid_tgid;
    struct actrail_event event;

    actrail_launch_binding_cleanup_current(ctx, host_pid);
    fd_pending_thread_cleanup(kernel_pid_tgid, ctx);
    if (!host_pid || !current_process_group_dead()) {
        discard_thread_exit_code(kernel_pid_tgid);
        return 0;
    }
    trace_id = lookup_trace_for_context_pid(context_pid, &pid, &tid, &lookup_flags);
    if (!pid) {
        pid = host_pid;
        tid = host_tid;
    }
    if (trace_id) {
        exit_trace_id = *trace_id;
    }
    state_pid = pid;
    start_time = lookup_process_start_time(state_pid);
    if (!start_time && state_pid != host_pid) {
        state_pid = host_pid;
        start_time = lookup_process_start_time(state_pid);
    }
    if (!start_time) {
        discard_thread_exit_code(kernel_pid_tgid);
        return 0;
    }
    exit_generation = *start_time;
    if (bpf_map_delete_elem(&process_start_times, &state_pid) != 0) {
        discard_thread_exit_code(kernel_pid_tgid);
        return 0;
    }
    if (exit_trace_id) {
        init_event(&event, ACTRAIL_PROC_EXIT, pid, exit_trace_id);
        event.pid_generation = exit_generation;
        attach_exit_code(&event, kernel_pid_tgid);
        emit_event(ctx, &event);
        cleanup_suppressed_fds_for_process(pid, event.pid_generation);
        delete_file_bulk_read_fast_process(pid, event.pid_generation);
    } else {
        discard_process_exit_codes(kernel_pid_tgid);
    }
    bpf_map_delete_elem(&tracked_traces, &pid);
    if (state_pid != pid) {
        bpf_map_delete_elem(&tracked_traces, &state_pid);
    }
    bpf_map_delete_elem(&fork_trace_bindings, &host_pid);
    return 0;
}

SEC("raw_tracepoint/sched_process_fork")
int handle_fd_sched_process_fork(struct bpf_raw_tracepoint_args *ctx) {
    struct task_struct *parent_task = (struct task_struct *)ctx->args[0];
    struct task_struct *child_task = (struct task_struct *)ctx->args[1];
    __u32 parent_host_pid = 0;
    __u32 child_host_pid = 0;

    if (!parent_task || !child_task || !fd_tracking_enabled()) {
        return 0;
    }
    if (ACTRAIL_CORE_READ(&parent_host_pid, parent_task, tgid) != 0 ||
        ACTRAIL_CORE_READ(&child_host_pid, child_task, tgid) != 0 ||
        !parent_host_pid || !child_host_pid || parent_host_pid == child_host_pid) {
        return 0;
    }
    fd_fork_seed(parent_host_pid, child_host_pid);
    return 0;
}

SEC("tracepoint/sched/sched_process_exec")
int handle_fd_sched_process_exec(struct sched_process_exec_ctx *ctx) {
    fd_process_exec_cleanup(current_kernel_tgid(), 0, ctx);
    return 0;
}

SEC("tracepoint/sched/sched_process_exit")
int handle_fd_sched_process_exit(struct sched_process_exit_ctx *ctx) {
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    __u32 host_pid = kernel_pid_tgid >> 32;

    fd_pending_thread_cleanup(kernel_pid_tgid, ctx);
    if (host_pid && current_process_group_dead()) {
        fd_process_exit_cleanup(host_pid, 0, ctx);
    }
    return 0;
}

#endif
