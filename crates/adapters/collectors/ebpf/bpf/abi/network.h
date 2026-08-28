#ifndef ACTRAIL_ABI_NETWORK_H
#define ACTRAIL_ABI_NETWORK_H

#include "observation.h"

enum actrail_syscall_family {
    ACTRAIL_SYSCALL_FAMILY_SOCKET = 1,
    ACTRAIL_SYSCALL_FAMILY_FD_IO = 2,
    ACTRAIL_SYSCALL_FAMILY_FD_IO_WRITEV = 3,
};

enum actrail_syscall_arg_slot {
    ACTRAIL_SYSCALL_ARG_MISSING = 6,
};

struct actrail_network_event {
    struct actrail_event_header header;
    __s32 syscall_result;
    __u32 fd;
    __u32 syscall_family;
    __u32 operation_flags;
    __u64 fd_object_generation;
    __u32 endpoint_role;
    struct actrail_endpoint endpoint;
} __attribute__((packed));

#endif
