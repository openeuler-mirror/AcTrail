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

struct actrail_rustls_payload_loop {
    void *program_ctx;
    struct actrail_tls_immediate_payload_args args;
    __u64 chunks;
    __u64 start;
    __u64 end;
    __u64 cursor;
};

static long rustls_payload_chunk_step(__u32 index, void *opaque) {
    struct actrail_rustls_payload_loop *state = opaque;
    struct actrail_rustls_chunk chunk = {};
    __u64 chunk_start;
    __u64 chunk_end;
    __u64 overlap_start;
    __u64 overlap_end;

    if (!state->chunks) {
        emit_tls_immediate_payload_args(state->program_ctx, &state->args);
        return 1;
    }
    if (bpf_probe_read_user(
            &chunk,
            sizeof(chunk),
            (void *)(unsigned long)(state->chunks + ((__u64)index * sizeof(chunk)))
        ) != 0) {
        return 1;
    }
    chunk_start = state->cursor;
    chunk_end = chunk_start + chunk.length;
    state->cursor = chunk_end;
    overlap_start = state->start > chunk_start ? state->start : chunk_start;
    overlap_end = state->end < chunk_end ? state->end : chunk_end;
    if (overlap_start >= overlap_end) {
        return 0;
    }
    state->args.buffer_ptr = chunk.pointer + (overlap_start - chunk_start);
    state->args.requested_size = overlap_end - overlap_start;
    emit_tls_immediate_payload_args(state->program_ctx, &state->args);
    return 0;
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
    __u32 count = 1;
    struct actrail_rustls_payload_loop state = {
        .program_ctx = ctx,
        .args = {
            .direction = ACTRAIL_TLS_PAYLOAD_OUTBOUND,
            .symbol = symbol,
            .library = ACTRAIL_TLS_LIBRARY_RUSTLS,
        },
    };

    if (!payload_ptr ||
        !lookup_current_detailed_trace(&tgid, &tid, &lookup_flags)) {
        return 0;
    }
    stream_key = rustls_stream_key(ctx, stream_key);
    if (!stream_key) {
        return 0;
    }
    state.args.stream_key = stream_key;
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
        state.args.direction = ACTRAIL_TLS_PAYLOAD_INBOUND;
        state.args.buffer_ptr = q1;
        state.args.requested_size = q2;
    } else {
        if (bpf_probe_read_user(
                &q3, sizeof(q3), (void *)(unsigned long)(payload_ptr + 24)
            ) != 0) {
            return 0;
        }
        if (q0 == ACTRAIL_RUSTLS_INLINE_TAG) {
            state.args.buffer_ptr = q1;
            state.args.requested_size = q2;
        } else {
            state.chunks = q0;
            state.start = q2;
            state.end = q3;
            count = q1 > ACTRAIL_RUSTLS_MAX_CHUNKS
                ? ACTRAIL_RUSTLS_MAX_CHUNKS : (__u32)q1;
        }
    }
#ifdef ACTRAIL_BPF_LOOP
    bpf_loop(count, rustls_payload_chunk_step, &state, 0);
#else
#pragma clang loop unroll(disable)
    for (__u32 index = 0; index < ACTRAIL_RUSTLS_MAX_CHUNKS; index++) {
        if (index >= count || rustls_payload_chunk_step(index, &state)) {
            break;
        }
    }
#endif
    return 0;
}

#endif
