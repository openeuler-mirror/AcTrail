/* Resolve IP endpoints and transport protocols for network policy. */
#ifndef ACTRAIL_ACCESS_CONTROL_NETWORK_H
#define ACTRAIL_ACCESS_CONTROL_NETWORK_H

#include "policy.h"

struct sock_common {
    __u32 skc_daddr;
    __u32 skc_rcv_saddr;
    __u16 skc_num;
    __u16 skc_dport;
    __u16 skc_family;
    __u8 skc_state;
    struct in6_addr skc_v6_daddr;
    struct in6_addr skc_v6_rcv_saddr;
} __attribute__((preserve_access_index));
struct sock {
    struct sock_common __sk_common;
    __u16 sk_protocol;
} __attribute__((preserve_access_index));
struct socket {
    struct sock *sk;
} __attribute__((preserve_access_index));
struct msghdr {
    void *msg_name;
    int msg_namelen;
    unsigned int msg_flags;
} __attribute__((preserve_access_index));

/* Only the control object reads these socket fields. */
static __always_inline int ac_network_protocol(struct ac_work *work, struct socket *socket) {
    struct sock *sk = 0;
    __u16 family = 0, protocol = 0;
    if (ACTRAIL_CORE_READ(&sk, socket, sk) || !sk ||
        ACTRAIL_CORE_READ(&family, sk, __sk_common.skc_family)) return -14;
    if (family != 2 && family != 10) return 0;
    if (ACTRAIL_CORE_READ(&protocol, sk, sk_protocol)) return -14;
    work->key.resource[0] = protocol;
    return 1;
}

static __always_inline int ac_network_local(struct ac_work *work, struct socket *socket) {
    struct sock *sk = 0;
    __u16 family = 0, port = 0;
    if (ACTRAIL_CORE_READ(&sk, socket, sk) || !sk ||
        ACTRAIL_CORE_READ(&family, sk, __sk_common.skc_family) ||
        ACTRAIL_CORE_READ(&port, sk, __sk_common.skc_num)) return -14;
    if (family != 2 && family != 10) return 0;
    int bytes = family == 2 ? 6 : 18;
    work->key.resource[1] = family == 2 ? 4 : 6;
    if (family == 2) {
        if (bpf_probe_read_kernel(work->key.resource + 2, 4,
            __builtin_preserve_access_index(&sk->__sk_common.skc_rcv_saddr))) return -14;
    } else if (bpf_probe_read_kernel(work->key.resource + 2, 16,
            __builtin_preserve_access_index(&sk->__sk_common.skc_v6_rcv_saddr))) return -14;
    work->key.resource[bytes] = port >> 8;
    work->key.resource[bytes + 1] = port;
    return bytes;
}

static __always_inline int ac_network_address(
    struct ac_work *work, void *address, int length, __u16 unspecified_family
) {
    __u16 family = 0;
    if (length < 2 || bpf_probe_read_kernel(&family, sizeof(family), address))
        return -14;
    if (!family) family = unspecified_family;
    if (family == 2) {
        if (length < 16) return -22;
        work->key.resource[1] = 4;
        if (bpf_probe_read_kernel(work->key.resource + 2, 4, address + 4) ||
            bpf_probe_read_kernel(work->key.resource + 6, 2, address + 2)) return -14;
        return 6;
    }
    if (family == 10) {
        /* A 24-byte sockaddr_in6 omits only the optional scope ID. */
        if (length < 24) return -22;
        work->key.resource[1] = 6;
        if (bpf_probe_read_kernel(work->key.resource + 2, 16, address + 8) ||
            bpf_probe_read_kernel(work->key.resource + 18, 2, address + 2)) return -14;
        return 18;
    }
    return 0;
}

static __always_inline int ac_network_message(
    struct ac_work *work, struct socket *socket, struct msghdr *message
) {
    struct sock *socket_state = 0;
    __u16 family = 0;
    void *address = 0;
    int length = 0;
    if (ACTRAIL_CORE_READ(&socket_state, socket, sk) || !socket_state ||
        ACTRAIL_CORE_READ(&family, socket_state, __sk_common.skc_family)) return -14;
    /* Unix and other families remain outside the IP endpoint policy. */
    if (family != 2 && family != 10) return 0;
    if (ACTRAIL_CORE_READ(&address, message, msg_name) ||
        ACTRAIL_CORE_READ(&length, message, msg_namelen)) return -14;
    if (address) {
        __u16 protocol = 0, destination_family = 0;
        if (ACTRAIL_CORE_READ(&protocol, socket_state, sk_protocol)) return -14;
        if (protocol == 6) {
            __u8 state = 0;
            unsigned int flags = 0;
            if (ACTRAIL_CORE_READ(&state, socket_state, __sk_common.skc_state) ||
                ACTRAIL_CORE_READ(&flags, message, msg_flags)) return -14;
            /* TCP ignores msg_name except Fast Open on an unconnected socket.
             * A supplied address must not replace the established peer. */
            if (state != 7 || !(flags & 0x20000000)) address = 0;
        } else if (protocol == 17) {
            if (length < 2 || bpf_probe_read_kernel(&destination_family, 2, address))
                return -14;
            /* IPv4 UDP accepts AF_UNSPEC as IPv4; IPv6 UDP uses its peer. */
            if (family == 10 && !destination_family) address = 0;
        }
        if (address) return ac_network_address(work, address, length, family == 2 ? 2 : 0);
    }

    /* Connected sends carry no address. Read the actual peer in network byte
     * order and use the same exact-port / any-port lookup as connect. */
    if (family == 2) {
        work->key.resource[1] = 4;
        if (bpf_probe_read_kernel(work->key.resource + 2, 4,
                __builtin_preserve_access_index(&socket_state->__sk_common.skc_daddr)) ||
            bpf_probe_read_kernel(work->key.resource + 6, 2,
                __builtin_preserve_access_index(&socket_state->__sk_common.skc_dport)))
            return -14;
        return 6;
    }
    work->key.resource[1] = 6;
    if (bpf_probe_read_kernel(work->key.resource + 2, 16,
            __builtin_preserve_access_index(&socket_state->__sk_common.skc_v6_daddr)) ||
        bpf_probe_read_kernel(work->key.resource + 18, 2,
            __builtin_preserve_access_index(&socket_state->__sk_common.skc_dport)))
        return -14;
    return 18;
}

/* The snapshot records the active port prefix lengths in its default value.
 * Each bucket uses IP LPM. Rank resolves port and protocol precedence. */
static __attribute__((noinline)) int ac_network_finish(struct ac_context *context, int bytes) {
    if (bytes < 0) return ac_finish(context, bytes);
    if (!bytes) return 0;
    __u8 endpoint[20];
    __builtin_memcpy(endpoint, context->work->key.resource, sizeof(endpoint));
    __u32 prefixes = context->work->selected.reserved;
    __u16 port = bytes == 6 ? ((__u16)endpoint[6] << 8) | endpoint[7]
        : ((__u16)endpoint[18] << 8) | endpoint[19];
    context->work->key.resource[4] = endpoint[1];
    if (bytes == 6) {
        __builtin_memcpy(context->work->key.resource + 5, endpoint + 2, 4);
    } else {
        __builtin_memcpy(context->work->key.resource + 5, endpoint + 2, 16);
    }
    for (__u32 prefix = 0; prefix <= 16; prefix++) {
        if (!(prefixes & (1U << prefix))) continue;
        __u16 masked = port & (0xffffU << (16 - prefix));
        context->work->key.resource[0] = endpoint[0];
        context->work->key.resource[1] = prefix;
        context->work->key.resource[2] = masked >> 8;
        context->work->key.resource[3] = masked;
        ac_candidate(context, bytes == 6 ? 9 : 21);
        context->work->key.resource[0] = 0;
        ac_candidate(context, bytes == 6 ? 9 : 21);
    }
    /* Audit always carries the original protocol and full endpoint. */
    __builtin_memset(context->work->key.resource, 0, 24);
    __builtin_memcpy(context->work->key.resource, endpoint, sizeof(endpoint));
    return ac_finish(context, 0);
}

#endif
