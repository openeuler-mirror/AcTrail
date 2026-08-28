#ifndef ACTRAIL_COMMON_KERNEL_TYPES_H
#define ACTRAIL_COMMON_KERNEL_TYPES_H

#include "helpers.h"

struct actrail_atomic_counter {
    int counter;
} __attribute__((preserve_access_index));

struct signal_struct {
    struct actrail_atomic_counter live;
} __attribute__((preserve_access_index));

struct file;

struct fdtable {
    unsigned int max_fds;
    struct file **fd;
    unsigned long *close_on_exec;
} __attribute__((preserve_access_index));

struct files_struct {
    struct fdtable *fdt;
} __attribute__((preserve_access_index));

struct task_struct {
    int pid;
    int tgid;
    __u64 start_boottime;
    struct task_struct *group_leader;
    struct pid *thread_pid;
    struct signal_struct *signal;
    struct files_struct *files;
} __attribute__((preserve_access_index));

struct bpf_iter_meta;

struct bpf_iter__task {
    struct bpf_iter_meta *meta;
    struct task_struct *task;
} __attribute__((preserve_access_index));

struct ns_common {
    unsigned int inum;
} __attribute__((preserve_access_index));

struct pid_namespace {
    struct ns_common ns;
} __attribute__((preserve_access_index));

struct upid {
    int nr;
    struct pid_namespace *ns;
} __attribute__((preserve_access_index));

struct pid {
    unsigned int level;
    struct upid numbers[1];
} __attribute__((preserve_access_index));

struct tracepoint_common {
    __u16 common_type;
    __u8 common_flags;
    __u8 common_preempt_count;
    __s32 common_pid;
};

struct sched_process_fork_ctx {
    struct tracepoint_common common;
    char parent_comm[16];
    __s32 parent_pid;
    char child_comm[16];
    __s32 child_pid;
};

struct sched_process_exec_ctx {
    struct tracepoint_common common;
    __u32 filename_loc;
    __s32 pid;
    __s32 old_pid;
};

struct sched_process_exit_ctx {
    struct tracepoint_common common;
    char comm[16];
    __s32 pid;
    __s32 prio;
};

struct signal_generate_ctx {
    struct tracepoint_common common;
    __s32 sig;
    __s32 error;
    __s32 code;
    char comm[16];
    __s32 pid;
    __s32 group;
    __s32 signal_result;
};

struct trace_event_raw_sys_enter {
    struct tracepoint_common common;
    long id;
    unsigned long args[6];
};

struct trace_event_raw_sys_exit {
    struct tracepoint_common common;
    long id;
    long ret;
};

struct actrail_sockaddr_storage {
    __u16 family;
    __u8 data[126];
};

#endif
