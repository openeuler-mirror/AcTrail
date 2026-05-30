#ifndef ACTRAIL_TLS_PAYLOAD_H
#define ACTRAIL_TLS_PAYLOAD_H

#include "actrail_runtime.h"
#include "actrail_uprobe_regs.h"

enum actrail_tls_payload_direction {
    ACTRAIL_TLS_PAYLOAD_OUTBOUND = 1,
    ACTRAIL_TLS_PAYLOAD_INBOUND = 2,
};

enum actrail_tls_payload_symbol {
    ACTRAIL_TLS_SYMBOL_SSL_WRITE = 1,
    ACTRAIL_TLS_SYMBOL_SSL_READ = 2,
    ACTRAIL_TLS_SYMBOL_SSL_WRITE_EX = 3,
    ACTRAIL_TLS_SYMBOL_SSL_READ_EX = 4,
    ACTRAIL_TLS_SYMBOL_RUSTLS_WRITE = 5,
    ACTRAIL_TLS_SYMBOL_RUSTLS_WRITE_VECTORED = 6,
};

enum actrail_tls_payload_library {
    ACTRAIL_TLS_LIBRARY_OPENSSL = 1,
    ACTRAIL_TLS_LIBRARY_BORINGSSL = 2,
    ACTRAIL_TLS_LIBRARY_RUSTLS = 3,
};

enum actrail_tls_completion_flags {
    ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED = 2,
};

enum actrail_tls_payload_capture_backend {
    ACTRAIL_TLS_BACKEND_SECCOMP_USER_READ = 1,
    ACTRAIL_TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK = 2,
};

enum actrail_tls_payload_capture_state {
    ACTRAIL_TLS_CAPTURE_STATE_NEEDS_SECCOMP = 1,
    ACTRAIL_TLS_CAPTURE_STATE_BPF_COPIED_FULL = 2,
};

enum actrail_tls_capture_signal {
    ACTRAIL_TLS_CAPTURE_SIGSTOP = 19,
};

enum actrail_tls_payload_copy_limit {
    ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_ABI_BYTES = 4194304,
    ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES = 4194303,
};

enum actrail_tls_payload_diagnostic_counter {
    ACTRAIL_TLS_DIAG_ENTER_TOTAL = 0,
    ACTRAIL_TLS_DIAG_NAMESPACE_FALLBACK = 1,
    ACTRAIL_TLS_DIAG_TRACE_LOOKUP_MISS = 2,
    ACTRAIL_TLS_DIAG_TRACE_LOOKUP_HOST_FALLBACK = 3,
    ACTRAIL_TLS_DIAG_EMPTY_BUFFER = 4,
    ACTRAIL_TLS_DIAG_DIRECT_COPY_ATTEMPT = 5,
    ACTRAIL_TLS_DIAG_DIRECT_COPY_TOO_LARGE = 6,
    ACTRAIL_TLS_DIAG_DIRECT_RESERVE_FAIL = 7,
    ACTRAIL_TLS_DIAG_DIRECT_READ_FAIL = 8,
    ACTRAIL_TLS_DIAG_DIRECT_SUBMIT_OK = 9,
    ACTRAIL_TLS_DIAG_PENDING_UPDATE_FAIL = 10,
    ACTRAIL_TLS_DIAG_PENDING_UPDATE_OK = 11,
    ACTRAIL_TLS_DIAG_CAPTURE_REQUEST_RESERVE_FAIL = 12,
    ACTRAIL_TLS_DIAG_CAPTURE_REQUEST_SIGNAL_FAIL = 13,
    ACTRAIL_TLS_DIAG_CAPTURE_REQUEST_SUBMIT_OK = 14,
    ACTRAIL_TLS_DIAG_COMPLETION_TOTAL = 15,
    ACTRAIL_TLS_DIAG_COMPLETION_MISSING_PENDING = 16,
    ACTRAIL_TLS_DIAG_COMPLETION_RESERVE_FAIL = 17,
    ACTRAIL_TLS_DIAG_COMPLETION_SUBMIT_OK = 18,
    ACTRAIL_TLS_DIAG_COUNTER_COUNT = 19,
};

struct actrail_tls_payload_config {
    __u32 library;
    __u32 capture_backend;
    __u32 max_segment_bytes;
    __u32 diagnostics_enabled;
};

struct actrail_pending_tls_payload_op {
    __u64 trace_id;
    __u64 operation_id;
    __u64 stream_key;
    __u64 buffer_ptr;
    __u64 requested_size;
    __u64 size_ptr;
    __u64 pid_generation;
    __u32 direction;
    __u32 symbol;
    __u32 library;
    __u32 capture_state;
};

struct actrail_tls_completion_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 stream_key;
    __u64 operation_id;
    __u32 completed_size;
    __u32 flags;
    __u32 symbol;
    __u32 library;
    __u64 pid_generation;
    __u64 buffer_ptr;
};

struct actrail_tls_capture_request_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 stream_key;
    __u64 operation_id;
    __u64 requested_size;
    __u64 buffer_ptr;
    __u64 pid_generation;
    __u32 symbol;
    __u32 library;
};

struct actrail_tls_direct_capture_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 stream_key;
    __u64 operation_id;
    __u32 original_size;
    __u32 captured_size;
    __u32 flags;
    __u32 symbol;
    __u32 library;
    __u32 reserved;
    __u64 pid_generation;
    __u8 bytes[ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_ABI_BYTES];
};

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_tls_payload_config);
} payload_tls_config SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, ACTRAIL_TLS_DIAG_COUNTER_COUNT);
    __type(key, __u32);
    __type(value, __u64);
} payload_tls_diagnostics SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_tls_payload_op);
} pending_tls_payload_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, __u64);
} tls_pending_ns SEC(".maps");

static __always_inline __u32 payload_tls_library(void) {
    __u32 key = 0;
    struct actrail_tls_payload_config *config =
        bpf_map_lookup_elem(&payload_tls_config, &key);

    if (!config) {
        return 0;
    }
    return config->library;
}

static __always_inline __u32 payload_tls_diagnostics_enabled(void) {
    __u32 key = 0;
    struct actrail_tls_payload_config *config =
        bpf_map_lookup_elem(&payload_tls_config, &key);

    if (!config) {
        return 0;
    }
    return config->diagnostics_enabled;
}

static __always_inline void tls_diag_inc(__u32 counter_id) {
    __u64 *counter;

    if (!payload_tls_diagnostics_enabled()) {
        return;
    }
    if (counter_id >= ACTRAIL_TLS_DIAG_COUNTER_COUNT) {
        return;
    }
    counter = bpf_map_lookup_elem(&payload_tls_diagnostics, &counter_id);
    if (!counter) {
        return;
    }
    __sync_fetch_and_add(counter, 1);
}

static __always_inline __u32 payload_tls_capture_backend(void) {
    __u32 key = 0;
    struct actrail_tls_payload_config *config =
        bpf_map_lookup_elem(&payload_tls_config, &key);

    if (!config) {
        return 0;
    }
    return config->capture_backend;
}

static __always_inline __u32 payload_tls_direct_copy_limit(void) {
    __u32 key = 0;
    struct actrail_tls_payload_config *config =
        bpf_map_lookup_elem(&payload_tls_config, &key);

    if (!config) {
        return 0;
    }
    if (config->max_segment_bytes > ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES) {
        return ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES;
    }
    return config->max_segment_bytes;
}

static __always_inline __u32 tls_op_metadata(__u32 direction, __u32 symbol) {
    return direction | (symbol << 16);
}

#include "tls/actrail_tls_payload_capture.h"
#include "tls/actrail_tls_payload_diagnostics.h"

static __always_inline int store_tls_payload_op(
    __u32 metadata,
    __u64 stream_key,
    __u64 buffer_ptr,
    __u64 requested_size,
    __u64 size_ptr
) {
    __u64 host_pid_tgid = current_pid_tgid();
    __u64 namespace_pid_tgid = current_namespace_pid_tgid();
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id = lookup_current_trace(&tgid, &tid, &lookup_flags);
    struct actrail_pending_tls_payload_op op = {};

    tls_diag_inc(ACTRAIL_TLS_DIAG_ENTER_TOTAL);
    if (!namespace_pid_tgid) {
        namespace_pid_tgid = host_pid_tgid;
        tls_diag_inc(ACTRAIL_TLS_DIAG_NAMESPACE_FALLBACK);
    }
    if (lookup_flags & ACTRAIL_TRACE_LOOKUP_FLAG_HOST_FALLBACK) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_TRACE_LOOKUP_HOST_FALLBACK);
        emit_tls_payload_diagnostic_event(
            ACTRAIL_TLS_DIAG_EVENT_TRACE_LOOKUP_HOST_FALLBACK,
            host_pid_tgid,
            namespace_pid_tgid,
            metadata,
            lookup_flags,
            requested_size,
            buffer_ptr
        );
    }
    if (!trace_id) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_TRACE_LOOKUP_MISS);
        emit_tls_payload_diagnostic_event(
            ACTRAIL_TLS_DIAG_EVENT_TRACE_LOOKUP_MISS,
            host_pid_tgid,
            namespace_pid_tgid,
            metadata,
            lookup_flags,
            requested_size,
            buffer_ptr
        );
        return 0;
    }
    if (!buffer_ptr) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_EMPTY_BUFFER);
        emit_tls_payload_diagnostic_event(
            ACTRAIL_TLS_DIAG_EVENT_EMPTY_BUFFER,
            host_pid_tgid,
            namespace_pid_tgid,
            metadata,
            lookup_flags,
            requested_size,
            buffer_ptr
        );
        return 0;
    }

    op.trace_id = *trace_id;
    op.operation_id = bpf_ktime_get_ns() ^ host_pid_tgid;
    op.stream_key = stream_key;
    op.buffer_ptr = buffer_ptr;
    op.requested_size = requested_size;
    op.size_ptr = size_ptr;
    op.pid_generation = ensure_process_generation(tgid);
    op.direction = metadata & 0xffff;
    op.symbol = metadata >> 16;
    op.library = payload_tls_library();
    op.capture_state = ACTRAIL_TLS_CAPTURE_STATE_NEEDS_SECCOMP;
    if (op.direction == ACTRAIL_TLS_PAYLOAD_OUTBOUND &&
        payload_tls_capture_backend() == ACTRAIL_TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK &&
        emit_tls_direct_capture(&op, tgid, tid, op.requested_size) == 1) {
        op.capture_state = ACTRAIL_TLS_CAPTURE_STATE_BPF_COPIED_FULL;
    }
    if (bpf_map_update_elem(&pending_tls_payload_ops, &host_pid_tgid, &op, BPF_ANY) != 0) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_PENDING_UPDATE_FAIL);
        emit_tls_payload_diagnostic_event(
            ACTRAIL_TLS_DIAG_EVENT_PENDING_UPDATE_FAIL,
            host_pid_tgid,
            namespace_pid_tgid,
            metadata,
            lookup_flags,
            requested_size,
            buffer_ptr
        );
        return 0;
    }
    tls_diag_inc(ACTRAIL_TLS_DIAG_PENDING_UPDATE_OK);
    if (bpf_map_update_elem(&tls_pending_ns, &namespace_pid_tgid, &host_pid_tgid, BPF_ANY) != 0) {
        emit_tls_payload_diagnostic_event(
            ACTRAIL_TLS_DIAG_EVENT_PENDING_NAMESPACE_UPDATE_FAIL,
            host_pid_tgid,
            namespace_pid_tgid,
            metadata,
            lookup_flags,
            requested_size,
            buffer_ptr
        );
        bpf_map_delete_elem(&pending_tls_payload_ops, &host_pid_tgid);
        return 0;
    }
    if (op.direction == ACTRAIL_TLS_PAYLOAD_OUTBOUND &&
        payload_tls_capture_backend() == ACTRAIL_TLS_BACKEND_SECCOMP_USER_READ) {
        emit_tls_capture_request(&op, tgid, tid, op.requested_size);
    }
    return 0;
}

#include "tls/actrail_tls_payload_completion.h"

static __always_inline int emit_tls_payload_completion(__u64 completed_size, __u32 flags) {
    __u64 host_pid_tgid = current_pid_tgid();
    __u64 namespace_pid_tgid = current_namespace_pid_tgid();
    __u32 tgid = namespace_pid_tgid >> 32;
    __u32 tid = (__u32)namespace_pid_tgid;
    struct actrail_pending_tls_payload_op *op =
        bpf_map_lookup_elem(&pending_tls_payload_ops, &host_pid_tgid);
    struct actrail_tls_completion_event *event;

    tls_diag_inc(ACTRAIL_TLS_DIAG_COMPLETION_TOTAL);
    if (!namespace_pid_tgid) {
        namespace_pid_tgid = host_pid_tgid;
        tgid = namespace_pid_tgid >> 32;
        tid = (__u32)namespace_pid_tgid;
    }

    if (!op) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_COMPLETION_MISSING_PENDING);
        emit_tls_payload_diagnostic_event(
            ACTRAIL_TLS_DIAG_EVENT_COMPLETION_MISSING_PENDING,
            host_pid_tgid,
            namespace_pid_tgid,
            0,
            0,
            completed_size,
            0
        );
        bpf_map_delete_elem(&pending_tls_payload_ops, &host_pid_tgid);
        bpf_map_delete_elem(&tls_pending_ns, &namespace_pid_tgid);
        return 0;
    }

    capture_tls_payload_after_completion(op, tgid, tid, completed_size, flags);

    event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_COMPLETION_RESERVE_FAIL);
        bpf_map_delete_elem(&pending_tls_payload_ops, &host_pid_tgid);
        bpf_map_delete_elem(&tls_pending_ns, &namespace_pid_tgid);
        return 0;
    }

    event->kind = ACTRAIL_TLS_PAYLOAD_COMPLETION;
    event->pid = tgid;
    event->tid = tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->stream_key = op->stream_key;
    event->operation_id = op->operation_id;
    event->completed_size = completed_size > 0xffffffffULL ? 0xffffffffU : (__u32)completed_size;
    event->flags = flags;
    event->symbol = op->symbol;
    event->library = op->library;
    event->pid_generation = op->pid_generation;
    event->buffer_ptr = op->buffer_ptr;
    bpf_ringbuf_submit(event, 0);
    tls_diag_inc(ACTRAIL_TLS_DIAG_COMPLETION_SUBMIT_OK);
    bpf_map_delete_elem(&pending_tls_payload_ops, &host_pid_tgid);
    bpf_map_delete_elem(&tls_pending_ns, &namespace_pid_tgid);
    return 0;
}

static __always_inline int emit_tls_payload_completion_from_return(struct pt_regs *ctx) {
    int result = (int)ACTRAIL_UPROBE_RET(ctx);

    if (result <= 0) {
        return emit_tls_payload_completion(0, ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED);
    }
    return emit_tls_payload_completion((__u64)result, 0);
}

static __always_inline int emit_tls_payload_completion_from_size_ptr(struct pt_regs *ctx) {
    __u64 pid_tgid = current_pid_tgid();
    struct actrail_pending_tls_payload_op *op =
        bpf_map_lookup_elem(&pending_tls_payload_ops, &pid_tgid);
    __u64 written = 0;
    long result = (long)ACTRAIL_UPROBE_RET(ctx);

    if (result != 1 || !op || !op->size_ptr) {
        return emit_tls_payload_completion(0, ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED);
    }
    if (bpf_probe_read_user(&written, sizeof(written), (void *)(unsigned long)op->size_ptr) != 0) {
        return emit_tls_payload_completion(0, ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED);
    }
    return emit_tls_payload_completion(written, 0);
}

static __always_inline int emit_tls_payload_completion_from_rust_result_usize(struct pt_regs *ctx) {
    __u64 result_tag = ACTRAIL_UPROBE_RET(ctx);

    if (result_tag != 0) {
        return emit_tls_payload_completion(0, ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED);
    }
    return emit_tls_payload_completion(ACTRAIL_UPROBE_RET2(ctx), 0);
}

#include "tls/actrail_tls_payload_probes.h"

#endif
