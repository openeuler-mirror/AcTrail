#ifndef TLS_PAYLOAD_PROBE_HELPERS_H
#define TLS_PAYLOAD_PROBE_HELPERS_H

#include <linux/bpf.h>
#include <linux/types.h>

#define SEC(NAME) __attribute__((section(NAME), used))
#define __uint(name, val) int (*name)[val]
#define __type(name, val) val *name
#define tls_probe_barrier_var(var) asm volatile("" : "+r"(var))

#define TLS_PROBE_BPF_FUNC_PROBE_READ_USER 112
#define TLS_PROBE_BPF_FUNC_RINGBUF_RESERVE 131
#define TLS_PROBE_BPF_FUNC_RINGBUF_SUBMIT 132
#define TLS_PROBE_BPF_FUNC_RINGBUF_DISCARD 133
#define TLS_PROBE_BPF_MAP_TYPE_RINGBUF 27

static void *(*bpf_map_lookup_elem)(void *map, const void *key) =
    (void *)BPF_FUNC_map_lookup_elem;
static long (*bpf_map_update_elem)(void *map, const void *key, const void *value, __u64 flags) =
    (void *)BPF_FUNC_map_update_elem;
static long (*bpf_map_delete_elem)(void *map, const void *key) =
    (void *)BPF_FUNC_map_delete_elem;
static __u64 (*bpf_get_current_pid_tgid)(void) =
    (void *)BPF_FUNC_get_current_pid_tgid;
static __u64 (*bpf_ktime_get_ns)(void) = (void *)BPF_FUNC_ktime_get_ns;
#ifndef TLS_PROBE_EVENT_TRANSPORT_PERF
static void *(*bpf_ringbuf_reserve)(void *ringbuf, __u64 size, __u64 flags) =
    (void *)TLS_PROBE_BPF_FUNC_RINGBUF_RESERVE;
static void (*bpf_ringbuf_submit)(void *data, __u64 flags) =
    (void *)TLS_PROBE_BPF_FUNC_RINGBUF_SUBMIT;
static void (*bpf_ringbuf_discard)(void *data, __u64 flags) =
    (void *)TLS_PROBE_BPF_FUNC_RINGBUF_DISCARD;
#else
static long (*bpf_perf_event_output)(void *ctx, void *map, __u64 flags, void *data, __u64 size) =
    (void *)BPF_FUNC_perf_event_output;
#endif
static long (*bpf_probe_read_user)(void *dst, __u32 size, const void *unsafe_ptr) =
    (void *)TLS_PROBE_BPF_FUNC_PROBE_READ_USER;

#ifndef BPF_ANY
#define BPF_ANY 0
#endif

#ifndef BPF_F_CURRENT_CPU
#define BPF_F_CURRENT_CPU 0xffffffffULL
#endif

#if defined(__TARGET_ARCH_x86)
struct pt_regs {
    unsigned long r15;
    unsigned long r14;
    unsigned long r13;
    unsigned long r12;
    unsigned long bp;
    unsigned long bx;
    unsigned long r11;
    unsigned long r10;
    unsigned long r9;
    unsigned long r8;
    unsigned long ax;
    unsigned long cx;
    unsigned long dx;
    unsigned long si;
    unsigned long di;
    unsigned long orig_ax;
    unsigned long ip;
    unsigned long cs;
    unsigned long flags;
    unsigned long sp;
    unsigned long ss;
};

#define TLS_PROBE_ARG1(ctx) ((ctx)->di)
#define TLS_PROBE_ARG2(ctx) ((ctx)->si)
#define TLS_PROBE_ARG3(ctx) ((ctx)->dx)
#define TLS_PROBE_ARG4(ctx) ((ctx)->cx)
#define TLS_PROBE_RET(ctx) ((ctx)->ax)
#elif defined(__TARGET_ARCH_arm64)
struct pt_regs {
    unsigned long regs[31];
    unsigned long sp;
    unsigned long pc;
    unsigned long pstate;
};

#define TLS_PROBE_ARG1(ctx) ((ctx)->regs[0])
#define TLS_PROBE_ARG2(ctx) ((ctx)->regs[1])
#define TLS_PROBE_ARG3(ctx) ((ctx)->regs[2])
#define TLS_PROBE_ARG4(ctx) ((ctx)->regs[3])
#define TLS_PROBE_RET(ctx) ((ctx)->regs[0])
#else
#error "unsupported BPF target architecture for tls-payload-probe uprobes"
#endif

static __always_inline __u64 tls_probe_positive_i32_size(unsigned long raw) {
    __s32 value = (__s32)raw;

    if (value <= 0) {
        return 0;
    }
    return (__u64)(__u32)value;
}

static __always_inline __u64 tls_probe_positive_isize_size(unsigned long raw) {
    long value = (long)raw;

    if (value <= 0) {
        return 0;
    }
    return (__u64)value;
}

#endif
