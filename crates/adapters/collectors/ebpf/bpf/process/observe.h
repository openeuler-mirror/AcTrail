#ifndef ACTRAIL_PROCESS_OBSERVE_H
#define ACTRAIL_PROCESS_OBSERVE_H

#include "../runtime/event_transport.h"
#include "../runtime/fetch_add_compat.h"
#include "../fd/suppressed.h"
#include "../launch_binding/binding.h"
#include "state.h"

enum actrail_proc_coord_syscall_id {
    ACTRAIL_PROC_COORD_TRACEPOINT_SIGNAL_GENERATE = 1,
};

enum actrail_process_exec_abi {
    ACTRAIL_PROCESS_EXEC_PATH_ABI_MAX_BYTES = 4096,
    ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES = 4096,
    ACTRAIL_PROCESS_EXEC_ARGV_COPY_MAX_BYTES = 4095,
    ACTRAIL_PROCESS_EXEC_ARG_MAX = 128,
};

enum actrail_process_exec_syscall {
    ACTRAIL_PROCESS_EXECVE = 1,
    ACTRAIL_PROCESS_EXECVEAT = 2,
};

enum actrail_process_fork_syscall {
    ACTRAIL_PROCESS_FORK = 3,
    ACTRAIL_PROCESS_VFORK = 4,
    ACTRAIL_PROCESS_CLONE = 5,
    ACTRAIL_PROCESS_CLONE3 = 6,
};

enum actrail_process_fork_capture_flag {
    ACTRAIL_PROCESS_FORK_CLONE3_TRUNCATED = 1,
    ACTRAIL_PROCESS_FORK_CLONE3_READ_FAILED = 2,
};

enum actrail_process_clone_flag {
    ACTRAIL_PROCESS_CLONE_THREAD = 0x00010000,
};

struct actrail_user_clone_args_prefix {
    __u64 flags;
};

enum actrail_process_exec_capture_flag {
    ACTRAIL_PROCESS_EXEC_PATH_TRUNCATED = 1,
    ACTRAIL_PROCESS_EXEC_ARGV_TRUNCATED = 2,
    ACTRAIL_PROCESS_EXEC_READ_FAILED = 4,
    ACTRAIL_PROCESS_EXEC_ARG_LIMIT = 8,
    ACTRAIL_PROCESS_EXEC_ARGV_COMPLETE = 16,
};

struct actrail_process_exec_config {
    __u32 max_args;
    __u32 max_arg_bytes;
    __u32 max_total_arg_bytes;
};

struct actrail_pending_process_exec {
    __u64 trace_id;
    __u64 attempt_id;
    __u64 observed_ktime_ns;
    __u64 pid_generation;
    __u32 pid;
    __u32 tid;
    __u32 host_pid;
    __u32 host_tid;
    __u32 syscall;
};

struct actrail_process_exec_attempt_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 syscall;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 attempt_id;
    __u64 pid_generation;
    __s32 execveat_dirfd;
    __u32 execveat_flags;
    __u32 path_size;
    __u32 argv_size;
    __u32 argc;
    __u32 capture_flags;
    __u32 host_pid;
    __u32 host_tid;
    char path[ACTRAIL_PROCESS_EXEC_PATH_ABI_MAX_BYTES];
};

struct actrail_process_exec_arg_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 syscall;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 attempt_id;
    __u64 pid_generation;
    __u32 index;
    __u32 arg_size;
    __u32 capture_flags;
    __u32 host_pid;
    __u32 host_tid;
    __u32 reserved;
    __u8 arg[ACTRAIL_PROCESS_EXEC_ARGV_ABI_MAX_BYTES];
};

struct actrail_process_exec_result_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 syscall;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 attempt_id;
    __u64 pid_generation;
    __s64 result;
    __u32 host_pid;
    __u32 host_tid;
};

struct actrail_pending_process_fork {
    __u64 trace_id;
    __u64 attempt_id;
    __u64 observed_ktime_ns;
    __u64 pid_generation;
    __u64 flags;
    __u64 clone3_args_ptr;
    __u64 clone3_args_size;
    __u32 pid;
    __u32 tid;
    __u32 host_pid;
    __u32 host_tid;
    __u32 syscall;
    __u32 capture_flags;
};

struct actrail_process_fork_attempt_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 syscall;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 attempt_id;
    __u64 pid_generation;
    __u64 flags;
    __u64 clone3_args_ptr;
    __u64 clone3_args_size;
    __u32 capture_flags;
    __u32 host_pid;
    __u32 host_tid;
    __u32 reserved;
};

struct actrail_process_fork_result_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 syscall;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 attempt_id;
    __u64 pid_generation;
    __s64 result;
    __u32 host_pid;
    __u32 host_tid;
};

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_process_exec_config);
} process_exec_config SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_process_exec);
} pending_process_exec_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} pending_process_exec_tgid_index SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, __u64);
} process_exec_sequences SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_process_fork);
} pending_process_fork_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, __u64);
} process_fork_sequences SEC(".maps");

static __always_inline struct actrail_process_exec_config *current_process_exec_config(void) {
    __u32 key = 0;

    return bpf_map_lookup_elem(&process_exec_config, &key);
}

static __always_inline __u64 next_process_exec_attempt_id(
    __u64 kernel_pid_tgid
) {
    __u64 initial = 1;
    __u64 *current =
        bpf_map_lookup_elem(&process_exec_sequences, &kernel_pid_tgid);

    if (!current) {
        if (bpf_map_update_elem(
                &process_exec_sequences,
                &kernel_pid_tgid,
                &initial,
                BPF_NOEXIST
            ) == 0) {
            return actrail_thread_sequence_id(kernel_pid_tgid, initial);
        }
        current =
            bpf_map_lookup_elem(&process_exec_sequences, &kernel_pid_tgid);
        if (!current) {
            return 0;
        }
    }
    return actrail_thread_sequence_id(
        kernel_pid_tgid,
        actrail_fetch_add_next(current)
    );
}

static __always_inline __u64 next_process_fork_attempt_id(
    __u64 kernel_pid_tgid
) {
    __u64 initial = 1;
    __u64 *current =
        bpf_map_lookup_elem(&process_fork_sequences, &kernel_pid_tgid);

    if (!current) {
        if (bpf_map_update_elem(
                &process_fork_sequences,
                &kernel_pid_tgid,
                &initial,
                BPF_NOEXIST
            ) == 0) {
            return actrail_thread_sequence_id(kernel_pid_tgid, initial);
        }
        current =
            bpf_map_lookup_elem(&process_fork_sequences, &kernel_pid_tgid);
        if (!current) {
            return 0;
        }
    }
    return actrail_thread_sequence_id(
        kernel_pid_tgid,
        actrail_fetch_add_next(current)
    );
}

static __always_inline __u64 current_process_fork_attempt_id(void) {
    __u64 key = current_kernel_pid_tgid();
    struct actrail_pending_process_fork *pending =
        bpf_map_lookup_elem(&pending_process_fork_ops, &key);

    return pending ? pending->attempt_id : 0;
}

static __always_inline void delete_pending_process_exec(
    __u32 host_tgid,
    __u64 kernel_pid_tgid
) {
    __u64 *indexed = bpf_map_lookup_elem(&pending_process_exec_tgid_index, &host_tgid);

    bpf_map_delete_elem(&pending_process_exec_ops, &kernel_pid_tgid);
    if (indexed && *indexed == kernel_pid_tgid) {
        bpf_map_delete_elem(&pending_process_exec_tgid_index, &host_tgid);
    }
}

static __always_inline __u64 take_successful_process_exec_attempt(__u32 host_tgid) {
    __u64 *kernel_pid_tgid = bpf_map_lookup_elem(
        &pending_process_exec_tgid_index,
        &host_tgid
    );
    struct actrail_pending_process_exec *pending;
    __u64 key;
    __u64 attempt_id;

    if (!kernel_pid_tgid) {
        return 0;
    }
    key = *kernel_pid_tgid;
    pending = bpf_map_lookup_elem(&pending_process_exec_ops, &key);
    if (!pending) {
        bpf_map_delete_elem(&pending_process_exec_tgid_index, &host_tgid);
        return 0;
    }
    attempt_id = pending->attempt_id;
    delete_pending_process_exec(host_tgid, key);
    return attempt_id;
}

static __always_inline int finalize_fork_trace_binding(__u32 child_kernel_pid) {
    __u32 child_pid = 0;
    struct actrail_fork_trace_binding *binding =
        bpf_map_lookup_elem(&fork_trace_bindings, &child_kernel_pid);
    struct actrail_process_identity *identity;
    __u64 *explicit_trace_id;
    int tracked_trace_updated;

    if (!binding) {
        return 0;
    }
    child_pid = current_tgid();
    if (!child_pid) {
        return 0;
    }
    explicit_trace_id = bpf_map_lookup_elem(&tracked_traces, &child_pid);
    if (explicit_trace_id) {
        bpf_map_delete_elem(&fork_trace_bindings, &child_kernel_pid);
        return 0;
    }
    /* Equal host and namespace PIDs still require promotion from the
     * fork-only binding into the normal lifecycle maps. */
    tracked_trace_updated = bpf_map_update_elem(
        &tracked_traces,
        &child_pid,
        &binding->trace_id,
        BPF_ANY
    );
    if (tracked_trace_updated != 0) {
        return 0;
    }

    identity = lookup_process_identity(child_kernel_pid);
    if (!identity || identity->start_boottime_ns != binding->child_generation) {
        bpf_map_delete_elem(&tracked_traces, &child_pid);
        return 0;
    }
    inherit_suppressed_fds_for_child(
        binding->parent_pid,
        binding->parent_generation,
        child_pid,
        binding->child_generation
    );
    return 0;
}

static __always_inline int emit_exec_proc_event(
    struct sched_process_exec_ctx *ctx,
    __u32 pid,
    __u64 trace_id
) {
    struct actrail_process_exec_event *event;
    __u32 filename_offset;
    __u32 filename_data_size;
    long filename_size;

    event = actrail_event_reserve(sizeof(*event));
    if (!event) {
        return -1;
    }

    __builtin_memset(event, 0, sizeof(*event));
    init_current_event_header(
        &event->header,
        ACTRAIL_PROC_EXEC,
        sizeof(*event),
        trace_id
    );
    event->attempt_id = take_successful_process_exec_attempt(current_kernel_tgid());
    event->filename_size = 0;
    event->filename_flags = 0;
    event->filename[0] = 0;
    filename_offset = ctx->filename_loc & 0xffff;
    filename_data_size = ctx->filename_loc >> 16;
    if (filename_offset) {
        const void *filename = (const void *)ctx + filename_offset;

        filename_size = bpf_probe_read_kernel_str(
            event->filename,
            sizeof(event->filename),
            filename
        );
        if (filename_size > 0) {
            event->filename_size = (__u32)(filename_size - 1);
            if (filename_size == sizeof(event->filename) ||
                filename_data_size > sizeof(event->filename)) {
                event->filename_flags |= ACTRAIL_EXEC_FILENAME_FLAG_TRUNCATED;
            }
        }
    }

    actrail_event_submit(ctx, event);
    return 0;
}

static __noinline int store_pending_exit_op(
    struct trace_event_raw_sys_enter *ctx,
    __u32 group_exit
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u64 *trace_id = 0;
    struct actrail_pending_exit_op op = {};

    if (pid) {
        trace_id = bpf_map_lookup_elem(&tracked_traces, &pid);
    }
    if (!trace_id) {
        __u64 kernel_pid_tgid = current_kernel_pid_tgid();
        __u32 kernel_pid = kernel_pid_tgid >> 32;

        if (kernel_pid_tgid && kernel_pid_tgid != pid_tgid) {
            trace_id = bpf_map_lookup_elem(&tracked_traces, &kernel_pid);
            if (trace_id) {
                pid_tgid = kernel_pid_tgid;
                pid = kernel_pid;
            }
        }
    }
    if (!pid || !trace_id) {
        return 0;
    }

    op.code = (__s32)ctx->args[0];
    op.group_exit = group_exit;
    if (group_exit) {
        pid_tgid = ((__u64)pid << 32) | pid;
    }
    bpf_map_update_elem(&pending_exit_ops, &pid_tgid, &op, BPF_ANY);
    return 0;
}

static __always_inline void attach_exit_code(
    struct actrail_process_exit_event *event,
    __u64 pid_tgid
) {
    __u64 group_key = (pid_tgid & 0xffffffff00000000ULL) | (pid_tgid >> 32);
    struct actrail_pending_exit_op *op = bpf_map_lookup_elem(&pending_exit_ops, &pid_tgid);

    if (!op && group_key != pid_tgid) {
        op = bpf_map_lookup_elem(&pending_exit_ops, &group_key);
    }
    if (op) {
        event->exit_code = op->code;
        event->exit_flags |= ACTRAIL_PROCESS_EXIT_CODE_VALID;
    }
    bpf_map_delete_elem(&pending_exit_ops, &pid_tgid);
    if (group_key != pid_tgid) {
        bpf_map_delete_elem(&pending_exit_ops, &group_key);
    }
}

static __always_inline void discard_process_exit_codes(__u64 pid_tgid) {
    __u64 group_key = (pid_tgid & 0xffffffff00000000ULL) | (pid_tgid >> 32);

    bpf_map_delete_elem(&pending_exit_ops, &pid_tgid);
    if (group_key != pid_tgid) {
        bpf_map_delete_elem(&pending_exit_ops, &group_key);
    }
}

static __always_inline void discard_thread_exit_code(__u64 pid_tgid) {
    struct actrail_pending_exit_op *op = bpf_map_lookup_elem(&pending_exit_ops, &pid_tgid);

    if (op && !op->group_exit) {
        bpf_map_delete_elem(&pending_exit_ops, &pid_tgid);
    }
}

#endif
