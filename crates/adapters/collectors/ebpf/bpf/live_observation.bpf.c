#include "actrail_net.h"
#include "actrail_file.h"
#include "file/actrail_file_open.h"
#include "actrail_proc.h"
#include "actrail_tls_payload.h"
#include "payload/actrail_socket_payload.h"
#include "payload/actrail_stdio_payload.h"

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u32);
} fork_child_pid_offset SEC(".maps");

SEC("tracepoint/sched/sched_process_fork")
int handle_sched_process_fork(struct sched_process_fork_ctx *ctx) {
    __u32 key = 0;
    __u32 *child_pid_offset = bpf_map_lookup_elem(&fork_child_pid_offset, &key);
    __u32 parent_pid = current_namespace_tgid();
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &parent_pid);
    struct actrail_pending_proc_op op = {};
    __u32 child_global_pid = 0;

    if (!parent_pid || !trace_id || !child_pid_offset) {
        return 0;
    }
    if (bpf_probe_read(
            &child_global_pid,
            sizeof(child_global_pid),
            (void *)((__u64)(void *)ctx + *child_pid_offset)
        ) != 0) {
        return 0;
    }
    if (!child_global_pid) {
        return 0;
    }

    op.trace_id = *trace_id;
    op.parent_generation = ensure_process_generation(parent_pid);
    op.child_generation = bpf_ktime_get_ns();
    op.parent_pid = parent_pid;
    bpf_map_update_elem(&pending_child_proc_ops, &child_global_pid, &op, BPF_ANY);
    return 0;
}

SEC("tracepoint/sched/sched_process_exec")
int handle_sched_process_exec(struct sched_process_exec_ctx *ctx) {
    __u32 pid = current_namespace_tgid();
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &pid);

    if (!pid) {
        return 0;
    }
    emit_pending_child_proc_op();
    trace_id = bpf_map_lookup_elem(&tracked_traces, &pid);
    if (!trace_id) {
        return 0;
    }

    return emit_exec_proc_event(ctx, pid, *trace_id);
}

SEC("tracepoint/sched/sched_process_exit")
int handle_sched_process_exit(struct sched_process_exit_ctx *ctx) {
    __u64 pid_tgid = current_pid_tgid();
    __u64 namespace_pid_tgid = current_namespace_pid_tgid();
    __u32 pid = namespace_pid_tgid >> 32;
    __u32 tid = (__u32)namespace_pid_tgid;
    __u64 *trace_id;
    struct actrail_event event;

    if (!namespace_pid_tgid) {
        pid = pid_tgid >> 32;
        tid = (__u32)pid_tgid;
    }
    if (!pid) {
        return 0;
    }
    if (pid != tid) {
        return 0;
    }
    trace_id = bpf_map_lookup_elem(&tracked_traces, &pid);
    emit_pending_child_proc_op();
    trace_id = bpf_map_lookup_elem(&tracked_traces, &pid);
    if (!trace_id) {
        return 0;
    }

    init_event(&event, ACTRAIL_PROC_EXIT, pid, *trace_id);
    attach_exit_code(&event, pid_tgid);
    emit_event(&event);
    cleanup_suppressed_fds_for_process(pid, event.pid_generation);
    bpf_map_delete_elem(&tracked_traces, &pid);
    delete_process_generation(pid);
    return 0;
}

SEC("tracepoint/syscalls/sys_enter_exit")
int handle_sys_enter_exit(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_exit_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_exit_group")
int handle_sys_enter_exit_group(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_exit_op(ctx);
}

SEC("tracepoint/signal/signal_generate")
int handle_signal_generate(struct signal_generate_ctx *ctx) {
    __u32 pid = current_namespace_tgid();
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &pid);
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
    return emit_event(&event);
}

SEC("tracepoint/syscalls/sys_enter_connect")
int handle_sys_enter_connect(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_net_op(
        ctx,
        ACTRAIL_NET_CONNECT,
        0,
        ACTRAIL_SYSCALL_ARG_MISSING,
        1,
        ACTRAIL_NET_SYSCALL_SOCKET
    );
}

SEC("tracepoint/syscalls/sys_exit_connect")
int handle_sys_exit_connect(struct trace_event_raw_sys_exit *ctx) {
    socket_payload_track_connect_exit(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_accept")
int handle_sys_enter_accept(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_net_op(
        ctx,
        ACTRAIL_NET_ACCEPT,
        0,
        ACTRAIL_SYSCALL_ARG_MISSING,
        1,
        ACTRAIL_NET_SYSCALL_SOCKET
    );
}

SEC("tracepoint/syscalls/sys_enter_accept4")
int handle_sys_enter_accept4(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_net_op(
        ctx,
        ACTRAIL_NET_ACCEPT,
        0,
        ACTRAIL_SYSCALL_ARG_MISSING,
        1,
        ACTRAIL_NET_SYSCALL_SOCKET
    );
}

SEC("tracepoint/syscalls/sys_exit_accept")
int handle_sys_exit_accept(struct trace_event_raw_sys_exit *ctx) {
    socket_payload_track_accept_exit(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_exit_accept4")
int handle_sys_exit_accept4(struct trace_event_raw_sys_exit *ctx) {
    socket_payload_track_accept_exit(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_sendto")
int handle_sys_enter_sendto(struct trace_event_raw_sys_enter *ctx) {
    store_socket_payload_sendto_op(ctx);
    return store_pending_net_op(ctx, ACTRAIL_NET_SEND, 0, 2, 4, ACTRAIL_NET_SYSCALL_SOCKET);
}

SEC("tracepoint/syscalls/sys_exit_sendto")
int handle_sys_exit_sendto(struct trace_event_raw_sys_exit *ctx) {
    emit_socket_payload_op(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_writev")
int handle_sys_enter_writev(struct trace_event_raw_sys_enter *ctx) {
    store_socket_payload_writev_op(ctx);
    return 0;
}

SEC("tracepoint/syscalls/sys_exit_writev")
int handle_sys_exit_writev(struct trace_event_raw_sys_exit *ctx) {
    return emit_socket_payload_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_sendmsg")
int handle_sys_enter_sendmsg(struct trace_event_raw_sys_enter *ctx) {
    store_socket_payload_sendmsg_op(ctx);
    return 0;
}

SEC("tracepoint/syscalls/sys_exit_sendmsg")
int handle_sys_exit_sendmsg(struct trace_event_raw_sys_exit *ctx) {
    return emit_socket_payload_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_recvfrom")
int handle_sys_enter_recvfrom(struct trace_event_raw_sys_enter *ctx) {
    store_socket_payload_recvfrom_op(ctx);
    return store_pending_net_op(ctx, ACTRAIL_NET_RECV, 0, 2, 4, ACTRAIL_NET_SYSCALL_SOCKET);
}

SEC("tracepoint/syscalls/sys_exit_recvfrom")
int handle_sys_exit_recvfrom(struct trace_event_raw_sys_exit *ctx) {
    emit_socket_payload_op(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_bind")
int handle_sys_enter_bind(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_net_op(
        ctx,
        ACTRAIL_NET_BIND,
        0,
        ACTRAIL_SYSCALL_ARG_MISSING,
        1,
        ACTRAIL_NET_SYSCALL_SOCKET
    );
}

SEC("tracepoint/syscalls/sys_exit_bind")
int handle_sys_exit_bind(struct trace_event_raw_sys_exit *ctx) {
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_listen")
int handle_sys_enter_listen(struct trace_event_raw_sys_enter *ctx) {
    return store_pending_net_op(
        ctx,
        ACTRAIL_NET_LISTEN,
        0,
        ACTRAIL_SYSCALL_ARG_MISSING,
        ACTRAIL_SYSCALL_ARG_MISSING,
        ACTRAIL_NET_SYSCALL_SOCKET
    );
}

SEC("tracepoint/syscalls/sys_exit_listen")
int handle_sys_exit_listen(struct trace_event_raw_sys_exit *ctx) {
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_write")
int handle_sys_enter_write(struct trace_event_raw_sys_enter *ctx) {
    store_stdio_payload_op(ctx, ACTRAIL_STDIO_SYSCALL_WRITE);
    store_socket_payload_write_op(ctx);
    return store_pending_net_op(
        ctx,
        ACTRAIL_NET_SEND,
        0,
        2,
        ACTRAIL_SYSCALL_ARG_MISSING,
        ACTRAIL_NET_SYSCALL_FD_IO
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
    return store_pending_net_op(
        ctx,
        ACTRAIL_NET_RECV,
        0,
        2,
        ACTRAIL_SYSCALL_ARG_MISSING,
        ACTRAIL_NET_SYSCALL_FD_IO
    );
}

SEC("tracepoint/syscalls/sys_exit_read")
int handle_sys_exit_read(struct trace_event_raw_sys_exit *ctx) {
    emit_stdio_payload_op(ctx);
    emit_socket_payload_op(ctx);
    return emit_pending_net_op(ctx);
}

SEC("tracepoint/syscalls/sys_enter_open")
int handle_sys_enter_open(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_open_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_open")
int handle_sys_exit_open(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_OPEN, ACTRAIL_FILE_SYSCALL_OPEN);
}

SEC("tracepoint/syscalls/sys_enter_openat")
int handle_sys_enter_openat(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_openat_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_openat")
int handle_sys_exit_openat(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_OPEN, ACTRAIL_FILE_SYSCALL_OPENAT);
}

SEC("tracepoint/syscalls/sys_enter_openat2")
int handle_sys_enter_openat2(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_openat2_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_openat2")
int handle_sys_exit_openat2(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_OPEN, ACTRAIL_FILE_SYSCALL_OPENAT2);
}

SEC("tracepoint/syscalls/sys_enter_creat")
int handle_sys_enter_creat(struct trace_event_raw_sys_enter *ctx) {
    return emit_file_creat_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_creat")
int handle_sys_exit_creat(struct trace_event_raw_sys_exit *ctx) {
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
    if (suppressed_fd_close_enter(ctx)) {
        socket_payload_close_enter(ctx);
        return 0;
    }
    socket_payload_close_enter(ctx);
    return emit_file_close_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_close")
int handle_sys_exit_close(struct trace_event_raw_sys_exit *ctx) {
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_CLOSE);
}

SEC("tracepoint/syscalls/sys_enter_dup")
int handle_sys_enter_dup(struct trace_event_raw_sys_enter *ctx) {
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
    return emit_file_dup_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_dup")
int handle_sys_exit_dup(struct trace_event_raw_sys_exit *ctx) {
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_DUP);
}

SEC("tracepoint/syscalls/sys_enter_dup2")
int handle_sys_enter_dup2(struct trace_event_raw_sys_enter *ctx) {
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
    return emit_file_dup2_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_dup2")
int handle_sys_exit_dup2(struct trace_event_raw_sys_exit *ctx) {
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_DUP2);
}

SEC("tracepoint/syscalls/sys_enter_dup3")
int handle_sys_enter_dup3(struct trace_event_raw_sys_enter *ctx) {
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
    return emit_file_dup3_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_dup3")
int handle_sys_exit_dup3(struct trace_event_raw_sys_exit *ctx) {
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_DUP3);
}

SEC("tracepoint/syscalls/sys_enter_fcntl")
int handle_sys_enter_fcntl(struct trace_event_raw_sys_enter *ctx) {
    if (suppressed_fd_fcntl_enter(ctx)) {
        socket_payload_fcntl_enter(ctx);
        return 0;
    }
    socket_payload_fcntl_enter(ctx);
    return emit_file_fcntl_enter(ctx);
}

SEC("tracepoint/syscalls/sys_exit_fcntl")
int handle_sys_exit_fcntl(struct trace_event_raw_sys_exit *ctx) {
    suppressed_fd_dup_exit(ctx);
    socket_payload_dup_exit(ctx);
    return emit_file_exit(ctx, ACTRAIL_FILE_CONTEXT, ACTRAIL_FILE_SYSCALL_FCNTL);
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
