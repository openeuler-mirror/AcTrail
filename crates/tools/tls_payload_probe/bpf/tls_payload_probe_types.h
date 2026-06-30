#ifndef TLS_PAYLOAD_PROBE_TYPES_H
#define TLS_PAYLOAD_PROBE_TYPES_H

#include "tls_payload_probe_helpers.h"

#define TLS_PROBE_RUSTLS_INLINE_TAG 0ULL
#define TLS_PROBE_RUSTLS_BORROWED_TAG 0x8000000000000000ULL

enum tls_probe_provider {
    TLS_PROBE_PROVIDER_OPENSSL = 1,
    TLS_PROBE_PROVIDER_BORINGSSL = 2,
    TLS_PROBE_PROVIDER_RUSTLS = 3,
};

enum tls_probe_direction {
    TLS_PROBE_DIRECTION_OUTBOUND = 1,
    TLS_PROBE_DIRECTION_INBOUND = 2,
};

enum tls_probe_symbol {
    TLS_PROBE_SYMBOL_SSL_WRITE = 1,
    TLS_PROBE_SYMBOL_SSL_READ = 2,
    TLS_PROBE_SYMBOL_SSL_WRITE_EX = 3,
    TLS_PROBE_SYMBOL_SSL_READ_EX = 4,
    TLS_PROBE_SYMBOL_RUSTLS_BUFFER_PLAINTEXT = 5,
    TLS_PROBE_SYMBOL_RUSTLS_TAKE_RECEIVED_PLAINTEXT = 6,
};

enum tls_probe_event_constants {
    TLS_PROBE_EVENT_PAYLOAD = 1,
    TLS_PROBE_EVENT_FLAG_TRUNCATED = 1,
    TLS_PROBE_EVENT_FLAG_RUSTLS_CHUNK = 2,
    TLS_PROBE_EVENT_HEADER_BYTES = 72,
    TLS_PROBE_DEFAULT_MAX_CAPTURE_BYTES = 65535,
    TLS_PROBE_ABI_MAX_CAPTURE_BYTES = 65535,
    TLS_PROBE_DEFAULT_RING_BUFFER_BYTES = 4194304,
    TLS_PROBE_RUSTLS_MAX_CHUNKS = 8,
    TLS_PROBE_MAX_SEGMENTS = 8,
};

enum tls_probe_openssl_layout {
    TLS_PROBE_EMPTY_SIZE_POINTER = 0,
    TLS_PROBE_OPENSSL_EX_SUCCESS = 1,
};

enum tls_probe_payload_size_class {
    TLS_PROBE_PAYLOAD_CLASS_512 = 512,
    TLS_PROBE_PAYLOAD_CLASS_2048 = 2048,
    TLS_PROBE_PAYLOAD_CLASS_4096 = 4096,
    TLS_PROBE_PAYLOAD_CLASS_8192 = 8192,
};

struct tls_probe_config {
    __u32 max_capture_bytes;
    __u32 provider;
    __u32 rustls_max_chunks;
    __u32 reserved;
};

struct tls_probe_pending_op {
    __u64 buffer_ptr;
    __u64 requested_size;
    __u64 size_ptr;
    __u64 stream_key;
    __u32 symbol;
    __u32 direction;
};

struct tls_probe_emit_op {
    __u64 buffer_ptr;
    __u64 requested_size;
    __u64 stream_key;
    __u32 symbol;
    __u32 direction;
    __u32 flags;
};

struct tls_probe_emit_segment {
    struct tls_probe_emit_op op;
    __u64 pid_tgid;
    __u64 operation_time_ns;
    __u64 segment_offset;
    __u64 operation_size;
    __u64 reserve_size;
    __u32 captured_size;
};

struct tls_probe_chunk {
    __u64 pointer;
    __u64 length;
};

struct tls_probe_payload_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u32 provider;
    __u32 symbol;
    __u32 flags;
    __u32 captured_size;
    __u64 requested_size;
    __u64 observed_ktime_ns;
    __u64 stream_key;
    __u64 segment_offset;
    __u64 operation_size;
    __u8 bytes[TLS_PROBE_ABI_MAX_CAPTURE_BYTES];
};

struct tls_probe_event_scratch {
    struct tls_probe_payload_event event;
};

struct tls_probe_ring_diagnostics {
    __u64 reserve_fail_events;
    __u64 reserve_fail_actual_bytes;
    __u64 reserve_fail_reserved_bytes;
    __u64 read_user_fail_events;
    __u64 read_user_fail_actual_bytes;
    __u64 read_user_fail_reserved_bytes;
    __u64 output_fail_events;
    __u64 output_fail_actual_bytes;
    __u64 output_fail_reserved_bytes;
};

#endif
