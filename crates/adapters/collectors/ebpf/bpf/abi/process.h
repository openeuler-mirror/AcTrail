#ifndef ACTRAIL_ABI_PROCESS_H
#define ACTRAIL_ABI_PROCESS_H

#include "observation.h"

enum actrail_process_exit_flag {
    ACTRAIL_PROCESS_EXIT_CODE_VALID = 1,
};

struct actrail_process_fork_event {
    struct actrail_event_header header;
    __u32 parent_observer_namespace_tgid;
    __u32 parent_kernel_tgid;
    __u64 parent_start_boottime_ns;
    __u64 attempt_id;
} __attribute__((packed));

struct actrail_exec_file_identity {
    __u32 valid;
    __u32 device_major;
    __u32 device_minor;
    __u64 inode;
    __u64 size;
    __s64 mtime_seconds;
    __s64 ctime_seconds;
    __u32 mtime_nanoseconds;
    __u32 ctime_nanoseconds;
} __attribute__((packed));

struct actrail_process_exec_event {
    struct actrail_event_header header;
    __u64 attempt_id;
    __u32 filename_size;
    __u32 filename_flags;
    char filename[ACTRAIL_EXEC_FILENAME_ABI_MAX_BYTES];
    struct actrail_exec_file_identity file_identity;
} __attribute__((packed));

struct actrail_tls_mapping_event {
    struct actrail_event_header header;
    struct actrail_exec_file_identity file_identity;
    __u64 start;
    __u64 end;
} __attribute__((packed));

struct actrail_process_exit_event {
    struct actrail_event_header header;
    __s32 exit_code;
    __u32 exit_flags;
} __attribute__((packed));

struct actrail_process_signal_event {
    struct actrail_event_header header;
    __s32 signal_result;
    __u32 signal;
    __u32 target_kernel_tid;
    __u32 target_group;
} __attribute__((packed));

#endif
