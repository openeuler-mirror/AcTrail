/* Programs owned by the tls/gnutls loading node. */
#include "../definitions.h"

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

#ifndef ACTRAIL_COMBINED_OBJECT
char LICENSE[] SEC("license") = "GPL";
#endif
