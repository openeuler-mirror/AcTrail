#ifndef ACTRAIL_FD_LIFECYCLE_H
#define ACTRAIL_FD_LIFECYCLE_H

#include "sweep.h"
#include "../network/state.h"

static __always_inline __u32 fd_creation_flags(__u64 raw_flags) {
    return (raw_flags & O_CLOEXEC) ? ACTRAIL_FD_FLAG_CLOEXEC : 0;
}

static __always_inline int fd_socket_enter(struct trace_event_raw_sys_enter *ctx) {
    __u64 operation_key = current_kernel_pid_tgid();
    __u32 category = fd_category_for_domain((__u32)ctx->args[0]);
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id;
    struct actrail_pending_net_op op = {};

    if (!operation_key || !fd_category_enabled(category)) {
        return 0;
    }
    trace_id = lookup_current_trace(&tgid, &tid, &lookup_flags);
    if (!tgid || !trace_id) {
        return 0;
    }
    op.trace_id = *trace_id;
    op.pid = tgid;
    op.kind = ACTRAIL_FD_SOCKET_PENDING_KIND;
    op.syscall_family = (__u32)ctx->args[0];
    op.flags = (__u32)ctx->args[1];
    bpf_map_update_elem(&pending_net_ops, &operation_key, &op, BPF_ANY);
    return 0;
}

static __always_inline int fd_socket_exit(struct trace_event_raw_sys_exit *ctx) {
    __u64 operation_key = current_kernel_pid_tgid();
    struct actrail_pending_net_op *pending =
        bpf_map_lookup_elem(&pending_net_ops, &operation_key);
    struct actrail_pending_net_op op;
    struct actrail_fd_registration registration = {};
    __u32 category;

    if (!pending || pending->kind != ACTRAIL_FD_SOCKET_PENDING_KIND) {
        return 0;
    }
    op = *pending;
    bpf_map_delete_elem(&pending_net_ops, &operation_key);
    if (ctx->ret < 0) {
        return 0;
    }
    category = fd_category_for_domain(op.syscall_family);
    registration.trace_id = op.trace_id;
    registration.program_ctx = ctx;
    registration.pid = op.pid;
    registration.fd = (__u32)ctx->ret;
    registration.category = category;
    registration.flags = fd_creation_flags(op.flags);
    fd_register(&registration);
    return 0;
}

static __always_inline int fd_open_enter(
    struct trace_event_raw_sys_enter *ctx,
    __u64 raw_flags
) {
    __u64 operation_key = current_kernel_pid_tgid();
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id;
    struct actrail_pending_fd_open_op op = {};

    if (!operation_key || !fd_category_enabled(ACTRAIL_FD_CATEGORY_FILE)) {
        return 0;
    }
    trace_id = lookup_current_trace(&tgid, &tid, &lookup_flags);
    if (!tgid || !trace_id) {
        return 0;
    }
    op.trace_id = *trace_id;
    op.pid = tgid;
    op.flags = fd_creation_flags(raw_flags);
    bpf_map_update_elem(&pending_fd_open_ops, &operation_key, &op, BPF_ANY);
    return 0;
}

static __always_inline int fd_register_open_exit(struct trace_event_raw_sys_exit *ctx) {
    __u64 operation_key = current_kernel_pid_tgid();
    struct actrail_pending_fd_open_op *pending =
        bpf_map_lookup_elem(&pending_fd_open_ops, &operation_key);
    struct actrail_pending_fd_open_op op;
    struct actrail_fd_registration registration = {};

    if (!pending) {
        return 0;
    }
    op = *pending;
    bpf_map_delete_elem(&pending_fd_open_ops, &operation_key);
    if (ctx->ret >= 0) {
        registration.trace_id = op.trace_id;
        registration.program_ctx = ctx;
        registration.pid = op.pid;
        registration.fd = (__u32)ctx->ret;
        registration.category = ACTRAIL_FD_CATEGORY_FILE;
        registration.flags = op.flags;
        fd_register(&registration);
    }
    return 0;
}

static __always_inline int fd_close_dispatch_enter(struct trace_event_raw_sys_enter *ctx) {
    __u64 operation_key = current_kernel_pid_tgid();
    __u32 pid = operation_key >> 32;
    __u32 fd = (__u32)ctx->args[0];
    struct actrail_fd_state *state;
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id;
    struct actrail_pending_fd_close_op op = {};

    if (!operation_key) {
        return 0;
    }
    state = fd_lookup(pid, fd);
    if (!state) {
        return 0;
    }
    op.expected_generation = state->generation;
    trace_id = lookup_current_trace(&tgid, &tid, &lookup_flags);
    if (!tgid || !trace_id) {
        return 0;
    }
    op.trace_id = *trace_id;
    op.pid = tgid;
    op.first_fd = fd;
    op.last_fd = fd;
    op.mode = ACTRAIL_FD_CLOSE_ONE;
    bpf_map_update_elem(&pending_fd_close_ops, &operation_key, &op, BPF_ANY);
    return 0;
}

static __always_inline int fd_close_range_dispatch_enter(
    struct trace_event_raw_sys_enter *ctx
) {
    __u64 operation_key = current_kernel_pid_tgid();
    __u32 pid = operation_key >> 32;
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id;
    struct actrail_pending_fd_close_op op = {};

    if (!operation_key || !fd_tracking_enabled()) {
        return 0;
    }
    trace_id = lookup_current_trace(&tgid, &tid, &lookup_flags);
    if (!tgid || !trace_id) {
        return 0;
    }
    op.trace_id = *trace_id;
    op.pid = tgid;
    op.first_fd = (__u32)ctx->args[0];
    op.last_fd = (__u32)ctx->args[1];
    op.flags = (__u32)ctx->args[2];
    op.mode = ACTRAIL_FD_CLOSE_RANGE;
    bpf_map_update_elem(&pending_fd_close_ops, &operation_key, &op, BPF_ANY);
    return 0;
}

static __always_inline int fd_close_dispatch_exit(struct trace_event_raw_sys_exit *ctx) {
    __u64 operation_key = current_kernel_pid_tgid();
    struct actrail_pending_fd_close_op *pending =
        bpf_map_lookup_elem(&pending_fd_close_ops, &operation_key);
    struct actrail_pending_fd_close_op op;
    struct actrail_fd_state *state;
    struct actrail_fd_object_state *object;
    __u64 file_identity = 0;

    if (!pending) {
        return 0;
    }
    op = *pending;
    bpf_map_delete_elem(&pending_fd_close_ops, &operation_key);
    if (op.mode == ACTRAIL_FD_CLOSE_ONE) {
        file_identity = fd_kernel_file_identity(op.first_fd);
        if (file_identity == ACTRAIL_FD_FILE_IDENTITY_READ_FAILED) {
            return 0;
        }
        state = fd_lookup(op.pid, op.first_fd);
        object = state ? fd_object_lookup(op.pid, state->generation) : 0;
        if (state && (!file_identity || !object
            || object->file_identity != file_identity)) {
            __u64 generation = state->generation;

            fd_release(
                op.pid,
                op.first_fd,
                generation,
                op.trace_id,
                ctx
            );
        }
    } else if (op.mode == ACTRAIL_FD_CLOSE_RANGE && ctx->ret == 0) {
        fd_close_range_dispatch(
            op.pid,
            op.first_fd,
            op.last_fd,
            op.flags,
            op.trace_id,
            ctx
        );
    }
    return 0;
}

static __always_inline void fd_pending_dup_cleanup(__u64 operation_key, void *ctx) {
    struct actrail_pending_fd_dup_op *pending =
        bpf_map_lookup_elem(&pending_fd_dup_ops, &operation_key);

    if (pending) {
        struct actrail_pending_fd_dup_op op = *pending;

        bpf_map_delete_elem(&pending_fd_dup_ops, &operation_key);
        if (op.reference_reserved) {
            fd_reserved_ref_release(
                op.pid,
                op.source_fd,
                &op.source,
                op.trace_id,
                ctx
            );
        }
    }
}

static __always_inline int fd_dup_enter(
    struct trace_event_raw_sys_enter *ctx,
    __u32 source_fd,
    __u32 target_fd,
    __u32 mode,
    __u32 target_flags
) {
    __u64 operation_key = current_kernel_pid_tgid();
    __u32 pid = operation_key >> 32;
    struct actrail_fd_state *source;
    struct actrail_fd_object_state *source_object;
    struct actrail_pending_fd_dup_op op = {};
    __u64 source_identity = 0;
    __u32 tgid = 0;
    __u32 tid = 0;
    __u32 lookup_flags = 0;
    __u64 *trace_id;

    if (!operation_key) {
        return 0;
    }
    fd_pending_dup_cleanup(operation_key, ctx);
    source = fd_lookup(pid, source_fd);
    if (!source) {
        return 0;
    }
    op.source = *source;
    source_object = fd_object_lookup(pid, op.source.generation);
    if (!source_object) {
        return 0;
    }
    op.source_file_identity = source_object->file_identity;
    source_identity = fd_kernel_file_identity(source_fd);
    if (source_identity == ACTRAIL_FD_FILE_IDENTITY_READ_FAILED
        || source_identity != op.source_file_identity) {
        return 0;
    }
    op.target_fd = target_fd;
    trace_id = lookup_current_trace(&tgid, &tid, &lookup_flags);
    if (!tgid || !trace_id) {
        return 0;
    }
    op.trace_id = *trace_id;
    op.pid = pid;
    op.source_fd = source_fd;
    op.target_flags = target_flags;
    op.mode = mode;
    if (!(mode == ACTRAIL_FD_DUP_TARGET_FD && op.target_fd == source_fd)) {
        if (!fd_object_ref_reserve(pid, source_fd, &op.source, op.trace_id, ctx)) {
            return 0;
        }
        op.reference_reserved = 1;
    }
    if (bpf_map_update_elem(&pending_fd_dup_ops, &operation_key, &op, BPF_NOEXIST) != 0
        && op.reference_reserved) {
        fd_reserved_ref_release(pid, source_fd, &op.source, op.trace_id, ctx);
    }
    return 0;
}

static __always_inline int fd_dup_exit(struct trace_event_raw_sys_exit *ctx) {
    __u64 operation_key = current_kernel_pid_tgid();
    struct actrail_pending_fd_dup_op *pending =
        bpf_map_lookup_elem(&pending_fd_dup_ops, &operation_key);
    struct actrail_pending_fd_dup_op op;
    struct actrail_fd_state source;
    struct actrail_fd_state candidate;
    struct actrail_fd_state *current;
    struct actrail_fd_object_state *candidate_object;
    struct actrail_fd_object_state *target_object;
    __u64 source_identity;
    __u64 target_identity = 0;
    __u64 generation;
    __u32 descriptor_flags;
    __u32 target_fd;
    int close_on_exec = 0;
    int reservation_held;

    if (!pending) {
        return 0;
    }
    op = *pending;
    source = op.source;
    source_identity = op.source_file_identity;
    descriptor_flags = op.target_flags;
    reservation_held = op.reference_reserved;
    bpf_map_delete_elem(&pending_fd_dup_ops, &operation_key);
    if (ctx->ret < 0) {
        goto rollback;
    }
    target_fd = op.mode == ACTRAIL_FD_DUP_RET_FD ? (__u32)ctx->ret : op.target_fd;
    if (target_fd == op.source_fd) {
        return 0;
    }
    target_identity = fd_kernel_file_identity(target_fd);
    if (target_identity == ACTRAIL_FD_FILE_IDENTITY_READ_FAILED) {
        goto rollback;
    }
    if (!target_identity) {
        goto reconcile;
    }
    if (source_identity != target_identity) {
        current = fd_lookup(op.pid, op.source_fd);
        if (!current) {
            goto reconcile;
        }
        candidate = *current;
        candidate_object = fd_object_lookup(op.pid, candidate.generation);
        if (!candidate_object || candidate_object->file_identity != target_identity) {
            goto reconcile;
        }
        if (!fd_object_ref_reserve(
                op.pid,
                op.source_fd,
                &candidate,
                op.trace_id,
                ctx
            )) {
            goto reconcile;
        }
        if (reservation_held) {
            fd_reserved_ref_release(
                op.pid,
                op.source_fd,
                &source,
                op.trace_id,
                ctx
            );
        }
        source = candidate;
        source_identity = target_identity;
        reservation_held = 1;
    }
    target_identity = fd_kernel_file_identity(target_fd);
    if (target_identity == ACTRAIL_FD_FILE_IDENTITY_READ_FAILED) {
        goto rollback;
    }
    if (target_identity != source_identity) {
        goto reconcile;
    }
    close_on_exec = fd_kernel_cloexec(target_fd);
    if (close_on_exec >= 0) {
        descriptor_flags = close_on_exec ? ACTRAIL_FD_FLAG_CLOEXEC : 0;
    }
    current = fd_lookup(op.pid, target_fd);
    if (current && current->generation == source.generation) {
        current->flags = descriptor_flags;
        goto rollback;
    }
    if (current) {
        generation = current->generation;
        target_identity = fd_kernel_file_identity(target_fd);
        if (target_identity == ACTRAIL_FD_FILE_IDENTITY_READ_FAILED) {
            goto rollback;
        }
        if (target_identity != source_identity) {
            goto reconcile;
        }
        fd_release(
            op.pid,
            target_fd,
            generation,
            op.trace_id,
            ctx
        );
    }
    target_identity = fd_kernel_file_identity(target_fd);
    if (target_identity == ACTRAIL_FD_FILE_IDENTITY_READ_FAILED) {
        goto rollback;
    }
    if (target_identity != source_identity) {
        goto reconcile;
    }
    if (!fd_install_state(op.pid, target_fd, &source, descriptor_flags)) {
        goto rollback;
    }
    reservation_held = 0;
    target_identity = fd_kernel_file_identity(target_fd);
    if (target_identity != ACTRAIL_FD_FILE_IDENTITY_READ_FAILED
        && target_identity != source_identity) {
        fd_release(
            op.pid,
            target_fd,
            source.generation,
            op.trace_id,
            ctx
        );
    }
    return 0;

reconcile:
    current = fd_lookup(op.pid, target_fd);
    if (current) {
        generation = current->generation;
        target_object = fd_object_lookup(op.pid, generation);
        if (!target_identity || !target_object
            || target_object->file_identity != target_identity) {
            fd_release(
                op.pid,
                target_fd,
                generation,
                op.trace_id,
                ctx
            );
        }
    }
rollback:
    if (reservation_held) {
        fd_reserved_ref_release(
            op.pid,
            op.source_fd,
            &source,
            op.trace_id,
            ctx
        );
    }
    return 0;
}

static __always_inline int fd_flag_enter(
    struct trace_event_raw_sys_enter *ctx,
    __u32 fd,
    __u32 flags,
    __u32 context_kind
) {
    __u64 operation_key = current_kernel_pid_tgid();
    __u32 pid = operation_key >> 32;
    struct actrail_fd_state *state;
    struct actrail_pending_fd_flag_op op = {};

    if (!operation_key) {
        return 0;
    }
    state = fd_lookup(pid, fd);
    if (state) {
        op.generation = state->generation;
    } else if (context_kind == ACTRAIL_FD_FLAG_CONTEXT_NONE) {
        return 0;
    }
    op.pid = pid;
    op.fd = fd;
    op.flags = flags;
    op.context_kind = context_kind;
    return bpf_map_update_elem(
        &pending_fd_flag_ops,
        &operation_key,
        &op,
        BPF_ANY
    ) == 0;
}

static __always_inline void fd_fcntl_flag_enter(struct trace_event_raw_sys_enter *ctx) {
    if ((__u32)ctx->args[1] == F_SETFD) {
        fd_flag_enter(
            ctx,
            (__u32)ctx->args[0],
            ((__u32)ctx->args[2] & FD_CLOEXEC) ? ACTRAIL_FD_FLAG_CLOEXEC : 0,
            ACTRAIL_FD_FLAG_CONTEXT_NONE
        );
    }
}

static __always_inline __u32 fd_ioctl_flag_enter(struct trace_event_raw_sys_enter *ctx) {
    __u32 command = (__u32)ctx->args[1];
    __u32 context_kind;

    if (command == FIOCLEX) {
        context_kind = ACTRAIL_FD_FLAG_CONTEXT_IOCTL_CLOEXEC;
    } else if (command == FIONCLEX) {
        context_kind = ACTRAIL_FD_FLAG_CONTEXT_IOCTL_NCLOEXEC;
    } else {
        return ACTRAIL_FD_FLAG_CONTEXT_NONE;
    }
    return fd_flag_enter(
        ctx,
        (__u32)ctx->args[0],
        command == FIOCLEX ? ACTRAIL_FD_FLAG_CLOEXEC : 0,
        context_kind
    ) ? context_kind : ACTRAIL_FD_FLAG_CONTEXT_NONE;
}

static __always_inline __u32 fd_flag_exit(struct trace_event_raw_sys_exit *ctx) {
    __u64 operation_key = current_kernel_pid_tgid();
    struct actrail_pending_fd_flag_op *pending =
        bpf_map_lookup_elem(&pending_fd_flag_ops, &operation_key);
    struct actrail_pending_fd_flag_op op;
    struct actrail_fd_state *state;

    if (!pending) {
        return ACTRAIL_FD_FLAG_CONTEXT_NONE;
    }
    op = *pending;
    bpf_map_delete_elem(&pending_fd_flag_ops, &operation_key);
    if (ctx->ret != 0) {
        return op.context_kind;
    }
    state = fd_lookup(op.pid, op.fd);
    if (state && state->generation == op.generation) {
        state->flags = op.flags;
    }
    return op.context_kind;
}

static __always_inline void fd_pending_thread_cleanup(__u64 operation_key, void *ctx) {
    fd_pending_dup_cleanup(operation_key, ctx);
    bpf_map_delete_elem(&pending_fd_open_ops, &operation_key);
    bpf_map_delete_elem(&pending_fd_close_ops, &operation_key);
    bpf_map_delete_elem(&pending_fd_flag_ops, &operation_key);
    bpf_map_delete_elem(&pending_net_ops, &operation_key);
    bpf_map_delete_elem(&pending_ipc_fd_pair_ops, &operation_key);
}

#endif
