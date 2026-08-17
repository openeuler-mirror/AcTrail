#ifndef ACTRAIL_FD_H
#define ACTRAIL_FD_H

/*
 * Unified file-descriptor lifecycle state.
 *
 * fd_table keeps the hot-path (pid, fd) classification lookup.  Lifecycle
 * sweeps use a dense slot index whose slot number is independent of the real
 * fd value, so high-numbered descriptors work without scanning 0..fd.  The
 * per-process slot limit is configured by the loader and bounded only for the
 * verifier; read/write still perform one fd_table lookup.
 */

#include "actrail_runtime.h"

#define ACTRAIL_FD_INDEX_SLOT_BITS 64
#define ACTRAIL_FD_INDEX_HARD_MAX_CHUNKS 4
#define ACTRAIL_FD_INDEX_HARD_MAX_ENTRIES \
    (ACTRAIL_FD_INDEX_SLOT_BITS * ACTRAIL_FD_INDEX_HARD_MAX_CHUNKS)
#define ACTRAIL_FD_CLOSE_RANGE_CLOEXEC (1UL << 2)
#define ACTRAIL_FD_SOCKET_PENDING_KIND 1000
#define ACTRAIL_FD_REFCOUNT_MISSING 0xffffffffU
#define ACTRAIL_FD_REFCOUNT_NOT_LAST 0xfffffffeU
#define ACTRAIL_FD_FILE_IDENTITY_READ_FAILED 1ULL

#ifndef O_CLOEXEC
#define O_CLOEXEC 02000000
#endif
#ifndef FD_CLOEXEC
#define FD_CLOEXEC 1
#endif
#ifndef FIOCLEX
#define FIOCLEX 0x5451
#endif
#ifndef FIONCLEX
#define FIONCLEX 0x5450
#endif

enum actrail_fd_category {
    ACTRAIL_FD_CATEGORY_NONE = 0,
    ACTRAIL_FD_CATEGORY_NET = 1,
    ACTRAIL_FD_CATEGORY_IPC_UNIX_SOCKET = 2,
    ACTRAIL_FD_CATEGORY_IPC_PIPE = 3,
    ACTRAIL_FD_CATEGORY_FILE = 4,
};

enum actrail_fd_flag {
    ACTRAIL_FD_FLAG_CLOEXEC = 1,
};

enum actrail_fd_flag_context_kind {
    ACTRAIL_FD_FLAG_CONTEXT_NONE = 0,
    ACTRAIL_FD_FLAG_CONTEXT_IOCTL_CLOEXEC = 1,
    ACTRAIL_FD_FLAG_CONTEXT_IOCTL_NCLOEXEC = 2,
};

enum actrail_fd_category_flag {
    ACTRAIL_FD_CATEGORY_FLAG_NET = 1 << ACTRAIL_FD_CATEGORY_NET,
    ACTRAIL_FD_CATEGORY_FLAG_IPC_UNIX_SOCKET = 1 << ACTRAIL_FD_CATEGORY_IPC_UNIX_SOCKET,
    ACTRAIL_FD_CATEGORY_FLAG_IPC_PIPE = 1 << ACTRAIL_FD_CATEGORY_IPC_PIPE,
    ACTRAIL_FD_CATEGORY_FLAG_FILE = 1 << ACTRAIL_FD_CATEGORY_FILE,
};

enum actrail_fd_dup_mode {
    ACTRAIL_FD_DUP_RET_FD = 1,
    ACTRAIL_FD_DUP_TARGET_FD = 2,
};

enum actrail_fd_close_mode {
    ACTRAIL_FD_CLOSE_ONE = 1,
    ACTRAIL_FD_CLOSE_RANGE = 2,
};

struct actrail_fd_key {
    __u32 pid;
    __u32 fd;
};

struct actrail_fd_state {
    __u32 category;
    __u32 flags;
    __u32 index_slot;
    __u32 reserved;
    __u64 generation;
};

struct actrail_fd_object_key {
    __u32 pid;
    __u32 reserved;
    __u64 generation;
};

struct actrail_fd_object_state {
    __u32 refcount;
    __u32 category;
    __u64 trace_id;
    __u64 file_identity;
    struct actrail_endpoint remote;
};

struct actrail_fd_object_snapshot {
    __u32 refcount;
    __u32 category;
    __u64 trace_id;
    struct actrail_endpoint remote;
};

struct actrail_fd_index_slot_key {
    __u32 pid;
    __u32 slot;
};

struct actrail_fd_index_slot {
    __u32 fd;
    __u32 reserved;
    __u64 generation;
};

struct actrail_fd_config {
    __u32 category_flags;
    __u32 max_slots_per_process;
};

struct actrail_fd_registration {
    __u64 trace_id;
    void *program_ctx;
    __u32 pid;
    __u32 fd;
    __u32 category;
    __u32 flags;
};

struct actrail_pending_fd_open_op {
    __u64 trace_id;
    __u32 pid;
    __u32 flags;
};

struct actrail_pending_fd_close_op {
    __u64 trace_id;
    __u32 pid;
    __u32 first_fd;
    __u32 last_fd;
    __u32 flags;
    __u32 mode;
    __u64 expected_generation;
};

struct actrail_pending_fd_dup_op {
    __u64 trace_id;
    struct actrail_fd_state source;
    __u32 pid;
    __u32 source_fd;
    __u32 target_fd;
    __u32 target_flags;
    __u32 mode;
    __u32 reference_reserved;
    __u64 source_file_identity;
};

struct actrail_pending_fd_flag_op {
    __u32 pid;
    __u32 fd;
    __u32 flags;
    __u32 context_kind;
    __u64 generation;
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_fd_key);
    __type(value, struct actrail_fd_state);
} fd_table SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_fd_object_key);
    __type(value, struct actrail_fd_object_state);
} fd_objects SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, struct actrail_fd_index_slot_key);
    __type(value, struct actrail_fd_index_slot);
} fd_index_slots SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u32);
} fd_process_active_counts SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_fd_open_op);
} pending_fd_open_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_fd_close_op);
} pending_fd_close_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_fd_dup_op);
} pending_fd_dup_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __type(key, __u64);
    __type(value, struct actrail_pending_fd_flag_op);
} pending_fd_flag_ops SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_fd_config);
} fd_category_config SEC(".maps");

#include "fd/actrail_fd_index.h"
#include "fd/actrail_fd_sweep.h"
#include "fd/actrail_fd_lifecycle.h"

#endif
