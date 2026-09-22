#ifndef ACTRAIL_PROCESS_PROGRAMS_H
#define ACTRAIL_PROCESS_PROGRAMS_H

#include "../fd/lifecycle.h"
#include "../file/observe.h"
#include "../file/io/state.h"
#include "../payload/socket_types.h"
#include "observe.h"

#ifdef ACTRAIL_BPF_LOOP
#define ACTRAIL_PROCESS_EXEC_HELPER __noinline
#else
/* Tail calls are rejected on this kernel when the caller still has
 * BPF-to-BPF calls. Inline the two exec helpers in the pre-5.17 fallback so
 * the entry and continuation programs contain no subprogram calls. */
#define ACTRAIL_PROCESS_EXEC_HELPER __always_inline
#endif

static ACTRAIL_PROCESS_EXEC_HELPER int emit_process_exec_attempt(
    struct trace_event_raw_sys_enter *ctx,
    struct actrail_pending_process_exec *pending,
    __u64 path_ptr,
    __s32 execveat_dirfd,
    __u32 execveat_flags
) {
    struct actrail_process_exec_config *config = current_process_exec_config();
    __u32 scratch_key = 0;
    union actrail_process_exec_scratch *scratch;
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

    scratch = bpf_map_lookup_elem(&process_exec_scratch, &scratch_key);
    if (!scratch) {
        event_transport_diag_inc(ACTRAIL_EVENT_TRANSPORT_RESERVE_FAIL);
        return 0;
    }
    event = &scratch->attempt;
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

    __u64 event_size = __builtin_offsetof(struct actrail_process_exec_attempt_event, path)
        + (__u64)event->path_size;
    if (event_size > sizeof(*event)) {
        event_size = sizeof(*event);
    }
    return emit_event(ctx, event, event_size) == 0;
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

static ACTRAIL_PROCESS_EXEC_HELPER long emit_process_exec_arg(
    struct trace_event_raw_sys_enter *ctx,
    struct actrail_pending_process_exec *pending,
    __u32 index,
    __u64 arg_ptr,
    __u32 read_limit
) {
    __u32 scratch_key = 0;
    union actrail_process_exec_scratch *scratch =
        bpf_map_lookup_elem(&process_exec_scratch, &scratch_key);
    struct actrail_process_exec_arg_event *event;
    long arg_size;

    if (!scratch) {
        event_transport_diag_inc(ACTRAIL_EVENT_TRANSPORT_RESERVE_FAIL);
        return -1;
    }
    event = &scratch->arg;
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
        emit_event(ctx, event, __builtin_offsetof(struct actrail_process_exec_arg_event, arg));
        return 0;
    }

#ifdef ACTRAIL_BPF_LOOP
    /* Mask to u32 and clamp to the ABI maximum before the helper call; old
     * verifiers otherwise track the noinline parameter with a possibly
     * negative 64-bit range and reject the load. */
    __u64 bounded_read_limit = (__u64)read_limit & 0xffffffffULL;
    if (bounded_read_limit > ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES) {
        bounded_read_limit = ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES;
    }
    actrail_barrier_var(bounded_read_limit);
#else
    /* The fallback helper is inlined to avoid BPF-to-BPF calls in tail-call
     * programs. A fixed upper bound keeps the verifier from tracking the
     * caller's wrapped 32-bit arithmetic as an unbounded helper size. The
     * output is still truncated to read_limit below. */
    __u32 bounded_read_limit = ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES;
#endif
    arg_size = bpf_probe_read_user_str(
        event->arg,
        bounded_read_limit,
        (void *)(unsigned long)arg_ptr
    );
    if (arg_size < 0) {
        event->capture_flags = ACTRAIL_PROCESS_EXEC_READ_FAILED;
        emit_event(ctx, event, __builtin_offsetof(struct actrail_process_exec_arg_event, arg));
        return -1;
    }
    if (arg_size > 0) {
        __u32 captured_size = (__u32)arg_size;
#ifndef ACTRAIL_BPF_LOOP
        if (captured_size > read_limit) {
            captured_size = read_limit;
        }
#endif
        event->arg_size = captured_size ? captured_size - 1 : 0;
    }
    if ((__u32)arg_size >= read_limit) {
        event->capture_flags = ACTRAIL_PROCESS_EXEC_ARGV_TRUNCATED;
    }
    __u64 event_size = __builtin_offsetof(struct actrail_process_exec_arg_event, arg)
        + (__u64)event->arg_size;
    if (event_size > sizeof(*event)) {
        event_size = sizeof(*event);
    }
    if (emit_event(ctx, event, event_size) != 0) {
        return -1;
    }
    /* Some kernels return zero for a successfully read empty string. Keep
     * walking argv: only the marker path or a read failure ends the walk. */
    return arg_size ? arg_size : 1;
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

#ifndef ACTRAIL_BPF_LOOP
static __always_inline void process_exec_argv_chunk(
    struct trace_event_raw_sys_enter *ctx,
    struct actrail_pending_process_exec *pending
) {
    __u32 captured_total = pending->argv_captured_total;
    __u32 chunk_index;

#pragma clang loop unroll(disable)
    for (chunk_index = 0; chunk_index < ACTRAIL_PROCESS_EXEC_ARG_CHUNK;
         chunk_index++) {
        __u32 index = pending->argv_index;
        __u64 arg_ptr = 0;
        __u32 remaining;
        __u32 read_limit;
        long arg_size;

        if (index >= pending->argv_max_args ||
            index >= ACTRAIL_PROCESS_EXEC_ARG_MAX) {
            emit_process_exec_arg(
                ctx, pending, index, ACTRAIL_PROCESS_EXEC_ARG_MARK_LIMIT, 0
            );
            pending->argv_stopped = 1;
            return;
        }
        if (captured_total >= pending->argv_max_total_arg_bytes) {
            emit_process_exec_arg(
                ctx, pending, index, ACTRAIL_PROCESS_EXEC_ARG_MARK_TOTAL, 0
            );
            pending->argv_stopped = 1;
            return;
        }
        if (bpf_probe_read_user(
                &arg_ptr,
                sizeof(arg_ptr),
                (void *)(unsigned long)(
                    pending->argv_ptr + ((__u64)index * sizeof(__u64))
                )
            ) != 0) {
            emit_process_exec_arg(
                ctx,
                pending,
                index,
                ACTRAIL_PROCESS_EXEC_ARG_MARK_READ_FAILED,
                0
            );
            pending->argv_stopped = 1;
            return;
        }
        if (!arg_ptr) {
            emit_process_exec_arg(
                ctx,
                pending,
                index,
                ACTRAIL_PROCESS_EXEC_ARG_MARK_COMPLETE,
                0
            );
            pending->argv_stopped = 1;
            return;
        }
        remaining = pending->argv_max_total_arg_bytes - captured_total;
        read_limit = pending->argv_max_arg_bytes;
        if (read_limit > remaining) {
            read_limit = remaining;
        }
        read_limit++;
        if (read_limit > ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES) {
            read_limit = ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES;
        }
        arg_size = emit_process_exec_arg(ctx, pending, index, arg_ptr, read_limit);
        if (arg_size <= 0) {
            pending->argv_stopped = 1;
            return;
        }
        captured_total += (__u32)arg_size - 1;
        pending->argv_index = index + 1;
        pending->argv_captured_total = captured_total;
        if ((__u32)arg_size == read_limit) {
            pending->argv_stopped = 1;
            return;
        }
    }

    if (pending->argv_index >= pending->argv_max_args ||
        pending->argv_index >= ACTRAIL_PROCESS_EXEC_ARG_MAX) {
        emit_process_exec_arg(
            ctx,
            pending,
            pending->argv_index,
            ACTRAIL_PROCESS_EXEC_ARG_MARK_LIMIT,
            0
        );
        pending->argv_stopped = 1;
    }
}

static __always_inline void process_exec_argv_persist_cursor(
    struct actrail_pending_process_exec *target,
    const struct actrail_pending_process_exec *source
) {
    target->argv_ptr = source->argv_ptr;
    target->argv_index = source->argv_index;
    target->argv_captured_total = source->argv_captured_total;
    target->argv_stopped = source->argv_stopped;
    target->argv_max_args = source->argv_max_args;
    target->argv_max_arg_bytes = source->argv_max_arg_bytes;
    target->argv_max_total_arg_bytes = source->argv_max_total_arg_bytes;
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
    pending->argv_ptr = argv_ptr;
    pending->argv_index = 0;
    pending->argv_captured_total = 0;
    pending->argv_stopped = 0;
    pending->argv_max_args = max_args;
    pending->argv_max_arg_bytes = max_arg_bytes;
    pending->argv_max_total_arg_bytes = max_total_arg_bytes;

    process_exec_argv_chunk(ctx, pending);
    if (pending->argv_stopped) {
        return;
    }

    {
        __u64 kernel_pid_tgid = current_kernel_pid_tgid();
        struct actrail_pending_process_exec *stored =
            bpf_map_lookup_elem(&pending_process_exec_ops, &kernel_pid_tgid);

        if (!stored) {
            emit_process_exec_arg(
                ctx,
                pending,
                pending->argv_index,
                ACTRAIL_PROCESS_EXEC_ARG_MARK_LIMIT,
                0
            );
            pending->argv_stopped = 1;
            return;
        }
        process_exec_argv_persist_cursor(stored, pending);
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
#endif
}

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

#include "lifecycle_programs.h"

#endif
