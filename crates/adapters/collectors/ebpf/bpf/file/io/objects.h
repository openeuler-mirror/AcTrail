/* Shared domain definitions; BPF entry programs reside in units/. */
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

#endif
