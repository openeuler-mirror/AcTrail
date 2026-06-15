#ifndef ACTRAIL_SOCKET_PAYLOAD_TYPES_H
#define ACTRAIL_SOCKET_PAYLOAD_TYPES_H

#include "actrail_runtime.h"

enum actrail_socket_payload_abi {
    ACTRAIL_SOCKET_PAYLOAD_ABI_MAX_BYTES = 4096,
    ACTRAIL_SOCKET_PAYLOAD_COPY_MAX_BYTES = 4095,
};

enum actrail_socket_payload_direction {
    ACTRAIL_SOCKET_PAYLOAD_INBOUND = 1,
    ACTRAIL_SOCKET_PAYLOAD_OUTBOUND = 2,
};

enum actrail_socket_payload_syscall {
    ACTRAIL_SOCKET_SYSCALL_READ = 1,
    ACTRAIL_SOCKET_SYSCALL_WRITE = 2,
    ACTRAIL_SOCKET_SYSCALL_SENDTO = 3,
    ACTRAIL_SOCKET_SYSCALL_RECVFROM = 4,
    ACTRAIL_SOCKET_SYSCALL_WRITEV = 5,
    ACTRAIL_SOCKET_SYSCALL_SENDMSG = 6,
};

enum actrail_socket_payload_flags {
    ACTRAIL_SOCKET_PAYLOAD_TRUNCATED = 1,
};

enum actrail_socket_dup_mode {
    ACTRAIL_SOCKET_DUP_RET_FD = 1,
    ACTRAIL_SOCKET_DUP_TARGET_FD = 2,
};

struct actrail_socket_payload_config {
    __u32 enabled;
    __u32 max_segment_bytes;
    __u32 user_read_enabled;
};

struct actrail_socket_payload_fd_key {
    __u32 pid;
    __u32 fd;
};

struct actrail_socket_payload_sequence_key {
    __u32 pid;
    __u32 direction;
    __u32 fd;
    __u32 fd_generation;
};

struct actrail_pending_socket_payload_op {
    __u64 trace_id;
    __u64 buffer_ptr;
    __u64 requested_size;
    __u32 fd;
    __u32 fd_generation;
    __u32 direction;
    __u32 syscall;
};

struct actrail_pending_socket_dup_op {
    __u32 source_fd;
    __u32 target_fd;
    __u32 source_generation;
    __u32 target_generation;
    __u32 mode;
};

struct actrail_socket_payload_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 sequence;
    __u32 fd;
    __u32 original_size;
    __u32 captured_size;
    __u32 flags;
    __u32 syscall;
    __u32 fd_generation;
    __u64 pid_generation;
    __u8 bytes[ACTRAIL_SOCKET_PAYLOAD_ABI_MAX_BYTES];
};

struct actrail_socket_payload_completion_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 sequence;
    __u64 completed_size;
    __u64 requested_size;
    __u64 buffer_ptr;
    __u64 pid_generation;
    __u32 fd;
    __u32 flags;
    __u32 syscall;
    __u32 fd_generation;
};

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
    __type(value, __u32);
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
