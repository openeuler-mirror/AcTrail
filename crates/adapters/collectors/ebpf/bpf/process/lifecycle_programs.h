#ifndef ACTRAIL_PROCESS_LIFECYCLE_PROGRAMS_H
#define ACTRAIL_PROCESS_LIFECYCLE_PROGRAMS_H

SEC("tracepoint/syscalls/sys_enter_fork")
int handle_sys_enter_fork(struct trace_event_raw_sys_enter *ctx) {
    return store_process_fork_attempt(ctx, ACTRAIL_PROCESS_FORK, 0, 0, 0);
}

SEC("tracepoint/syscalls/sys_exit_fork")
int handle_sys_exit_fork(struct trace_event_raw_sys_exit *ctx) {
    return emit_process_fork_result(ctx);
}

SEC("tracepoint/syscalls/sys_enter_vfork")
int handle_sys_enter_vfork(struct trace_event_raw_sys_enter *ctx) {
    return store_process_fork_attempt(ctx, ACTRAIL_PROCESS_VFORK, 0, 0, 0);
}

SEC("tracepoint/syscalls/sys_exit_vfork")
int handle_sys_exit_vfork(struct trace_event_raw_sys_exit *ctx) {
    return emit_process_fork_result(ctx);
}

SEC("tracepoint/syscalls/sys_enter_clone")
int handle_sys_enter_clone(struct trace_event_raw_sys_enter *ctx) {
    return store_process_fork_attempt(
        ctx,
        ACTRAIL_PROCESS_CLONE,
        (__u64)ctx->args[0],
        0,
        0
    );
}

SEC("tracepoint/syscalls/sys_exit_clone")
int handle_sys_exit_clone(struct trace_event_raw_sys_exit *ctx) {
    return emit_process_fork_result(ctx);
}

SEC("tracepoint/syscalls/sys_enter_clone3")
int handle_sys_enter_clone3(struct trace_event_raw_sys_enter *ctx) {
    return store_process_fork_attempt(
        ctx,
        ACTRAIL_PROCESS_CLONE3,
        0,
        (__u64)ctx->args[0],
        (__u64)ctx->args[1]
    );
}

SEC("tracepoint/syscalls/sys_exit_clone3")
int handle_sys_exit_clone3(struct trace_event_raw_sys_exit *ctx) {
    return emit_process_fork_result(ctx);
}
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
    __u32 parent_observer_pid = 0;
    __u32 child_observer_pid = 0;
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
    struct actrail_observer_fork_binding observer_binding = {};
    struct actrail_process_fork_event event = {};
    struct actrail_process_observation_scope *parent_observation_scope;
    struct actrail_process_observation_scope child_observation_scope = {};
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

    child_observer_pid = observer_tgid_for_task(child_task);
    parent_observer_pid = observer_tgid_for_task(parent_task);
    if (!child_observer_pid || !parent_observer_pid) {
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
    if (set_process_identity(
            child_kernel_pid,
            child_start_boottime_ns,
            child_observer_pid) != 0) {
        bpf_map_delete_elem(&fork_trace_bindings, &child_kernel_pid);
        event_transport_diag_inc(ACTRAIL_FORK_IDENTITY_PUBLISH_FAIL);
        return 0;
    }
    parent_observation_scope = lookup_process_observation_scope(parent_host_pid);
    if (!parent_observation_scope) {
        bpf_map_delete_elem(&process_observation_depths, &child_kernel_pid);
    } else {
        child_observation_scope.start_boottime_ns = child_start_boottime_ns;
        child_observation_scope.remaining_depth =
            parent_observation_scope->remaining_depth;
        if (child_observation_scope.remaining_depth == 0) {
            child_observation_scope.remaining_depth =
                ACTRAIL_OBSERVATION_DEPTH_LIFECYCLE_ONLY;
        } else if (child_observation_scope.remaining_depth > 0) {
            child_observation_scope.remaining_depth--;
        }
        bpf_map_update_elem(
            &process_observation_depths,
            &child_kernel_pid,
            &child_observation_scope,
            BPF_ANY
        );
    }
    observer_binding.binding = binding;
    observer_binding.kernel_tgid = child_kernel_pid;
    if (bpf_map_update_elem(
            &observer_fork_trace_bindings,
            &child_observer_pid,
            &observer_binding,
            BPF_ANY) != 0) {
        bpf_map_delete_elem(&process_observation_depths, &child_kernel_pid);
        if (bpf_map_delete_elem(&process_identities, &child_kernel_pid) != 0) {
            event_transport_diag_inc(ACTRAIL_PROCESS_IDENTITY_CLEANUP_FAIL);
        }
        bpf_map_delete_elem(&fork_trace_bindings, &child_kernel_pid);
        observer_pid_diag_inc(ACTRAIL_OBSERVER_PID_INDEX_PUBLISH_FAIL);
        return 0;
    }

    init_event_header(
        &event.header,
        ACTRAIL_PROC_FORK,
        sizeof(event),
        inherited_trace_id,
        child_observer_pid,
        child_host_pid,
        child_start_boottime_ns
    );
    event.parent_observer_namespace_tgid = parent_observer_pid;
    event.parent_kernel_tgid = parent_host_pid;
    event.parent_start_boottime_ns = binding.parent_generation;
    event.attempt_id = current_process_fork_attempt_id();
    return emit_event(ctx, &event, sizeof(event));
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
    struct actrail_process_identity *identity;
    __u64 exit_trace_id = 0;
    __u64 exit_generation;
    __u32 exit_observer_pid;
    __u32 context_pid = (__u32)ctx->pid;
    __u32 host_pid = kernel_pid_tgid >> 32;
    __u32 host_tid = (__u32)kernel_pid_tgid;
    __u32 observer_pid = 0;
    struct actrail_process_exit_event event = {};

    actrail_launch_binding_cleanup_current(ctx, host_pid);
    delete_pending_process_exec(host_pid, kernel_pid_tgid);
    bpf_map_delete_elem(&pending_process_fork_ops, &kernel_pid_tgid);
    fd_pending_thread_cleanup(kernel_pid_tgid, ctx);
    bpf_map_delete_elem(&pending_file_completion_ops, &kernel_pid_tgid);
    bpf_map_delete_elem(&pending_file_open_observations, &kernel_pid_tgid);
    delete_trace_namespace_thread_identity(kernel_pid_tgid);
    bpf_map_delete_elem(&process_exec_sequences, &kernel_pid_tgid);
    bpf_map_delete_elem(&process_fork_sequences, &kernel_pid_tgid);
    bpf_map_delete_elem(&payload_socket_operation_sequence, &kernel_pid_tgid);
    bpf_map_delete_elem(&file_io_sequence, &kernel_pid_tgid);
    if (!host_pid || !current_process_group_dead()) {
        discard_thread_exit_code(kernel_pid_tgid);
        return 0;
    }
    bpf_map_delete_elem(&process_observation_depths, &host_pid);
    trace_id = lookup_trace_for_context_pid(context_pid, &pid, &tid, &lookup_flags);
    if (!pid) {
        pid = host_pid;
        tid = host_tid;
    }
    if (trace_id) {
        exit_trace_id = *trace_id;
    }
    state_pid = pid;
    identity = lookup_process_identity(state_pid);
    if (!identity && state_pid != host_pid) {
        state_pid = host_pid;
        identity = lookup_process_identity(state_pid);
    }
    if (!identity) {
        discard_thread_exit_code(kernel_pid_tgid);
        return 0;
    }
    if (!claim_process_exit(identity, state_pid)) {
        discard_thread_exit_code(kernel_pid_tgid);
        return 0;
    }
    exit_generation = identity->start_boottime_ns;
    exit_observer_pid = identity->observer_namespace_tgid;
    if (fd_tracking_enabled()) {
        fd_process_exit_cleanup(host_pid, exit_trace_id, ctx);
    }
    if (bpf_map_delete_elem(&process_identities, &state_pid) != 0) {
        event_transport_diag_inc(ACTRAIL_PROCESS_IDENTITY_CLEANUP_FAIL);
        discard_thread_exit_code(kernel_pid_tgid);
        return 0;
    }
    actrail_once_u64_release(state_pid);
    if (exit_trace_id) {
        init_event_header(
            &event.header,
            ACTRAIL_PROC_EXIT,
            sizeof(event),
            exit_trace_id,
            exit_observer_pid,
            host_pid,
            exit_generation
        );
        attach_exit_code(&event, kernel_pid_tgid);
        emit_event(ctx, &event, sizeof(event));
        cleanup_suppressed_fds_for_process(pid, exit_generation);
    } else {
        discard_process_exit_codes(kernel_pid_tgid);
    }
    bpf_map_delete_elem(&tracked_traces, &pid);
    if (state_pid != pid) {
        bpf_map_delete_elem(&tracked_traces, &state_pid);
    }
    bpf_map_delete_elem(&fork_trace_bindings, &host_pid);
    observer_pid = observer_tgid_for_task(
        (struct task_struct *)actrail_bpf_get_current_task());
    if (observer_pid) {
        bpf_map_delete_elem(&observer_fork_trace_bindings, &observer_pid);
    }
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
    if (!process_observation_child_is_detailed(parent_host_pid)) {
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

SEC("tracepoint/syscalls/sys_enter_exit")
int handle_sys_enter_exit(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_exit_op(ctx, 0);
}

SEC("tracepoint/syscalls/sys_enter_exit_group")
int handle_sys_enter_exit_group(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_exit_op(ctx, 1);
}

SEC("tracepoint/signal/signal_generate")
int handle_signal_generate(struct signal_generate_ctx *ctx) {
    __u32 pid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id = lookup_current_trace(&pid, &tid, &lookup_flags);
    struct actrail_process_signal_event event = {};

    if (!pid || !trace_id) {
        return 0;
    }

    init_current_event_header(
        &event.header,
        ACTRAIL_PROC_SIGNAL,
        sizeof(event),
        *trace_id
    );
    event.signal_result = ctx->signal_result;
    event.signal = (__u32)ctx->sig;
    event.target_kernel_tid = (__u32)ctx->pid;
    event.target_group = (__u32)ctx->group;
    return emit_event(ctx, &event, sizeof(event));
}


#endif
