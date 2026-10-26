#ifndef ACTRAIL_FILE_OPEN_H
#define ACTRAIL_FILE_OPEN_H

#include "observe.h"

struct actrail_open_how {
    __u64 flags;
    __u64 mode;
    __u64 resolve;
};

static __always_inline void read_file_open_how(
    struct trace_event_raw_sys_enter *ctx,
    struct actrail_open_how *how
) {
    __u64 how_ptr = (__u64)ctx->args[2];
    if (how_ptr) {
        bpf_probe_read_user(how, sizeof(*how), (void *)(unsigned long)how_ptr);
    }
}

static __always_inline int capture_file_open_enter(
    struct trace_event_raw_sys_enter *ctx,
    __u32 syscall_id,
    const struct actrail_open_how *how
) {
    __u64 operation_key = current_kernel_pid_tgid();
    __u32 zero = 0;
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id;
    struct actrail_file_event *event;
    __u64 path_ptr;

    if (!operation_key || !file_context_capture_enabled(syscall_id)) {
        return 0;
    }
    // Remove an abandoned operation before accepting a new one on the thread.
    bpf_map_delete_elem(&pending_file_open_observations, &operation_key);
    trace_id = lookup_current_detailed_trace(&tgid, &tid, &lookup_flags);
    if (!tgid || !trace_id) {
        return 0;
    }
    event = (struct actrail_file_event *)bpf_map_lookup_elem(&file_open_scratch, &zero);
    if (!event) {
        event_transport_diag_inc(ACTRAIL_FILE_PENDING_UPDATE_FAIL);
        return 0;
    }
    init_file_event_primary_path(event, ACTRAIL_FILE_OPEN);
    event->pid = tgid;
    event->tid = tid;
    event->pid_generation = current_process_start_time(tgid);
    event->trace_id = *trace_id;
    event->aux = syscall_id;
    event->phase = ACTRAIL_FILE_PHASE_EXIT;
    if (syscall_id == ACTRAIL_FILE_SYSCALL_OPENAT2) {
        event->arg0 = ctx->args[0];
        event->arg1 = ctx->args[1];
        event->arg2 = how->flags;
        event->arg3 = how->mode;
        event->arg4 = how->resolve;
        event->arg5 = ctx->args[3];
    } else {
        fill_file_args(event, ctx, syscall_id == ACTRAIL_FILE_SYSCALL_OPENAT ? 4 :
            syscall_id == ACTRAIL_FILE_SYSCALL_OPEN ? 3 : 2);
    }
    path_ptr = syscall_id == ACTRAIL_FILE_SYSCALL_OPENAT
        || syscall_id == ACTRAIL_FILE_SYSCALL_OPENAT2 ? ctx->args[1] : ctx->args[0];
    if (file_path_capture_enabled()) {
        read_file_path(event, path_ptr, ACTRAIL_FILE_PRIMARY_PATH);
    }
    if (bpf_map_update_elem(&pending_file_open_observations, &operation_key, event, BPF_ANY)) {
        event_transport_diag_inc(ACTRAIL_FILE_PENDING_UPDATE_FAIL);
    }
    return 0;
}

static __always_inline int emit_file_open_exit(
    struct trace_event_raw_sys_exit *ctx,
    __u32 syscall_id
) {
    __u64 operation_key = current_kernel_pid_tgid();
    struct actrail_file_event *pending;
    struct actrail_file_event *event;
    __u32 flags = file_capture_flags();
    __u32 category = ACTRAIL_FD_CATEGORY_NONE;
    __u64 open_flags;
    int writable;
    int observable;
    int context;

    pending = (struct actrail_file_event *)bpf_map_lookup_elem(&pending_file_open_observations, &operation_key);
    if (!pending) {
        return 0;
    }
    if (pending->aux != syscall_id) {
        bpf_map_delete_elem(&pending_file_open_observations, &operation_key);
        event_transport_diag_inc(ACTRAIL_FILE_PENDING_UPDATE_FAIL);
        return 0;
    }
    open_flags = syscall_id == ACTRAIL_FILE_SYSCALL_OPEN ? pending->arg1 : pending->arg2;
    // __O_TMPFILE is the high bit only; O_DIRECTORY alone is not write intent.
    writable = syscall_id == ACTRAIL_FILE_SYSCALL_CREAT
        || (open_flags & (3U | 0100U | 01000U | 020000000U));
    if (ctx->ret >= 0 && (flags & ACTRAIL_CAPTURE_FILE_PATH)) {
        category = fd_open_category(fd_kernel_file_identity((__u32)ctx->ret));
    }
    observable = ((flags & ACTRAIL_CAPTURE_FILE_WRITABLE_OPEN) && writable)
        || ((flags & ACTRAIL_CAPTURE_FILE_FD_MUTATIONS) && file_path_capture_enabled());
    if (category == ACTRAIL_FD_CATEGORY_DIRECTORY) {
        observable = flags & ACTRAIL_CAPTURE_FILE_ENUMERATE;
    }
    context = ctx->ret >= 0 && ((category == ACTRAIL_FD_CATEGORY_DIRECTORY
        && file_path_capture_enabled()) || (flags & ACTRAIL_CAPTURE_MCP_STDIO));
    if (!observable && !context) {
        bpf_map_delete_elem(&pending_file_open_observations, &operation_key);
        return 0;
    }
    if (observable || category == ACTRAIL_FD_CATEGORY_DIRECTORY) {
        event = actrail_event_reserve(ACTRAIL_FILE_EVENT_PRIMARY_PATH_SIZE);
        if (!event) {
            bpf_map_delete_elem(&pending_file_open_observations, &operation_key);
            return 0;
        }
        __builtin_memcpy(event, pending, ACTRAIL_FILE_EVENT_PRIMARY_PATH_SIZE);
    } else {
        event = actrail_event_reserve(ACTRAIL_FILE_EVENT_HEADER_SIZE);
        if (!event) {
            bpf_map_delete_elem(&pending_file_open_observations, &operation_key);
            return 0;
        }
        __builtin_memcpy(event, pending, ACTRAIL_FILE_EVENT_HEADER_SIZE);
        event->path_size = 0;
        event->path_flags = 0;
        event->path_max_bytes = 0;
    }
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->result = ctx->ret;
    if (category == ACTRAIL_FD_CATEGORY_DIRECTORY) {
        event->path_flags |= ACTRAIL_FILE_PATH_DIRECTORY;
    }
    // FD mutation context is internal unless a writable-open consumer also
    // needs this result. I/O summaries carry their own identity and path.
    if (!observable || (ctx->ret >= 0
        && !(writable && (flags & ACTRAIL_CAPTURE_FILE_WRITABLE_OPEN))
        && category == ACTRAIL_FD_CATEGORY_FILE)) {
        event->path_flags |= ACTRAIL_FILE_PATH_INTERNAL_CONTEXT;
    }
    bpf_map_delete_elem(&pending_file_open_observations, &operation_key);
    actrail_event_submit(ctx, event);

    return 0;
}

#endif
