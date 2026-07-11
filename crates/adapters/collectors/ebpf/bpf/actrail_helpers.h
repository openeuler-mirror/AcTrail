#ifndef ACTRAIL_HELPERS_H
#define ACTRAIL_HELPERS_H

#include <linux/bpf.h>
#include <linux/types.h>
#include <linux/socket.h>
#include <linux/in.h>
#include <linux/in6.h>
#include <linux/sched.h>
#include <linux/fcntl.h>

#define SEC(NAME) __attribute__((section(NAME), used))
#define __uint(name, val) int (*name)[val]
#define __type(name, val) val *name
#ifndef __noinline
#define __noinline __attribute__((noinline))
#endif
#define actrail_barrier_var(var) asm volatile("" : "+r"(var))

#define ACTRAIL_BPF_FUNC_SEND_SIGNAL 109
#define ACTRAIL_BPF_FUNC_PROBE_READ_USER 112
#define ACTRAIL_BPF_FUNC_PROBE_READ_KERNEL 113
#define ACTRAIL_BPF_FUNC_PROBE_READ_USER_STR 114
#define ACTRAIL_BPF_FUNC_PROBE_READ_KERNEL_STR 115
#define ACTRAIL_BPF_FUNC_GET_NS_CURRENT_PID_TGID 120
#define ACTRAIL_BPF_FUNC_RINGBUF_OUTPUT 130
#define ACTRAIL_BPF_FUNC_RINGBUF_RESERVE 131
#define ACTRAIL_BPF_FUNC_RINGBUF_SUBMIT 132
#define ACTRAIL_BPF_FUNC_RINGBUF_DISCARD 133
#define ACTRAIL_BPF_MAP_TYPE_RINGBUF 27

struct actrail_bpf_pidns_info {
    __u32 pid;
    __u32 tgid;
};

static void *(*bpf_map_lookup_elem)(void *map, const void *key) = (void *)BPF_FUNC_map_lookup_elem;
static long (*bpf_map_update_elem)(void *map, const void *key, const void *value, __u64 flags) =
    (void *)BPF_FUNC_map_update_elem;
static long (*bpf_map_delete_elem)(void *map, const void *key) = (void *)BPF_FUNC_map_delete_elem;
static __u64 (*bpf_get_current_pid_tgid)(void) = (void *)BPF_FUNC_get_current_pid_tgid;
static long (*bpf_get_current_comm)(void *buf, __u32 size_of_buf) =
    (void *)BPF_FUNC_get_current_comm;
static __u64 (*bpf_ktime_get_ns)(void) = (void *)BPF_FUNC_ktime_get_ns;
#ifndef ACTRAIL_EVENT_TRANSPORT_PERF
static long (*bpf_ringbuf_output)(void *ringbuf, void *data, __u64 size, __u64 flags) =
    (void *)ACTRAIL_BPF_FUNC_RINGBUF_OUTPUT;
static void *(*bpf_ringbuf_reserve)(void *ringbuf, __u64 size, __u64 flags) =
    (void *)ACTRAIL_BPF_FUNC_RINGBUF_RESERVE;
static void (*bpf_ringbuf_submit)(void *data, __u64 flags) =
    (void *)ACTRAIL_BPF_FUNC_RINGBUF_SUBMIT;
static void (*bpf_ringbuf_discard)(void *data, __u64 flags) =
    (void *)ACTRAIL_BPF_FUNC_RINGBUF_DISCARD;
#endif
static long (*bpf_perf_event_output)(void *ctx, void *map, __u64 flags, void *data, __u64 size) =
    (void *)BPF_FUNC_perf_event_output;
static long (*bpf_send_signal)(__u32 sig) = (void *)ACTRAIL_BPF_FUNC_SEND_SIGNAL;
static long (*bpf_probe_read_user)(void *dst, __u32 size, const void *unsafe_ptr) =
    (void *)ACTRAIL_BPF_FUNC_PROBE_READ_USER;
static long (*bpf_probe_read_kernel)(void *dst, __u32 size, const void *unsafe_ptr) =
    (void *)ACTRAIL_BPF_FUNC_PROBE_READ_KERNEL;
static long (*bpf_probe_read_user_str)(void *dst, __u32 size, const void *unsafe_ptr) =
    (void *)ACTRAIL_BPF_FUNC_PROBE_READ_USER_STR;
static long (*bpf_probe_read)(void *dst, __u32 size, const void *unsafe_ptr) =
    (void *)BPF_FUNC_probe_read;
static long (*bpf_probe_read_kernel_str)(void *dst, __u32 size, const void *unsafe_ptr) =
    (void *)ACTRAIL_BPF_FUNC_PROBE_READ_KERNEL_STR;
static long (*bpf_get_ns_current_pid_tgid)(
    __u64 dev,
    __u64 ino,
    struct actrail_bpf_pidns_info *nsdata,
    __u32 size
) = (void *)ACTRAIL_BPF_FUNC_GET_NS_CURRENT_PID_TGID;

#define ACTRAIL_CORE_READ(dst, source, field) \
    bpf_probe_read_kernel( \
        (dst), \
        sizeof(*(dst)), \
        __builtin_preserve_access_index(&(source)->field) \
    )

#ifndef BPF_ANY
#define BPF_ANY 0
#endif

#ifndef BPF_F_CURRENT_CPU
#define BPF_F_CURRENT_CPU 0xffffffffULL
#endif

#ifndef AF_INET
#define AF_INET 2
#endif

#ifndef AF_UNIX
#define AF_UNIX 1
#endif

#ifndef AF_INET6
#define AF_INET6 10
#endif

#endif
