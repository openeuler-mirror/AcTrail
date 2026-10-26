#ifndef ACTRAIL_COMMON_MAP_FLAGS_H
#define ACTRAIL_COMMON_MAP_FLAGS_H

#include <linux/bpf.h>

/*
 * Hash maps referenced by trace-type programs cannot use run-time allocation
 * on kernels before 6.1 (upstream commit 96da3f7d489d lifted the
 * restriction). build.rs selects preallocation on those kernels through
 * -DACTRAIL_BPF_TRACE_HASH_PREALLOC and keeps lazy allocation
 * (BPF_F_NO_PREALLOC) where it is safe.
 */
#ifdef ACTRAIL_BPF_TRACE_HASH_PREALLOC
#define ACTRAIL_TRACE_HASH_MAP_FLAGS 0
#else
#define ACTRAIL_TRACE_HASH_MAP_FLAGS BPF_F_NO_PREALLOC
#endif

#endif
