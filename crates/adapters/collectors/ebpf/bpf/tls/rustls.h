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

union actrail_rustls_write_view {
    struct {
        __u64 tag;
        __u64 pointer;
        __u64 length;
        __u64 unused;
    } single;
    struct {
        __u64 chunks;
        __u64 count;
        __u64 start;
        __u64 end;
    } multiple;
};

struct actrail_rustls_pending_write {
    __u64 stream_key;
    union actrail_rustls_write_view view;
};

/* Capacity follows tls.pending_operation_max_entries. */
struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_rustls_pending_write);
} rustls_pending_writes SEC(".maps");

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

static __noinline long rustls_write_chunk_step(__u32 index, void *opaque) {
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
    if (index >= ACTRAIL_RUSTLS_MAX_CHUNKS) {
        return 1;
    }
    if (bpf_probe_read_user(
            &chunk,
            sizeof(chunk),
            (void *)(unsigned long)(state->chunks + ((__u64)index * sizeof(chunk)))
        ) != 0) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_READ_FAIL);
        return 1;
    }
    chunk_start = state->cursor;
    chunk_end = chunk_start + chunk.length;
    if (chunk_end < chunk_start || chunk.pointer + chunk.length < chunk.pointer ||
        (!chunk.pointer && chunk.length)) {
        return 1;
    }
    state->cursor = chunk_end;
    overlap_start = state->start > chunk_start ? state->start : chunk_start;
    overlap_end = state->end < chunk_end ? state->end : chunk_end;
    if (overlap_start >= overlap_end) {
        return 0;
    }
    state->args.buffer_ptr = chunk.pointer + (overlap_start - chunk_start);
    state->args.requested_size = overlap_end - overlap_start;
    emit_tls_immediate_payload_args(state->program_ctx, &state->args);
    return chunk_end >= state->end;
}

static __always_inline int store_rustls_write(
    void *ctx, __u64 connection, __u64 payload_ptr
) {
    __u32 tgid = 0, tid = 0, lookup_flags = 0;
    __u64 key = current_pid_tgid();
    struct actrail_rustls_pending_write call;
    if (!payload_ptr || !lookup_current_detailed_trace(&tgid, &tid, &lookup_flags)) {
        return 0;
    }
    if (bpf_map_lookup_elem(&rustls_pending_writes, &key)) {
        bpf_map_delete_elem(&rustls_pending_writes, &key);
        tls_diag_inc(ACTRAIL_TLS_DIAG_PENDING_UPDATE_FAIL);
        return 0;
    }
    call.stream_key = rustls_stream_key(ctx, connection);
    if (!call.stream_key) {
        return 0;
    }
    call.view.single.unused = 0;
    if (bpf_probe_read_user(&call.view, 3 * sizeof(__u64),
            (void *)(unsigned long)payload_ptr)) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_READ_FAIL);
        return 0;
    }
    if (call.view.single.tag == ACTRAIL_RUSTLS_INLINE_TAG) {
        if (!call.view.single.pointer || !call.view.single.length ||
            call.view.single.pointer + call.view.single.length < call.view.single.pointer) {
            return 0;
        }
    } else {
        if (bpf_probe_read_user(&call.view.multiple.end, sizeof(__u64),
                (void *)(unsigned long)(payload_ptr + 3 * sizeof(__u64)))) {
            tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_READ_FAIL);
            return 0;
        }
        if (!call.view.multiple.count ||
            call.view.multiple.end <= call.view.multiple.start ||
            call.view.multiple.chunks >
                ~(__u64)0 - ACTRAIL_RUSTLS_MAX_CHUNKS * sizeof(struct actrail_rustls_chunk)) {
            return 0;
        }
    }
    if (bpf_map_update_elem(&rustls_pending_writes, &key, &call, BPF_NOEXIST)) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_PENDING_UPDATE_FAIL);
    }
    return 0;
}

static __always_inline int emit_rustls_completed_write(void *ctx, __u64 accepted) {
    __u64 key = current_pid_tgid();
    struct actrail_rustls_pending_write *call =
        bpf_map_lookup_elem(&rustls_pending_writes, &key);
    if (!call) {
        return 0;
    }
    if (!accepted) {
        bpf_map_delete_elem(&rustls_pending_writes, &key);
        return 0;
    }
    struct actrail_rustls_payload_loop state = {
        .program_ctx = ctx,
        .args = {
            .direction = ACTRAIL_TLS_PAYLOAD_OUTBOUND,
            .symbol = ACTRAIL_TLS_SYMBOL_RUSTLS_BUFFER_PLAINTEXT,
            .library = ACTRAIL_TLS_LIBRARY_RUSTLS,
            .stream_key = call->stream_key,
        },
    };
    if (call->view.single.tag == ACTRAIL_RUSTLS_INLINE_TAG) {
        if (accepted > call->view.single.length) {
            bpf_map_delete_elem(&rustls_pending_writes, &key);
            return 0;
        }
        state.args.buffer_ptr = call->view.single.pointer;
        state.args.requested_size = accepted;
        bpf_map_delete_elem(&rustls_pending_writes, &key);
        rustls_write_chunk_step(0, &state);
        return 0;
    }
    if (accepted > call->view.multiple.end - call->view.multiple.start) {
        bpf_map_delete_elem(&rustls_pending_writes, &key);
        return 0;
    }
    state.chunks = call->view.multiple.chunks;
    state.start = call->view.multiple.start;
    state.end = call->view.multiple.start + accepted;
    __u32 count = call->view.multiple.count > ACTRAIL_RUSTLS_MAX_CHUNKS
        ? ACTRAIL_RUSTLS_MAX_CHUNKS : (__u32)call->view.multiple.count;
    bpf_map_delete_elem(&rustls_pending_writes, &key);
#ifdef ACTRAIL_BPF_LOOP
    bpf_loop(count, rustls_write_chunk_step, &state, 0);
#else
#pragma clang loop unroll(disable)
    for (__u32 index = 0; index < ACTRAIL_RUSTLS_MAX_CHUNKS; index++) {
        if (index >= count || rustls_write_chunk_step(index, &state)) {
            break;
        }
    }
#endif
    return 0;
}

static __always_inline int emit_rustls_received_payload(
    void *ctx, __u64 connection, __u64 payload_ptr
) {
    struct {
        __u64 tag;
        __u64 pointer;
        __u64 length;
    } payload;
    __u32 tgid = 0, tid = 0, lookup_flags = 0;
    if (!payload_ptr || !lookup_current_detailed_trace(&tgid, &tid, &lookup_flags)) {
        return 0;
    }
    __u64 stream_key = rustls_stream_key(ctx, connection);
    if (!stream_key || bpf_probe_read_user(&payload, sizeof(payload),
            (void *)(unsigned long)payload_ptr)) {
        return 0;
    }
    if (payload.tag != ACTRAIL_RUSTLS_BORROWED_TAG_SIGNED_MIN &&
        payload.tag != ACTRAIL_RUSTLS_BORROWED_TAG_UNSIGNED_MAX) {
        return 0;
    }
    return emit_tls_immediate_payload(ctx, ACTRAIL_TLS_PAYLOAD_INBOUND,
        ACTRAIL_TLS_SYMBOL_RUSTLS_TAKE_RECEIVED_PLAINTEXT, ACTRAIL_TLS_LIBRARY_RUSTLS,
        stream_key, payload.pointer, payload.length);
}

#endif
