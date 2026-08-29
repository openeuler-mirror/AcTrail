#ifndef ACTRAIL_TLS_STATE_H
#define ACTRAIL_TLS_STATE_H

#include "../abi/payload.h"
#include "../runtime/event_transport.h"
#include "../common/uprobe_registers.h"

enum actrail_tls_payload_capture_backend {
    ACTRAIL_TLS_BACKEND_SECCOMP_USER_READ = 1,
    ACTRAIL_TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK = 2,
    ACTRAIL_TLS_BACKEND_BPF_COPY_ONLY = 3,
};

enum actrail_tls_payload_capture_state {
    ACTRAIL_TLS_CAPTURE_STATE_NEEDS_SECCOMP = 1,
    ACTRAIL_TLS_CAPTURE_STATE_BPF_COPIED_FULL = 2,
};

enum actrail_tls_capture_signal {
    ACTRAIL_TLS_CAPTURE_SIGCONT = 18,
    ACTRAIL_TLS_CAPTURE_SIGSTOP = 19,
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
    __u32 max_operation_bytes;
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

struct actrail_tls_payload_op_args {
    __u32 metadata;
    __u64 stream_key;
    __u64 buffer_ptr;
    __u64 requested_size;
    __u64 size_ptr;
};

struct actrail_tls_immediate_payload_args {
    __u32 direction;
    __u32 symbol;
    __u32 library;
    __u64 stream_key;
    __u64 buffer_ptr;
    __u64 requested_size;
};

struct actrail_go_tls_read_buffer_key {
    __u32 tgid;
    __u32 reserved;
    __u64 buffer_ptr;
};

struct actrail_go_tls_read_buffer {
    __u64 stream_key;
    __u64 requested_size;
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

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_go_tls_read_buffer_key);
    __type(value, struct actrail_go_tls_read_buffer);
} go_tls_read_buffers SEC(".maps");

static __always_inline __u32 payload_tls_library(void) {
    __u32 key = 0;
    struct actrail_tls_payload_config *config =
        bpf_map_lookup_elem(&payload_tls_config, &key);

    if (!config) {
        return 0;
    }
    return config->library;
}

static __always_inline __u32 payload_tls_library_for_symbol(__u32 symbol) {
    __u32 configured = payload_tls_library();

    if (configured != 0) {
        return configured;
    }
    if (symbol >= ACTRAIL_TLS_SYMBOL_SSL_WRITE &&
        symbol <= ACTRAIL_TLS_SYMBOL_SSL_READ_EX) {
        return ACTRAIL_TLS_LIBRARY_OPENSSL;
    }
    if (symbol >= ACTRAIL_TLS_SYMBOL_RUSTLS_WRITE &&
        symbol <= ACTRAIL_TLS_SYMBOL_RUSTLS_WRITE_VECTORED) {
        return ACTRAIL_TLS_LIBRARY_RUSTLS;
    }
    if (symbol >= ACTRAIL_TLS_SYMBOL_GO_CONN_WRITE &&
        symbol <= ACTRAIL_TLS_SYMBOL_GO_CONN_READ) {
        return ACTRAIL_TLS_LIBRARY_GO;
    }
    if (symbol >= ACTRAIL_TLS_SYMBOL_GNUTLS_RECORD_SEND &&
        symbol <= ACTRAIL_TLS_SYMBOL_GNUTLS_RECORD_RECV) {
        return ACTRAIL_TLS_LIBRARY_GNUTLS;
    }
    if (symbol >= ACTRAIL_TLS_SYMBOL_NSPR_PR_WRITE &&
        symbol <= ACTRAIL_TLS_SYMBOL_NSPR_PR_RECV) {
        return ACTRAIL_TLS_LIBRARY_NSS;
    }
    if (symbol >= ACTRAIL_TLS_SYMBOL_RUSTLS_BUFFER_PLAINTEXT &&
        symbol <= ACTRAIL_TLS_SYMBOL_RUSTLS_TAKE_RECEIVED_PLAINTEXT) {
        return ACTRAIL_TLS_LIBRARY_RUSTLS;
    }
    return 0;
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

static __always_inline int payload_tls_bpf_copy_enabled(void) {
    __u32 backend = payload_tls_capture_backend();

    return backend == ACTRAIL_TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK ||
        backend == ACTRAIL_TLS_BACKEND_BPF_COPY_ONLY;
}

static __always_inline __u32 payload_tls_direct_copy_limit(void) {
    __u32 key = 0;
    struct actrail_tls_payload_config *config =
        bpf_map_lookup_elem(&payload_tls_config, &key);

    if (!config) {
        return 0;
    }
    if (config->max_operation_bytes > ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES) {
        return ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES;
    }
    return config->max_operation_bytes;
}

static __always_inline __u32 tls_op_metadata(__u32 direction, __u32 symbol) {
    return direction | (symbol << 16);
}

static __always_inline __u64 positive_uprobe_isize(unsigned long value) {
    long signed_value = (long)value;

    if (signed_value <= 0) {
        return 0;
    }
    return (__u64)signed_value;
}

static __always_inline __u64 positive_uprobe_i32(unsigned long value) {
    __s32 signed_value = (__s32)(__u32)value;

    if (signed_value <= 0) {
        return 0;
    }
    return (__u64)(__u32)signed_value;
}



#endif
