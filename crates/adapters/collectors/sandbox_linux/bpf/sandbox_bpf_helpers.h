#ifndef ACTRAIL_SANDBOX_BPF_HELPERS_H
#define ACTRAIL_SANDBOX_BPF_HELPERS_H

#include <linux/bpf.h>
#include <linux/types.h>

#define SEC(NAME) __attribute__((section(NAME), used))
#define __uint(name, value) int (*name)[value]
#define __type(name, value) value *name

#ifndef BPF_ANY
#define BPF_ANY 0
#endif

#ifndef BPF_NOEXIST
#define BPF_NOEXIST 1
#endif

static void *(*bpf_map_lookup_elem)(void *map, const void *key) =
    (void *)BPF_FUNC_map_lookup_elem;
static long (*bpf_map_update_elem)(void *map, const void *key, const void *value, __u64 flags) =
    (void *)BPF_FUNC_map_update_elem;
static long (*bpf_map_delete_elem)(void *map, const void *key) =
    (void *)BPF_FUNC_map_delete_elem;
static __u64 (*bpf_get_current_pid_tgid)(void) =
    (void *)BPF_FUNC_get_current_pid_tgid;

struct tracepoint_common {
    __u16 common_type;
    __u8 common_flags;
    __u8 common_preempt_count;
    __s32 common_pid;
};

struct syscall_enter_ctx {
    struct tracepoint_common common;
    __s64 syscall_nr;
    __u64 args[6];
};

struct syscall_exit_ctx {
    struct tracepoint_common common;
    __s64 syscall_nr;
    __s64 result;
};

struct sched_process_fork_ctx {
    struct tracepoint_common common;
    char parent_comm[16];
    __s32 parent_pid;
    char child_comm[16];
    __s32 child_pid;
};

struct sched_process_exit_ctx {
    struct tracepoint_common common;
    char comm[16];
    __s32 pid;
    __s32 prio;
};

#endif
