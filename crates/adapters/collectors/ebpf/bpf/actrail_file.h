#ifndef ACTRAIL_FILE_H
#define ACTRAIL_FILE_H

#include "actrail_runtime.h"

enum actrail_file_path_abi {
    ACTRAIL_FILE_EVENT_HEADER_SIZE = 128,
    ACTRAIL_FILE_EVENT_PRIMARY_PATH_SIZE = 384,
    ACTRAIL_FILE_PATH_ABI_MAX_BYTES = 256,
    ACTRAIL_FILE_PATH_COPY_MAX_BYTES = 255,
};

enum actrail_file_fd {
    ACTRAIL_FILE_FD_MISSING = 0xffffffff,
};

enum actrail_file_path_flags {
    ACTRAIL_FILE_PATH_CAPTURED = 1,
    ACTRAIL_FILE_PATH_TRUNCATED = 2,
    ACTRAIL_FILE_PATH_FAULT = 4,
};

enum actrail_file_path_role {
    ACTRAIL_FILE_PRIMARY_PATH = 0,
    ACTRAIL_FILE_SECONDARY_PATH = 1,
};

enum actrail_file_event_phase {
    ACTRAIL_FILE_PHASE_ENTER = 1,
    ACTRAIL_FILE_PHASE_EXIT = 2,
};

enum actrail_file_syscall_id {
    ACTRAIL_FILE_SYSCALL_OPEN = 1,
    ACTRAIL_FILE_SYSCALL_OPENAT = 2,
    ACTRAIL_FILE_SYSCALL_CREAT = 3,
    ACTRAIL_FILE_SYSCALL_UNLINK = 4,
    ACTRAIL_FILE_SYSCALL_UNLINKAT = 5,
    ACTRAIL_FILE_SYSCALL_RENAME = 6,
    ACTRAIL_FILE_SYSCALL_RENAMEAT = 7,
    ACTRAIL_FILE_SYSCALL_RENAMEAT2 = 8,
    ACTRAIL_FILE_SYSCALL_MKDIR = 9,
    ACTRAIL_FILE_SYSCALL_MKDIRAT = 10,
    ACTRAIL_FILE_SYSCALL_RMDIR = 11,
    ACTRAIL_FILE_SYSCALL_TRUNCATE = 12,
    ACTRAIL_FILE_SYSCALL_FTRUNCATE = 13,
    ACTRAIL_FILE_SYSCALL_MMAP = 14,
    ACTRAIL_FILE_SYSCALL_CLOSE = 15,
    ACTRAIL_FILE_SYSCALL_DUP = 16,
    ACTRAIL_FILE_SYSCALL_DUP2 = 17,
    ACTRAIL_FILE_SYSCALL_DUP3 = 18,
    ACTRAIL_FILE_SYSCALL_FCNTL = 19,
    ACTRAIL_FILE_SYSCALL_CHDIR = 20,
    ACTRAIL_FILE_SYSCALL_FCHDIR = 21,
    ACTRAIL_FILE_SYSCALL_OPENAT2 = 22,
    ACTRAIL_FILE_SYSCALL_PIPE = 23,
    ACTRAIL_FILE_SYSCALL_PIPE2 = 24,
    ACTRAIL_FILE_SYSCALL_SOCKETPAIR = 25,
    ACTRAIL_FILE_SYSCALL_READ_SUMMARY = 26,
};

enum actrail_file_syscall_arg_count {
    ACTRAIL_FILE_SYSCALL_ARGC_CHDIR = 1,
    ACTRAIL_FILE_SYSCALL_ARGC_CLOSE = 1,
    ACTRAIL_FILE_SYSCALL_ARGC_CREAT = 2,
    ACTRAIL_FILE_SYSCALL_ARGC_DUP = 1,
    ACTRAIL_FILE_SYSCALL_ARGC_DUP2 = 2,
    ACTRAIL_FILE_SYSCALL_ARGC_DUP3 = 3,
    ACTRAIL_FILE_SYSCALL_ARGC_FCHDIR = 1,
    ACTRAIL_FILE_SYSCALL_ARGC_FCNTL = 3,
    ACTRAIL_FILE_SYSCALL_ARGC_MKDIRAT = 3,
    ACTRAIL_FILE_SYSCALL_ARGC_MMAP = 6,
    ACTRAIL_FILE_SYSCALL_ARGC_OPEN = 3,
    ACTRAIL_FILE_SYSCALL_ARGC_OPENAT = 4,
    ACTRAIL_FILE_SYSCALL_ARGC_OPENAT2 = 4,
    ACTRAIL_FILE_SYSCALL_ARGC_PIPE = 1,
    ACTRAIL_FILE_SYSCALL_ARGC_PIPE2 = 2,
    ACTRAIL_FILE_SYSCALL_ARGC_RENAMEAT = 4,
    ACTRAIL_FILE_SYSCALL_ARGC_SOCKETPAIR = 4,
    ACTRAIL_FILE_SYSCALL_ARGC_UNLINKAT = 3,
};

enum actrail_file_ipc_fd_kind {
    ACTRAIL_FILE_IPC_FD_PIPE = 1,
    ACTRAIL_FILE_IPC_FD_UNIX_SOCKET = 2,
};

enum actrail_file_enter_descriptor {
    ACTRAIL_FILE_DESCRIPTOR_KIND_BITS = 16,
    ACTRAIL_FILE_DESCRIPTOR_SYSCALL_BITS = 16,
    ACTRAIL_FILE_DESCRIPTOR_KIND_MASK = 0xffff,
    ACTRAIL_FILE_DESCRIPTOR_SYSCALL_MASK = 0xffff,
    ACTRAIL_FILE_DESCRIPTOR_SYSCALL_SHIFT = 16,
    ACTRAIL_FILE_DESCRIPTOR_ARGC_SHIFT = 32,
};

struct actrail_file_config {
    __u32 path_max_bytes;
};

struct actrail_file_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 phase;
    __s64 result;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u32 fd;
    __u32 aux;
    __u32 path_size;
    __u32 path_flags;
    __u32 secondary_path_size;
    __u32 secondary_path_flags;
    __u32 path_max_bytes;
    __u32 reserved;
    __u64 arg0;
    __u64 arg1;
    __u64 arg2;
    __u64 arg3;
    __u64 arg4;
    __u64 arg5;
    __u64 pid_generation;
    char path[ACTRAIL_FILE_PATH_ABI_MAX_BYTES];
    char secondary_path[ACTRAIL_FILE_PATH_ABI_MAX_BYTES];
} __attribute__((packed));

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_file_config);
} file_config SEC(".maps");

static __always_inline void init_file_event_header(
    struct actrail_file_event *event,
    __u32 kind
);

#include "file/actrail_file_bulk_read_fast.h"
#include "file/actrail_file_path.h"

static __always_inline int emit_file_primary_path_enter(
    struct trace_event_raw_sys_enter *ctx,
    __u64 descriptor,
    __u32 fd,
    __u64 path_ptr
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_file_event *event;
    __u64 generation;
    __u32 kind = (__u32)(descriptor & ACTRAIL_FILE_DESCRIPTOR_KIND_MASK);
    __u32 syscall_id = (__u32)(
        (descriptor >> ACTRAIL_FILE_DESCRIPTOR_SYSCALL_SHIFT) &
        ACTRAIL_FILE_DESCRIPTOR_SYSCALL_MASK
    );
    __u32 arg_count = (__u32)(descriptor >> ACTRAIL_FILE_DESCRIPTOR_ARGC_SHIFT);

    if (!tgid) {
        return 0;
    }
    if (!trace_id) {
        return 0;
    }

    event = actrail_event_reserve(ACTRAIL_FILE_EVENT_PRIMARY_PATH_SIZE);
    if (!event) {
        return 0;
    }

    init_file_event_primary_path(event, kind);
    event->pid = tgid;
    event->tid = (__u32)pid_tgid;
    generation = current_process_start_time(tgid);
    event->pid_generation = generation;
    event->phase = ACTRAIL_FILE_PHASE_ENTER;
    event->trace_id = *trace_id;
    event->aux = syscall_id;
    event->fd = fd;
    fill_file_args(event, ctx, arg_count);
    read_file_path(event, path_ptr, ACTRAIL_FILE_PRIMARY_PATH);
    actrail_event_submit(ctx, event);
    return 0;
}

static __always_inline int emit_file_full_path_enter(
    struct trace_event_raw_sys_enter *ctx,
    __u64 descriptor,
    __u32 fd,
    __u64 path_ptr,
    __u64 secondary_path_ptr
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_file_event *event;
    __u64 generation;
    __u32 kind = (__u32)(descriptor & ACTRAIL_FILE_DESCRIPTOR_KIND_MASK);
    __u32 syscall_id = (__u32)(
        (descriptor >> ACTRAIL_FILE_DESCRIPTOR_SYSCALL_SHIFT) &
        ACTRAIL_FILE_DESCRIPTOR_SYSCALL_MASK
    );
    __u32 arg_count = (__u32)(descriptor >> ACTRAIL_FILE_DESCRIPTOR_ARGC_SHIFT);

    if (!tgid) {
        return 0;
    }
    if (!trace_id) {
        return 0;
    }

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        return 0;
    }

    init_file_event(event, kind);
    event->pid = tgid;
    event->tid = (__u32)pid_tgid;
    event->pid_generation = current_process_start_time(tgid);
    event->phase = ACTRAIL_FILE_PHASE_ENTER;
    event->trace_id = *trace_id;
    event->aux = syscall_id;
    event->fd = fd;
    fill_file_args(event, ctx, arg_count);
    if (path_ptr) {
        read_file_path(event, path_ptr, ACTRAIL_FILE_PRIMARY_PATH);
    }
    read_file_path(event, secondary_path_ptr, ACTRAIL_FILE_SECONDARY_PATH);
    actrail_event_submit(ctx, event);
    return 0;
}

static __always_inline int emit_file_header_enter(
    struct trace_event_raw_sys_enter *ctx,
    __u64 descriptor,
    __u32 fd
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_file_event *event;
    __u32 kind = (__u32)(descriptor & ACTRAIL_FILE_DESCRIPTOR_KIND_MASK);
    __u32 syscall_id = (__u32)(
        (descriptor >> ACTRAIL_FILE_DESCRIPTOR_SYSCALL_SHIFT) &
        ACTRAIL_FILE_DESCRIPTOR_SYSCALL_MASK
    );
    __u32 arg_count = (__u32)(descriptor >> ACTRAIL_FILE_DESCRIPTOR_ARGC_SHIFT);

    if (!tgid) {
        return 0;
    }
    if (!trace_id) {
        return 0;
    }

    event = actrail_event_reserve(ACTRAIL_FILE_EVENT_HEADER_SIZE);
    if (!event) {
        return 0;
    }

    init_file_event_header(event, kind);
    event->pid = tgid;
    event->tid = (__u32)pid_tgid;
    event->pid_generation = current_process_start_time(tgid);
    event->phase = ACTRAIL_FILE_PHASE_ENTER;
    event->trace_id = *trace_id;
    event->aux = syscall_id;
    event->fd = fd;
    fill_file_args(event, ctx, arg_count);
    actrail_event_submit(ctx, event);
    return 0;
}

static __always_inline int emit_file_enter(
    struct trace_event_raw_sys_enter *ctx,
    __u64 descriptor,
    __u32 fd,
    __u64 path_ptr,
    __u64 secondary_path_ptr
) {
    if (secondary_path_ptr) {
        return emit_file_full_path_enter(
            ctx,
            descriptor,
            fd,
            path_ptr,
            secondary_path_ptr
        );
    }
    if (path_ptr) {
        return emit_file_primary_path_enter(ctx, descriptor, fd, path_ptr);
    }
    return emit_file_header_enter(ctx, descriptor, fd);
}

static __always_inline int emit_file_unlinkat_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_UNLINK,
            ACTRAIL_FILE_SYSCALL_UNLINKAT,
            ACTRAIL_FILE_SYSCALL_ARGC_UNLINKAT
        ),
        ACTRAIL_FILE_FD_MISSING,
        (__u64)ctx->args[1],
        0
    );
}

static __always_inline int emit_file_renameat_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_RENAME,
            ACTRAIL_FILE_SYSCALL_RENAMEAT,
            ACTRAIL_FILE_SYSCALL_ARGC_RENAMEAT
        ),
        ACTRAIL_FILE_FD_MISSING,
        (__u64)ctx->args[1],
        (__u64)ctx->args[3]
    );
}

static __always_inline int emit_file_mkdirat_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_MKDIR,
            ACTRAIL_FILE_SYSCALL_MKDIRAT,
            ACTRAIL_FILE_SYSCALL_ARGC_MKDIRAT
        ),
        ACTRAIL_FILE_FD_MISSING,
        (__u64)ctx->args[1],
        0
    );
}

static __always_inline int emit_file_mmap_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_header_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_MMAP,
            ACTRAIL_FILE_SYSCALL_MMAP,
            ACTRAIL_FILE_SYSCALL_ARGC_MMAP
        ),
        (__u32)ctx->args[4]
    );
}

static __always_inline int emit_file_close_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_header_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_CLOSE,
            ACTRAIL_FILE_SYSCALL_ARGC_CLOSE
        ),
        (__u32)ctx->args[0]
    );
}

static __always_inline int emit_file_dup_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_header_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_DUP,
            ACTRAIL_FILE_SYSCALL_ARGC_DUP
        ),
        (__u32)ctx->args[0]
    );
}

static __always_inline int emit_file_dup2_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_header_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_DUP2,
            ACTRAIL_FILE_SYSCALL_ARGC_DUP2
        ),
        (__u32)ctx->args[0]
    );
}

static __always_inline int emit_file_dup3_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_header_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_DUP3,
            ACTRAIL_FILE_SYSCALL_ARGC_DUP3
        ),
        (__u32)ctx->args[0]
    );
}

static __always_inline int emit_file_fcntl_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_header_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_FCNTL,
            ACTRAIL_FILE_SYSCALL_ARGC_FCNTL
        ),
        (__u32)ctx->args[0]
    );
}

static __always_inline int emit_file_chdir_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_CHDIR,
            ACTRAIL_FILE_SYSCALL_ARGC_CHDIR
        ),
        ACTRAIL_FILE_FD_MISSING,
        (__u64)ctx->args[0],
        0
    );
}

static __always_inline int emit_file_fchdir_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_header_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_FCHDIR,
            ACTRAIL_FILE_SYSCALL_ARGC_FCHDIR
        ),
        (__u32)ctx->args[0]
    );
}

static __always_inline int emit_file_exit(
    struct trace_event_raw_sys_exit *ctx,
    __u32 kind,
    __u32 syscall_id
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_file_event *event;
    __u64 generation;

    if (!tgid) {
        return 0;
    }
    if (!trace_id) {
        return 0;
    }

    event = actrail_event_reserve(ACTRAIL_FILE_EVENT_HEADER_SIZE);
    if (!event) {
        return 0;
    }

    init_file_event_header(event, kind);
    event->pid = tgid;
    event->tid = (__u32)pid_tgid;
    generation = current_process_start_time(tgid);
    event->pid_generation = generation;
    event->phase = ACTRAIL_FILE_PHASE_EXIT;
    event->result = ctx->ret;
    event->trace_id = *trace_id;
    event->aux = syscall_id;
    actrail_event_submit(ctx, event);
    if (ctx->ret >= 0
        && (syscall_id == ACTRAIL_FILE_SYSCALL_OPEN
            || syscall_id == ACTRAIL_FILE_SYSCALL_OPENAT
            || syscall_id == ACTRAIL_FILE_SYSCALL_OPENAT2
            || syscall_id == ACTRAIL_FILE_SYSCALL_CREAT)) {
        maybe_insert_file_bulk_read_fast_open_fd(
            tgid,
            (__u32)ctx->ret,
            generation,
            *trace_id
        );
    }
    return 0;
}

static __always_inline int store_pending_ipc_fd_pair_op(
    struct trace_event_raw_sys_enter *ctx,
    __u32 kind,
    __u32 fd_pair_arg,
    __u32 domain
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_pending_ipc_fd_pair_op op = {};

    if (!tgid || !trace_id || !ctx->args[fd_pair_arg]) {
        return 0;
    }

    op.trace_id = *trace_id;
    op.fd_pair_ptr = (__u64)ctx->args[fd_pair_arg];
    op.kind = kind;
    op.domain = domain;
    bpf_map_update_elem(&pending_ipc_fd_pair_ops, &pid_tgid, &op, BPF_ANY);
    return 0;
}

static __always_inline int emit_ipc_fd_pair_exit(
    struct trace_event_raw_sys_exit *ctx,
    __u32 syscall_id
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    struct actrail_pending_ipc_fd_pair_op *op =
        bpf_map_lookup_elem(&pending_ipc_fd_pair_ops, &pid_tgid);
    struct actrail_file_event *event;
    int fds[2] = {};

    if (!tgid || !op) {
        return 0;
    }
    if (ctx->ret != 0) {
        bpf_map_delete_elem(&pending_ipc_fd_pair_ops, &pid_tgid);
        return 0;
    }
    if (op->kind == ACTRAIL_FILE_IPC_FD_UNIX_SOCKET && op->domain != AF_UNIX) {
        bpf_map_delete_elem(&pending_ipc_fd_pair_ops, &pid_tgid);
        return 0;
    }
    if (bpf_probe_read_user(&fds, sizeof(fds), (void *)(unsigned long)op->fd_pair_ptr) != 0) {
        bpf_map_delete_elem(&pending_ipc_fd_pair_ops, &pid_tgid);
        return 0;
    }
    if (fds[0] < 0 || fds[1] < 0) {
        bpf_map_delete_elem(&pending_ipc_fd_pair_ops, &pid_tgid);
        return 0;
    }

    event = actrail_event_reserve(ACTRAIL_FILE_EVENT_HEADER_SIZE);
    if (!event) {
        bpf_map_delete_elem(&pending_ipc_fd_pair_ops, &pid_tgid);
        return 0;
    }

    init_file_event_header(event, ACTRAIL_FILE_CONTEXT);
    event->pid = tgid;
    event->tid = (__u32)pid_tgid;
    event->pid_generation = current_process_start_time(tgid);
    event->phase = ACTRAIL_FILE_PHASE_EXIT;
    event->result = ctx->ret;
    event->trace_id = op->trace_id;
    event->aux = syscall_id;
    event->fd = (__u32)fds[0];
    event->arg0 = (__u32)fds[1];
    event->arg1 = op->kind;
    actrail_event_submit(ctx, event);
    bpf_map_delete_elem(&pending_ipc_fd_pair_ops, &pid_tgid);
    return 0;
}

#endif
