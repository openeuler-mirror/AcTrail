#include "actrail_fd.h"
#include "actrail_net.h"
#include "actrail_file.h"
#include "file/actrail_file_open.h"
#include "actrail_proc.h"
#include "actrail_tls_payload.h"
#include "payload/actrail_socket_payload.h"
#include "payload/actrail_stdio_payload.h"
#include "process/actrail_process_programs.h"

SEC("tracepoint/syscalls/sys_enter_exit")
int handle_sys_enter_exit(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_exit_op(ctx, 0);
}

SEC("tracepoint/syscalls/sys_enter_exit_group")
int handle_sys_enter_exit_group(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_exit_op(ctx, 1);
}

SEC("tracepoint/signal/signal_generate")
int handle_signal_generate(struct signal_generate_ctx *ctx) {
    __u32 pid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id = lookup_current_trace(&pid, &tid, &lookup_flags);
    struct actrail_event event;

    if (!pid || !trace_id) {
        return 0;
    }

    init_event(&event, ACTRAIL_PROC_SIGNAL, pid, *trace_id);
    event.aux = ACTRAIL_PROC_COORD_TRACEPOINT_SIGNAL_GENERATE;
    event.result = ctx->signal_result;
    event.fd = (__u32)ctx->sig;
    event.reserved = (__u32)ctx->group;
    event.requested_size = (__u64)ctx->pid;
    return emit_event(ctx, &event);
}

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

SEC("tracepoint/syscalls/sys_enter_pipe")
int handle_sys_enter_pipe(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_ipc_fd_pair_op(
        ctx,
        ACTRAIL_FILE_IPC_FD_PIPE,
        0,
        0,
        0
    );
}

SEC("tracepoint/syscalls/sys_exit_pipe")
int handle_sys_exit_pipe(struct trace_event_raw_sys_exit *ctx) {
    return emit_ipc_fd_pair_exit(ctx, ACTRAIL_FILE_SYSCALL_PIPE);
}

SEC("tracepoint/syscalls/sys_enter_pipe2")
int handle_sys_enter_pipe2(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_ipc_fd_pair_op(
        ctx,
        ACTRAIL_FILE_IPC_FD_PIPE,
        0,
        0,
        (__u32)ctx->args[1]
    );
}

SEC("tracepoint/syscalls/sys_exit_pipe2")
int handle_sys_exit_pipe2(struct trace_event_raw_sys_exit *ctx) {
    return emit_ipc_fd_pair_exit(ctx, ACTRAIL_FILE_SYSCALL_PIPE2);
}

SEC("tracepoint/syscalls/sys_enter_socketpair")
int handle_sys_enter_socketpair(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_ipc_fd_pair_op(
        ctx,
        ACTRAIL_FILE_IPC_FD_UNIX_SOCKET,
        3,
        (__u32)ctx->args[0],
        (__u32)ctx->args[1]
    );
}

SEC("tracepoint/syscalls/sys_exit_socketpair")
int handle_sys_exit_socketpair(struct trace_event_raw_sys_exit *ctx) {
    return emit_ipc_fd_pair_exit(ctx, ACTRAIL_FILE_SYSCALL_SOCKETPAIR);
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

SEC("tracepoint/syscalls/sys_enter_open")
int handle_sys_enter_open(struct trace_event_raw_sys_enter *ctx) {
    fd_open_enter(ctx, (__u64)ctx->args[1]);
    return emit_file_open_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_open")
int handle_sys_exit_open(struct trace_event_raw_sys_exit *ctx) {
    fd_register_open_exit(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_OPEN, ACTRAIL_FILE_SYSCALL_OPEN);
}

SEC("tracepoint/syscalls/sys_enter_openat")
int handle_sys_enter_openat(struct trace_event_raw_sys_enter *ctx) {
    fd_open_enter(ctx, (__u64)ctx->args[2]);
    return emit_file_openat_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_openat")
int handle_sys_exit_openat(struct trace_event_raw_sys_exit *ctx) {
    fd_register_open_exit(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_OPEN, ACTRAIL_FILE_SYSCALL_OPENAT);
}

SEC("tracepoint/syscalls/sys_enter_openat2")
int handle_sys_enter_openat2(struct trace_event_raw_sys_enter *ctx) {
    struct actrail_open_how how = {};
    read_file_open_how(ctx, &how);
    fd_open_enter(ctx, how.flags);
    return emit_file_openat2_enter(ctx, &how);
}

SEC("tracepoint/syscalls/sys_exit_openat2")
int handle_sys_exit_openat2(struct trace_event_raw_sys_exit *ctx) {
    fd_register_open_exit(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_OPEN, ACTRAIL_FILE_SYSCALL_OPENAT2);
}

SEC("tracepoint/syscalls/sys_enter_creat")
int handle_sys_enter_creat(struct trace_event_raw_sys_enter *ctx) {
    fd_open_enter(ctx, 0);
    return emit_file_creat_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_creat")
int handle_sys_exit_creat(struct trace_event_raw_sys_exit *ctx) {
    fd_register_open_exit(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_OPEN, ACTRAIL_FILE_SYSCALL_CREAT);
}

SEC("tracepoint/syscalls/sys_enter_unlinkat")
int handle_sys_enter_unlinkat(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_unlinkat_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_unlinkat")
int handle_sys_exit_unlinkat(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_UNLINK, ACTRAIL_FILE_SYSCALL_UNLINKAT);
}

SEC("tracepoint/syscalls/sys_enter_renameat")
int handle_sys_enter_renameat(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_renameat_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_renameat")
int handle_sys_exit_renameat(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_RENAME, ACTRAIL_FILE_SYSCALL_RENAMEAT);
}

SEC("tracepoint/syscalls/sys_enter_mkdirat")
int handle_sys_enter_mkdirat(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_mkdirat_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_mkdirat")
int handle_sys_exit_mkdirat(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_MKDIR, ACTRAIL_FILE_SYSCALL_MKDIRAT);
}

SEC("tracepoint/syscalls/sys_enter_mmap")
int handle_sys_enter_mmap(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_mmap_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_mmap")
int handle_sys_exit_mmap(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_MMAP, ACTRAIL_FILE_SYSCALL_MMAP);
}

SEC("tracepoint/syscalls/sys_enter_close")
int handle_sys_enter_close(struct trace_event_raw_sys_enter *ctx) {
    store_file_bulk_read_fast_close_op(ctx);
    fd_close_dispatch_enter(ctx);
    if (suppressed_fd_close_enter(ctx)) {
        socket_payload_close_enter(ctx);
        return 0;
    }
    socket_payload_close_enter(ctx);
    return emit_file_close_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_close")
int handle_sys_exit_close(struct trace_event_raw_sys_exit *ctx) {
    fd_close_dispatch_exit(ctx);
    emit_file_bulk_read_fast_close_op(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_CLOSE);
}

SEC("tracepoint/syscalls/sys_enter_close_range")
int handle_sys_enter_close_range(struct trace_event_raw_sys_enter *ctx) {
    fd_close_range_dispatch_enter(ctx);
    return emit_file_close_range_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_close_range")
int handle_sys_exit_close_range(struct trace_event_raw_sys_exit *ctx) {
    fd_close_dispatch_exit(ctx);
    return emit_file_exit(
        ctx,
        ACTRAIL_FILE_CONTEXT,
        ACTRAIL_FILE_SYSCALL_CLOSE_RANGE
    );
}

SEC("tracepoint/syscalls/sys_enter_dup")
int handle_sys_enter_dup(struct trace_event_raw_sys_enter *ctx) {
    fd_dup_enter(ctx, (__u32)ctx->args[0], 0, ACTRAIL_FD_DUP_RET_FD, 0);
    if (suppressed_fd_dup_enter(
            (__u32)ctx->args[0],
            0,
            0,
            ACTRAIL_SUPPRESSED_FD_DUP_RET_FD
        )) {
        socket_payload_dup_enter(
            ctx,
            0,
            ACTRAIL_SYSCALL_ARG_MISSING,
            ACTRAIL_SOCKET_DUP_RET_FD
        );
        return 0;
    }
    socket_payload_dup_enter(
        ctx,
        0,
        ACTRAIL_SYSCALL_ARG_MISSING,
        ACTRAIL_SOCKET_DUP_RET_FD
    );
    store_file_bulk_read_fast_dup_op(
        (__u32)ctx->args[0],
        0,
        0,
        ACTRAIL_FILE_BULK_READ_FAST_DUP_RET_FD
    );
    return emit_file_dup_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_dup")
int handle_sys_exit_dup(struct trace_event_raw_sys_exit *ctx) {
    fd_dup_exit(ctx);
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    emit_file_bulk_read_fast_dup_op(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_DUP);
}

SEC("tracepoint/syscalls/sys_enter_dup2")
int handle_sys_enter_dup2(struct trace_event_raw_sys_enter *ctx) {
    fd_dup_enter(
        ctx,
        (__u32)ctx->args[0],
        (__u32)ctx->args[1],
        ACTRAIL_FD_DUP_TARGET_FD,
        0
    );
    if (suppressed_fd_dup_enter(
            (__u32)ctx->args[0],
            (__u32)ctx->args[1],
            1,
            ACTRAIL_SUPPRESSED_FD_DUP_TARGET_FD
        )) {
        socket_payload_dup_enter(ctx, 0, 1, ACTRAIL_SOCKET_DUP_TARGET_FD);
        return 0;
    }
    socket_payload_dup_enter(ctx, 0, 1, ACTRAIL_SOCKET_DUP_TARGET_FD);
    store_file_bulk_read_fast_dup_op(
        (__u32)ctx->args[0],
        (__u32)ctx->args[1],
        1,
        ACTRAIL_FILE_BULK_READ_FAST_DUP_TARGET_FD
    );
    return emit_file_dup2_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_dup2")
int handle_sys_exit_dup2(struct trace_event_raw_sys_exit *ctx) {
    fd_dup_exit(ctx);
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    emit_file_bulk_read_fast_dup_op(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_DUP2);
}

SEC("tracepoint/syscalls/sys_enter_dup3")
int handle_sys_enter_dup3(struct trace_event_raw_sys_enter *ctx) {
    fd_dup_enter(
        ctx,
        (__u32)ctx->args[0],
        (__u32)ctx->args[1],
        ACTRAIL_FD_DUP_TARGET_FD,
        fd_creation_flags((__u64)ctx->args[2])
    );
    if (suppressed_fd_dup_enter(
            (__u32)ctx->args[0],
            (__u32)ctx->args[1],
            1,
            ACTRAIL_SUPPRESSED_FD_DUP_TARGET_FD
        )) {
        socket_payload_dup_enter(ctx, 0, 1, ACTRAIL_SOCKET_DUP_TARGET_FD);
        return 0;
    }
    socket_payload_dup_enter(ctx, 0, 1, ACTRAIL_SOCKET_DUP_TARGET_FD);
    store_file_bulk_read_fast_dup_op(
        (__u32)ctx->args[0],
        (__u32)ctx->args[1],
        1,
        ACTRAIL_FILE_BULK_READ_FAST_DUP_TARGET_FD
    );
    return emit_file_dup3_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_dup3")
int handle_sys_exit_dup3(struct trace_event_raw_sys_exit *ctx) {
    fd_dup_exit(ctx);
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    emit_file_bulk_read_fast_dup_op(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_DUP3);
}

SEC("tracepoint/syscalls/sys_enter_fcntl")
int handle_sys_enter_fcntl(struct trace_event_raw_sys_enter *ctx) {
    __u32 command = (__u32)ctx->args[1];
    if (command == F_DUPFD || command == F_DUPFD_CLOEXEC) {
        fd_dup_enter(
            ctx,
            (__u32)ctx->args[0],
            0,
            ACTRAIL_FD_DUP_RET_FD,
            command == F_DUPFD_CLOEXEC ? ACTRAIL_FD_FLAG_CLOEXEC : 0
        );
    }
    fd_fcntl_flag_enter(ctx);
    if (suppressed_fd_fcntl_enter(ctx)) {
        socket_payload_fcntl_enter(ctx);
        return 0;
    }
    socket_payload_fcntl_enter(ctx);
    store_file_bulk_read_fast_fcntl_op(ctx);
    return emit_file_fcntl_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_fcntl")
int handle_sys_exit_fcntl(struct trace_event_raw_sys_exit *ctx) {
    fd_dup_exit(ctx);
    fd_flag_exit(ctx);
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    emit_file_bulk_read_fast_dup_op(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_FCNTL);
}

SEC("tracepoint/syscalls/sys_enter_ioctl")
int handle_sys_enter_ioctl(struct trace_event_raw_sys_enter *ctx) {
    __u32 context_kind = fd_ioctl_flag_enter(ctx);

    return emit_file_ioctl_flag_enter(ctx, context_kind);
}

SEC("tracepoint/syscalls/sys_exit_ioctl")
int handle_sys_exit_ioctl(struct trace_event_raw_sys_exit *ctx) {
    __u32 syscall_id = file_ioctl_flag_syscall_id(fd_flag_exit(ctx));

    return syscall_id
        ? emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, syscall_id)
        : 0;
}

SEC("tracepoint/syscalls/sys_enter_chdir")
int handle_sys_enter_chdir(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_chdir_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_chdir")
int handle_sys_exit_chdir(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_CHDIR);
}

SEC("tracepoint/syscalls/sys_enter_fchdir")
int handle_sys_enter_fchdir(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_fchdir_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_fchdir")
int handle_sys_exit_fchdir(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_FCHDIR);
}

char LICENSE[] SEC("license") = "GPL";
