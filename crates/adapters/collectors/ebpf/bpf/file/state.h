#ifndef ACTRAIL_FILE_STATE_H
#define ACTRAIL_FILE_STATE_H

#include "../abi/file_path.h"
#include "../runtime/event_transport.h"

struct actrail_file_config {
    __u32 path_max_bytes;
    __u32 capture_flags;
};
struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_file_config);
} file_config SEC(".maps");

struct actrail_pending_file_completion {
    __u64 trace_id;
    __u64 pid_generation;
    __u64 args[6];
    __u32 pid;
    __u32 tid;
    __u32 fd;
    __u32 syscall_id;
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(map_flags, BPF_F_NO_PREALLOC);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_file_completion);
} pending_file_completion_ops SEC(".maps");

// Open carries one primary path, never a secondary path.
struct actrail_pending_file_open_observation {
    __u8 bytes[ACTRAIL_FILE_EVENT_PRIMARY_PATH_SIZE];
};
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(map_flags, BPF_F_NO_PREALLOC);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_file_open_observation);
} pending_file_open_observations SEC(".maps");
struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_pending_file_open_observation);
} file_open_scratch SEC(".maps");

#define ACTRAIL_CAPTURE_FILE_PATH 1U
#define ACTRAIL_CAPTURE_MCP_STDIO 2U
#define ACTRAIL_CAPTURE_FILE_WRITABLE_OPEN (1U << 2)
#define ACTRAIL_CAPTURE_FILE_PATH_MUTATIONS (1U << 3)
#define ACTRAIL_CAPTURE_FILE_FD_MUTATIONS (1U << 4)
#define ACTRAIL_CAPTURE_FILE_READ (1U << 5)
#define ACTRAIL_CAPTURE_FILE_WRITE (1U << 6)
#define ACTRAIL_CAPTURE_FILE_ENUMERATE (1U << 7)

static __always_inline __u32 file_capture_flags(void) {
    __u32 key = 0;
    struct actrail_file_config *config = bpf_map_lookup_elem(&file_config, &key);

    return config ? config->capture_flags : 0;
}

static __always_inline int file_path_capture_enabled(void) {
    return file_capture_flags() & ACTRAIL_CAPTURE_FILE_PATH;
}

static __always_inline int file_context_capture_enabled(__u32 syscall_id) {
    __u32 flags = file_capture_flags();

    switch (syscall_id) {
    case ACTRAIL_FILE_SYSCALL_IOCTL_CLOEXEC:
    case ACTRAIL_FILE_SYSCALL_IOCTL_NCLOEXEC:
        return flags & ACTRAIL_CAPTURE_MCP_STDIO;
    case ACTRAIL_FILE_SYSCALL_OPEN:
    case ACTRAIL_FILE_SYSCALL_OPENAT:
    case ACTRAIL_FILE_SYSCALL_OPENAT2:
    case ACTRAIL_FILE_SYSCALL_CREAT:
    case ACTRAIL_FILE_SYSCALL_CLOSE:
    case ACTRAIL_FILE_SYSCALL_CLOSE_RANGE:
    case ACTRAIL_FILE_SYSCALL_DUP:
    case ACTRAIL_FILE_SYSCALL_DUP2:
    case ACTRAIL_FILE_SYSCALL_DUP3:
    case ACTRAIL_FILE_SYSCALL_FCNTL:
        return flags != 0;
    case ACTRAIL_FILE_SYSCALL_CHDIR:
    case ACTRAIL_FILE_SYSCALL_FCHDIR:
        return flags & ACTRAIL_CAPTURE_FILE_PATH;
    case ACTRAIL_FILE_SYSCALL_MMAP:
    case ACTRAIL_FILE_SYSCALL_FTRUNCATE:
        return flags & ACTRAIL_CAPTURE_FILE_FD_MUTATIONS;
    default:
        return flags & ACTRAIL_CAPTURE_FILE_PATH_MUTATIONS;
    }
}

#endif
