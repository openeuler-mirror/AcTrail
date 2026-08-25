#include "sandbox_bpf_helpers.h"

enum sandbox_io_kind {
    SANDBOX_IO_READ = 1,
    SANDBOX_IO_WRITE = 2,
};

enum sandbox_diagnostic_kind {
    SANDBOX_DIAG_PENDING_IO_DROP = 0,
    SANDBOX_DIAG_AGGREGATE_DROP = 1,
    SANDBOX_DIAG_DESCENDANT_TRACKING_DROP = 2,
    SANDBOX_DIAG_OOM_EVENT_DROP = 3,
    SANDBOX_DIAG_OOM_COMM_DROP = 4,
    SANDBOX_DIAG_COUNT = 5,
};

enum sandbox_oom_attribution {
    SANDBOX_OOM_UNKNOWN = 0,
    SANDBOX_OOM_MONITORED = 1,
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

struct sandbox_oom_event {
    __u64 event_boot_ns;
    __u64 publication_generation;
    __u32 victim_pid;
    __u8 victim_comm[16];
    __u8 attribution;
    __u8 reserved[3];
    struct sandbox_root_marker monitored_root;
};

struct sandbox_process_comm {
    __u8 value[16];
};

struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
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

struct {
    __uint(type, BPF_MAP_TYPE_QUEUE);
    __uint(max_entries, 256);
    __type(value, struct sandbox_oom_event);
} oom_events SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 16384);
    __type(key, __u32);
    __type(value, struct sandbox_process_comm);
} process_comms SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} publication_state SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u32);
} fork_pid_offset SEC(".maps");

static __attribute__((always_inline)) void diagnostic_increment(__u32 kind) {
    __u64 *counter = bpf_map_lookup_elem(&collection_diagnostics, &kind);

    if (counter) {
        __sync_fetch_and_add(counter, 1);
    }
}

static __attribute__((always_inline)) void cache_current_comm(__u32 pid) {
    struct sandbox_process_comm comm = {};

    if (!pid || bpf_get_current_comm(comm.value, sizeof(comm.value)) != 0 ||
        bpf_map_update_elem(&process_comms, &pid, &comm, BPF_ANY) != 0) {
        diagnostic_increment(SANDBOX_DIAG_OOM_COMM_DROP);
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
int sandbox_process_fork(void *ctx) {
    __u32 offset_key = 0;
    __u32 *child_pid_offset = bpf_map_lookup_elem(&fork_pid_offset, &offset_key);
    __u32 parent_tgid = bpf_get_current_pid_tgid() >> 32;
    __u32 child_pid = 0;
    struct sandbox_root_marker *root;

    if (!child_pid_offset || !*child_pid_offset) {
        return 0;
    }
    if (bpf_probe_read(
            &child_pid,
            sizeof(child_pid),
            (void *)((__u64)ctx + *child_pid_offset)
        ) != 0) {
        return 0;
    }
    if (!parent_tgid || !child_pid || parent_tgid == child_pid) {
        return 0;
    }
    cache_current_comm(child_pid);
    root = bpf_map_lookup_elem(&tracked_processes, &parent_tgid);
    if (root) {
        if (bpf_map_update_elem(&tracked_processes, &child_pid, root, BPF_ANY) != 0) {
            diagnostic_increment(SANDBOX_DIAG_DESCENDANT_TRACKING_DROP);
        }
    }
    return 0;
}

SEC("tracepoint/sched/sched_process_exec")
int sandbox_process_exec(void *ctx) {
    __u32 tid = (__u32)bpf_get_current_pid_tgid();

    (void)ctx;
    cache_current_comm(tid);
    return 0;
}

SEC("tracepoint/sched/sched_process_exit")
int sandbox_process_exit(struct sched_process_exit_ctx *ctx) {
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 tid = (__u32)pid_tgid;

    (void)ctx;
    bpf_map_delete_elem(&pending_io, &pid_tgid);
    if (tid) {
        bpf_map_delete_elem(&process_comms, &tid);
        /* Fork tracepoints expose a child PID, including thread IDs.  Removing
         * the exiting TID bounds those inherited thread entries without
         * disturbing the process-wide TGID entry used by live siblings. */
        bpf_map_delete_elem(&tracked_processes, &tid);
    }
    return 0;
}

SEC("tracepoint/oom/mark_victim")
int sandbox_oom_mark_victim(struct oom_mark_victim_ctx *ctx) {
    struct sandbox_oom_event event = {};
    struct sandbox_process_comm *comm;
    struct sandbox_root_marker *root;
    __u64 current_pid_tgid = bpf_get_current_pid_tgid();
    __u32 victim_pid = (__u32)ctx->pid;
    __u32 state_key = 0;
    __u64 *generation = bpf_map_lookup_elem(&publication_state, &state_key);
    __u64 generation_snapshot;

    if (!generation || !victim_pid) {
        return 0;
    }
    generation_snapshot = *generation;
    if (!generation_snapshot) {
        return 0;
    }
    event.event_boot_ns = bpf_ktime_get_ns();
    event.publication_generation = generation_snapshot;
    event.victim_pid = victim_pid;
    if (victim_pid == (__u32)current_pid_tgid ||
        victim_pid == (__u32)(current_pid_tgid >> 32)) {
        if (bpf_get_current_comm(event.victim_comm, sizeof(event.victim_comm)) != 0) {
            diagnostic_increment(SANDBOX_DIAG_OOM_COMM_DROP);
            return 0;
        }
    } else {
        comm = bpf_map_lookup_elem(&process_comms, &victim_pid);
        if (!comm) {
            diagnostic_increment(SANDBOX_DIAG_OOM_COMM_DROP);
            return 0;
        }
        __builtin_memcpy(event.victim_comm, comm->value, sizeof(event.victim_comm));
    }
    root = bpf_map_lookup_elem(&tracked_processes, &victim_pid);
    if (root) {
        event.attribution = SANDBOX_OOM_MONITORED;
        event.monitored_root = *root;
    } else {
        event.attribution = SANDBOX_OOM_UNKNOWN;
    }
    if (bpf_map_push_elem(&oom_events, &event, BPF_ANY) != 0) {
        diagnostic_increment(SANDBOX_DIAG_OOM_EVENT_DROP);
    }
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
