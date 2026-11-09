/* Process argv observation helpers. */
#ifndef ACTRAIL_PROCESS_OBSERVATION_ARGV_H
#define ACTRAIL_PROCESS_OBSERVATION_ARGV_H

#include "exec_emit.h"

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

#endif
