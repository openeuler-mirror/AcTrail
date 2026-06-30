#ifndef ACTRAIL_TLS_PAYLOAD_COMPLETION_H
#define ACTRAIL_TLS_PAYLOAD_COMPLETION_H

static __always_inline void capture_tls_payload_after_completion(
    void *ctx,
    const struct actrail_pending_tls_payload_op *op,
    __u32 tgid,
    __u32 tid,
    __u64 completed_size,
    __u32 flags
) {
    __u32 backend;

    if ((flags & ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED) != 0 ||
        completed_size == 0 ||
        op->direction != ACTRAIL_TLS_PAYLOAD_INBOUND) {
        return;
    }

    backend = payload_tls_capture_backend();
    if (backend == ACTRAIL_TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK &&
        emit_tls_direct_capture(ctx, op, tgid, tid, completed_size) == 1) {
        return;
    }
    if (backend == ACTRAIL_TLS_BACKEND_BPF_COPY_SECCOMP_FALLBACK ||
        backend == ACTRAIL_TLS_BACKEND_SECCOMP_USER_READ) {
        emit_tls_capture_request(ctx, op, tgid, tid, completed_size);
    }
}

#endif
