#ifndef ACTRAIL_NETWORK_INTERNAL_SOCKET_H
#define ACTRAIL_NETWORK_INTERNAL_SOCKET_H

#include "../fd/suppressed.h"

#define ACTRAIL_INTERNAL_SOCKET_PATH_BYTES 108

struct actrail_internal_socket_config {
    __u32 path_len;
    char path[ACTRAIL_INTERNAL_SOCKET_PATH_BYTES];
};

struct actrail_pending_internal_connect {
    __u64 trace_id;
    __u32 fd;
};

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_internal_socket_config);
} internal_socket_config SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_internal_connect);
} pending_internal_connects SEC(".maps");

#ifdef ACTRAIL_BPF_LOOP
struct actrail_internal_path_match {
    const struct actrail_internal_socket_config *config;
    const char *path;
    int matches;
};

static long internal_path_match_step(__u32 index, void *opaque) {
    struct actrail_internal_path_match *match = opaque;
    __u64 offset = index;

    /* Keep the array bound in a full register for older verifiers; a 32-bit
     * comparison can lose its range when widened for pointer arithmetic. */
    actrail_barrier_var(offset);
    if (offset >= ACTRAIL_INTERNAL_SOCKET_PATH_BYTES || offset >= match->config->path_len) {
        return 1;
    }
    if (match->path[offset] != match->config->path[offset]) {
        match->matches = 0;
        return 1;
    }
    return 0;
}
#endif

static __always_inline int store_pending_internal_connect(struct trace_event_raw_sys_enter *ctx) {
    __u32 key = 0;
    const struct actrail_internal_socket_config *config =
        bpf_map_lookup_elem(&internal_socket_config, &key);
    __u32 pid = 0, tid = 0, flags = 0;
    __u64 *trace_id;
    __u16 family = 0;
    char path[ACTRAIL_INTERNAL_SOCKET_PATH_BYTES] = {};
    const void *address = (const void *)(unsigned long)ctx->args[1];

    if (!config) {
        return 0;
    }
    __u64 path_len = config->path_len;
    if (!path_len) {
        return 0;
    }
    /* Keep the helper's upper bound visible to older verifiers in a full register. */
    actrail_barrier_var(path_len);
    if (path_len >= sizeof(path)) {
        return 0;
    }
    trace_id = lookup_current_trace(&pid, &tid, &flags);
    if (!trace_id || !pid || ctx->args[2] < sizeof(family) + path_len) {
        return 0;
    }
    if (bpf_probe_read_user(&family, sizeof(family), address) != 0 || family != AF_UNIX) {
        return 0;
    }
    if (bpf_probe_read_user(path, path_len, address + sizeof(family)) != 0) {
        return 0;
    }
    if (ctx->args[2] > sizeof(family) + path_len) {
        char terminator = 0;
        if (bpf_probe_read_user(&terminator, 1, address + sizeof(family) + path_len) != 0
            || terminator != 0) {
            return 0;
        }
    }
#ifdef ACTRAIL_BPF_LOOP
    struct actrail_internal_path_match match = { .config = config, .path = path, .matches = 1 };
    if (bpf_loop(ACTRAIL_INTERNAL_SOCKET_PATH_BYTES, internal_path_match_step, &match, 0) < 0
        || !match.matches) {
        return 0;
    }
#else
#pragma unroll
    for (__u32 index = 0; index < ACTRAIL_INTERNAL_SOCKET_PATH_BYTES; index++) {
        if (index >= config->path_len) {
            break;
        }
        if (path[index] != config->path[index]) {
            return 0;
        }
    }
#endif
    __u64 pid_tgid = current_kernel_pid_tgid();
    struct actrail_pending_internal_connect pending = {
        .trace_id = *trace_id,
        .fd = (__u32)ctx->args[0],
    };
    bpf_map_update_elem(&pending_internal_connects, &pid_tgid, &pending, BPF_ANY);
    return 1;
}

static __always_inline int finish_internal_connect(struct trace_event_raw_sys_exit *ctx) {
    __u64 pid_tgid = current_kernel_pid_tgid();
    struct actrail_pending_internal_connect *pending =
        bpf_map_lookup_elem(&pending_internal_connects, &pid_tgid);

    if (!pending) {
        return 0;
    }
    if (ctx->ret == 0) {
        __u32 pid = pid_tgid >> 32;
        struct actrail_suppressed_fd_value value = {
            .trace_id = pending->trace_id,
            .purpose = 1, /* TLS_SYNC_EVENT */
        };
        upsert_suppressed_fd_for_generation(pid, current_process_start_time(pid), pending->fd, &value);
    }
    bpf_map_delete_elem(&pending_internal_connects, &pid_tgid);
    return 1;
}

#endif
