#ifndef ACTRAIL_FILE_OPEN_H
#define ACTRAIL_FILE_OPEN_H

#include "../actrail_file.h"

struct actrail_open_how {
    __u64 flags;
    __u64 mode;
    __u64 resolve;
};

static __always_inline int emit_file_open_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_OPEN,
            ACTRAIL_FILE_SYSCALL_OPEN,
            ACTRAIL_FILE_SYSCALL_ARGC_OPEN
        ),
        ACTRAIL_FILE_FD_MISSING,
        (__u64)ctx->args[0],
        0
    );
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

static __always_inline int emit_file_creat_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    return emit_file_enter(
        ctx,
        file_enter_descriptor(
            ACTRAIL_FILE_OPEN,
            ACTRAIL_FILE_SYSCALL_CREAT,
            ACTRAIL_FILE_SYSCALL_ARGC_CREAT
        ),
        ACTRAIL_FILE_FD_MISSING,
        (__u64)ctx->args[0],
        0
    );
}

static __always_inline int emit_file_openat2_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    __u64 pid_tgid = current_pid_tgid();
    __u32 tgid = current_namespace_tgid();
    __u64 *trace_id = bpf_map_lookup_elem(&tracked_traces, &tgid);
    struct actrail_file_event *event;
    struct actrail_open_how how = {};
    __u64 how_ptr = (__u64)ctx->args[2];

    if (!tgid) {
        return 0;
    }
    if (!trace_id) {
        return 0;
    }
    if (how_ptr) {
        bpf_probe_read_user(&how, sizeof(how), (void *)(unsigned long)how_ptr);
    }

    event = bpf_ringbuf_reserve(&events, sizeof(*event), 0);
    if (!event) {
        return 0;
    }

    init_file_event(event, ACTRAIL_FILE_OPEN);
    event->pid = tgid;
    event->tid = (__u32)pid_tgid;
    event->pid_generation = ensure_process_generation(tgid);
    event->phase = ACTRAIL_FILE_PHASE_ENTER;
    event->trace_id = *trace_id;
    event->aux = ACTRAIL_FILE_SYSCALL_OPENAT2;
    event->arg0 = ctx->args[0];
    event->arg1 = ctx->args[1];
    event->arg2 = how.flags;
    event->arg3 = how.mode;
    event->arg4 = how.resolve;
    event->arg5 = ctx->args[3];
    read_file_path(event, (__u64)ctx->args[1], ACTRAIL_FILE_PRIMARY_PATH);
    bpf_ringbuf_submit(event, 0);
    return 0;
}

#endif
