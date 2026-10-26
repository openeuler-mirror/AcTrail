#ifndef ACTRAIL_FILE_IO_PROGRAMS_H
#define ACTRAIL_FILE_IO_PROGRAMS_H

#include "objects.h"

static __always_inline int file_io_accumulate(
    struct file *file,
    __s64 result,
    __u32 direction,
    __u32 scratch_slot
) {
    struct actrail_file_io_config *config = file_io_config_get();
    struct actrail_file_io_scratch *scratch;
    struct actrail_file_io_object *object;
    struct actrail_file_io_total *total;
    struct actrail_file_io_total *candidate;
    struct actrail_file_io_key key = {};
    struct actrail_process_identity *identity;
    __u32 tgid = 0, tid = 0, lookup_flags = 0;
    __u32 flags;
    __u64 *trace_id;
    __u64 now;
    int overflow = 0;
    int changed = 0;

    if (!config) {
        return 0;
    }
    flags = direction == ACTRAIL_FILE_IO_READ ? config->read_flags : config->write_flags;
    if (!flags || (result < 0 && !(flags & ACTRAIL_FILE_IO_ERRORS))) {
        return 0;
    }
    if (result >= 0 && !(flags & (ACTRAIL_FILE_IO_OBSERVED | ACTRAIL_FILE_IO_COUNTS
        | ACTRAIL_FILE_IO_BYTES))) {
        return 0;
    }
    if (result == 0 && !(flags & (ACTRAIL_FILE_IO_OBSERVED | ACTRAIL_FILE_IO_COUNTS))) {
        return 0;
    }
    trace_id = lookup_current_detailed_trace(&tgid, &tid, &lookup_flags);
    if (!trace_id || !tgid) {
        return 0;
    }
    key.trace_id = *trace_id;
    key.kernel_tgid = tgid;
    key.process_start_boottime_ns = current_process_start_time(tgid);
    key.direction = direction;
    key.error_result = result < 0 ? (__s32)result : 0;
    if (!key.process_start_boottime_ns) {
        file_io_diag(ACTRAIL_FILE_IO_IDENTITY_FAIL);
        return 0;
    }
    scratch = bpf_map_lookup_elem(&file_io_scratch, &scratch_slot);
    if (!scratch) {
        file_io_diag(ACTRAIL_FILE_IO_TOTAL_INSERT_FAIL);
        return 0;
    }
    object = file_io_object_get(file, config, scratch);
    if (!object) {
        return 0;
    }
    key.file_token = object->file_token;
    total = bpf_map_lookup_elem(&file_io_totals, &key);
    candidate = (void *)scratch->total;
    if (!total) {
        __builtin_memset(candidate, 0, sizeof(*candidate));
        candidate->valid_flags = flags;
        candidate->target_kind = object->target_kind;
        identity = lookup_process_identity(tgid);
        candidate->observer_namespace_tgid = identity ? identity->observer_namespace_tgid
            : observer_tgid_for_task(actrail_bpf_get_current_task());
        if (!candidate->observer_namespace_tgid) {
            file_io_diag(ACTRAIL_FILE_IO_IDENTITY_FAIL);
            return 0;
        }
        actrail_file_io_lock(&object->lock);
        candidate->path_len = object->path_len;
        candidate->path_status = object->path_status;
        __builtin_memcpy(candidate->path, object->path, sizeof(candidate->path));
        actrail_file_io_unlock(&object->lock);
        bpf_map_update_elem(&file_io_totals, &key, candidate, BPF_NOEXIST);
        total = bpf_map_lookup_elem(&file_io_totals, &key);
        if (!total) {
            file_io_diag(ACTRAIL_FILE_IO_TOTAL_INSERT_FAIL);
            return 0;
        }
    }
    // Copy a newly available path only on the unknown->known transition.
    candidate->path_status = ACTRAIL_FILE_IO_PATH_UNKNOWN;
    if (total->path_status == ACTRAIL_FILE_IO_PATH_UNKNOWN
        && object->path_status != ACTRAIL_FILE_IO_PATH_UNKNOWN) {
        actrail_file_io_lock(&object->lock);
        candidate->path_len = object->path_len;
        candidate->path_status = object->path_status;
        __builtin_memcpy(candidate->path, object->path, sizeof(candidate->path));
        actrail_file_io_unlock(&object->lock);
    }
    now = bpf_ktime_get_ns();
    actrail_file_io_lock(&total->lock);
    if (total->path_status == ACTRAIL_FILE_IO_PATH_UNKNOWN
        && candidate->path_status != ACTRAIL_FILE_IO_PATH_UNKNOWN) {
        total->path_len = candidate->path_len;
        total->path_status = candidate->path_status;
        __builtin_memcpy(total->path, candidate->path, sizeof(total->path));
        changed = 1;
    }
    if (result >= 0 && (flags & ACTRAIL_FILE_IO_OBSERVED)
        && !(total->valid_flags & ACTRAIL_FILE_IO_SEEN)) {
        total->valid_flags |= ACTRAIL_FILE_IO_SEEN;
        changed = 1;
    }
    if ((flags & ACTRAIL_FILE_IO_COUNTS) || result < 0) {
        if (total->operation_count == ~0ULL) {
            overflow = 1;
        } else {
            total->operation_count += 1;
            changed = 1;
        }
    }
    if (result > 0 && (flags & ACTRAIL_FILE_IO_BYTES)) {
        if (total->bytes > ~0ULL - (__u64)result) {
            overflow = 1;
        } else {
            total->bytes += (__u64)result;
            changed = 1;
        }
    }
    if (changed) {
        if (!total->first_ktime_ns || now < total->first_ktime_ns) {
            total->first_ktime_ns = now;
        }
        if (now > total->last_ktime_ns) {
            total->last_ktime_ns = now;
        }
        if (total->snapshot_sequence == ~0ULL) {
            overflow = 1;
        } else {
            total->snapshot_sequence += 1;
        }
    }
    actrail_file_io_unlock(&total->lock);
    if (overflow) {
        file_io_diag(ACTRAIL_FILE_IO_COUNTER_OVERFLOW);
    }
    return 0;
}

SEC("fexit/vfs_read")
int handle_file_io_read(__u64 *ctx) {
    return file_io_accumulate((void *)ctx[0], (__s64)ctx[4], ACTRAIL_FILE_IO_READ, 1);
}
SEC("fexit/vfs_readv")
int handle_file_io_readv(__u64 *ctx) {
    return file_io_accumulate((void *)ctx[0], (__s64)ctx[5], ACTRAIL_FILE_IO_READ, 2);
}
SEC("fexit/vfs_write")
int handle_file_io_write(__u64 *ctx) {
    return file_io_accumulate((void *)ctx[0], (__s64)ctx[4], ACTRAIL_FILE_IO_WRITE, 3);
}
SEC("fexit/vfs_writev")
int handle_file_io_writev(__u64 *ctx) {
    return file_io_accumulate((void *)ctx[0], (__s64)ctx[5], ACTRAIL_FILE_IO_WRITE, 4);
}

#endif
