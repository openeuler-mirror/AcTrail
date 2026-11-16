/* Process exec_emit observation helpers. */
#ifndef ACTRAIL_PROCESS_OBSERVATION_EXEC_EMIT_H
#define ACTRAIL_PROCESS_OBSERVATION_EXEC_EMIT_H

#include "../observe.h"

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

#endif
