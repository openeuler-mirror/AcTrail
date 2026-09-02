#ifndef ACTRAIL_RUNTIME_FETCH_ADD_COMPAT_H
#define ACTRAIL_RUNTIME_FETCH_ADD_COMPAT_H

/*
 * Compile-time fetch-and-add compatibility.
 *
 * When the target kernel supports BPF_ATOMIC (>= 5.12, selected by build.rs
 * through ACTRAIL_BPF_ONCE_CAS), native __sync_fetch_and_add is used. Older
 * kernels only provide XADD without a return value, so the portable backend
 * performs a plain read-modify-write. That is safe here because every caller
 * owns a per-thread counter (keyed kernel_pid_tgid): a single thread cannot
 * execute two syscalls concurrently.
 */

#include "../common/helpers.h"

/* Return the next value of a per-thread counter (old value + 1). */
static __always_inline __u64 actrail_fetch_add_next(__u64 *counter) {
#ifdef ACTRAIL_BPF_ONCE_CAS
    return __sync_fetch_and_add(counter, 1) + 1;
#else
    __u64 next = *counter + 1;

    *counter = next;
    return next;
#endif
}

/* Build a process-unique id from the thread and its per-thread counter. */
static __always_inline __u64 actrail_thread_sequence_id(
    __u64 kernel_pid_tgid,
    __u64 thread_count
) {
    return ((__u64)(__u32)kernel_pid_tgid << 32) | thread_count;
}

#endif
