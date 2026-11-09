/* Programs owned by the process/observation loading node. */
#include "../definitions.h"

#ifndef ACTRAIL_BPF_LOOP
SEC("tracepoint/syscalls/sys_enter_execve")
int handle_process_exec_argv_continue(struct trace_event_raw_sys_enter *ctx) {
    __u64 kernel_pid_tgid = current_kernel_pid_tgid();
    struct actrail_pending_process_exec *pending =
        bpf_map_lookup_elem(&pending_process_exec_ops, &kernel_pid_tgid);

    if (!pending || pending->argv_stopped) {
        return 0;
    }
    process_exec_argv_chunk(ctx, pending);
    if (pending->argv_stopped) {
        return 0;
    }
    {
        __u32 tail_call_index = 0;
        bpf_tail_call(ctx, &process_exec_argv_tail_calls, tail_call_index);
    }
    emit_process_exec_arg(
        ctx,
        pending,
        pending->argv_index,
        ACTRAIL_PROCESS_EXEC_ARG_MARK_LIMIT,
        0
    );
    pending->argv_stopped = 1;
    return 0;
}
#endif

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

SEC("tracepoint/syscalls/sys_enter_execve")
int handle_sys_enter_execve(struct trace_event_raw_sys_enter *ctx) {
    return store_process_exec_attempt(
        ctx,
        ACTRAIL_PROCESS_EXECVE,
        (__u64)ctx->args[0],
        (__u64)ctx->args[1],
        ((__u64)(__u32)-1) << 32
    );
}

SEC("tracepoint/syscalls/sys_enter_execveat")
int handle_sys_enter_execveat(struct trace_event_raw_sys_enter *ctx) {
    return store_process_exec_attempt(
        ctx,
        ACTRAIL_PROCESS_EXECVEAT,
        (__u64)ctx->args[1],
        (__u64)ctx->args[2],
        ((__u64)(__u32)ctx->args[0] << 32) | (__u32)ctx->args[4]
    );
}

SEC("tracepoint/syscalls/sys_enter_exit")
int handle_sys_enter_exit(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_exit_op(ctx, 0);
}

SEC("tracepoint/syscalls/sys_enter_exit_group")
int handle_sys_enter_exit_group(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_exit_op(ctx, 1);
}

SEC("tracepoint/syscalls/sys_enter_fork")
int handle_sys_enter_fork(struct trace_event_raw_sys_enter *ctx) {
    return store_process_fork_attempt(ctx, ACTRAIL_PROCESS_FORK, 0, 0, 0);
}

SEC("tracepoint/syscalls/sys_enter_vfork")
int handle_sys_enter_vfork(struct trace_event_raw_sys_enter *ctx) {
    return store_process_fork_attempt(ctx, ACTRAIL_PROCESS_VFORK, 0, 0, 0);
}

SEC("tracepoint/syscalls/sys_exit_clone")
int handle_sys_exit_clone(struct trace_event_raw_sys_exit *ctx) {
    return emit_process_fork_result(ctx);
}

SEC("tracepoint/syscalls/sys_exit_clone3")
int handle_sys_exit_clone3(struct trace_event_raw_sys_exit *ctx) {
    return emit_process_fork_result(ctx);
}

SEC("tracepoint/syscalls/sys_exit_execve")
int handle_sys_exit_execve(struct trace_event_raw_sys_exit *ctx) {
    return emit_process_exec_failure(ctx);
}

SEC("tracepoint/syscalls/sys_exit_execveat")
int handle_sys_exit_execveat(struct trace_event_raw_sys_exit *ctx) {
    return emit_process_exec_failure(ctx);
}

SEC("tracepoint/syscalls/sys_exit_fork")
int handle_sys_exit_fork(struct trace_event_raw_sys_exit *ctx) {
    return emit_process_fork_result(ctx);
}

SEC("tracepoint/syscalls/sys_exit_vfork")
int handle_sys_exit_vfork(struct trace_event_raw_sys_exit *ctx) {
    return emit_process_fork_result(ctx);
}

#ifndef ACTRAIL_COMBINED_OBJECT
char LICENSE[] SEC("license") = "GPL";
#endif
