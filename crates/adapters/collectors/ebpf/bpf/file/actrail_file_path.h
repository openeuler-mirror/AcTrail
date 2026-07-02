#ifndef ACTRAIL_FILE_PATH_H
#define ACTRAIL_FILE_PATH_H

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

static __always_inline void init_file_event_header(
    struct actrail_file_event *event,
    __u32 kind
) {
    __builtin_memset(event, 0, ACTRAIL_FILE_EVENT_HEADER_SIZE);
    event->kind = kind;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->fd = ACTRAIL_FILE_FD_MISSING;
}

static __always_inline void init_file_event_primary_path(
    struct actrail_file_event *event,
    __u32 kind
) {
    __builtin_memset(event, 0, ACTRAIL_FILE_EVENT_PRIMARY_PATH_SIZE);
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

#endif
