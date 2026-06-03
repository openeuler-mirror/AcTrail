#ifndef TLS_PAYLOAD_PROBE_MAPS_H
#define TLS_PAYLOAD_PROBE_MAPS_H

#include "tls_payload_probe_types.h"

struct {
    __uint(type, BPF_MAP_TYPE_RINGBUF);
    __uint(max_entries, TLS_PROBE_DEFAULT_RING_BUFFER_BYTES);
} events SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 4096);
    __type(key, __u64);
    __type(value, struct tls_probe_pending_op);
} pending_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct tls_probe_ring_diagnostics);
} ring_diagnostics SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct tls_probe_config);
} probe_config SEC(".maps");

static __always_inline struct tls_probe_config *tls_probe_config(void) {
    __u32 key = 0;

    return bpf_map_lookup_elem(&probe_config, &key);
}

static __always_inline __u32 tls_probe_max_capture_bytes(void) {
    struct tls_probe_config *config = tls_probe_config();
    __u32 max_capture_bytes;

    if (!config) {
        return 0;
    }
    max_capture_bytes = config->max_capture_bytes;
    if (!max_capture_bytes) {
        max_capture_bytes = TLS_PROBE_DEFAULT_MAX_CAPTURE_BYTES;
    }
    if (max_capture_bytes > TLS_PROBE_ABI_MAX_CAPTURE_BYTES) {
        return TLS_PROBE_ABI_MAX_CAPTURE_BYTES;
    }
    return max_capture_bytes;
}

static __always_inline __u32 tls_probe_provider(void) {
    struct tls_probe_config *config = tls_probe_config();

    if (!config) {
        return 0;
    }
    return config->provider;
}

static __always_inline __u32 tls_probe_rustls_max_chunks(void) {
    struct tls_probe_config *config = tls_probe_config();
    __u32 max_chunks;

    if (!config) {
        return TLS_PROBE_RUSTLS_MAX_CHUNKS;
    }
    max_chunks = config->rustls_max_chunks;
    if (!max_chunks || max_chunks > TLS_PROBE_RUSTLS_MAX_CHUNKS) {
        return TLS_PROBE_RUSTLS_MAX_CHUNKS;
    }
    return max_chunks;
}

static __always_inline void ring_diag_record_reserve_fail(
    __u32 captured_size,
    __u64 reserve_size
) {
    __u32 key = 0;
    struct tls_probe_ring_diagnostics *diag = bpf_map_lookup_elem(&ring_diagnostics, &key);

    if (!diag) {
        return;
    }
    __sync_fetch_and_add(&diag->reserve_fail_events, 1);
    __sync_fetch_and_add(
        &diag->reserve_fail_actual_bytes,
        TLS_PROBE_EVENT_HEADER_BYTES + (__u64)captured_size
    );
    __sync_fetch_and_add(&diag->reserve_fail_reserved_bytes, reserve_size);
}

static __always_inline void ring_diag_record_read_user_fail(
    __u32 captured_size,
    __u64 reserve_size
) {
    __u32 key = 0;
    struct tls_probe_ring_diagnostics *diag = bpf_map_lookup_elem(&ring_diagnostics, &key);

    if (!diag) {
        return;
    }
    __sync_fetch_and_add(&diag->read_user_fail_events, 1);
    __sync_fetch_and_add(
        &diag->read_user_fail_actual_bytes,
        TLS_PROBE_EVENT_HEADER_BYTES + (__u64)captured_size
    );
    __sync_fetch_and_add(&diag->read_user_fail_reserved_bytes, reserve_size);
}

#endif
