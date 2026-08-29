#ifndef ACTRAIL_NETWORK_PROGRAMS_H
#define ACTRAIL_NETWORK_PROGRAMS_H

#include "observe.h"
#include "../payload/socket_capture.h"

SEC("tracepoint/syscalls/sys_enter_socket")
int handle_sys_enter_socket(struct trace_event_raw_sys_enter *ctx) {
    return fd_socket_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_socket")
int handle_sys_exit_socket(struct trace_event_raw_sys_exit *ctx) {
    return fd_socket_exit(ctx);
}

SEC("tracepoint/syscalls/sys_enter_connect")
int handle_sys_enter_connect(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_net_op_resolved(
        net_descriptor(ACTRAIL_NET_CONNECT, ACTRAIL_SYSCALL_FAMILY_SOCKET),
        (__u32)ctx->args[0],
        0,
        (__u64)ctx->args[1]
    );
}

SEC("tracepoint/syscalls/sys_exit_connect")
int handle_sys_exit_connect(struct trace_event_raw_sys_exit *ctx) {
    socket_payload_track_connect_exit(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_accept")
int handle_sys_enter_accept(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_net_op_resolved(
        net_descriptor(ACTRAIL_NET_ACCEPT, ACTRAIL_SYSCALL_FAMILY_SOCKET),
        (__u32)ctx->args[0],
        0,
        (__u64)ctx->args[1]
    );
}

SEC("tracepoint/syscalls/sys_enter_accept4")
int handle_sys_enter_accept4(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_net_op_with_flags(
        net_descriptor(ACTRAIL_NET_ACCEPT, ACTRAIL_SYSCALL_FAMILY_SOCKET),
        (__u32)ctx->args[0],
        0,
        (__u64)ctx->args[1],
        (__u32)ctx->args[3]
    );
}

SEC("tracepoint/syscalls/sys_exit_accept")
int handle_sys_exit_accept(struct trace_event_raw_sys_exit *ctx) {
    socket_payload_track_accept_exit(ctx);
    fd_accept_exit(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_exit_accept4")
int handle_sys_exit_accept4(struct trace_event_raw_sys_exit *ctx) {
    socket_payload_track_accept_exit(ctx);
    fd_accept_exit(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_sendto")
int handle_sys_enter_sendto(struct trace_event_raw_sys_enter *ctx) {
    store_socket_payload_sendto_op(ctx);
    return store_pending_net_op_resolved(
        net_descriptor(ACTRAIL_FD_IO_SEND, ACTRAIL_SYSCALL_FAMILY_SOCKET),
        (__u32)ctx->args[0],
        (__u64)ctx->args[2],
        (__u64)ctx->args[4]
    );
}

SEC("tracepoint/syscalls/sys_exit_sendto")
int handle_sys_exit_sendto(struct trace_event_raw_sys_exit *ctx) {
    emit_socket_payload_op(ctx);
    return emit_pending_net_op(ctx);
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

SEC("tracepoint/syscalls/sys_exit_writev")
int handle_sys_exit_writev(struct trace_event_raw_sys_exit *ctx) {
    emit_socket_payload_op(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_sendmsg")
int handle_sys_enter_sendmsg(struct trace_event_raw_sys_enter *ctx) {
    store_socket_payload_sendmsg_op(ctx);
    return store_pending_net_op_resolved(
        net_descriptor(ACTRAIL_FD_IO_SEND, ACTRAIL_SYSCALL_FAMILY_SOCKET),
        (__u32)ctx->args[0],
        0,
        0
    );
}

SEC("tracepoint/syscalls/sys_exit_sendmsg")
int handle_sys_exit_sendmsg(struct trace_event_raw_sys_exit *ctx) {
    emit_socket_payload_op(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_recvfrom")
int handle_sys_enter_recvfrom(struct trace_event_raw_sys_enter *ctx) {
    store_socket_payload_recvfrom_op(ctx);
    return store_pending_net_op_resolved(
        net_descriptor(ACTRAIL_FD_IO_RECV, ACTRAIL_SYSCALL_FAMILY_SOCKET),
        (__u32)ctx->args[0],
        (__u64)ctx->args[2],
        (__u64)ctx->args[4]
    );
}

SEC("tracepoint/syscalls/sys_exit_recvfrom")
int handle_sys_exit_recvfrom(struct trace_event_raw_sys_exit *ctx) {
    emit_socket_payload_op(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_recvmsg")
int handle_sys_enter_recvmsg(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_net_op_resolved(
        net_descriptor(ACTRAIL_FD_IO_RECV, ACTRAIL_SYSCALL_FAMILY_SOCKET),
        (__u32)ctx->args[0],
        0,
        0
    );
}

SEC("tracepoint/syscalls/sys_exit_recvmsg")
int handle_sys_exit_recvmsg(struct trace_event_raw_sys_exit *ctx) {
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_bind")
int handle_sys_enter_bind(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_net_op_resolved(
        net_descriptor(ACTRAIL_NET_BIND, ACTRAIL_SYSCALL_FAMILY_SOCKET),
        (__u32)ctx->args[0],
        0,
        (__u64)ctx->args[1]
    );
}

SEC("tracepoint/syscalls/sys_exit_bind")
int handle_sys_exit_bind(struct trace_event_raw_sys_exit *ctx) {
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_listen")
int handle_sys_enter_listen(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_net_op_resolved(
        net_descriptor(ACTRAIL_NET_LISTEN, ACTRAIL_SYSCALL_FAMILY_SOCKET),
        (__u32)ctx->args[0],
        0,
        0
    );
}

SEC("tracepoint/syscalls/sys_exit_listen")
int handle_sys_exit_listen(struct trace_event_raw_sys_exit *ctx) {
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_shutdown")
int handle_sys_enter_shutdown(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_net_op_with_flags(
        net_descriptor(ACTRAIL_NET_SHUTDOWN, ACTRAIL_SYSCALL_FAMILY_SOCKET),
        (__u32)ctx->args[0],
        0,
        0,
        (__u32)ctx->args[1]
    );
}

SEC("tracepoint/syscalls/sys_exit_shutdown")
int handle_sys_exit_shutdown(struct trace_event_raw_sys_exit *ctx) {
    return emit_pending_net_op(ctx);
}


#endif
