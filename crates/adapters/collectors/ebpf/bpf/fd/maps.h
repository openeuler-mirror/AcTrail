#ifndef ACTRAIL_FD_MAPS_H
#define ACTRAIL_FD_MAPS_H

#include "types.h"

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_fd_key);
    __type(value, struct actrail_fd_state);
} fd_table SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_fd_object_key);
    __type(value, struct actrail_fd_object_state);
} fd_objects SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_fd_index_slot_key);
    __type(value, struct actrail_fd_index_slot);
} fd_index_slots SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u32);
} fd_process_active_counts SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_fd_open_op);
} pending_fd_open_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_fd_close_op);
} pending_fd_close_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_fd_dup_op);
} pending_fd_dup_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_fd_flag_op);
} pending_fd_flag_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_ipc_fd_pair_op);
} pending_ipc_fd_pair_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_fd_config);
} fd_category_config SEC(".maps");

#endif
