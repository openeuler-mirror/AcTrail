#ifndef ACTRAIL_TLS_PAYLOAD_PROBES_H
#define ACTRAIL_TLS_PAYLOAD_PROBES_H

SEC("uprobe")
int handle_ssl_write_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_OUTBOUND, ACTRAIL_TLS_SYMBOL_SSL_WRITE),
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx),
        ACTRAIL_UPROBE_ARG3(ctx),
        0
    );
}

SEC("uprobe")
int handle_ssl_write_exit(struct pt_regs *ctx) {
    return emit_tls_payload_completion_from_return(ctx);
}

SEC("uprobe")
int handle_ssl_read_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_INBOUND, ACTRAIL_TLS_SYMBOL_SSL_READ),
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx),
        ACTRAIL_UPROBE_ARG3(ctx),
        0
    );
}

SEC("uprobe")
int handle_ssl_read_exit(struct pt_regs *ctx) {
    return emit_tls_payload_completion_from_return(ctx);
}

SEC("uprobe")
int handle_ssl_write_ex_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_OUTBOUND, ACTRAIL_TLS_SYMBOL_SSL_WRITE_EX),
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx),
        ACTRAIL_UPROBE_ARG3(ctx),
        ACTRAIL_UPROBE_ARG4(ctx)
    );
}

SEC("uprobe")
int handle_ssl_write_ex_exit(struct pt_regs *ctx) {
    return emit_tls_payload_completion_from_size_ptr(ctx);
}

SEC("uprobe")
int handle_ssl_read_ex_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_INBOUND, ACTRAIL_TLS_SYMBOL_SSL_READ_EX),
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx),
        ACTRAIL_UPROBE_ARG3(ctx),
        ACTRAIL_UPROBE_ARG4(ctx)
    );
}

SEC("uprobe")
int handle_ssl_read_ex_exit(struct pt_regs *ctx) {
    return emit_tls_payload_completion_from_size_ptr(ctx);
}

SEC("uprobe")
int handle_rustls_write_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_OUTBOUND, ACTRAIL_TLS_SYMBOL_RUSTLS_WRITE),
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx),
        ACTRAIL_UPROBE_ARG3(ctx),
        0
    );
}

SEC("uprobe")
int handle_rustls_write_exit(struct pt_regs *ctx) {
    return emit_tls_payload_completion_from_rust_result_usize(ctx);
}

SEC("uprobe")
int handle_rustls_write_vectored_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_OUTBOUND, ACTRAIL_TLS_SYMBOL_RUSTLS_WRITE_VECTORED),
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx),
        ACTRAIL_UPROBE_ARG3(ctx),
        0
    );
}

SEC("uprobe")
int handle_rustls_write_vectored_exit(struct pt_regs *ctx) {
    return emit_tls_payload_completion_from_rust_result_usize(ctx);
}

SEC("uprobe")
int handle_go_tls_write_enter(struct pt_regs *ctx) {
    __u64 requested_size = positive_uprobe_isize(ACTRAIL_GO_UPROBE_ARG3(ctx));
    int stored = 0;

    if (!requested_size) {
        return 0;
    }
    stored = store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_OUTBOUND, ACTRAIL_TLS_SYMBOL_GO_CONN_WRITE),
        ACTRAIL_GO_UPROBE_ARG1(ctx),
        ACTRAIL_GO_UPROBE_ARG2(ctx),
        requested_size,
        0
    );
    if (stored != 1) {
        return 0;
    }
    return emit_tls_payload_completion(ctx, requested_size, 0);
}

SEC("uprobe")
int handle_go_tls_conn_read_enter(struct pt_regs *ctx) {
    __u64 host_pid_tgid = current_pid_tgid();
    __u32 host_tgid = host_pid_tgid >> 32;
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id = lookup_current_trace(&tgid, &tid, &lookup_flags);
    struct actrail_go_tls_read_buffer_key key = {};
    struct actrail_go_tls_read_buffer value = {};
    __u64 requested_size = positive_uprobe_isize(ACTRAIL_GO_UPROBE_ARG3(ctx));

    if (!trace_id || !ACTRAIL_GO_UPROBE_ARG1(ctx) || !ACTRAIL_GO_UPROBE_ARG2(ctx) ||
        !requested_size) {
        return 0;
    }
    key.tgid = host_tgid;
    key.buffer_ptr = ACTRAIL_GO_UPROBE_ARG2(ctx);
    value.stream_key = ACTRAIL_GO_UPROBE_ARG1(ctx);
    value.requested_size = requested_size;
    bpf_map_update_elem(&go_tls_read_buffers, &key, &value, BPF_ANY);
    return 0;
}

SEC("uprobe")
int handle_go_tls_memmove_enter(struct pt_regs *ctx) {
    __u64 host_pid_tgid = current_pid_tgid();
    __u32 host_tgid = host_pid_tgid >> 32;
    struct actrail_go_tls_read_buffer_key key = {};
    struct actrail_go_tls_read_buffer *found;
    struct actrail_go_tls_read_buffer read_state = {};
    __u64 source_ptr = ACTRAIL_GO_UPROBE_ARG2(ctx);
    __u64 copy_size = positive_uprobe_isize(ACTRAIL_GO_UPROBE_ARG3(ctx));
    __u64 capture_size;
    int stored = 0;

    if (!ACTRAIL_GO_UPROBE_ARG1(ctx) || !source_ptr || !copy_size) {
        return 0;
    }
    key.tgid = host_tgid;
    key.buffer_ptr = ACTRAIL_GO_UPROBE_ARG1(ctx);
    found = bpf_map_lookup_elem(&go_tls_read_buffers, &key);
    if (!found) {
        return 0;
    }
    read_state = *found;
    bpf_map_delete_elem(&go_tls_read_buffers, &key);
    capture_size = copy_size;
    if (capture_size > read_state.requested_size) {
        capture_size = read_state.requested_size;
    }
    if (!capture_size) {
        return 0;
    }
    stored = store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_INBOUND, ACTRAIL_TLS_SYMBOL_GO_CONN_READ),
        read_state.stream_key,
        source_ptr,
        capture_size,
        0
    );
    if (stored != 1) {
        return 0;
    }
    return emit_tls_payload_completion(ctx, capture_size, 0);
}

SEC("uprobe")
int handle_gnutls_record_send_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_OUTBOUND, ACTRAIL_TLS_SYMBOL_GNUTLS_RECORD_SEND),
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx),
        ACTRAIL_UPROBE_ARG3(ctx),
        0
    );
}

SEC("uprobe")
int handle_gnutls_record_send_exit(struct pt_regs *ctx) {
    return emit_tls_payload_completion_from_isize_return(ctx);
}

SEC("uprobe")
int handle_gnutls_record_recv_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_INBOUND, ACTRAIL_TLS_SYMBOL_GNUTLS_RECORD_RECV),
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx),
        ACTRAIL_UPROBE_ARG3(ctx),
        0
    );
}

SEC("uprobe")
int handle_gnutls_record_recv_exit(struct pt_regs *ctx) {
    return emit_tls_payload_completion_from_isize_return(ctx);
}

SEC("uprobe")
int handle_nspr_pr_write_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_OUTBOUND, ACTRAIL_TLS_SYMBOL_NSPR_PR_WRITE),
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx),
        positive_uprobe_i32(ACTRAIL_UPROBE_ARG3(ctx)),
        0
    );
}

SEC("uprobe")
int handle_nspr_pr_write_exit(struct pt_regs *ctx) {
    return emit_tls_payload_completion_from_return(ctx);
}

SEC("uprobe")
int handle_nspr_pr_send_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_OUTBOUND, ACTRAIL_TLS_SYMBOL_NSPR_PR_SEND),
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx),
        positive_uprobe_i32(ACTRAIL_UPROBE_ARG3(ctx)),
        0
    );
}

SEC("uprobe")
int handle_nspr_pr_send_exit(struct pt_regs *ctx) {
    return emit_tls_payload_completion_from_return(ctx);
}

SEC("uprobe")
int handle_nspr_pr_read_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_INBOUND, ACTRAIL_TLS_SYMBOL_NSPR_PR_READ),
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx),
        positive_uprobe_i32(ACTRAIL_UPROBE_ARG3(ctx)),
        0
    );
}

SEC("uprobe")
int handle_nspr_pr_read_exit(struct pt_regs *ctx) {
    return emit_tls_payload_completion_from_return(ctx);
}

SEC("uprobe")
int handle_nspr_pr_recv_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
        ctx,
        tls_op_metadata(ACTRAIL_TLS_PAYLOAD_INBOUND, ACTRAIL_TLS_SYMBOL_NSPR_PR_RECV),
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx),
        positive_uprobe_i32(ACTRAIL_UPROBE_ARG3(ctx)),
        0
    );
}

SEC("uprobe")
int handle_nspr_pr_recv_exit(struct pt_regs *ctx) {
    return emit_tls_payload_completion_from_return(ctx);
}

#endif
