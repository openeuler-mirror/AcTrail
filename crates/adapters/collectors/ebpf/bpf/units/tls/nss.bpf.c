/* Programs owned by the tls/nss loading node. */
#include "../definitions.h"

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

#ifndef ACTRAIL_COMBINED_OBJECT
char LICENSE[] SEC("license") = "GPL";
#endif
