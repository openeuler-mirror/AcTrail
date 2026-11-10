/* Shared domain definitions; BPF entry programs reside in units/. */
#ifndef ACTRAIL_TLS_BORINGSSL_H
#define ACTRAIL_TLS_BORINGSSL_H

#include "capture.h"

/* The entry program identifies the provider without newer-kernel attach cookies.
 * Completion reuses the SSL ABI and the library already stored in the operation. */
static __always_inline int store_boringssl_payload_op(
    struct pt_regs *ctx, __u32 direction, __u32 symbol
) {
    struct actrail_tls_payload_op_args args = {};
    args.metadata = tls_op_metadata(direction, symbol);
    args.library = ACTRAIL_TLS_LIBRARY_BORINGSSL;
    args.stream_key = ACTRAIL_UPROBE_ARG1(ctx);
    args.buffer_ptr = ACTRAIL_UPROBE_ARG2(ctx);
    args.requested_size = ACTRAIL_UPROBE_ARG3(ctx);
    return store_tls_payload_op_args(ctx, &args);
}

#endif
