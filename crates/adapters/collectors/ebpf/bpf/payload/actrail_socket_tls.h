#ifndef ACTRAIL_SOCKET_TLS_H
#define ACTRAIL_SOCKET_TLS_H

/*
 * TLS handshake records expose a fixed nine-byte prefix.  Checking only this
 * prefix keeps the syscall hot path bounded; fragmented hellos are deliberately
 * left to the user-space HTTP CONNECT gate.
 */
#define ACTRAIL_SOCKET_TLS_HELLO_PREFIX_SIZE 9
#define ACTRAIL_TLS_HANDSHAKE_CONTENT_TYPE 22
#define ACTRAIL_TLS_CLIENT_HELLO 1
#define ACTRAIL_TLS_SERVER_HELLO 2
#define ACTRAIL_TLS_MIN_HELLO_BODY_SIZE 38
#define ACTRAIL_TLS_MAX_PLAINTEXT_RECORD_SIZE 16384

struct actrail_socket_tls_hello_prefix {
    __u8 bytes[ACTRAIL_SOCKET_TLS_HELLO_PREFIX_SIZE];
};

static __always_inline int socket_payload_prefix_is_tls_hello(
    const __u8 *bytes,
    __u32 size
) {
    __u32 record_size;
    __u32 handshake_size;

    if (size < ACTRAIL_SOCKET_TLS_HELLO_PREFIX_SIZE
        || bytes[0] != ACTRAIL_TLS_HANDSHAKE_CONTENT_TYPE
        || bytes[1] != 3
        || bytes[2] < 1
        || bytes[2] > 3
        || (bytes[5] != ACTRAIL_TLS_CLIENT_HELLO
            && bytes[5] != ACTRAIL_TLS_SERVER_HELLO)) {
        return 0;
    }

    record_size = ((__u32)bytes[3] << 8) | bytes[4];
    handshake_size =
        ((__u32)bytes[6] << 16) | ((__u32)bytes[7] << 8) | bytes[8];
    return record_size >= ACTRAIL_SOCKET_TLS_HELLO_PREFIX_SIZE - 5
        && record_size <= ACTRAIL_TLS_MAX_PLAINTEXT_RECORD_SIZE
        && handshake_size >= ACTRAIL_TLS_MIN_HELLO_BODY_SIZE
        && handshake_size + 4 <= record_size;
}

static __always_inline int socket_payload_read_tls_hello_prefix(
    __u64 buffer_ptr,
    __u64 buffer_size
) {
    struct actrail_socket_tls_hello_prefix prefix = {};

    if (!buffer_ptr || buffer_size < ACTRAIL_SOCKET_TLS_HELLO_PREFIX_SIZE) {
        return 0;
    }
    if (bpf_probe_read_user(
            prefix.bytes,
            sizeof(prefix.bytes),
            (void *)(unsigned long)buffer_ptr
        ) != 0) {
        return 0;
    }
    return socket_payload_prefix_is_tls_hello(prefix.bytes, sizeof(prefix.bytes));
}

#endif
