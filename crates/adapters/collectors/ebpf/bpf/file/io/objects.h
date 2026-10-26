#ifndef ACTRAIL_FILE_IO_OBJECTS_H
#define ACTRAIL_FILE_IO_OBJECTS_H

#include "state.h"
#include "../../runtime/fetch_add_compat.h"

static __always_inline __u64 file_io_next_token(void) {
    __u64 kernel_pid_tgid = bpf_get_current_pid_tgid();
    __u64 initial = 1;
    __u64 *sequence;
    __u64 count;

    if (!kernel_pid_tgid) {
        file_io_diag(ACTRAIL_FILE_IO_OBJECT_INSERT_FAIL);
        return 0;
    }
    sequence = bpf_map_lookup_elem(&file_io_sequence, &kernel_pid_tgid);
    if (!sequence) {
        if (bpf_map_update_elem(
                &file_io_sequence,
                &kernel_pid_tgid,
                &initial,
                BPF_NOEXIST
            ) == 0) {
            return actrail_thread_sequence_id(kernel_pid_tgid, initial);
        }
        sequence = bpf_map_lookup_elem(&file_io_sequence, &kernel_pid_tgid);
        if (!sequence) {
            file_io_diag(ACTRAIL_FILE_IO_OBJECT_INSERT_FAIL);
            return 0;
        }
    }
    count = actrail_fetch_add_next(sequence);
    if (!count) {
        file_io_diag(ACTRAIL_FILE_IO_OBJECT_INSERT_FAIL);
        return 0;
    }
    if (count > 0xffffffffULL) {
        file_io_diag(ACTRAIL_FILE_IO_COUNTER_OVERFLOW);
        return 0;
    }
    return actrail_thread_sequence_id(kernel_pid_tgid, count);
}

static __always_inline struct actrail_file_io_object *file_io_object_get(
    struct file *file,
    const struct actrail_file_io_config *config,
    struct actrail_file_io_scratch *scratch
) {
    __u64 pointer = (__u64)file;
    struct actrail_file_io_object *object = bpf_map_lookup_elem(&file_io_objects, &pointer);
    struct actrail_file_io_object *candidate = (void *)scratch->object;
    struct inode *inode = 0;
    unsigned short mode = 0;
    __u32 target_kind;
    if (object) {
        if (object->target_kind == ACTRAIL_FILE_IO_CHARACTER && !(config->flags & ACTRAIL_FILE_IO_TTY)) {
            return 0;
        }
        return object;
    }
    if (!file || ACTRAIL_CORE_READ(&inode, file, f_inode) || !inode
        || ACTRAIL_CORE_READ(&mode, inode, i_mode)) {
        file_io_diag(ACTRAIL_FILE_IO_IDENTITY_FAIL);
        return 0;
    }
    if ((mode & 0170000) == 0100000) {
        target_kind = ACTRAIL_FILE_IO_REGULAR;
    } else if ((mode & 0170000) == 0020000 && (config->flags & ACTRAIL_FILE_IO_TTY)) {
        target_kind = ACTRAIL_FILE_IO_CHARACTER;
    } else {
        return 0;
    }
    __builtin_memset(candidate, 0, sizeof(*candidate));
    candidate->file_token = file_io_next_token();
    candidate->target_kind = target_kind;
    if (!candidate->file_token) {
        file_io_diag(ACTRAIL_FILE_IO_OBJECT_INSERT_FAIL);
        return 0;
    }
    // The live VFS file reference prevents free/reuse during this insertion.
    bpf_map_update_elem(&file_io_objects, &pointer, candidate, BPF_NOEXIST);
    object = bpf_map_lookup_elem(&file_io_objects, &pointer);
    if (!object) {
        file_io_diag(ACTRAIL_FILE_IO_OBJECT_INSERT_FAIL);
    }
    return object;
}

SEC("fentry/security_file_permission")
int handle_file_io_permission(__u64 *ctx) {
    struct file *file = (void *)ctx[0];
    int mask = (int)ctx[1];
    __u32 tgid = 0, tid = 0, lookup_flags = 0;
    __u32 slot = 0;
    struct actrail_file_io_config *config = file_io_config_get();
    struct actrail_file_io_scratch *scratch;
    struct actrail_file_io_object *object;
    __u32 max_bytes;
    long copied;
    if (!config || !(((mask & 4) && config->read_flags)
        || ((mask & 2) && config->write_flags))) {
        return 0;
    }
    if (!lookup_current_detailed_trace(&tgid, &tid, &lookup_flags)) {
        return 0;
    }
    scratch = bpf_map_lookup_elem(&file_io_scratch, &slot);
    if (!scratch) {
        file_io_diag(ACTRAIL_FILE_IO_OBJECT_INSERT_FAIL);
        return 0;
    }
    object = file_io_object_get(file, config, scratch);
    if (!object || object->path_status != ACTRAIL_FILE_IO_PATH_UNKNOWN) {
        return 0;
    }
    max_bytes = config->max_path_bytes;
    if (max_bytes < 2 || max_bytes > ACTRAIL_FILE_IO_PATH_BYTES) {
        file_io_diag(ACTRAIL_FILE_IO_PATH_CAPTURE_FAIL);
        return 0;
    }
    __builtin_memset(scratch->path, 0, sizeof(scratch->path));
    copied = actrail_file_io_d_path(__builtin_preserve_access_index(&file->f_path),
        (char *)scratch->path, max_bytes);
    if (copied < 1 || copied > ACTRAIL_FILE_IO_PATH_BYTES) {
        file_io_diag(ACTRAIL_FILE_IO_PATH_CAPTURE_FAIL);
        actrail_file_io_lock(&object->lock);
        if (object->path_status == ACTRAIL_FILE_IO_PATH_UNKNOWN) {
            object->path_status = ACTRAIL_FILE_IO_PATH_FAULT;
        }
        actrail_file_io_unlock(&object->lock);
        return 0;
    }
    actrail_file_io_lock(&object->lock);
    if (object->path_status == ACTRAIL_FILE_IO_PATH_UNKNOWN) {
        __builtin_memcpy(object->path, scratch->path, sizeof(object->path));
        object->path_len = (__u32)copied - 1;
        object->path_status = ACTRAIL_FILE_IO_PATH_CAPTURED;
    }
    actrail_file_io_unlock(&object->lock);
    return 0;
}

SEC("fentry/security_file_free")
int handle_file_io_free(__u64 *ctx) {
    __u64 pointer = ctx[0];
    if (bpf_map_lookup_elem(&file_io_objects, &pointer)
        && bpf_map_delete_elem(&file_io_objects, &pointer)) {
        file_io_diag(ACTRAIL_FILE_IO_OBJECT_DELETE_FAIL);
    }
    return 0;
}

#endif
