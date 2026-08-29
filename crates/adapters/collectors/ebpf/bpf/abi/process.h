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
} __attribute__((packed));

struct actrail_process_exec_event {
    struct actrail_event_header header;
    __u32 filename_size;
    __u32 filename_flags;
    char filename[ACTRAIL_EXEC_FILENAME_ABI_MAX_BYTES];
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
