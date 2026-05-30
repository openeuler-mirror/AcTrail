#ifndef ACTRAIL_FILE_H
#define ACTRAIL_FILE_H

#include "actrail_runtime.h"

enum actrail_file_path_abi {
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
};

enum actrail_file_syscall_arg_count {
    ACTRAIL_FILE_SYSCALL_ARGC_CHDIR = 1,
    ACTRAIL_FILE_SYSCALL_ARGC_CLOSE = 1,
    ACTRAIL_FILE_SYSCALL_ARGC_DUP = 1,
    ACTRAIL_FILE_SYSCALL_ARGC_DUP2 = 2,
    ACTRAIL_FILE_SYSCALL_ARGC_DUP3 = 3,
    ACTRAIL_FILE_SYSCALL_ARGC_FCHDIR = 1,
    ACTRAIL_FILE_SYSCALL_ARGC_FCNTL = 3,
    ACTRAIL_FILE_SYSCALL_ARGC_MKDIRAT = 3,
    ACTRAIL_FILE_SYSCALL_ARGC_MMAP = 6,
    ACTRAIL_FILE_SYSCALL_ARGC_OPENAT = 4,
    ACTRAIL_FILE_SYSCALL_ARGC_RENAMEAT = 4,
    ACTRAIL_FILE_SYSCALL_ARGC_UNLINKAT = 3,
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

static __always_inline __u32 configured_file_path_max_bytes(void) {
    __u32 key = 0;
    struct actrail_file_config *config = bpf_map_lookup_elem(&file_config, &key);
    __u32 max_bytes;

    if (!config) {
        return 0;
    }
    max_bytes = config->path_max_bytes;
    if (max_bytes > ACTRAIL_FILE_PATH_COPY_MAX_BYTES) {
        return ACTRAIL_FILE_PATH_COPY_MAX_BYTES;
    }
    return max_bytes;
}

static __always_inline void read_file_path(
    struct actrail_file_event *event,
    __u64 path_ptr,
    __u32 role
) {
    __u32 max_bytes = configured_file_path_max_bytes();
    __u32 helper_size;
    long copied;
    char *target;
    __u32 *size;
    __u32 *flags;

    if (!path_ptr || max_bytes == 0) {
        return;
    }

    if (role == ACTRAIL_FILE_SECONDARY_PATH) {
        target = event->secondary_path;
        size = &event->secondary_path_size;
        flags = &event->secondary_path_flags;
    } else {
        target = event->path;
        size = &event->path_size;
        flags = &event->path_flags;
    }

    event->path_max_bytes = max_bytes;
    helper_size = max_bytes + 1;
    copied = bpf_probe_read_user_str(
        target,
        helper_size,
        (void *)(unsigned long)path_ptr
    );
    if (copied <= 0) {
        *flags |= ACTRAIL_FILE_PATH_FAULT;
        return;
    }

    *flags |= ACTRAIL_FILE_PATH_CAPTURED;
    *size = (__u32)copied - 1;
    if (*size >= max_bytes) {
        *flags |= ACTRAIL_FILE_PATH_TRUNCATED;
    }
}

static __always_inline void fill_file_args(
    struct actrail_file_event *event,
    struct trace_event_raw_sys_enter *ctx,
    __u32 arg_count
) {
    if (arg_count > 0) {
        event->arg0 = ctx->args[0];
    }
    if (arg_count > 1) {
        event->arg1 = ctx->args[1];
    }
    if (arg_count > 2) {
        event->arg2 = ctx->args[2];
    }
    if (arg_count > 3) {
        event->arg3 = ctx->args[3];
    }
    if (arg_count > 4) {
        event->arg4 = ctx->args[4];
    }
    if (arg_count > 5) {
        event->arg5 = ctx->args[5];
    }
}

static __always_inline void init_file_event(
    struct actrail_file_event *event,
    __u32 kind
) {
    __builtin_memset(event, 0, sizeof(*event));
    event->kind = kind;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->fd = ACTRAIL_FILE_FD_MISSING;
}

static __always_inline __u64 file_enter_descriptor(
    __u32 kind,
    __u32 syscall_id,
    __u32 arg_count
) {
    return (__u64)kind |
        ((__u64)syscall_id << ACTRAIL_FILE_DESCRIPTOR_SYSCALL_SHIFT) |
        ((__u64)arg_count << ACTRAIL_FILE_DESCRIPTOR_ARGC_SHIFT);
}

static __always_inline int emit_file_enter(
    struct trace_event_raw_sys_enter *ctx,
    __u64 descriptor,
    __u32 fd,
    __u64 path_ptr,
    __u64 secondary_path_ptr
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = current_namespace_tgid();
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

    event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event) {
        return 0;
    }

    init_file_event(event, kind);
    event->pid = tgid;
    event->tid = (__u32)pid_tgid;
    event->pid_generation = ensure_process_generation(tgid);
    event->phase = ACTRAIL_FILE_PHASE_ENTER;
    event->trace_id = *trace_id;
    event->aux = syscall_id;
    event->fd = fd;
    fill_file_args(event, ctx, arg_count);
    if (path_ptr) {
        read_file_path(event, path_ptr, ACTRAIL_FILE_PRIMARY_PATH);
    }
    if (secondary_path_ptr) {
        read_file_path(event, secondary_path_ptr, ACTRAIL_FILE_SECONDARY_PATH);
    }
    bpf_ringbuf_submit(event, 0);
    return 0;
}

static __always_inline int emit_file_openat_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_OPEN,
            ACTRAIL_FILE_SYSCALL_OPENAT,
            ACTRAIL_FILE_SYSCALL_ARGC_OPENAT
        ),
        ACTRAIL_FILE_FD_MISSING,
        (__u64)ctx->args[1],
        0
    );
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
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_MMAP,
            ACTRAIL_FILE_SYSCALL_MMAP,
            ACTRAIL_FILE_SYSCALL_ARGC_MMAP
        ),
        (__u32)ctx->args[4],
        0,
        0
    );
}

static __always_inline int emit_file_close_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_CLOSE,
            ACTRAIL_FILE_SYSCALL_ARGC_CLOSE
        ),
        (__u32)ctx->args[0],
        0,
        0
    );
}

static __always_inline int emit_file_dup_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_DUP,
            ACTRAIL_FILE_SYSCALL_ARGC_DUP
        ),
        (__u32)ctx->args[0],
        0,
        0
    );
}

static __always_inline int emit_file_dup2_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_DUP2,
            ACTRAIL_FILE_SYSCALL_ARGC_DUP2
        ),
        (__u32)ctx->args[0],
        0,
        0
    );
}

static __always_inline int emit_file_dup3_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_DUP3,
            ACTRAIL_FILE_SYSCALL_ARGC_DUP3
        ),
        (__u32)ctx->args[0],
        0,
        0
    );
}

static __always_inline int emit_file_fcntl_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_FCNTL,
            ACTRAIL_FILE_SYSCALL_ARGC_FCNTL
        ),
        (__u32)ctx->args[0],
        0,
        0
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
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_CONTEXT,
            ACTRAIL_FILE_SYSCALL_FCHDIR,
            ACTRAIL_FILE_SYSCALL_ARGC_FCHDIR
        ),
        (__u32)ctx->args[0],
        0,
        0
    );
}

static __always_inline int emit_file_exit(
    struct trace_event_raw_sys_exit *ctx,
    __u32 kind,
    __u32 syscall_id
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = current_namespace_tgid();
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_file_event *event;

    if (!tgid) {
        return 0;
    }
    if (!trace_id) {
        return 0;
    }

    event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event) {
        return 0;
    }

    init_file_event(event, kind);
    event->pid = tgid;
    event->tid = (__u32)pid_tgid;
    event->pid_generation = ensure_process_generation(tgid);
    event->phase = ACTRAIL_FILE_PHASE_EXIT;
    event->result = ctx->ret;
    event->trace_id = *trace_id;
    event->aux = syscall_id;
    bpf_ringbuf_submit(event, 0);
    return 0;
}

#endif
