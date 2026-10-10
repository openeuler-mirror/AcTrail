#ifndef ACTRAIL_ACCESS_CONTROL_POLICY_H
#define ACTRAIL_ACCESS_CONTROL_POLICY_H

#include "scope.h"

struct ac_context {
    struct ac_work *work;
    void *policy;
    __u64 trace;
};

static __always_inline void ac_count(__u32 index) {
    __u64 *count = bpf_map_lookup_elem(&ac_counters, &index);
    if (count) (*count)++;
}

static __always_inline int ac_begin(__u32 domain, struct ac_context *context) {
    __u32 zero = 0;
    int scoped = ac_current_trace(&context->trace);
    if (scoped <= 0) return scoped;
    context->work = bpf_map_lookup_elem(&ac_scratch, &zero);
    context->policy = bpf_map_lookup_elem(&ac_policy_slots, &zero);
    if (!context->work || !context->policy) return -13;
    context->work->key.prefixlen = 32;
    context->work->key.domain = domain;
    __builtin_memset(context->work->key.resource, 0, AC_RESOURCE_BYTES);
    struct ac_policy_value *fallback = bpf_map_lookup_elem(
        context->policy, &context->work->key);
    if (!fallback) return -13;
    /* Exec stays attached to enroll launches before observation consumes the
     * pending binding, including configurations that control only network. */
    if (!(fallback->flags & AC_ENABLED)) return 0;
    __builtin_memcpy(&context->work->selected, fallback, sizeof(*fallback));
    return 1;
}

static __always_inline void ac_candidate(struct ac_context *context, __u32 bytes) {
    if (bytes > AC_RESOURCE_BYTES) return;
    context->work->key.prefixlen = 32 + bytes * 8;
    struct ac_policy_value *candidate =
        bpf_map_lookup_elem(context->policy, &context->work->key);
    if (candidate && candidate->rank < context->work->selected.rank)
        __builtin_memcpy(&context->work->selected, candidate, sizeof(*candidate));
}

static __always_inline int ac_path_candidates(struct ac_context *context, long length) {
    if (length <= 0) return length ? length : -22;
    if (length > AC_RESOURCE_BYTES ||
        length > context->work->selected.path_max_bytes) return -36;
    __u32 recursive = context->work->selected.flags & AC_RECURSIVE;
    ac_candidate(context, length);
    if (!recursive) return 0;
    /* Every slash boundary is queried: LPM alone orders by path length,
     * whereas access policy precedence is defined by the rule rank. */
    for (__u32 i = 0; i < AC_RESOURCE_BYTES; i++) {
        if (i >= length - 1) break;
        if (context->work->key.resource[i] == '/') ac_candidate(context, i + 1);
    }
    return 0;
}

static __always_inline void ac_emit(
    struct ac_context *context, int decision, int error, int signal_result
) {
    struct ac_audit_event *event = bpf_ringbuf_reserve(&ac_events, sizeof(*event), 0);
    if (!event) {
        ac_count(0);
        return;
    }
    struct task_struct *task = actrail_bpf_get_current_task();
    struct task_struct *leader = 0;
    __u64 started = 0;
    if (!ACTRAIL_CORE_READ(&leader, task, group_leader) && leader)
        ACTRAIL_CORE_READ(&started, leader, start_boottime);
    event->revision = context->work->selected.revision;
    event->trace_id = context->trace;
    event->observed_ktime_ns = bpf_ktime_get_ns();
    event->start_boottime_ns = started;
    event->kernel_tgid = bpf_get_current_pid_tgid() >> 32;
    event->observer_tgid = ac_observer_tgid(task);
    event->domain = context->work->key.domain;
    event->decision = decision;
    event->error = error;
    event->signal_result = signal_result;
    event->retain_audit = error || (context->work->selected.flags & AC_AUDIT);
    __builtin_memcpy(event->rule_id, context->work->selected.rule_id, AC_ID_BYTES);
    __builtin_memcpy(event->owner, context->work->selected.owner, AC_ID_BYTES);
    __builtin_memcpy(event->resource, context->work->key.resource, AC_RESOURCE_BYTES);
    __builtin_memset(event->resource + AC_RESOURCE_BYTES, 0,
        AC_AUDIT_RESOURCE_BYTES - AC_RESOURCE_BYTES);
    bpf_ringbuf_submit(event, 0);
}

static __always_inline int ac_finish(struct ac_context *context, int error) {
    __u32 decision = context->work->selected.decision;
    int signal_result = 0;
    if (error) {
        ac_count(1);
        decision = !!(context->work->selected.flags & AC_DENY_ERROR);
    }
    if (decision && context->work->selected.signal) {
        signal_result = bpf_send_signal(context->work->selected.signal);
        if (signal_result) ac_count(2);
    }
    int boundary_denial = decision && !error &&
        context->work->key.domain <= 4 && context->work->selected.rule_id[0];
    if (error || (context->work->selected.flags & AC_AUDIT) || boundary_denial)
        ac_emit(context, decision, error, signal_result);
    return decision ? -13 : 0;
}

#endif
