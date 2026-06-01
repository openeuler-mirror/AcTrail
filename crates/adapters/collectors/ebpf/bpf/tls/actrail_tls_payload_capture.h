#ifndef ACTRAIL_TLS_PAYLOAD_CAPTURE_H
#define ACTRAIL_TLS_PAYLOAD_CAPTURE_H

static __always_inline int emit_tls_direct_capture(
    const struct actrail_pending_tls_payload_op *op,
    __u32 tgid,
    __u32 tid,
    __u64 requested_size
) {
    __u64 bounded_size;
    __u32 capture_size;
    __u32 copy_limit = payload_tls_direct_copy_limit();
    struct actrail_tls_direct_capture_event *event;

    tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_COPY_ATTEMPT);
    bounded_size = requested_size & ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES;
    if (requested_size > ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES) {
        bounded_size = ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES;
    }
    if (bounded_size > copy_limit) {
        bounded_size = copy_limit;
    }
    actrail_barrier_var(bounded_size);
    bounded_size &= ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES;
    capture_size = (__u32)bounded_size;
    if (!capture_size || requested_size != bounded_size) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_COPY_TOO_LARGE);
        return 0;
    }

    event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_RESERVE_FAIL);
        return 0;
    }
    event->kind = ACTRAIL_TLS_PAYLOAD_DIRECT_CAPTURE;
    event->pid = tgid;
    event->tid = tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->stream_key = op->stream_key;
    event->operation_id = op->operation_id;
    event->original_size = capture_size;
    event->captured_size = capture_size;
    event->flags = 0;
    event->symbol = op->symbol;
    event->library = op->library;
    event->reserved = 0;
    event->pid_generation = op->pid_generation;
    if (bpf_probe_read_user(
            event->bytes,
            bounded_size,
            (void *)(unsigned long)op->buffer_ptr
        ) != 0) {
        bpf_ringbuf_discard(event, 0);
        tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_READ_FAIL);
        return 0;
    }
    bpf_ringbuf_submit(event, 0);
    tls_diag_inc(ACTRAIL_TLS_DIAG_DIRECT_SUBMIT_OK);
    return 1;
}

static __always_inline int emit_tls_capture_request(
    const struct actrail_pending_tls_payload_op *op,
    __u32 tgid,
    __u32 tid,
    __u64 requested_size
) {
    struct actrail_tls_capture_request_event *event =
        bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event) {
        tls_diag_inc(ACTRAIL_TLS_DIAG_CAPTURE_REQUEST_RESERVE_FAIL);
        return 0;
    }
    event->kind = ACTRAIL_TLS_PAYLOAD_CAPTURE_REQUEST;
    event->pid = tgid;
    event->tid = tid;
    event->direction = op->direction;
    event->trace_id = op->trace_id;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->stream_key = op->stream_key;
    event->operation_id = op->operation_id;
    event->requested_size = requested_size;
    event->buffer_ptr = op->buffer_ptr;
    event->pid_generation = op->pid_generation;
    event->symbol = op->symbol;
    event->library = op->library;
    if (bpf_send_signal(ACTRAIL_TLS_CAPTURE_SIGSTOP) == 0) {
        bpf_ringbuf_submit(event, 0);
        tls_diag_inc(ACTRAIL_TLS_DIAG_CAPTURE_REQUEST_SUBMIT_OK);
    } else {
        bpf_ringbuf_discard(event, 0);
        tls_diag_inc(ACTRAIL_TLS_DIAG_CAPTURE_REQUEST_SIGNAL_FAIL);
    }
    return 0;
}

#endif
