#ifndef ACTRAIL_PROCESS_PROGRAMS_H
#define ACTRAIL_PROCESS_PROGRAMS_H

#include "../fd/lifecycle.h"
#include "../file/observe.h"
#include "../payload/socket_types.h"
#include "observe.h"

static __noinline int emit_process_exec_attempt(
    struct trace_event_raw_sys_enter *ctx,
    struct actrail_pending_process_exec *pending,
    __u64 path_ptr,
    __s32 execveat_dirfd,
    __u32 execveat_flags
) {
    struct actrail_process_exec_config *config = current_process_exec_config();
    struct actrail_process_exec_attempt_event *event;
    __u32 max_arg_bytes;
    __u32 path_read_limit;
    long path_size;

    if (!config || !config->max_args || !config->max_arg_bytes ||
        !config->max_total_arg_bytes) {
        return 0;
    }
    max_arg_bytes = config->max_arg_bytes;
    if (max_arg_bytes > ACTRAIL_PROCESS_EXEC_ARGV_COPY_MAX_BYTES) {
        max_arg_bytes = ACTRAIL_PROCESS_EXEC_ARGV_COPY_MAX_BYTES;
    }
    path_read_limit = max_arg_bytes + 1;
    if (path_read_limit > ACTRAIL_PROCESS_EXEC_PATH_ABI_MAX_BYTES) {
        path_read_limit = ACTRAIL_PROCESS_EXEC_PATH_ABI_MAX_BYTES;
    }

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        return 0;
    }
    event->kind = ACTRAIL_PROC_EXEC_ATTEMPT;
    event->pid = pending->pid;
    event->tid = pending->tid;
    event->syscall = pending->syscall;
    event->trace_id = pending->trace_id;
    event->observed_ktime_ns = pending->observed_ktime_ns;
    event->attempt_id = pending->attempt_id;
    event->pid_generation = pending->pid_generation;
    event->execveat_dirfd = execveat_dirfd;
    event->execveat_flags = execveat_flags;
    event->path_size = 0;
    event->argv_size = 0;
    event->argc = 0;
    event->capture_flags = 0;
    event->host_pid = pending->host_pid;
    event->host_tid = pending->host_tid;
    event->path[0] = 0;

    path_size = bpf_probe_read_user_str(
        event->path,
        path_read_limit,
        (void *)(unsigned long)path_ptr
    );
    if (path_size < 0) {
        event->capture_flags |= ACTRAIL_PROCESS_EXEC_READ_FAILED;
    } else if (path_size > 0) {
        event->path_size = (__u32)(path_size - 1);
        if ((__u32)path_size == path_read_limit) {
            event->capture_flags |= ACTRAIL_PROCESS_EXEC_PATH_TRUNCATED;
        }
    }

    return actrail_event_submit(ctx, event) == 0;
}

/* Marker values are used only when read_limit is zero, so they are never
 * dereferenced as user pointers. This keeps the BPF subprogram at five
 * arguments while still reporting why argv assembly stopped. */
enum actrail_process_exec_arg_marker {
    ACTRAIL_PROCESS_EXEC_ARG_MARK_COMPLETE = 0,
    ACTRAIL_PROCESS_EXEC_ARG_MARK_READ_FAILED = 1,
    ACTRAIL_PROCESS_EXEC_ARG_MARK_LIMIT = 2,
    ACTRAIL_PROCESS_EXEC_ARG_MARK_TOTAL = 3,
};

static __noinline long emit_process_exec_arg(
    struct trace_event_raw_sys_enter *ctx,
    struct actrail_pending_process_exec *pending,
    __u32 index,
    __u64 arg_ptr,
    __u32 read_limit
) {
    struct actrail_process_exec_arg_event *event =
        actrail_event_reserve(sizeof(*event));
    long arg_size;

    if (!event) {
        return -1;
    }
    event->kind = ACTRAIL_PROC_EXEC_ARG;
    event->pid = pending->pid;
    event->tid = pending->tid;
    event->syscall = pending->syscall;
    event->trace_id = pending->trace_id;
    event->observed_ktime_ns = pending->observed_ktime_ns;
    event->attempt_id = pending->attempt_id;
    event->pid_generation = pending->pid_generation;
    event->index = index;
    event->arg_size = 0;
    event->capture_flags = 0;
    event->host_pid = pending->host_pid;
    event->host_tid = pending->host_tid;
    event->reserved = 0;
    event->arg[0] = 0;

    if (!read_limit) {
        if (arg_ptr == ACTRAIL_PROCESS_EXEC_ARG_MARK_COMPLETE) {
            event->capture_flags = ACTRAIL_PROCESS_EXEC_ARGV_COMPLETE;
        } else if (arg_ptr == ACTRAIL_PROCESS_EXEC_ARG_MARK_READ_FAILED) {
            event->capture_flags = ACTRAIL_PROCESS_EXEC_READ_FAILED;
        } else if (arg_ptr == ACTRAIL_PROCESS_EXEC_ARG_MARK_LIMIT) {
            event->capture_flags = ACTRAIL_PROCESS_EXEC_ARG_LIMIT;
        } else {
            event->capture_flags = ACTRAIL_PROCESS_EXEC_ARGV_TRUNCATED;
        }
        actrail_event_submit(ctx, event);
        return 0;
    }

    /* Mask to u32 and clamp to the ABI maximum before the helper call; old
     * verifiers otherwise track the noinline parameter with a possibly
     * negative 64-bit range and reject the load. */
    __u64 bounded_read_limit = (__u64)read_limit & 0xffffffffULL;
    if (bounded_read_limit > ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES) {
        bounded_read_limit = ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES;
    }
    actrail_barrier_var(bounded_read_limit);
    arg_size = bpf_probe_read_user_str(
        event->arg,
        bounded_read_limit,
        (void *)(unsigned long)arg_ptr
    );
    if (arg_size < 0) {
        event->capture_flags = ACTRAIL_PROCESS_EXEC_READ_FAILED;
        actrail_event_submit(ctx, event);
        return -1;
    }
    if (arg_size > 0) {
        event->arg_size = (__u32)(arg_size - 1);
        if ((__u32)arg_size == read_limit) {
            event->capture_flags = ACTRAIL_PROCESS_EXEC_ARGV_TRUNCATED;
        }
    }
    actrail_event_submit(ctx, event);
    return arg_size;
}

#ifdef ACTRAIL_BPF_LOOP
struct actrail_process_exec_arg_loop {
    struct trace_event_raw_sys_enter *program_ctx;
    struct actrail_pending_process_exec pending;
    __u64 argv_ptr;
    __u32 max_arg_bytes;
    __u32 max_total_arg_bytes;
    __u32 captured_total;
    __u32 stopped;
};

static long process_exec_arg_loop_callback(__u32 index, void *opaque) {
    struct actrail_process_exec_arg_loop *state = opaque;
    __u64 arg_ptr = 0;
    __u32 remaining;
    __u32 read_limit;
    long arg_size;

    if (bpf_probe_read_user(
            &arg_ptr,
            sizeof(arg_ptr),
            (void *)(unsigned long)(state->argv_ptr + ((__u64)index * sizeof(__u64)))
        ) != 0) {
        emit_process_exec_arg(
            state->program_ctx,
            &state->pending,
            index,
            ACTRAIL_PROCESS_EXEC_ARG_MARK_READ_FAILED,
            0
        );
        state->stopped = 1;
        return 1;
    }
    if (!arg_ptr) {
        emit_process_exec_arg(
            state->program_ctx,
            &state->pending,
            index,
            ACTRAIL_PROCESS_EXEC_ARG_MARK_COMPLETE,
            0
        );
        state->stopped = 1;
        return 1;
    }
    if (state->captured_total >= state->max_total_arg_bytes) {
        emit_process_exec_arg(
            state->program_ctx,
            &state->pending,
            index,
            ACTRAIL_PROCESS_EXEC_ARG_MARK_TOTAL,
            0
        );
        state->stopped = 1;
        return 1;
    }
    remaining = state->max_total_arg_bytes - state->captured_total;
    read_limit = state->max_arg_bytes;
    if (read_limit > remaining) {
        read_limit = remaining;
    }
    read_limit++;
    if (read_limit > ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES) {
        read_limit = ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES;
    }
    arg_size = emit_process_exec_arg(
        state->program_ctx,
        &state->pending,
        index,
        arg_ptr,
        read_limit
    );
    if (arg_size <= 0) {
        state->stopped = 1;
        return 1;
    }
    state->captured_total += (__u32)arg_size - 1;
    if ((__u32)arg_size == read_limit) {
        state->stopped = 1;
        return 1;
    }
    return 0;
}
#endif

static __always_inline void emit_process_exec_args(
    struct trace_event_raw_sys_enter *ctx,
    struct actrail_pending_process_exec *pending,
    __u64 argv_ptr
) {
    struct actrail_process_exec_config *config = current_process_exec_config();
    __u32 max_args;
    __u32 max_arg_bytes;
    __u32 max_total_arg_bytes;

    if (!config || !config->max_args || !config->max_arg_bytes ||
        !config->max_total_arg_bytes) {
        return;
    }
    max_args = config->max_args;
    if (max_args > ACTRAIL_PROCESS_EXEC_ARG_MAX) {
        max_args = ACTRAIL_PROCESS_EXEC_ARG_MAX;
    }
    max_arg_bytes = config->max_arg_bytes;
    if (max_arg_bytes > ACTRAIL_PROCESS_EXEC_ARGV_COPY_MAX_BYTES) {
        max_arg_bytes = ACTRAIL_PROCESS_EXEC_ARGV_COPY_MAX_BYTES;
    }
    max_total_arg_bytes = config->max_total_arg_bytes;
    if (max_total_arg_bytes > ACTRAIL_PROCESS_EXEC_ARGV_COPY_MAX_BYTES) {
        max_total_arg_bytes = ACTRAIL_PROCESS_EXEC_ARGV_COPY_MAX_BYTES;
    }

#ifdef ACTRAIL_BPF_LOOP
    struct actrail_process_exec_arg_loop state = {
        .program_ctx = ctx,
        .pending = *pending,
        .argv_ptr = argv_ptr,
        .max_arg_bytes = max_arg_bytes,
        .max_total_arg_bytes = max_total_arg_bytes,
    };
    long loop_result = bpf_loop(
        max_args,
        process_exec_arg_loop_callback,
        &state,
        0
    );

    if (loop_result >= 0 && !state.stopped) {
        emit_process_exec_arg(
            ctx,
            pending,
            max_args,
            ACTRAIL_PROCESS_EXEC_ARG_MARK_LIMIT,
            0
        );
    }
#else
    __u32 captured_total = 0;
#pragma clang loop unroll(full)
    for (__u32 index = 0; index < ACTRAIL_PROCESS_EXEC_ARG_MAX; index++) {
        __u64 arg_ptr = 0;
        __u32 remaining;
        __u32 read_limit;
        long arg_size;

        if (index >= max_args) {
            emit_process_exec_arg(
                ctx, pending, index, ACTRAIL_PROCESS_EXEC_ARG_MARK_LIMIT, 0
            );
            break;
        }
        if (bpf_probe_read_user(
                &arg_ptr,
                sizeof(arg_ptr),
                (void *)(unsigned long)(argv_ptr + ((__u64)index * sizeof(__u64)))
            ) != 0) {
            emit_process_exec_arg(
                ctx, pending, index, ACTRAIL_PROCESS_EXEC_ARG_MARK_READ_FAILED, 0
            );
            break;
        }
        if (!arg_ptr) {
            emit_process_exec_arg(
                ctx, pending, index, ACTRAIL_PROCESS_EXEC_ARG_MARK_COMPLETE, 0
            );
            break;
        }
        if (captured_total >= max_total_arg_bytes) {
            emit_process_exec_arg(
                ctx, pending, index, ACTRAIL_PROCESS_EXEC_ARG_MARK_TOTAL, 0
            );
            break;
        }
        remaining = max_total_arg_bytes - captured_total;
        read_limit = max_arg_bytes;
        if (read_limit > remaining) {
            read_limit = remaining;
        }
        read_limit++;
        if (read_limit > ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES) {
            read_limit = ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES;
        }
        arg_size = emit_process_exec_arg(ctx, pending, index, arg_ptr, read_limit);
        if (arg_size <= 0) {
            break;
        }
        captured_total += (__u32)arg_size - 1;
        if ((__u32)arg_size == read_limit) {
            break;
        }
    }
#endif
}

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

SEC("tracepoint/syscalls/sys_exit_execve")
int handle_sys_exit_execve(struct trace_event_raw_sys_exit *ctx) {
    return emit_process_exec_failure(ctx);
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

SEC("tracepoint/syscalls/sys_exit_execveat")
int handle_sys_exit_execveat(struct trace_event_raw_sys_exit *ctx) {
    return emit_process_exec_failure(ctx);
}

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
    delete_trace_namespace_thread_identity(kernel_pid_tgid);
    bpf_map_delete_elem(&process_exec_sequences, &kernel_pid_tgid);
    bpf_map_delete_elem(&process_fork_sequences, &kernel_pid_tgid);
    bpf_map_delete_elem(&payload_socket_operation_sequence, &kernel_pid_tgid);
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
        delete_file_bulk_read_fast_process(pid, exit_generation);
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
