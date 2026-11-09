/* Programs owned by the tls/rustls loading node. */
#include "../definitions.h"

SEC("uprobe")
int handle_rustls_buffer_plaintext(struct pt_regs *ctx) {
    return store_rustls_write(
        ctx,
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx)
    );
}

SEC("uprobe")
int handle_rustls_buffer_plaintext_exit(struct pt_regs *ctx) {
    return emit_rustls_completed_write(ctx, ACTRAIL_UPROBE_RET(ctx));
}

SEC("uprobe")
int handle_rustls_start_outgoing_traffic(struct pt_regs *ctx) {
    return register_rustls_connection(ctx, ACTRAIL_UPROBE_ARG1(ctx));
}

SEC("uprobe")
int handle_rustls_take_received_plaintext(struct pt_regs *ctx) {
    return emit_rustls_received_payload(
        ctx,
        ACTRAIL_UPROBE_ARG1(ctx),
        ACTRAIL_UPROBE_ARG2(ctx)
    );
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

#ifndef ACTRAIL_COMBINED_OBJECT
char LICENSE[] SEC("license") = "GPL";
#endif
