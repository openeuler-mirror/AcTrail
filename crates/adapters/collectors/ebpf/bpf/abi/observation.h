#ifndef ACTRAIL_ABI_OBSERVATION_H
#define ACTRAIL_ABI_OBSERVATION_H

#include "../common/constants.h"
#include "../common/helpers.h"

enum actrail_proc_event_kind {
    ACTRAIL_PROC_FORK = 1,
    ACTRAIL_PROC_EXEC = 2,
    ACTRAIL_PROC_EXIT = 3,
    ACTRAIL_PROC_SIGNAL = 4,
    ACTRAIL_NET_CONNECT = 100,
    ACTRAIL_NET_ACCEPT = 101,
    ACTRAIL_FD_IO_SEND = 102,
    ACTRAIL_FD_IO_RECV = 103,
    ACTRAIL_NET_BIND = 104,
    ACTRAIL_NET_LISTEN = 105,
    ACTRAIL_NET_CLOSE = 106,
    ACTRAIL_NET_SHUTDOWN = 107,
    ACTRAIL_SOCKET_FD_RELEASE = 108,
    ACTRAIL_TLS_PAYLOAD_COMPLETION = 201,
    ACTRAIL_TLS_PAYLOAD_CAPTURE_REQUEST = 202,
    ACTRAIL_TLS_PAYLOAD_DIRECT_CAPTURE = 203,
    ACTRAIL_TLS_PAYLOAD_DIAGNOSTIC = 204,
    ACTRAIL_LAUNCH_BINDING_FAILURE = 205,
    ACTRAIL_FILE_OPEN = 300,
    ACTRAIL_FILE_UNLINK = 301,
    ACTRAIL_FILE_RENAME = 302,
    ACTRAIL_FILE_MKDIR = 303,
    ACTRAIL_FILE_RMDIR = 304,
    ACTRAIL_FILE_TRUNCATE = 305,
    ACTRAIL_FILE_MMAP = 306,
    ACTRAIL_FILE_CONTEXT = 307,
    ACTRAIL_FILE_READ_SUMMARY = 308,
    ACTRAIL_STDIO_PAYLOAD = 400,
    ACTRAIL_STDIO_PAYLOAD_COMPLETION = 401,
    ACTRAIL_SOCKET_PAYLOAD = 500,
    ACTRAIL_SOCKET_PAYLOAD_COMPLETION = 501,
};

enum actrail_event_abi {
    ACTRAIL_EVENT_ABI_REVISION = 1,
};

enum actrail_endpoint_role {
    ACTRAIL_ENDPOINT_NONE = 0,
    ACTRAIL_ENDPOINT_LOCAL = 1,
    ACTRAIL_ENDPOINT_REMOTE = 2,
};

struct actrail_endpoint {
    __u16 family;
    __u16 port_be;
    __u32 addr4_be;
    __u8 addr6[16];
};

struct actrail_event_header {
    __u32 kind;
    __u16 abi_revision;
    __u16 record_size;
    __u64 trace_id;
    __u64 observed_ktime_ns;
    __u32 subject_observer_namespace_tgid;
    __u32 subject_kernel_tgid;
    __u64 subject_start_boottime_ns;
} __attribute__((packed));

#endif
