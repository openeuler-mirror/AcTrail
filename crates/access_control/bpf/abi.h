#ifndef ACTRAIL_ACCESS_CONTROL_ABI_H
#define ACTRAIL_ACCESS_CONTROL_ABI_H

#include "../../adapters/collectors/ebpf/bpf/common/kernel_types.h"

#define AC_RESOURCE_BYTES 252
#define AC_AUDIT_RESOURCE_BYTES 512
#define AC_ID_BYTES 128
#define AC_AUDIT 1
#define AC_RECURSIVE 2
#define AC_DENY_ERROR 4
#define AC_ENABLED 8

struct ac_policy_key {
    __u32 prefixlen;
    __u32 domain;
    __u8 resource[AC_RESOURCE_BYTES];
};

struct ac_policy_value {
    __u32 rank;
    __u32 decision;
    __u32 flags;
    __u32 signal;
    __u32 path_max_bytes;
    __u32 reserved;
    __u64 revision;
    __u8 rule_id[AC_ID_BYTES];
    __u8 owner[AC_ID_BYTES];
};

struct ac_audit_event {
    __u64 revision;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 start_boottime_ns;
    __u32 kernel_tgid;
    __u32 observer_tgid;
    __u32 domain;
    __u32 decision;
    __s32 error;
    __s32 signal_result;
    __u8 rule_id[AC_ID_BYTES];
    __u8 owner[AC_ID_BYTES];
    __u8 resource[AC_AUDIT_RESOURCE_BYTES];
    __u64 retain_audit;
};

struct ac_work {
    struct ac_policy_key key;
    struct ac_policy_value selected;
    /* Extra addressable space permits bounded variable-size probe reads
     * while constructing a path backwards in the first resource window. */
    __u8 name[AC_RESOURCE_BYTES * 2];
};

struct ac_policy_map {
    __uint(type, BPF_MAP_TYPE_LPM_TRIE);
    __uint(map_flags, BPF_F_NO_PREALLOC);
    __uint(max_entries, 1);
    __type(key, struct ac_policy_key);
    __type(value, struct ac_policy_value);
};
struct ac_policy_map ac_policy_template SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY_OF_MAPS);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u32);
    struct ac_policy_map *values[];
} ac_policy_slots SEC(".maps") = {
    .values = { &ac_policy_template },
};

struct {
    __uint(type, BPF_MAP_TYPE_RINGBUF);
    __uint(max_entries, 4096);
} ac_events SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct ac_work);
} ac_scratch SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 3);
    __type(key, __u32);
    __type(value, __u64);
} ac_counters SEC(".maps");

_Static_assert(sizeof(struct ac_policy_key) == 260, "policy key ABI");
_Static_assert(sizeof(struct ac_policy_value) == 288, "policy value ABI");
_Static_assert(sizeof(struct ac_audit_event) == 832, "audit ABI");

#endif
