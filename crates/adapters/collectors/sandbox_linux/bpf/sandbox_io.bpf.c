#include "sandbox_bpf_helpers.h"

enum sandbox_io_kind {
    SANDBOX_IO_READ = 1,
    SANDBOX_IO_WRITE = 2,
};

enum sandbox_diagnostic_kind {
    SANDBOX_DIAG_PENDING_IO_DROP = 0,
    SANDBOX_DIAG_AGGREGATE_DROP = 1,
    SANDBOX_DIAG_DESCENDANT_TRACKING_DROP = 2,
    SANDBOX_DIAG_COUNT = 3,
};

struct sandbox_root_marker {
    __u32 pid;
    __u32 reserved;
    __u64 start_time_ticks;
    __u8 executable_name[16];
};

struct sandbox_pending_io {
    struct sandbox_root_marker root;
    __u8 kind;
    __u8 reserved[7];
};

struct sandbox_io_counters {
    __u64 read_operations;
    __u64 read_bytes;
    __u64 write_operations;
    __u64 write_bytes;
    __u64 failed_read_operations;
    __u64 failed_write_operations;
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 16384);
    __type(key, __u32);
    __type(value, struct sandbox_root_marker);
} tracked_processes SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 32768);
    __type(key, __u64);
    __type(value, struct sandbox_pending_io);
} pending_io SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 4096);
    __type(key, struct sandbox_root_marker);
    __type(value, struct sandbox_io_counters);
} io_aggregates SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, SANDBOX_DIAG_COUNT);
    __type(key, __u32);
    __type(value, __u64);
} collection_diagnostics SEC(".maps");

static __attribute__((always_inline)) void diagnostic_increment(__u32 kind) {
    __u64 *counter = bpf_map_lookup_elem(&collection_diagnostics, &kind);

    if (counter) {
        __sync_fetch_and_add(counter, 1);
    }
}

static __attribute__((always_inline)) int begin_io(__u8 kind) {
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 tgid = pid_tgid >> 32;
    struct sandbox_root_marker *root = bpf_map_lookup_elem(&tracked_processes, &tgid);
    struct sandbox_pending_io pending = {};

    if (!root) {
        return 0;
    }
    pending.root = *root;
    pending.kind = kind;
    if (bpf_map_update_elem(&pending_io, &pid_tgid, &pending, BPF_ANY) != 0) {
        diagnostic_increment(SANDBOX_DIAG_PENDING_IO_DROP);
    }
    return 0;
}

static __attribute__((always_inline)) int finish_io(struct syscall_exit_ctx *ctx) {
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    struct sandbox_pending_io *pending = bpf_map_lookup_elem(&pending_io, &pid_tgid);
    struct sandbox_pending_io snapshot = {};
    struct sandbox_io_counters zero = {};
    struct sandbox_io_counters *counters;

    if (!pending) {
        return 0;
    }
    snapshot = *pending;
    bpf_map_delete_elem(&pending_io, &pid_tgid);
    counters = bpf_map_lookup_elem(&io_aggregates, &snapshot.root);
    if (!counters) {
        bpf_map_update_elem(&io_aggregates, &snapshot.root, &zero, BPF_NOEXIST);
        counters = bpf_map_lookup_elem(&io_aggregates, &snapshot.root);
        if (!counters) {
            diagnostic_increment(SANDBOX_DIAG_AGGREGATE_DROP);
            return 0;
        }
    }
    if (snapshot.kind == SANDBOX_IO_READ) {
        if (ctx->result < 0) {
            __sync_fetch_and_add(&counters->failed_read_operations, 1);
        } else {
            __sync_fetch_and_add(&counters->read_operations, 1);
            __sync_fetch_and_add(&counters->read_bytes, (__u64)ctx->result);
        }
    } else if (snapshot.kind == SANDBOX_IO_WRITE) {
        if (ctx->result < 0) {
            __sync_fetch_and_add(&counters->failed_write_operations, 1);
        } else {
            __sync_fetch_and_add(&counters->write_operations, 1);
            __sync_fetch_and_add(&counters->write_bytes, (__u64)ctx->result);
        }
    }
    return 0;
}

SEC("tracepoint/syscalls/sys_enter_read")
int sandbox_enter_read(struct syscall_enter_ctx *ctx) {
    (void)ctx;
    return begin_io(SANDBOX_IO_READ);
}

SEC("tracepoint/syscalls/sys_exit_read")
int sandbox_exit_read(struct syscall_exit_ctx *ctx) {
    return finish_io(ctx);
}

SEC("tracepoint/syscalls/sys_enter_write")
int sandbox_enter_write(struct syscall_enter_ctx *ctx) {
    (void)ctx;
    return begin_io(SANDBOX_IO_WRITE);
}

SEC("tracepoint/syscalls/sys_exit_write")
int sandbox_exit_write(struct syscall_exit_ctx *ctx) {
    return finish_io(ctx);
}

SEC("tracepoint/sched/sched_process_fork")
int sandbox_process_fork(struct sched_process_fork_ctx *ctx) {
    __u32 parent_tgid = bpf_get_current_pid_tgid() >> 32;
    __u32 child_pid = (__u32)ctx->child_pid;
    struct sandbox_root_marker *root;

    if (!parent_tgid || !child_pid || parent_tgid == child_pid) {
        return 0;
    }
    root = bpf_map_lookup_elem(&tracked_processes, &parent_tgid);
    if (root) {
        if (bpf_map_update_elem(&tracked_processes, &child_pid, root, BPF_ANY) != 0) {
            diagnostic_increment(SANDBOX_DIAG_DESCENDANT_TRACKING_DROP);
        }
    }
    return 0;
}

SEC("tracepoint/sched/sched_process_exit")
int sandbox_process_exit(struct sched_process_exit_ctx *ctx) {
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 tid = (__u32)pid_tgid;

    (void)ctx;
    bpf_map_delete_elem(&pending_io, &pid_tgid);
    if (tid) {
        /* Fork tracepoints expose a child PID, including thread IDs.  Removing
         * the exiting TID bounds those inherited thread entries without
         * disturbing the process-wide TGID entry used by live siblings. */
        bpf_map_delete_elem(&tracked_processes, &tid);
    }
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
