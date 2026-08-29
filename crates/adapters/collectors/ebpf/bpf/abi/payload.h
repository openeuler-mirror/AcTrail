#ifndef ACTRAIL_ABI_PAYLOAD_H
#define ACTRAIL_ABI_PAYLOAD_H

#include "observation.h"

enum actrail_socket_payload_abi {
    ACTRAIL_SOCKET_PAYLOAD_ABI_MAX_BYTES = 4096,
    ACTRAIL_SOCKET_PAYLOAD_COPY_MAX_BYTES = 4095,
};

enum actrail_socket_payload_direction {
    ACTRAIL_SOCKET_PAYLOAD_INBOUND = 1,
    ACTRAIL_SOCKET_PAYLOAD_OUTBOUND = 2,
};

enum actrail_socket_payload_syscall {
    ACTRAIL_SOCKET_SYSCALL_READ = 1,
    ACTRAIL_SOCKET_SYSCALL_WRITE = 2,
    ACTRAIL_SOCKET_SYSCALL_SENDTO = 3,
    ACTRAIL_SOCKET_SYSCALL_RECVFROM = 4,
    ACTRAIL_SOCKET_SYSCALL_WRITEV = 5,
    ACTRAIL_SOCKET_SYSCALL_SENDMSG = 6,
};

enum actrail_socket_payload_flags {
    ACTRAIL_SOCKET_PAYLOAD_TRUNCATED = 1,
};
struct actrail_socket_payload_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 sequence;
    __u32 fd;
    __u32 original_size;
    __u32 captured_size;
    __u32 flags;
    __u32 syscall;
    __u32 fd_generation;
    __u64 pid_generation;
    __u32 host_pid;
    __u32 host_tid;
    __u8 bytes[ACTRAIL_SOCKET_PAYLOAD_ABI_MAX_BYTES];
};

struct actrail_socket_payload_completion_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 sequence;
    __u64 completed_size;
    __u64 requested_size;
    __u64 buffer_ptr;
    __u64 pid_generation;
    __u32 fd;
    __u32 flags;
    __u32 syscall;
    __u32 fd_generation;
    __u32 host_pid;
    __u32 host_tid;
};
enum actrail_stdio_payload_abi {
    ACTRAIL_STDIO_PAYLOAD_ABI_MAX_BYTES = 4096,
    ACTRAIL_STDIO_PAYLOAD_COPY_MAX_BYTES = 4095,
};

enum actrail_stdio_payload_direction {
    ACTRAIL_STDIO_PAYLOAD_INBOUND = 1,
    ACTRAIL_STDIO_PAYLOAD_OUTBOUND = 2,
};

enum actrail_stdio_payload_stream {
    ACTRAIL_STDIO_STREAM_STDIN = 0,
    ACTRAIL_STDIO_STREAM_STDOUT = 1,
    ACTRAIL_STDIO_STREAM_STDERR = 2,
};

enum actrail_stdio_payload_syscall {
    ACTRAIL_STDIO_SYSCALL_READ = 1,
    ACTRAIL_STDIO_SYSCALL_WRITE = 2,
};

enum actrail_stdio_payload_flags {
    ACTRAIL_STDIO_PAYLOAD_TRUNCATED = 1,
    ACTRAIL_STDIO_PAYLOAD_STAGED = 2,
};
struct actrail_stdio_payload_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 sequence;
    __u32 stream;
    __u32 original_size;
    __u32 captured_size;
    __u32 flags;
    __u32 fd;
    __u32 syscall;
    __u64 pid_generation;
    __u32 host_pid;
    __u32 host_tid;
    __u8 bytes[ACTRAIL_STDIO_PAYLOAD_ABI_MAX_BYTES];
};

struct actrail_stdio_payload_completion_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 sequence;
    __s64 result;
    __u64 requested_size;
    __u64 pid_generation;
    __u32 stream;
    __u32 fd;
    __u32 syscall;
    __u32 host_pid;
    __u32 host_tid;
    __u32 reserved;
};
enum actrail_tls_payload_direction {
    ACTRAIL_TLS_PAYLOAD_OUTBOUND = 1,
    ACTRAIL_TLS_PAYLOAD_INBOUND = 2,
};

enum actrail_tls_payload_symbol {
    ACTRAIL_TLS_SYMBOL_SSL_WRITE = 1,
    ACTRAIL_TLS_SYMBOL_SSL_READ = 2,
    ACTRAIL_TLS_SYMBOL_SSL_WRITE_EX = 3,
    ACTRAIL_TLS_SYMBOL_SSL_READ_EX = 4,
    ACTRAIL_TLS_SYMBOL_RUSTLS_WRITE = 5,
    ACTRAIL_TLS_SYMBOL_RUSTLS_WRITE_VECTORED = 6,
    ACTRAIL_TLS_SYMBOL_GO_CONN_WRITE = 7,
    ACTRAIL_TLS_SYMBOL_GO_CONN_READ = 8,
    ACTRAIL_TLS_SYMBOL_GNUTLS_RECORD_SEND = 9,
    ACTRAIL_TLS_SYMBOL_GNUTLS_RECORD_RECV = 10,
    ACTRAIL_TLS_SYMBOL_NSPR_PR_WRITE = 11,
    ACTRAIL_TLS_SYMBOL_NSPR_PR_SEND = 12,
    ACTRAIL_TLS_SYMBOL_NSPR_PR_READ = 13,
    ACTRAIL_TLS_SYMBOL_NSPR_PR_RECV = 14,
    ACTRAIL_TLS_SYMBOL_RUSTLS_BUFFER_PLAINTEXT = 15,
    ACTRAIL_TLS_SYMBOL_RUSTLS_TAKE_RECEIVED_PLAINTEXT = 16,
};

enum actrail_tls_payload_library {
    ACTRAIL_TLS_LIBRARY_OPENSSL = 1,
    ACTRAIL_TLS_LIBRARY_BORINGSSL = 2,
    ACTRAIL_TLS_LIBRARY_RUSTLS = 3,
    ACTRAIL_TLS_LIBRARY_GO = 4,
    ACTRAIL_TLS_LIBRARY_GNUTLS = 5,
    ACTRAIL_TLS_LIBRARY_NSS = 6,
};

enum actrail_tls_completion_flags {
    ACTRAIL_TLS_PAYLOAD_COMPLETION_FAILED = 2,
};
enum actrail_tls_payload_copy_limit {
    ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_ABI_BYTES = 65536,
    ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_BYTES = 65535,
    ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_MAX_CHUNKS = 1,
};
struct actrail_tls_completion_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 stream_key;
    __u64 operation_id;
    __u32 completed_size;
    __u32 flags;
    __u32 symbol;
    __u32 library;
    __u64 pid_generation;
    __u64 buffer_ptr;
    __u32 host_pid;
    __u32 host_tid;
};

struct actrail_tls_capture_request_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 stream_key;
    __u64 operation_id;
    __u64 requested_size;
    __u64 buffer_ptr;
    __u64 pid_generation;
    __u32 symbol;
    __u32 library;
    __u32 host_pid;
    __u32 host_tid;
};

struct actrail_tls_direct_capture_event {
    __u32 kind;
    __u32 pid;
    __u32 tid;
    __u32 direction;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u64 stream_key;
    __u64 operation_id;
    __u32 original_size;
    __u32 captured_size;
    __u32 flags;
    __u32 symbol;
    __u32 library;
    __u32 operation_offset;
    __u64 pid_generation;
    __u32 host_pid;
    __u32 host_tid;
    __u8 bytes[ACTRAIL_TLS_PAYLOAD_DIRECT_COPY_ABI_BYTES];
};

#endif
