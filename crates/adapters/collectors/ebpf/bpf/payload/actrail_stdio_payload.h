#ifndef ACTRAIL_STDIO_PAYLOAD_H
#define ACTRAIL_STDIO_PAYLOAD_H

#include "actrail_runtime.h"

enum actrail_stdio_payload_abi {
    ACTRAIL_STDIO_PAYLOAD_ABI_MAX_BYTES = 4096,
    ACTRAIL_STDIO_PAYLOAD_COPY_MAX_BYTES = 4095,
};

enum actrail_stdio_payload_direction {
    ACTRAIL_STDIO_PAYLOAD_INBOUND = 1,
    ACTRAIL_STDIO_PAYLOAD_OUTBOUND = 2,
};

enum actrail_stdio_payload_stream {
    ACTRAIL_STDIO_STREAM_STDIN = 0,
    ACTRAIL_STDIO_STREAM_STDOUT = 1,
    ACTRAIL_STDIO_STREAM_STDERR = 2,
};

enum actrail_stdio_payload_syscall {
    ACTRAIL_STDIO_SYSCALL_READ = 1,
    ACTRAIL_STDIO_SYSCALL_WRITE = 2,
};

enum actrail_stdio_payload_flags {
    ACTRAIL_STDIO_PAYLOAD_TRUNCATED = 1,
};

struct actrail_stdio_payload_config {
    __u32 enabled;
    __u32 capture_stdin;
    __u32 capture_stdout;
    __u32 capture_stderr;
    __u32 max_segment_bytes;
};

struct actrail_pending_stdio_payload_op {
    __u64 trace_id;
    __u64 buffer_ptr;
    __u64 requested_size;
    __u32 fd;
    __u32 stream;
    __u32 direction;
    __u32 syscall;
};

struct actrail_stdio_stream_sequence_key {
    __u32 pid;
    __u32 stream;
};

struct actrail_stdio_payload_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 sequence;
    __u32 stream;
    __u32 original_size;
    __u32 captured_size;
    __u32 flags;
    __u32 fd;
    __u32 syscall;
    __u64 pid_generation;
    __u8 bytes[ACTRAIL_STDIO_PAYLOAD_ABI_MAX_BYTES];
};

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_stdio_payload_config);
} payload_stdio_config SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_stdio_payload_op);
} pending_stdio_payload_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_stdio_stream_sequence_key);
    __type(value, __u64);
} payload_stdio_stream_sequences SEC(".maps");

static __always_inline struct actrail_stdio_payload_config *stdio_payload_config(void) {
    __u32 key = 0;
    return bpf_map_lookup_elem(&payload_stdio_config, &key);
}

static __always_inline __u32 payload_stdio_capture_limit(void) {
    struct actrail_stdio_payload_config *config = stdio_payload_config();
    __u32 limit;

    if (!config || !config->enabled) {
        return 0;
    }
    limit = config->max_segment_bytes;
    if (limit > ACTRAIL_STDIO_PAYLOAD_COPY_MAX_BYTES) {
        return ACTRAIL_STDIO_PAYLOAD_COPY_MAX_BYTES;
    }
    return limit;
}

static __always_inline __u64 next_stdio_payload_sequence(__u32 pid, __u32 stream) {
    struct actrail_stdio_stream_sequence_key key = {};
    __u64 initial = 1;
    __u64 next;
    __u64 *current;

    key.pid = pid;
    key.stream = stream;
    current = bpf_map_lookup_elem(&payload_stdio_stream_sequences, &key);
    if (!current) {
        bpf_map_update_elem(&payload_stdio_stream_sequences, &key, &initial, BPF_ANY);
        return initial;
    }

    next = *current + 1;
    bpf_map_update_elem(&payload_stdio_stream_sequences, &key, &next, BPF_ANY);
    return next;
}

static __always_inline int store_stdio_payload_op(
    struct trace_event_raw_sys_enter *ctx,
    __u32 syscall
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_stdio_payload_config *config = stdio_payload_config();
    struct actrail_pending_stdio_payload_op op = {};
    __u32 fd = (__u32)ctx->args[0];

    if (!tgid || !trace_id || !config || !config->enabled || !ctx->args[1] || !ctx->args[2]) {
        return 0;
    }

    if (syscall == ACTRAIL_STDIO_SYSCALL_READ
        && fd == ACTRAIL_STDIO_STREAM_STDIN
        && config->capture_stdin) {
        op.stream = ACTRAIL_STDIO_STREAM_STDIN;
        op.direction = ACTRAIL_STDIO_PAYLOAD_INBOUND;
    } else if (syscall == ACTRAIL_STDIO_SYSCALL_WRITE
        && fd == ACTRAIL_STDIO_STREAM_STDOUT
        && config->capture_stdout) {
        op.stream = ACTRAIL_STDIO_STREAM_STDOUT;
        op.direction = ACTRAIL_STDIO_PAYLOAD_OUTBOUND;
    } else if (syscall == ACTRAIL_STDIO_SYSCALL_WRITE
        && fd == ACTRAIL_STDIO_STREAM_STDERR
        && config->capture_stderr) {
        op.stream = ACTRAIL_STDIO_STREAM_STDERR;
        op.direction = ACTRAIL_STDIO_PAYLOAD_OUTBOUND;
    } else {
        return 0;
    }

    op.trace_id = *trace_id;
    op.buffer_ptr = (__u64)ctx->args[1];
    op.requested_size = (__u64)ctx->args[2];
    op.fd = fd;
    op.syscall = syscall;
    bpf_map_update_elem(&pending_stdio_payload_ops, &pid_tgid, &op, BPF_ANY);
    return 0;
}

static __always_inline int emit_stdio_payload_op(struct trace_event_raw_sys_exit *ctx) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u32 tid = (__u32)pid_tgid;
    struct actrail_pending_stdio_payload_op *op =
        bpf_map_lookup_elem(&pending_stdio_payload_ops, &pid_tgid);
    struct actrail_stdio_payload_event *event;
    __u64 original_size = (__u64)ctx->ret;
    __u64 bounded_size;
    __u32 capture_size;
    __u32 limit = payload_stdio_capture_limit();

    if (!tgid || !op || ctx->ret <= 0 || !limit) {
        bpf_map_delete_elem(&pending_stdio_payload_ops, &pid_tgid);
        return 0;
    }

    bounded_size = original_size & ACTRAIL_STDIO_PAYLOAD_COPY_MAX_BYTES;
    if (original_size > ACTRAIL_STDIO_PAYLOAD_COPY_MAX_BYTES) {
        bounded_size = ACTRAIL_STDIO_PAYLOAD_COPY_MAX_BYTES;
    }
    if (bounded_size > limit) {
        bounded_size = limit;
    }
    actrail_barrier_var(bounded_size);
    bounded_size &= ACTRAIL_STDIO_PAYLOAD_COPY_MAX_BYTES;
    capture_size = (__u32)bounded_size;
    if (!capture_size) {
        bpf_map_delete_elem(&pending_stdio_payload_ops, &pid_tgid);
        return 0;
    }

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        bpf_map_delete_elem(&pending_stdio_payload_ops, &pid_tgid);
        return 0;
    }

    event->kind = ACTRAIL_STDIO_PAYLOAD;
    event->pid = tgid;
    event->tid = tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->sequence = next_stdio_payload_sequence(tgid, op->stream);
    event->stream = op->stream;
    event->original_size = (__u32)original_size;
    event->captured_size = capture_size;
    event->flags = original_size > bounded_size ? ACTRAIL_STDIO_PAYLOAD_TRUNCATED : 0;
    event->fd = op->fd;
    event->syscall = op->syscall;
    event->pid_generation = ensure_process_generation(tgid);
    if (bpf_probe_read_user(event->bytes, bounded_size, (void *)(unsigned long)op->buffer_ptr) != 0) {
        actrail_event_discard(event);
        bpf_map_delete_elem(&pending_stdio_payload_ops, &pid_tgid);
        return 0;
    }

    actrail_event_submit(ctx, event);
    bpf_map_delete_elem(&pending_stdio_payload_ops, &pid_tgid);
    return 0;
}

#endif
