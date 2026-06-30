#ifndef ACTRAIL_TLS_PAYLOAD_DIAGNOSTICS_H
#define ACTRAIL_TLS_PAYLOAD_DIAGNOSTICS_H

enum actrail_tls_payload_diagnostic_reason {
    ACTRAIL_TLS_DIAG_EVENT_TRACE_LOOKUP_MISS = 1,
    ACTRAIL_TLS_DIAG_EVENT_TRACE_LOOKUP_HOST_FALLBACK = 2,
    ACTRAIL_TLS_DIAG_EVENT_EMPTY_BUFFER = 3,
    ACTRAIL_TLS_DIAG_EVENT_PENDING_UPDATE_FAIL = 4,
    ACTRAIL_TLS_DIAG_EVENT_PENDING_NAMESPACE_UPDATE_FAIL = 5,
    ACTRAIL_TLS_DIAG_EVENT_COMPLETION_MISSING_PENDING = 6,
};

struct actrail_tls_payload_diagnostic_event {
    __u32 kind;
    __u32 reason;
    __u32 host_tgid;
    __u32 host_tid;
    __u32 namespace_tgid;
    __u32 namespace_tid;
    __u32 direction;
    __u32 symbol;
    __u32 library;
    __u32 lookup_flags;
    __u64 requested_size;
    __u64 buffer_ptr;
    __u64 observed_ktime_ns;
    char comm[16];
} __attribute__((packed));

struct actrail_tls_payload_diagnostic_args {
    __u32 reason;
    __u32 metadata;
    __u32 lookup_flags;
    __u32 reserved;
    __u64 host_pid_tgid;
    __u64 namespace_pid_tgid;
    __u64 requested_size;
    __u64 buffer_ptr;
};

static __always_inline void emit_tls_payload_diagnostic_event_args(
    void *ctx,
    const struct actrail_tls_payload_diagnostic_args *args
) {
    struct actrail_tls_payload_diagnostic_event *event;

    if (!payload_tls_diagnostics_enabled()) {
        return;
    }
    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        return;
    }
    event->kind = ACTRAIL_TLS_PAYLOAD_DIAGNOSTIC;
    event->reason = args->reason;
    event->host_tgid = args->host_pid_tgid >> 32;
    event->host_tid = (__u32)args->host_pid_tgid;
    event->namespace_tgid = args->namespace_pid_tgid >> 32;
    event->namespace_tid = (__u32)args->namespace_pid_tgid;
    event->direction = args->metadata & 0xffff;
    event->symbol = args->metadata >> 16;
    event->library = payload_tls_library();
    event->lookup_flags = args->lookup_flags;
    event->requested_size = args->requested_size;
    event->buffer_ptr = args->buffer_ptr;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    bpf_get_current_comm(event->comm, sizeof(event->comm));
    actrail_event_submit(ctx, event);
}

#define emit_tls_payload_diagnostic_event(ctx_arg, reason_arg, host_pid_tgid_arg, namespace_pid_tgid_arg, metadata_arg, lookup_flags_arg, requested_size_arg, buffer_ptr_arg) ({ \
    struct actrail_tls_payload_diagnostic_args diag_args = {}; \
    diag_args.reason = (reason_arg); \
    diag_args.metadata = (metadata_arg); \
    diag_args.lookup_flags = (lookup_flags_arg); \
    diag_args.host_pid_tgid = (host_pid_tgid_arg); \
    diag_args.namespace_pid_tgid = (namespace_pid_tgid_arg); \
    diag_args.requested_size = (requested_size_arg); \
    diag_args.buffer_ptr = (buffer_ptr_arg); \
    emit_tls_payload_diagnostic_event_args((ctx_arg), &diag_args); \
})

#endif
