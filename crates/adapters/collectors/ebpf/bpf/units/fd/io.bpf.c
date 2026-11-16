/* Programs owned by the fd/io loading node. */
#include "../definitions.h"

SEC("tracepoint/syscalls/sys_enter_read")
int handle_sys_enter_read(struct trace_event_raw_sys_enter *ctx) {
    store_stdio_payload_op(ctx, ACTRAIL_STDIO_SYSCALL_READ);
    store_socket_payload_read_op(ctx);
    return store_pending_net_op_resolved(
        net_descriptor(ACTRAIL_FD_IO_RECV, ACTRAIL_SYSCALL_FAMILY_FD_IO),
        (__u32)ctx->args[0],
        (__u64)ctx->args[2],
        0
    );
}

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

SEC("tracepoint/syscalls/sys_enter_writev")
int handle_sys_enter_writev(struct trace_event_raw_sys_enter *ctx) {
    store_socket_payload_writev_op(ctx);
    return store_pending_net_op_resolved(
        net_descriptor(ACTRAIL_FD_IO_SEND, ACTRAIL_SYSCALL_FAMILY_FD_IO_WRITEV),
        (__u32)ctx->args[0],
        0,
        0
    );
}

SEC("tracepoint/syscalls/sys_exit_read")
int handle_sys_exit_read(struct trace_event_raw_sys_exit *ctx) {
    emit_stdio_payload_op(ctx);
    emit_socket_payload_op(ctx, 0);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_exit_write")
int handle_sys_exit_write(struct trace_event_raw_sys_exit *ctx) {
    emit_stdio_payload_op(ctx);
    emit_socket_payload_op(ctx, 0);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_exit_writev")
int handle_sys_exit_writev(struct trace_event_raw_sys_exit *ctx) {
    emit_socket_payload_op(ctx, 1);
    return emit_pending_net_op(ctx);
}

#ifndef ACTRAIL_COMBINED_OBJECT
char LICENSE[] SEC("license") = "GPL";
#endif
