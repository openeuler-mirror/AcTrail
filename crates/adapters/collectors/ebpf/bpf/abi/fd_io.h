#ifndef ACTRAIL_ABI_FD_IO_H
#define ACTRAIL_ABI_FD_IO_H

#include "network.h"

struct actrail_fd_io_event {
    struct actrail_event_header header;
    __s32 syscall_result;
    __u32 fd;
    __u32 syscall_family;
    __u32 fd_category;
    __u64 requested_size;
    __u64 fd_object_generation;
    __u32 endpoint_role;
    struct actrail_endpoint endpoint;
} __attribute__((packed));

struct actrail_socket_release_event {
    struct actrail_event_header header;
    __u32 fd;
    __u64 fd_object_generation;
} __attribute__((packed));

#endif
