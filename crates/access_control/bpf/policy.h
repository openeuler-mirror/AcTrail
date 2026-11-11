/* Read immutable policy snapshots and emit access decisions. */
#ifndef ACTRAIL_ACCESS_CONTROL_POLICY_H
#define ACTRAIL_ACCESS_CONTROL_POLICY_H

#include "scope.h"

struct linux_binprm;

struct ac_context {
    struct ac_work *work;
    void *policy;
    const struct ac_policy_value *selected;
    __u64 trace;
    __u32 domain;
    int resource_length;
    int can_wait;
};

/* Prebuilt default keys avoid preparing a resource buffer for domain lookup.
 * Callers use only the fixed operation domains below. */
static const struct ac_policy_key ac_default_keys[15] = {
    [1] = { .prefixlen = 32, .domain = 1 },
    [2] = { .prefixlen = 32, .domain = 2 },
    [3] = { .prefixlen = 32, .domain = 3 },
    [4] = { .prefixlen = 32, .domain = 4 },
    [5] = { .prefixlen = 32, .domain = 5 },
    [6] = { .prefixlen = 32, .domain = 6 },
    [7] = { .prefixlen = 32, .domain = 7 },
    [8] = { .prefixlen = 32, .domain = 8 },
    [9] = { .prefixlen = 32, .domain = 9 },
    [10] = { .prefixlen = 32, .domain = 10 },
    [11] = { .prefixlen = 32, .domain = 11 },
    [12] = { .prefixlen = 32, .domain = 12 },
    [13] = { .prefixlen = 32, .domain = 13 },
    [14] = { .prefixlen = 32, .domain = 14 },
};

/* The immutable domain default includes both rules and audit requirements. */
static __always_inline int ac_needs_resource(struct ac_context *context) {
    return !!(context->selected->flags & AC_RESOURCE_REQUIRED);
}

#ifdef AC_GREY
extern int actrail_lsm_decide(void *data, __u32 data__sz)
    __attribute__((section(".ksyms")));
extern int actrail_lsm_exec_decide(struct linux_binprm *bprm, void *data, __u32 data__sz)
    __attribute__((section(".ksyms")));
#define AC_RESOURCE_HOOK(name) SEC("lsm.s/" name)
#else
#define AC_RESOURCE_HOOK(name) SEC("lsm/" name)
#endif

static __always_inline void ac_count(__u32 index) {
    __u64 *count = bpf_map_lookup_elem(&ac_counters, &index);
    if (count) (*count)++;
}

/* Select another operation without resolving the same file path again. */
static __always_inline int ac_select_domain(__u32 domain, struct ac_context *context) {
    context->domain = domain;
    struct ac_policy_value *fallback = bpf_map_lookup_elem(
        context->policy, &ac_default_keys[domain]);
    if (!fallback) return -13;
    /* Scope enrollment also runs in its own exec hook. Keep lookup idempotent
     * here: file hooks can run before exec enrollment during a launch. */
    if (!(fallback->flags & AC_ENABLED)) return 0;
    context->selected = fallback;
    return 1;
}

/* Repeated checks in one hook retain the same scope and policy snapshot. */
static __always_inline int ac_begin(__u32 domain, struct ac_context *context) {
    if (!context->policy) {
        __u32 zero = 0;
        int scoped = ac_current_trace(&context->trace);
        if (scoped <= 0) return scoped;
        context->policy = bpf_map_lookup_elem(&ac_policy_slots, &zero);
        if (!context->policy) return -13;
        context->can_wait = 1;
    }
    return ac_select_domain(domain, context);
}

static __always_inline int ac_prepare_resource(struct ac_context *context) {
    if (!ac_needs_resource(context)) return 0;
    if (!context->work) {
#ifdef AC_GREY
        context->work = (void *)ac_task_storage_get(
            &ac_scratch, ac_current_task_btf(), 0, 1);
#else
        __u32 zero = 0;
        context->work = bpf_map_lookup_elem(&ac_scratch, &zero);
#endif
        if (!context->work) return -13;
    }
    context->work->key.domain = context->domain;
    return 1;
}

static __always_inline void ac_candidate(
    struct ac_context *context, __u32 bytes,
    struct ac_policy_value **best, __u32 *rank
) {
    if (bytes > AC_RESOURCE_BYTES) return;
    context->work->key.prefixlen = 32 + bytes * 8;
    struct ac_policy_value *candidate =
        bpf_map_lookup_elem(context->policy, &context->work->key);
    if (candidate && candidate->rank < *rank) {
        *best = candidate;
        *rank = candidate->rank;
    }
}

static __attribute__((noinline)) int ac_path_candidates(struct ac_context *context, long length) {
    /* Keep the resolver result; emission excludes errors and oversized lengths. */
    context->resource_length = length;
    if (length <= 0) return length ? length : -22;
    if (length > AC_RESOURCE_BYTES ||
        length > context->selected->path_max_bytes) return -36;
    __u32 recursive = context->selected->flags & AC_RECURSIVE;
    struct ac_policy_value *best = 0;
    __u32 rank = context->selected->rank;
    ac_candidate(context, length, &best, &rank);
    /* Every slash boundary is queried: LPM alone orders by path length,
     * whereas access policy precedence is defined by the rule rank. */
    for (__u32 i = 0; i < AC_RESOURCE_BYTES; i++) {
        if (!recursive || i >= length - 1) break;
        if (context->work->key.resource[i] == '/')
            ac_candidate(context, i + 1, &best, &rank);
    }
    /* Publication freezes each inner map. The BPF invocation retains its
     * RCU lifetime, including tasks-trace RCU for sleepable hooks. */
    if (best) context->selected = best;
    return 0;
}

static __always_inline void ac_fill_event(
    struct ac_context *context, struct ac_audit_event *event,
    int decision, int error, int signal_result
) {
    struct task_struct *task = actrail_bpf_get_current_task();
    struct task_struct *leader = 0;
    __u64 started = 0;
    if (!ACTRAIL_CORE_READ(&leader, task, group_leader) && leader)
        ACTRAIL_CORE_READ(&started, leader, start_boottime);
    event->revision = context->selected->revision;
    event->trace_id = context->trace;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->start_boottime_ns = started;
    event->kernel_tgid = bpf_get_current_pid_tgid() >> 32;
    event->observer_tgid = ac_observer_tgid(task);
    event->domain = context->domain;
    event->decision = decision;
    event->error = error;
    event->signal_result = signal_result;
    event->retain_audit = error || (context->selected->flags & AC_AUDIT);
    __builtin_memcpy(event->rule_id, context->selected->rule_id, AC_ID_BYTES);
    __builtin_memcpy(event->owner, context->selected->owner, AC_ID_BYTES);
    /* The wire record has fixed capacity. Initialize it only when emitting,
     * then copy the valid bytes from the reusable matching buffer. */
    __builtin_memset(event->resource, 0, AC_AUDIT_RESOURCE_BYTES);
    __u32 length = context->resource_length;
    if (length <= AC_RESOURCE_BYTES)
        bpf_probe_read_kernel(event->resource, length, context->work->key.resource);
}

static __always_inline void ac_emit(
    struct ac_context *context, int decision, int error, int signal_result
) {
    struct ac_audit_event *event = bpf_ringbuf_reserve(&ac_events, sizeof(*event), 0);
    if (!event) {
        ac_count(0);
        return;
    }
    ac_fill_event(context, event, decision, error, signal_result);
    bpf_ringbuf_submit(event, 0);
}

static __always_inline int ac_grey(
    struct ac_context *context, int *error, struct linux_binprm *bprm
) {
    int fallback = !!context->selected->fallback_deny;
#ifdef AC_GREY
    if (context->can_wait) {
        struct actrail_lsm_grey_request *request = &context->work->request;
        request->request_id = 0;
        request->deadline_boottime_ns = 0;
        request->timeout_ms = context->selected->timeout_ms;
        request->concurrency_limit = context->selected->concurrency_limit;
        request->fallback_deny = fallback;
        request->reserved = 0;
        __builtin_memcpy(request->target, context->selected->target, AC_ID_BYTES);
        ac_fill_event(context, &request->event, 2, 0, 0);
        int result = bprm ? actrail_lsm_exec_decide(bprm, request, sizeof(*request))
                          : actrail_lsm_decide(request, sizeof(*request));
        if (result == 0 || result == 1) return result;
        *error = result < 0 ? result : -22;
        ac_count(1);
        if (result == AC_GREY_CONTEXT_INVALID) return 1;
    }
#endif
    return fallback;
}

static __always_inline int ac_finish_exec(
    struct ac_context *context, int error, struct linux_binprm *bprm
) {
    __u32 decision = context->selected->decision;
    int signal_result = 0;
    if (error) {
        ac_count(1);
        decision = !!(context->selected->flags & AC_DENY_ERROR);
    } else if (decision == 2) {
        decision = ac_grey(context, &error, bprm);
    }
    if (decision && context->selected->signal) {
        signal_result = bpf_send_signal(context->selected->signal);
        if (signal_result) ac_count(2);
    }
    int boundary_denial = decision && !error &&
        context->domain <= 14 && context->selected->rule_id[0];
    if (error || (context->selected->flags & AC_AUDIT) || boundary_denial)
        ac_emit(context, decision, error, signal_result);
    return decision ? -13 : 0;
}

static __always_inline int ac_finish(struct ac_context *context, int error) {
    return ac_finish_exec(context, error, 0);
}

#endif
