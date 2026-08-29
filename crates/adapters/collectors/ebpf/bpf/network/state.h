#ifndef ACTRAIL_NETWORK_STATE_H
#define ACTRAIL_NETWORK_STATE_H

#include "../abi/network.h"

struct actrail_pending_net_op {
    __u64 trace_id;
    __u64 requested_size;
    __u64 sockaddr_ptr;
    struct actrail_endpoint remote;
    __u32 pid;
    __u32 kind;
    __u32 fd;
    __u32 syscall_family;
    __u32 flags;
    __u32 category;
    __u64 generation;
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_net_op);
} pending_net_ops SEC(".maps");

#endif
