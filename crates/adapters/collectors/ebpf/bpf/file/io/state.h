#ifndef ACTRAIL_FILE_IO_STATE_H
#define ACTRAIL_FILE_IO_STATE_H

#include "../../runtime/event_transport.h"
#include "../../common/kernel_types.h"

#define ACTRAIL_FILE_IO_OBSERVED 1U
#define ACTRAIL_FILE_IO_COUNTS 2U
#define ACTRAIL_FILE_IO_BYTES 4U
#define ACTRAIL_FILE_IO_ERRORS 8U
#define ACTRAIL_FILE_IO_SEEN (1U << 8)
#define ACTRAIL_FILE_IO_TTY 1U
#define ACTRAIL_FILE_IO_REGULAR 1U
#define ACTRAIL_FILE_IO_CHARACTER 2U
#define ACTRAIL_FILE_IO_READ 1U
#define ACTRAIL_FILE_IO_WRITE 2U
#define ACTRAIL_FILE_IO_PATH_UNKNOWN 0U
#define ACTRAIL_FILE_IO_PATH_CAPTURED 1U
#define ACTRAIL_FILE_IO_PATH_TRUNCATED 2U
#define ACTRAIL_FILE_IO_PATH_FAULT 3U
#define ACTRAIL_FILE_IO_PATH_BYTES 256

struct actrail_file_io_config {
    __u32 read_flags;
    __u32 write_flags;
    __u32 max_path_bytes;
    __u32 flags;
};
struct actrail_file_io_object {
    struct bpf_spin_lock lock;
    __u32 path_status;
    __u64 file_token;
    __u32 path_len;
    __u32 target_kind;
    __u8 path[ACTRAIL_FILE_IO_PATH_BYTES];
};
struct actrail_file_io_key {
    __u64 trace_id;
    __u64 process_start_boottime_ns;
    __u64 file_token;
    __u32 kernel_tgid;
    __u32 direction;
    __s32 error_result;
    __u32 reserved;
};
struct actrail_file_io_total {
    struct bpf_spin_lock lock;
    __u32 valid_flags;
    __u64 snapshot_sequence;
    __u64 operation_count;
    __u64 bytes;
    __u64 first_ktime_ns;
    __u64 last_ktime_ns;
    __u32 observer_namespace_tgid;
    __u32 path_len;
    __u32 path_status;
    __u32 target_kind;
    __u8 path[ACTRAIL_FILE_IO_PATH_BYTES];
};
// Raw scratch deliberately has no embedded BTF spin_lock fields.
struct actrail_file_io_scratch {
    __u8 object[280];
    __u8 total[320];
    __u8 path[ACTRAIL_FILE_IO_PATH_BYTES];
};
_Static_assert(sizeof(struct actrail_file_io_object) == 280, "file object ABI");
_Static_assert(sizeof(struct actrail_file_io_key) == 40, "file summary key ABI");
_Static_assert(sizeof(struct actrail_file_io_total) == 320, "file summary value ABI");
_Static_assert(sizeof(struct actrail_file_io_scratch) == 856, "file summary scratch ABI");

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_file_io_config);
} file_io_config SEC(".maps");
struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} file_io_sequence SEC(".maps");
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __uint(map_flags, BPF_F_NO_PREALLOC);
    __type(key, __u64);
    __type(value, struct actrail_file_io_object);
} file_io_objects SEC(".maps");
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 1);
    __uint(map_flags, BPF_F_NO_PREALLOC);
    __type(key, struct actrail_file_io_key);
    __type(value, struct actrail_file_io_total);
} file_io_totals SEC(".maps");
struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 5);
    __type(key, __u32);
    __type(value, struct actrail_file_io_scratch);
} file_io_scratch SEC(".maps");
enum actrail_file_io_diagnostic {
    ACTRAIL_FILE_IO_OBJECT_INSERT_FAIL = 0,
    ACTRAIL_FILE_IO_TOTAL_INSERT_FAIL = 1,
    ACTRAIL_FILE_IO_PATH_CAPTURE_FAIL = 2,
    ACTRAIL_FILE_IO_COUNTER_OVERFLOW = 3,
    ACTRAIL_FILE_IO_IDENTITY_FAIL = 4,
    ACTRAIL_FILE_IO_OBJECT_DELETE_FAIL = 5,
};
struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 6);
    __type(key, __u32);
    __type(value, __u64);
} file_io_diagnostics SEC(".maps");

static void (*actrail_file_io_lock)(struct bpf_spin_lock *) = (void *)BPF_FUNC_spin_lock;
static void (*actrail_file_io_unlock)(struct bpf_spin_lock *) = (void *)BPF_FUNC_spin_unlock;
static long (*actrail_file_io_d_path)(struct path *, char *, __u32) = (void *)BPF_FUNC_d_path;

static __always_inline void file_io_diag(__u32 id) {
    __u64 *value = bpf_map_lookup_elem(&file_io_diagnostics, &id);
    if (value) {
        __sync_fetch_and_add(value, 1);
    }
}
static __always_inline struct actrail_file_io_config *file_io_config_get(void) {
    __u32 zero = 0;
    return bpf_map_lookup_elem(&file_io_config, &zero);
}

#endif
