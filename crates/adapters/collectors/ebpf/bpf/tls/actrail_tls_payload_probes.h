#ifndef ACTRAIL_TLS_PAYLOAD_PROBES_H
#define ACTRAIL_TLS_PAYLOAD_PROBES_H

SEC("uprobe")
int handle_ssl_write_enter(struct pt_regs *ctx) {
    return store_tls_payload_op(
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

#endif
