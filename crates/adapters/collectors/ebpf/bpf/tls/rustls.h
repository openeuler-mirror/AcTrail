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

static __always_inline int rustls_connection_key(
    void *ctx, __u64 state, struct actrail_rustls_connection_key *key
) {
    __u64 offset = bpf_get_attach_cookie(ctx);
    __u64 fields[3] = {};
    if (!state || !offset || offset > 32752 || (offset & 7) ||
        bpf_probe_read_user(fields, sizeof(fields), (void *)(unsigned long)(state + offset))) {
        return 0;
    }
    /* Option<Vec>: None uses the signed-min capacity niche. Empty chains do
     * not own an allocation and therefore cannot identify a connection. */
    if (!fields[0] || fields[0] >= ACTRAIL_RUSTLS_BORROWED_TAG_SIGNED_MIN ||
        !fields[1] || !fields[2] || fields[2] > fields[0]) {
        return 0;
    }
    key->pid = current_pid_tgid() >> 32;
    key->certificates = fields[1];
    return 1;
}

static __always_inline int register_rustls_connection(void *ctx, __u64 state) {
    struct actrail_rustls_connection_key key = {};
    __u32 tgid = 0, tid = 0, lookup_flags = 0, zero = 0;
    if (!lookup_current_detailed_trace(&tgid, &tid, &lookup_flags) ||
        !rustls_connection_key(ctx, state, &key)) {
        return 0;
    }
    /* Invalidate recycled allocations before attempting to register them. */
    bpf_map_delete_elem(&rustls_connections, &key);
    __u64 *sequence = bpf_map_lookup_elem(&rustls_connection_sequence, &zero);
    if (!sequence) {
        return 0;
    }
    __u64 generation = __sync_fetch_and_add(sequence, 1) + 1;
    if (!generation || generation >= ACTRAIL_RUSTLS_BORROWED_TAG_SIGNED_MIN) {
        return 0;
    }
    generation |= ACTRAIL_RUSTLS_BORROWED_TAG_SIGNED_MIN;
    bpf_map_update_elem(&rustls_connections, &key, &generation, BPF_ANY);
    return 0;
}

static __always_inline __u64 rustls_stream_key(void *ctx, __u64 state) {
    /* Unchanged legacy profiles have no verified lifecycle layout. */
    if (!bpf_get_attach_cookie(ctx)) {
        return state;
    }
    struct actrail_rustls_connection_key key = {};
    if (!rustls_connection_key(ctx, state, &key)) {
        return 0;
    }
    __u64 *generation = bpf_map_lookup_elem(&rustls_connections, &key);
    return generation ? *generation : 0;
}

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
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;

    if (!payload_ptr ||
        !lookup_current_detailed_trace(&tgid, &tid, &lookup_flags)) {
        return 0;
    }
    stream_key = rustls_stream_key(ctx, stream_key);
    if (!stream_key) {
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
