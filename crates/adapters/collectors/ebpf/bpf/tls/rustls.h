#ifndef ACTRAIL_TLS_RUSTLS_H
#define ACTRAIL_TLS_RUSTLS_H

#include "completion.h"


enum actrail_rustls_payload_layout {
    ACTRAIL_RUSTLS_INLINE_TAG = 0,
    ACTRAIL_RUSTLS_BORROWED_TAG_SIGNED_MIN = 0x8000000000000000ULL,
    ACTRAIL_RUSTLS_BORROWED_TAG_UNSIGNED_MAX = 0xffffffffffffffffULL,
    ACTRAIL_RUSTLS_MAX_CHUNKS = 8,
};

struct actrail_rustls_chunk {
    __u64 pointer;
    __u64 length;
};

static __always_inline int emit_rustls_internal_payload(
    void *ctx,
    __u64 stream_key,
    __u64 payload_ptr,
    __u32 symbol
) {
    __u64 q0 = 0;
    __u64 q1 = 0;
    __u64 q2 = 0;
    __u64 q3 = 0;

    if (!payload_ptr) {
        return 0;
    }
    if (bpf_probe_read_user(&q0, sizeof(q0), (void *)(unsigned long)payload_ptr) != 0 ||
        bpf_probe_read_user(&q1, sizeof(q1), (void *)(unsigned long)(payload_ptr + 8)) != 0 ||
        bpf_probe_read_user(&q2, sizeof(q2), (void *)(unsigned long)(payload_ptr + 16)) != 0) {
        return 0;
    }
    if (symbol == ACTRAIL_TLS_SYMBOL_RUSTLS_TAKE_RECEIVED_PLAINTEXT) {
        if (q0 != ACTRAIL_RUSTLS_BORROWED_TAG_SIGNED_MIN &&
            q0 != ACTRAIL_RUSTLS_BORROWED_TAG_UNSIGNED_MAX) {
            return 0;
        }
        return emit_tls_immediate_payload(
            ctx,
            ACTRAIL_TLS_PAYLOAD_INBOUND,
            symbol,
            ACTRAIL_TLS_LIBRARY_RUSTLS,
            stream_key,
            q1,
            q2
        );
    }
    if (bpf_probe_read_user(&q3, sizeof(q3), (void *)(unsigned long)(payload_ptr + 24)) != 0) {
        return 0;
    }
    if (q0 == ACTRAIL_RUSTLS_INLINE_TAG) {
        return emit_tls_immediate_payload(
            ctx,
            ACTRAIL_TLS_PAYLOAD_OUTBOUND,
            symbol,
            ACTRAIL_TLS_LIBRARY_RUSTLS,
            stream_key,
            q1,
            q2
        );
    }

    __u64 cursor = 0;
    for (__u32 index = 0; index < ACTRAIL_RUSTLS_MAX_CHUNKS; index++) {
        struct actrail_rustls_chunk chunk = {};
        __u64 chunk_start;
        __u64 chunk_end;
        __u64 overlap_start;
        __u64 overlap_end;

        if (index >= q1) {
            break;
        }
        if (bpf_probe_read_user(
                &chunk,
                sizeof(chunk),
                (void *)(unsigned long)(q0 + ((__u64)index * sizeof(chunk)))
            ) != 0) {
            break;
        }
        chunk_start = cursor;
        chunk_end = cursor + chunk.length;
        cursor = chunk_end;
        overlap_start = q2 > chunk_start ? q2 : chunk_start;
        overlap_end = q3 < chunk_end ? q3 : chunk_end;
        if (overlap_start >= overlap_end) {
            continue;
        }
        emit_tls_immediate_payload(
            ctx,
            ACTRAIL_TLS_PAYLOAD_OUTBOUND,
            symbol,
            ACTRAIL_TLS_LIBRARY_RUSTLS,
            stream_key,
            chunk.pointer + (overlap_start - chunk_start),
            overlap_end - overlap_start
        );
    }
    return 0;
}

#endif
