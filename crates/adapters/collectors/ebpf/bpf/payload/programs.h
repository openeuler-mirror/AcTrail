#ifndef ACTRAIL_PAYLOAD_PROGRAMS_H
#define ACTRAIL_PAYLOAD_PROGRAMS_H

#include "socket_capture.h"
#include "stdio_capture.h"
#include "../network/observe.h"
#include "../file/bulk_read.h"

SEC("tracepoint/syscalls/sys_enter_write")
int handle_sys_enter_write(struct trace_event_raw_sys_enter *ctx) {
    store_stdio_payload_op(ctx, ACTRAIL_STDIO_SYSCALL_WRITE);
    store_socket_payload_write_op(ctx);
    return store_pending_net_op_resolved(
        net_descriptor(ACTRAIL_FD_IO_SEND, ACTRAIL_SYSCALL_FAMILY_FD_IO),
        (__u32)ctx->args[0],
        (__u64)ctx->args[2],
        0
    );
}

SEC("tracepoint/syscalls/sys_exit_write")
int handle_sys_exit_write(struct trace_event_raw_sys_exit *ctx) {
    emit_stdio_payload_op(ctx);
    emit_socket_payload_op(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_read")
int handle_sys_enter_read(struct trace_event_raw_sys_enter *ctx) {
    store_stdio_payload_op(ctx, ACTRAIL_STDIO_SYSCALL_READ);
    store_socket_payload_read_op(ctx);
    if (store_file_bulk_read_fast_read_op(ctx)) {
        return 0;
    }
    return store_pending_net_op_resolved(
        net_descriptor(ACTRAIL_FD_IO_RECV, ACTRAIL_SYSCALL_FAMILY_FD_IO),
        (__u32)ctx->args[0],
        (__u64)ctx->args[2],
        0
    );
}

SEC("tracepoint/syscalls/sys_exit_read")
int handle_sys_exit_read(struct trace_event_raw_sys_exit *ctx) {
    emit_stdio_payload_op(ctx);
    emit_socket_payload_op(ctx);
    if (emit_file_bulk_read_fast_read_op(ctx)) {
        return 0;
    }
    return emit_pending_net_op(ctx);
}


#endif
