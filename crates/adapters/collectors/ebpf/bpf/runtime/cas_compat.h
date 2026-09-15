#ifndef ACTRAIL_RUNTIME_CAS_COMPAT_H
#define ACTRAIL_RUNTIME_CAS_COMPAT_H

/*
 * Minimal kernel-compatibility layer for once-only u64 operations.
 *
 * Business code keeps its own semantics (level cache, exit claim) and only
 * replaces the raw CAS expression with one of these calls. The backend is
 * selected at compile time by build.rs:
 *   - ACTRAIL_BPF_ONCE_CAS (kernel >= 5.12): native __sync_val_compare_and_swap;
 *   - otherwise: kernel 5.10 tracing cannot use cmpxchg or bpf_spin_lock, so
 *     the same once semantics use atomic BPF_NOEXIST inserts into the maps
 *     below. `native_slot` is ignored in this backend; `slot_key` is ignored
 *     in the native backend.
 */

#include "../common/helpers.h"

#ifndef ACTRAIL_BPF_ONCE_CAS
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} observer_pid_level_cache SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} process_exit_claims SEC(".maps");
#endif

/* Read the current fill-once cell (0 when empty). */
static __always_inline __u64 actrail_once_u64_get(
    __u64 *native_slot,
    __u32 slot_key
) {
#ifdef ACTRAIL_BPF_ONCE_CAS
    (void)slot_key;
    return *native_slot;
#else
    __u64 *cached = bpf_map_lookup_elem(&observer_pid_level_cache, &slot_key);

    (void)native_slot;
    return cached ? *cached : 0;
#endif
}

/* Fill an empty cell once; loses are ignored by design. */
static __always_inline void actrail_once_u64_fill(
    __u64 *native_slot,
    __u32 slot_key,
    __u64 value
) {
#ifdef ACTRAIL_BPF_ONCE_CAS
    (void)slot_key;
    __sync_val_compare_and_swap(native_slot, 0, value);
#else
    (void)native_slot;
    bpf_map_update_elem(
        &observer_pid_level_cache,
        &slot_key,
        &value,
        BPF_NOEXIST
    );
#endif
}

/*
 * Claim a cell exactly once. Native kernels CAS `expected` -> `desired`;
 * portable kernels win the BPF_NOEXIST insert for `slot_key`.
 */
static __always_inline int actrail_once_u64_claim(
    __u64 *native_slot,
    __u32 slot_key,
    __u64 expected,
    __u64 desired
) {
#ifdef ACTRAIL_BPF_ONCE_CAS
    (void)slot_key;
    return __sync_val_compare_and_swap(native_slot, expected, desired) ==
        expected;
#else
    __u64 marker = 0;

    (void)native_slot;
    (void)expected;
    (void)desired;
    return bpf_map_update_elem(
        &process_exit_claims,
        &slot_key,
        &marker,
        BPF_NOEXIST
    ) == 0;
#endif
}

/* Release a claim; native kernels store the claim inside the value entry. */
static __always_inline void actrail_once_u64_release(__u32 slot_key) {
#ifdef ACTRAIL_BPF_ONCE_CAS
    (void)slot_key;
#else
    bpf_map_delete_elem(&process_exit_claims, &slot_key);
#endif
}

#endif
