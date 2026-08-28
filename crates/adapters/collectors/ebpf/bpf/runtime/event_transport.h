#ifndef ACTRAIL_RUNTIME_EVENT_TRANSPORT_H
#define ACTRAIL_RUNTIME_EVENT_TRANSPORT_H

#include "../abi/fd_io.h"
#include "../abi/process.h"
#include "process_generation.h"

#ifdef ACTRAIL_EVENT_TRANSPORT_PERF
enum actrail_event_transport_scratch {
    ACTRAIL_EVENT_TRANSPORT_SCRATCH_BYTES = 8192,
};

struct actrail_event_scratch {
    __u64 size;
    __u8 bytes[ACTRAIL_EVENT_TRANSPORT_SCRATCH_BYTES];
};
#endif

enum actrail_event_transport_diagnostic_counter {
    ACTRAIL_EVENT_TRANSPORT_RESERVE_FAIL = 0,
    ACTRAIL_EVENT_TRANSPORT_OUTPUT_FAIL = 1,
    ACTRAIL_EVENT_TRANSPORT_OUTPUT_FAIL_BYTES = 2,
    ACTRAIL_FORK_IDENTITY_PUBLISH_FAIL = 3,
    ACTRAIL_STDIO_PENDING_UPDATE_FAIL = 4,
    ACTRAIL_STDIO_READ_USER_FAIL = 5,
    ACTRAIL_SOCKET_STATE_UPDATE_FAIL = 6,
    ACTRAIL_SOCKET_SEQUENCE_UPDATE_FAIL = 7,
    ACTRAIL_PROCESS_IDENTITY_CACHE_MISS = 8,
    ACTRAIL_PROCESS_IDENTITY_CLEANUP_FAIL = 9,
    ACTRAIL_EVENT_TRANSPORT_DIAG_COUNTER_COUNT = 10,
};

#ifdef ACTRAIL_EVENT_TRANSPORT_PERF
struct {
    __uint(type, BPF_MAP_TYPE_PERF_EVENT_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u32);
} events SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_event_scratch);
} event_scratch SEC(".maps");
#else
struct {
    __uint(type, ACTRAIL_BPF_MAP_TYPE_RINGBUF);
    __uint(max_entries, 1);
} events SEC(".maps");
#endif

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, ACTRAIL_EVENT_TRANSPORT_DIAG_COUNTER_COUNT);
    __type(key, __u32);
    __type(value, __u64);
} event_transport_diagnostics SEC(".maps");

static __always_inline void event_transport_diag_inc(__u32 counter_id) {
    __u64 *counter = bpf_map_lookup_elem(&event_transport_diagnostics, &counter_id);

    if (!counter) {
        return;
    }
    __sync_fetch_and_add(counter, 1);
}

static __always_inline void event_transport_diag_add(__u32 counter_id, __u64 value) {
    __u64 *counter = bpf_map_lookup_elem(&event_transport_diagnostics, &counter_id);

    if (!counter) {
        return;
    }
    __sync_fetch_and_add(counter, value);
}

static __always_inline void *actrail_event_reserve(__u64 size) {
#ifdef ACTRAIL_EVENT_TRANSPORT_PERF
    __u32 key = 0;
    struct actrail_event_scratch *scratch;

    if (size > ACTRAIL_EVENT_TRANSPORT_SCRATCH_BYTES) {
        event_transport_diag_inc(ACTRAIL_EVENT_TRANSPORT_RESERVE_FAIL);
        return 0;
    }
    scratch = bpf_map_lookup_elem(&event_scratch, &key);
    if (!scratch) {
        event_transport_diag_inc(ACTRAIL_EVENT_TRANSPORT_RESERVE_FAIL);
        return 0;
    }
    scratch->size = size;
    return scratch->bytes;
#else
    void *event = bpf_ringbuf_reserve(&events, size, 0);

    if (!event) {
        event_transport_diag_inc(ACTRAIL_EVENT_TRANSPORT_RESERVE_FAIL);
    }
    return event;
#endif
}

static __always_inline void actrail_event_discard(void *event) {
#ifndef ACTRAIL_EVENT_TRANSPORT_PERF
    bpf_ringbuf_discard(event, 0);
#endif
}

static __always_inline int actrail_event_submit(void *ctx, void *event) {
#ifdef ACTRAIL_EVENT_TRANSPORT_PERF
    __u32 key = 0;
    struct actrail_event_scratch *scratch = bpf_map_lookup_elem(&event_scratch, &key);
    long result;

    if (!scratch || scratch->size > ACTRAIL_EVENT_TRANSPORT_SCRATCH_BYTES) {
        event_transport_diag_inc(ACTRAIL_EVENT_TRANSPORT_OUTPUT_FAIL);
        return -1;
    }
    result = bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, event, scratch->size);
    if (result != 0) {
        event_transport_diag_inc(ACTRAIL_EVENT_TRANSPORT_OUTPUT_FAIL);
        event_transport_diag_add(ACTRAIL_EVENT_TRANSPORT_OUTPUT_FAIL_BYTES, scratch->size);
    }
    return result;
#else
    bpf_ringbuf_submit(event, 0);
    return 0;
#endif
}

static __always_inline int emit_event(void *ctx, void *event, __u64 size) {
#ifdef ACTRAIL_EVENT_TRANSPORT_PERF
    long result = bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, event, size);
#else
    long result = bpf_ringbuf_output(&events, event, size, 0);
#endif

    if (result != 0) {
        event_transport_diag_inc(ACTRAIL_EVENT_TRANSPORT_OUTPUT_FAIL);
        event_transport_diag_add(ACTRAIL_EVENT_TRANSPORT_OUTPUT_FAIL_BYTES, size);
    }
    return result;
}

static __always_inline void init_event_header(
    struct actrail_event_header *header,
    __u32 kind,
    __u16 record_size,
    __u64 trace_id,
    __u32 observer_namespace_tgid,
    __u32 kernel_tgid,
    __u64 start_boottime_ns
) {
    header->kind = kind;
    header->abi_revision = ACTRAIL_EVENT_ABI_REVISION;
    header->record_size = record_size;
    header->trace_id = trace_id;
    header->observed_ktime_ns = bpf_ktime_get_ns();
    header->subject_observer_namespace_tgid = observer_namespace_tgid;
    header->subject_kernel_tgid = kernel_tgid;
    header->subject_start_boottime_ns = start_boottime_ns;
}

static __always_inline void init_current_event_header(
    struct actrail_event_header *header,
    __u32 kind,
    __u16 record_size,
    __u64 trace_id
) {
    __u32 kernel_tgid = current_kernel_tgid();
    struct actrail_process_identity *identity =
        lookup_process_identity(kernel_tgid);

    if (!identity) {
        event_transport_diag_inc(ACTRAIL_PROCESS_IDENTITY_CACHE_MISS);
    }

    init_event_header(
        header,
        kind,
        record_size,
        trace_id,
        identity ? identity->observer_namespace_tgid : 0,
        kernel_tgid,
        identity ? identity->start_boottime_ns : 0
    );
}

#endif
