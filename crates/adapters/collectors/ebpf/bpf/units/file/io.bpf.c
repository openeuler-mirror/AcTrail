/* Programs owned by the file/io loading node. */
#include "../definitions.h"

SEC("fentry/security_file_free")
int handle_file_io_free(__u64 *ctx) {
    __u64 pointer = ctx[0];
    if (bpf_map_lookup_elem(&file_io_objects, &pointer)
        && bpf_map_delete_elem(&file_io_objects, &pointer)) {
        file_io_diag(ACTRAIL_FILE_IO_OBJECT_DELETE_FAIL);
    }
    return 0;
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

#ifndef ACTRAIL_COMBINED_OBJECT
char LICENSE[] SEC("license") = "GPL";
#endif
