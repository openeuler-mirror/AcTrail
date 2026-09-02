#ifndef ACTRAIL_PAYLOAD_SOCKET_TYPES_H
#define ACTRAIL_PAYLOAD_SOCKET_TYPES_H

#include "../abi/payload.h"
#include "../runtime/event_transport.h"

enum actrail_socket_payload_fd_flags {
    ACTRAIL_SOCKET_FD_TLS_OWNED = 1,
};

enum actrail_socket_dup_mode {
    ACTRAIL_SOCKET_DUP_RET_FD = 1,
    ACTRAIL_SOCKET_DUP_TARGET_FD = 2,
};

struct actrail_socket_payload_config {
    __u32 enabled;
    __u32 max_segment_bytes;
    __u32 max_operation_bytes;
    __u32 user_read_enabled;
    __u32 truncation_flags;
};

struct actrail_user_iovec {
    __u64 base;
    __u64 len;
};

/* Native 64-bit Linux userspace ABI. Supported collector targets are 64-bit. */
struct actrail_user_msghdr {
    __u64 name;
    __u32 name_len;
    __u32 padding;
    __u64 iov;
    __u64 iov_len;
    __u64 control;
    __u64 control_len;
    __u32 flags;
    __u32 padding2;
};

struct actrail_socket_payload_fd_key {
    __u32 pid;
    __u32 fd;
};

struct actrail_socket_payload_fd_state {
    __u32 generation;
    __u32 flags;
};

struct actrail_socket_payload_sequence_key {
    __u32 pid;
    __u32 direction;
    __u32 fd;
    __u32 fd_generation;
};

struct actrail_pending_socket_payload_op {
    __u64 trace_id;
    __u64 operation_id;
    __u64 buffer_ptr;
    __u64 requested_size;
    __u64 copy_base;
    __u64 copy_capacity;
    __u64 pid_generation;
    __u64 started_ktime_ns;
    __u32 fd;
    __u32 fd_generation;
    __u32 direction;
    __u32 syscall;
    __u32 delegated;
    __u32 truncation_flags;
};

struct actrail_pending_socket_dup_op {
    __u32 source_fd;
    __u32 target_fd;
    __u32 source_generation;
    __u32 target_generation;
    __u32 source_flags;
    __u32 mode;
};
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, __u64);
} payload_socket_operation_sequence SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_socket_payload_config);
} payload_socket_config SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_socket_payload_fd_key);
    __type(value, struct actrail_socket_payload_fd_state);
} payload_socket_fds SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u32);
} payload_socket_process_generations SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_socket_payload_op);
} pending_socket_payload_ops SEC(".maps");

/*
 * Keeping the pending operation value off the program stack avoids consuming
 * the arm64 512-byte BPF stack in every socket payload handler. Tracepoint
 * programs cannot migrate while running, so a single per-CPU slot safely
 * supplies the temporary operation value until it is copied into the pending
 * hash map.
 */
struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_pending_socket_payload_op);
} socket_payload_op_scratch SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_socket_dup_op);
} pending_socket_dup_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_socket_payload_sequence_key);
    __type(value, __u64);
} payload_socket_stream_sequences SEC(".maps");


#endif
