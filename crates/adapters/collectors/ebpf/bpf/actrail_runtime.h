#ifndef ACTRAIL_RUNTIME_H
#define ACTRAIL_RUNTIME_H

#include "actrail_helpers.h"
#include "include/actrail_const.h"

enum actrail_proc_event_kind {
    ACTRAIL_PROC_FORK = 1,
    ACTRAIL_PROC_EXEC = 2,
    ACTRAIL_PROC_EXIT = 3,
    ACTRAIL_PROC_SIGNAL = 4,
    ACTRAIL_NET_CONNECT = 100,
    ACTRAIL_NET_ACCEPT = 101,
    ACTRAIL_NET_SEND = 102,
    ACTRAIL_NET_RECV = 103,
    ACTRAIL_NET_BIND = 104,
    ACTRAIL_NET_LISTEN = 105,
    ACTRAIL_TLS_PAYLOAD_COMPLETION = 201,
    ACTRAIL_TLS_PAYLOAD_CAPTURE_REQUEST = 202,
    ACTRAIL_TLS_PAYLOAD_DIRECT_CAPTURE = 203,
    ACTRAIL_TLS_PAYLOAD_DIAGNOSTIC = 204,
    ACTRAIL_FILE_OPEN = 300,
    ACTRAIL_FILE_UNLINK = 301,
    ACTRAIL_FILE_RENAME = 302,
    ACTRAIL_FILE_MKDIR = 303,
    ACTRAIL_FILE_RMDIR = 304,
    ACTRAIL_FILE_TRUNCATE = 305,
    ACTRAIL_FILE_MMAP = 306,
    ACTRAIL_FILE_CONTEXT = 307,
    ACTRAIL_STDIO_PAYLOAD = 400,
    ACTRAIL_SOCKET_PAYLOAD = 500,
    ACTRAIL_SOCKET_PAYLOAD_COMPLETION = 501,
};

enum actrail_pid_namespace_slot {
    ACTRAIL_ACTIVE_PID_NAMESPACE = 0,
};

enum actrail_net_syscall_family {
    ACTRAIL_NET_SYSCALL_SOCKET = 1,
    ACTRAIL_NET_SYSCALL_FD_IO = 2,
};

enum actrail_syscall_arg_slot {
    ACTRAIL_SYSCALL_ARG_MISSING = 6,
};

enum actrail_trace_lookup_flag {
    ACTRAIL_TRACE_LOOKUP_FLAG_HOST_FALLBACK = 1,
};

struct actrail_endpoint {
    __u16 family;
    __u16 port_be;
    __u32 addr4_be;
    __u8 addr6[16];
};

struct actrail_event {
    __u32 kind;
    __u32 pid;
    __u32 aux;
    __s32 result;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u32 fd;
    __u32 reserved;
    __u64 requested_size;
    __u64 pid_generation;
    __u64 aux_generation;
    struct actrail_endpoint local;
    struct actrail_endpoint remote;
} __attribute__((packed));

struct actrail_exec_event {
    struct actrail_event event;
    __u32 filename_size;
    __u32 filename_flags;
    char filename[ACTRAIL_EXEC_FILENAME_ABI_MAX_BYTES];
} __attribute__((packed));

struct actrail_pending_net_op {
    __u64 trace_id;
    __u32 kind;
    __u32 fd;
    __u32 syscall_family;
    __u64 requested_size;
    __u64 sockaddr_ptr;
};

struct actrail_pending_proc_op {
    __u64 trace_id;
    __u64 parent_generation;
    __u64 child_generation;
    __u32 parent_pid;
};

struct actrail_pending_exit_op {
    __s32 code;
};

struct actrail_suppressed_fd_key {
    __u32 pid;
    __u32 fd;
    __u64 generation;
};

struct actrail_suppressed_fd_value {
    __u64 trace_id;
    __u32 purpose;
};

struct actrail_pending_suppressed_fd_dup_op {
    __u32 source_fd;
    __u32 target_fd;
    __u32 mode;
    __u32 source_suppressed;
    __u32 target_suppressed;
    struct actrail_suppressed_fd_value source_value;
};

struct actrail_pid_namespace {
    __u64 dev;
    __u64 ino;
};

struct tracepoint_common {
    __u16 common_type;
    __u8 common_flags;
    __u8 common_preempt_count;
    __s32 common_pid;
};

struct sched_process_fork_ctx {
    struct tracepoint_common common;
    char parent_comm[16];
    __s32 parent_pid;
    char child_comm[16];
    __s32 child_pid;
};

struct sched_process_exec_ctx {
    struct tracepoint_common common;
    __u32 filename_loc;
    __s32 pid;
    __s32 old_pid;
};

struct sched_process_exit_ctx {
    struct tracepoint_common common;
    char comm[16];
    __s32 pid;
    __s32 prio;
};

struct signal_generate_ctx {
    struct tracepoint_common common;
    __s32 sig;
    __s32 error;
    __s32 code;
    char comm[16];
    __s32 pid;
    __s32 group;
    __s32 signal_result;
};

struct trace_event_raw_sys_enter {
    struct tracepoint_common common;
    long id;
    unsigned long args[6];
};

struct trace_event_raw_sys_exit {
    struct tracepoint_common common;
    long id;
    long ret;
};

struct actrail_sockaddr_storage {
    __u16 family;
    __u8 data[126];
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} tracked_traces SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} process_generations SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_pid_namespace);
} pid_namespace SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_RINGBUF);
    __uint(max_entries, 1);
} events SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_net_op);
} pending_net_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_pending_proc_op);
} pending_child_proc_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_exit_op);
} pending_exit_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_suppressed_fd_key);
    __type(value, struct actrail_suppressed_fd_value);
} suppressed_fds SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_suppressed_fd_dup_op);
} pending_suppressed_fd_dup_ops SEC(".maps");

static __always_inline int emit_event(struct actrail_event *event) {
    return bpf_ringbuf_output(&events, event, sizeof(*event), 0);
}

static __always_inline __u64 current_pid_tgid(void) {
    return bpf_get_current_pid_tgid();
}

static __always_inline __u32 current_tgid(void) {
    return current_pid_tgid() >> 32;
}

static __always_inline __u64 current_namespace_pid_tgid(void) {
    __u32 key = ACTRAIL_ACTIVE_PID_NAMESPACE;
    struct actrail_pid_namespace *namespace = bpf_map_lookup_elem(&pid_namespace, &key);
    struct bpf_pidns_info namespace_pid = {};

    if (!namespace) {
        return 0;
    }
    if (bpf_get_ns_current_pid_tgid(
            namespace->dev,
            namespace->ino,
            &namespace_pid,
            sizeof(namespace_pid)) != 0) {
        return 0;
    }
    return ((__u64)namespace_pid.tgid << 32) | namespace_pid.pid;
}

static __always_inline __u32 current_namespace_tgid(void) {
    return current_namespace_pid_tgid() >> 32;
}

static __always_inline __u64 *lookup_current_trace(
    __u32 *tgid,
    __u32 *tid,
    __u32 *flags
) {
    __u64 host_pid_tgid = current_pid_tgid();
    __u64 namespace_pid_tgid = current_namespace_pid_tgid();
    __u32 lookup_tgid = namespace_pid_tgid >> 32;
    __u64 *trace_id = 0;

    *flags = 0;
    if (namespace_pid_tgid) {
        trace_id = bpf_map_lookup_elem(&tracked_traces, &lookup_tgid);
        if (trace_id) {
            *tgid = lookup_tgid;
            *tid = (__u32)namespace_pid_tgid;
            return trace_id;
        }
    }

    lookup_tgid = host_pid_tgid >> 32;
    trace_id = bpf_map_lookup_elem(&tracked_traces, &lookup_tgid);
    if (trace_id) {
        *tgid = lookup_tgid;
        *tid = (__u32)host_pid_tgid;
        *flags = ACTRAIL_TRACE_LOOKUP_FLAG_HOST_FALLBACK;
        return trace_id;
    }

    if (namespace_pid_tgid) {
        *tgid = namespace_pid_tgid >> 32;
        *tid = (__u32)namespace_pid_tgid;
    } else {
        *tgid = host_pid_tgid >> 32;
        *tid = (__u32)host_pid_tgid;
    }
    return 0;
}

static __always_inline __u64 ensure_process_generation(__u32 pid) {
    __u64 *generation;
    __u64 generated;

    if (!pid) {
        return 0;
    }
    generation = bpf_map_lookup_elem(&process_generations, &pid);
    if (generation) {
        return *generation;
    }
    generated = bpf_ktime_get_ns();
    bpf_map_update_elem(&process_generations, &pid, &generated, BPF_ANY);
    return generated;
}

static __always_inline void set_process_generation(__u32 pid, __u64 generation) {
    if (!pid || !generation) {
        return;
    }
    bpf_map_update_elem(&process_generations, &pid, &generation, BPF_ANY);
}

static __always_inline void delete_process_generation(__u32 pid) {
    if (!pid) {
        return;
    }
    bpf_map_delete_elem(&process_generations, &pid);
}

static __always_inline __u64 *lookup_process_generation(__u32 pid) {
    if (!pid) {
        return 0;
    }
    return bpf_map_lookup_elem(&process_generations, &pid);
}

static __always_inline struct actrail_suppressed_fd_value *lookup_suppressed_fd(
    __u32 pid,
    __u32 fd
) {
    __u64 *generation = lookup_process_generation(pid);
    struct actrail_suppressed_fd_key key = {};

    if (!generation) {
        return 0;
    }
    key.pid = pid;
    key.fd = fd;
    key.generation = *generation;
    return bpf_map_lookup_elem(&suppressed_fds, &key);
}

static __always_inline int is_suppressed_fd(__u32 pid, __u32 fd) {
    return lookup_suppressed_fd(pid, fd) != 0;
}

static __always_inline void delete_suppressed_fd(__u32 pid, __u32 fd) {
    __u64 *generation = lookup_process_generation(pid);
    struct actrail_suppressed_fd_key key = {};

    if (!generation) {
        return;
    }
    key.pid = pid;
    key.fd = fd;
    key.generation = *generation;
    bpf_map_delete_elem(&suppressed_fds, &key);
}

static __always_inline int suppressed_fd_close_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    __u32 pid = current_namespace_tgid();
    __u32 fd = (__u32)ctx->args[0];
    int suppressed;

    if (!pid) {
        return 0;
    }
    suppressed = is_suppressed_fd(pid, fd);
    if (suppressed) {
        delete_suppressed_fd(pid, fd);
    }
    return suppressed;
}

#define ACTRAIL_SUPPRESSED_FD_DUP_RET_FD 1
#define ACTRAIL_SUPPRESSED_FD_DUP_TARGET_FD 2

static __always_inline int suppressed_fd_dup_enter(
    __u32 source_fd,
    __u32 target_fd,
    __u32 has_target_fd,
    __u32 mode
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 pid = current_namespace_tgid();
    struct actrail_suppressed_fd_value *source;
    struct actrail_suppressed_fd_value *target;
    struct actrail_pending_suppressed_fd_dup_op op = {};

    if (!pid) {
        return 0;
    }
    op.source_fd = source_fd;
    op.target_fd = has_target_fd ? target_fd : 0;
    op.mode = mode;
    source = lookup_suppressed_fd(pid, op.source_fd);
    target = has_target_fd ? lookup_suppressed_fd(pid, op.target_fd) : 0;
    if (source) {
        op.source_suppressed = 1;
        op.source_value = *source;
    }
    if (target) {
        op.target_suppressed = 1;
    }
    if (!op.source_suppressed && !op.target_suppressed) {
        return 0;
    }
    bpf_map_update_elem(&pending_suppressed_fd_dup_ops, &pid_tgid, &op, BPF_ANY);
    return 1;
}

static __always_inline int suppressed_fd_fcntl_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    __u32 command = (__u32)ctx->args[1];

    if (command != F_DUPFD && command != F_DUPFD_CLOEXEC) {
        return 0;
    }
    return suppressed_fd_dup_enter(
        (__u32)ctx->args[0],
        0,
        0,
        ACTRAIL_SUPPRESSED_FD_DUP_RET_FD
    );
}

static __always_inline void suppressed_fd_dup_exit(
    struct trace_event_raw_sys_exit *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 pid = current_namespace_tgid();
    __u64 *generation = lookup_process_generation(pid);
    struct actrail_pending_suppressed_fd_dup_op *op =
        bpf_map_lookup_elem(&pending_suppressed_fd_dup_ops, &pid_tgid);
    struct actrail_suppressed_fd_key key = {};
    __u32 new_fd;

    if (!pid || !generation || !op) {
        return;
    }
    if (ctx->ret < 0) {
        bpf_map_delete_elem(&pending_suppressed_fd_dup_ops, &pid_tgid);
        return;
    }
    new_fd = op->mode == ACTRAIL_SUPPRESSED_FD_DUP_RET_FD ? (__u32)ctx->ret : op->target_fd;
    key.pid = pid;
    key.fd = new_fd;
    key.generation = *generation;
    if (op->source_suppressed) {
        bpf_map_update_elem(&suppressed_fds, &key, &op->source_value, BPF_ANY);
    } else if (op->target_suppressed) {
        bpf_map_delete_elem(&suppressed_fds, &key);
    }
    bpf_map_delete_elem(&pending_suppressed_fd_dup_ops, &pid_tgid);
}

static __always_inline void init_event(
    struct actrail_event *event,
    __u32 kind,
    __u32 pid,
    __u64 trace_id
) {
    __builtin_memset(event, 0, sizeof(*event));
    event->kind = kind;
    event->pid = pid;
    event->trace_id = trace_id;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->pid_generation = ensure_process_generation(pid);
}

static __always_inline void read_endpoint(__u64 user_ptr, struct actrail_endpoint *endpoint) {
    struct actrail_sockaddr_storage storage = {};
    struct sockaddr_in *addr4;
    struct sockaddr_in6 *addr6;

    if (!user_ptr) {
        return;
    }
    if (bpf_probe_read_user(&storage, sizeof(storage), (void *)(unsigned long)user_ptr) != 0) {
        return;
    }

    endpoint->family = storage.family;
    if (storage.family == AF_INET) {
        addr4 = (struct sockaddr_in *)&storage;
        endpoint->port_be = addr4->sin_port;
        endpoint->addr4_be = addr4->sin_addr.s_addr;
    } else if (storage.family == AF_INET6) {
        addr6 = (struct sockaddr_in6 *)&storage;
        endpoint->port_be = addr6->sin6_port;
        __builtin_memcpy(endpoint->addr6, &addr6->sin6_addr, sizeof(endpoint->addr6));
    }
}

#endif
