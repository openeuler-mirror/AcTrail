#ifndef ACTRAIL_RUNTIME_ENDPOINT_H
#define ACTRAIL_RUNTIME_ENDPOINT_H

#include "../abi/observation.h"
#include "../common/kernel_types.h"

static __always_inline void read_endpoint(__u64 user_ptr, struct actrail_endpoint *endpoint) {
    struct actrail_sockaddr_storage storage = {};
    struct sockaddr_in *addr4;
    struct sockaddr_in6 *addr6;

    if (!user_ptr) {
        return;
    }
    if (bpf_probe_read_user(&storage, sizeof(storage), (void *)(unsigned long)user_ptr) != 0) {
        return;
    }

    endpoint->family = storage.family;
    if (storage.family == AF_INET) {
        addr4 = (struct sockaddr_in *)&storage;
        endpoint->port_be = addr4->sin_port;
        endpoint->addr4_be = addr4->sin_addr.s_addr;
    } else if (storage.family == AF_INET6) {
        addr6 = (struct sockaddr_in6 *)&storage;
        endpoint->port_be = addr6->sin6_port;
        __builtin_memcpy(endpoint->addr6, &addr6->sin6_addr, sizeof(endpoint->addr6));
    }
}

#endif
