/* Fixed-width protocol shared by LSM programs and the grey request device. */
#ifndef ACTRAIL_LSM_GREY_PROTOCOL_H
#define ACTRAIL_LSM_GREY_PROTOCOL_H
#include <linux/types.h>

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
    __u8 rule_id[128];
    __u8 owner[128];
    __u8 resource[512];
    __u64 retain_audit;
};

struct actrail_lsm_grey_request {
    __u64 request_id;
    __u64 deadline_boottime_ns;
    __u32 timeout_ms;
    __u32 concurrency_limit;
    __u32 fallback_deny;
    __u32 reserved;
    __u8 target[128];
    struct ac_audit_event event;
};

struct actrail_lsm_grey_reply {
    __u64 request_id;
    __u32 decision;
    __u32 reserved;
};

/* Device-only command context. The BPF kfunc input remains the fixed request. */
#define AC_EXEC_ARG_COUNT 16
#define AC_EXEC_ARG_BYTES 256
/* Incomplete exec context must deny even when the channel fallback allows. */
#define AC_GREY_CONTEXT_INVALID (-74) /* EBADMSG */
struct actrail_lsm_exec_context {
    __u32 syscall; /* 1: execve, 2: execveat, 0: not a command */
    __u32 argc;
    __s32 dirfd;
    __u64 flags;
    __u8 requested_path[256];
    __u8 argv[AC_EXEC_ARG_COUNT][AC_EXEC_ARG_BYTES];
};

struct actrail_lsm_grey_message {
    struct actrail_lsm_grey_request request;
    struct actrail_lsm_exec_context command;
};

_Static_assert(sizeof(struct ac_audit_event) == 832, "audit ABI");
_Static_assert(sizeof(struct actrail_lsm_grey_request) == 992, "grey request ABI");
_Static_assert(sizeof(struct actrail_lsm_grey_reply) == 16, "grey reply ABI");
_Static_assert(sizeof(struct actrail_lsm_grey_message) == 5368, "grey message ABI");
#endif
