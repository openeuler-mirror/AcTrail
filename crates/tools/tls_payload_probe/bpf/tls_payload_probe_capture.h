#ifndef TLS_PAYLOAD_PROBE_CAPTURE_H
#define TLS_PAYLOAD_PROBE_CAPTURE_H

#include "tls_payload_probe_maps.h"

static __always_inline struct tls_probe_payload_event *tls_probe_event_reserve(
    __u32 captured_size,
    __u64 reserve_size
) {
#ifdef TLS_PROBE_EVENT_TRANSPORT_PERF
    __u32 key = 0;
    struct tls_probe_event_scratch *scratch;

    if (reserve_size > TLS_PROBE_EVENT_HEADER_BYTES + (__u64)TLS_PROBE_ABI_MAX_CAPTURE_BYTES) {
        ring_diag_record_reserve_fail(captured_size, reserve_size);
        return 0;
    }
    scratch = bpf_map_lookup_elem(&event_scratch, &key);
    if (!scratch) {
        ring_diag_record_reserve_fail(captured_size, reserve_size);
        return 0;
    }
    return &scratch->event;
#else
    struct tls_probe_payload_event *event = bpf_ringbuf_reserve(&events, reserve_size, 0);

    if (!event) {
        ring_diag_record_reserve_fail(captured_size, reserve_size);
    }
    return event;
#endif
}

static __always_inline void tls_probe_event_discard(struct tls_probe_payload_event *event) {
#ifndef TLS_PROBE_EVENT_TRANSPORT_PERF
    bpf_ringbuf_discard(event, 0);
#endif
}

static __always_inline int tls_probe_event_submit(
    void *ctx,
    struct tls_probe_payload_event *event,
    __u32 captured_size,
    __u64 reserve_size
) {
#ifdef TLS_PROBE_EVENT_TRANSPORT_PERF
    __u64 output_size = TLS_PROBE_EVENT_HEADER_BYTES + (__u64)captured_size;
    long result = bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, event, output_size);

    if (result != 0) {
        ring_diag_record_output_fail(captured_size, reserve_size);
    }
    return 0;
#else
    bpf_ringbuf_submit(event, 0);
    return 0;
#endif
}

static __always_inline int submit_reserved_payload(
    void *ctx,
    struct tls_probe_payload_event *event,
    struct tls_probe_emit_segment *segment
) {
    if (!event) {
        return 0;
    }
    event->kind = TLS_PROBE_EVENT_PAYLOAD;
    event->pid = (__u32)(segment->pid_tgid >> 32);
    event->tid = (__u32)segment->pid_tgid;
    event->direction = segment->op.direction;
    event->provider = tls_probe_provider();
    event->symbol = segment->op.symbol;
    event->flags = segment->op.flags;
    event->captured_size = segment->captured_size;
    event->requested_size = segment->op.requested_size;
    event->observed_ktime_ns = segment->operation_time_ns;
    event->stream_key = segment->op.stream_key;
    event->segment_offset = segment->segment_offset;
    event->operation_size = segment->operation_size;
    if (bpf_probe_read_user(
            event->bytes,
            segment->captured_size,
            (void *)(unsigned long)(segment->op.buffer_ptr + segment->segment_offset)
        ) != 0) {
        ring_diag_record_read_user_fail(segment->captured_size, segment->reserve_size);
        tls_probe_event_discard(event);
        return 0;
    }
    return tls_probe_event_submit(ctx, event, segment->captured_size, segment->reserve_size);
}

static __always_inline int emit_payload_segment(void *ctx, struct tls_probe_emit_segment *segment) {
    struct tls_probe_payload_event *event;
    __u32 captured_size = segment->captured_size;

    tls_probe_barrier_var(captured_size);
    captured_size &= TLS_PROBE_ABI_MAX_CAPTURE_BYTES;
    if (!captured_size) {
        return 0;
    }
    segment->captured_size = captured_size;
    if (captured_size <= TLS_PROBE_PAYLOAD_CLASS_512) {
        segment->reserve_size = TLS_PROBE_EVENT_HEADER_BYTES + TLS_PROBE_PAYLOAD_CLASS_512;
        event = tls_probe_event_reserve(captured_size, segment->reserve_size);
        return submit_reserved_payload(ctx, event, segment);
    }
    if (captured_size <= TLS_PROBE_PAYLOAD_CLASS_2048) {
        segment->reserve_size = TLS_PROBE_EVENT_HEADER_BYTES + TLS_PROBE_PAYLOAD_CLASS_2048;
        event = tls_probe_event_reserve(captured_size, segment->reserve_size);
        return submit_reserved_payload(ctx, event, segment);
    }
    if (captured_size <= TLS_PROBE_PAYLOAD_CLASS_4096) {
        segment->reserve_size = TLS_PROBE_EVENT_HEADER_BYTES + TLS_PROBE_PAYLOAD_CLASS_4096;
        event = tls_probe_event_reserve(captured_size, segment->reserve_size);
        return submit_reserved_payload(ctx, event, segment);
    }
    if (captured_size <= TLS_PROBE_PAYLOAD_CLASS_8192) {
        segment->reserve_size = TLS_PROBE_EVENT_HEADER_BYTES + TLS_PROBE_PAYLOAD_CLASS_8192;
        event = tls_probe_event_reserve(captured_size, segment->reserve_size);
        return submit_reserved_payload(ctx, event, segment);
    }
    segment->reserve_size = TLS_PROBE_EVENT_HEADER_BYTES + TLS_PROBE_ABI_MAX_CAPTURE_BYTES;
    event = tls_probe_event_reserve(captured_size, segment->reserve_size);
    return submit_reserved_payload(ctx, event, segment);
}

static __always_inline int emit_payload_segment_fixed(void *ctx, struct tls_probe_emit_segment *segment) {
    struct tls_probe_payload_event *event;
    __u32 captured_size = segment->captured_size;

    tls_probe_barrier_var(captured_size);
    captured_size &= TLS_PROBE_ABI_MAX_CAPTURE_BYTES;
    if (!captured_size) {
        return 0;
    }
    segment->captured_size = captured_size;
    segment->reserve_size = TLS_PROBE_EVENT_HEADER_BYTES + TLS_PROBE_ABI_MAX_CAPTURE_BYTES;
    event = tls_probe_event_reserve(captured_size, segment->reserve_size);
    return submit_reserved_payload(ctx, event, segment);
}

static __always_inline int emit_payload_single(void *ctx, struct tls_probe_emit_op *op) {
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 max_capture_bytes = tls_probe_max_capture_bytes();
    __u32 captured_size;
    struct tls_probe_emit_op segment_op = {
        .buffer_ptr = op->buffer_ptr,
        .requested_size = op->requested_size,
        .stream_key = op->stream_key,
        .symbol = op->symbol,
        .direction = op->direction,
        .flags = op->flags,
    };

    if (!op->buffer_ptr || !op->requested_size || !max_capture_bytes) {
        return 0;
    }
    captured_size = op->requested_size > max_capture_bytes
        ? max_capture_bytes
        : (__u32)op->requested_size;
    if (op->requested_size > captured_size) {
        segment_op.flags |= TLS_PROBE_EVENT_FLAG_TRUNCATED;
    }
    struct tls_probe_emit_segment segment = {
        .op = segment_op,
        .pid_tgid = pid_tgid,
        .operation_time_ns = bpf_ktime_get_ns(),
        .segment_offset = 0,
        .operation_size = captured_size,
        .reserve_size = 0,
        .captured_size = captured_size,
    };
    return emit_payload_segment(ctx, &segment);
}

static __always_inline int emit_payload(void *ctx, struct tls_probe_emit_op *op) {
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 max_capture_bytes = tls_probe_max_capture_bytes();
    __u64 max_operation_bytes;
    __u64 operation_size;
    __u64 operation_time_ns;
    __u64 emitted = 0;
    int operation_truncated = 0;

    if (!op->buffer_ptr || !op->requested_size || !max_capture_bytes) {
        return 0;
    }
    max_operation_bytes = (__u64)max_capture_bytes * TLS_PROBE_MAX_SEGMENTS;
    operation_size = op->requested_size;
    if (operation_size > max_operation_bytes) {
        operation_size = max_operation_bytes;
        operation_truncated = 1;
    }
    operation_time_ns = bpf_ktime_get_ns();
    if (operation_size <= max_capture_bytes) {
        struct tls_probe_emit_segment segment = {
            .op = *op,
            .pid_tgid = pid_tgid,
            .operation_time_ns = operation_time_ns,
            .segment_offset = 0,
            .operation_size = operation_size,
            .reserve_size = 0,
            .captured_size = (__u32)operation_size,
        };
        return emit_payload_segment(ctx, &segment);
    }
    for (__u32 segment_index = 0; segment_index < TLS_PROBE_MAX_SEGMENTS; segment_index++) {
        struct tls_probe_emit_op segment_op = {
            .buffer_ptr = op->buffer_ptr,
            .requested_size = op->requested_size,
            .stream_key = op->stream_key,
            .symbol = op->symbol,
            .direction = op->direction,
            .flags = op->flags,
        };
        __u64 remaining;
        __u32 captured_size;

        if (emitted >= operation_size) {
            break;
        }
        remaining = operation_size - emitted;
        captured_size = remaining > max_capture_bytes
            ? max_capture_bytes
            : (__u32)remaining;
        if (operation_truncated && emitted + captured_size >= operation_size) {
            segment_op.flags |= TLS_PROBE_EVENT_FLAG_TRUNCATED;
        }
        struct tls_probe_emit_segment segment = {
            .op = segment_op,
            .pid_tgid = pid_tgid,
            .operation_time_ns = operation_time_ns,
            .segment_offset = emitted,
            .operation_size = operation_size,
            .reserve_size = 0,
            .captured_size = captured_size,
        };
        emit_payload_segment_fixed(ctx, &segment);
        emitted += captured_size;
    }
    return 0;
}

static __always_inline int store_pending(struct tls_probe_pending_op *op) {
    __u64 key = bpf_get_current_pid_tgid();

    if (!op->buffer_ptr || !op->requested_size) {
        return 0;
    }
    bpf_map_update_elem(&pending_ops, &key, op, BPF_ANY);
    return 0;
}

static __always_inline int emit_pending_return(void *ctx, __u64 completed_size) {
    __u64 key = bpf_get_current_pid_tgid();
    struct tls_probe_pending_op *op = bpf_map_lookup_elem(&pending_ops, &key);

    if (!op) {
        return 0;
    }
    if (completed_size > op->requested_size) {
        completed_size = op->requested_size;
    }
    struct tls_probe_emit_op emit = {
        .buffer_ptr = op->buffer_ptr,
        .requested_size = completed_size,
        .stream_key = op->stream_key,
        .symbol = op->symbol,
        .direction = op->direction,
        .flags = 0,
    };
    emit_payload(ctx, &emit);
    bpf_map_delete_elem(&pending_ops, &key);
    return 0;
}

static __always_inline int emit_rustls_payload(
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
    if (bpf_probe_read_user(&q0, sizeof(q0), (void *)(unsigned long)(payload_ptr)) != 0 ||
        bpf_probe_read_user(&q1, sizeof(q1), (void *)(unsigned long)(payload_ptr + 8)) != 0 ||
        bpf_probe_read_user(&q2, sizeof(q2), (void *)(unsigned long)(payload_ptr + 16)) != 0) {
        return 0;
    }
    if (symbol == TLS_PROBE_SYMBOL_RUSTLS_TAKE_RECEIVED_PLAINTEXT) {
        if (q0 != TLS_PROBE_RUSTLS_BORROWED_TAG) {
            return 0;
        }
        struct tls_probe_emit_op op = {
            .buffer_ptr = q1,
            .requested_size = q2,
            .stream_key = stream_key,
            .symbol = symbol,
            .direction = TLS_PROBE_DIRECTION_INBOUND,
            .flags = 0,
        };
        return emit_payload_single(ctx, &op);
    }
    if (bpf_probe_read_user(&q3, sizeof(q3), (void *)(unsigned long)(payload_ptr + 24)) != 0) {
        return 0;
    }
    if (q0 == TLS_PROBE_RUSTLS_INLINE_TAG) {
        struct tls_probe_emit_op op = {
            .buffer_ptr = q1,
            .requested_size = q2,
            .stream_key = stream_key,
            .symbol = symbol,
            .direction = TLS_PROBE_DIRECTION_OUTBOUND,
            .flags = 0,
        };
        return emit_payload_single(ctx, &op);
    }

    __u64 cursor = 0;
    __u64 emitted = 0;
    __u32 max_capture_bytes = tls_probe_max_capture_bytes();
    __u32 max_chunks = tls_probe_rustls_max_chunks();
    for (__u32 index = 0; index < TLS_PROBE_RUSTLS_MAX_CHUNKS; index++) {
        struct tls_probe_chunk chunk = {};
        __u64 chunk_start;
        __u64 chunk_end;
        __u64 overlap_start;
        __u64 overlap_end;
        __u64 selected_ptr;
        __u64 selected_len;

        if (index >= max_chunks || index >= q1 || emitted >= max_capture_bytes) {
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
        selected_ptr = chunk.pointer + (overlap_start - chunk_start);
        selected_len = overlap_end - overlap_start;
        if (selected_len > max_capture_bytes - emitted) {
            selected_len = max_capture_bytes - emitted;
        }
        struct tls_probe_emit_op op = {
            .buffer_ptr = selected_ptr,
            .requested_size = selected_len,
            .stream_key = stream_key,
            .symbol = symbol,
            .direction = TLS_PROBE_DIRECTION_OUTBOUND,
            .flags = TLS_PROBE_EVENT_FLAG_RUSTLS_CHUNK,
        };
        emit_payload_single(ctx, &op);
        emitted += selected_len;
    }
    return 0;
}

#endif
