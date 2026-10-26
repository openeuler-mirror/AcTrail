#include "policy.h"
#include "directory.h"

static long (*ac_d_path)(struct path *path, char *buffer, __u32 size) =
    (void *)147;

struct linux_binprm {
    struct file *file;
} __attribute__((preserve_access_index));

static __always_inline int ac_file_path(struct file *file, __u32 domain) {
    struct ac_context context = {};
    int ready = ac_begin(domain, &context);
    if (ready <= 0) return ready;
    long length = ac_d_path(&file->f_path,
        (char *)context.work->key.resource, AC_RESOURCE_BYTES);
    int error = ac_path_candidates(&context, length);
    return ac_finish(&context, error);
}

static __always_inline int ac_directory_path(
    struct path *directory, struct dentry *entry, __u32 domain
) {
    struct ac_context context = {};
    int ready = ac_begin(domain, &context);
    if (ready <= 0) return ready;
    int length = ac_directory_resource(context.work, directory, entry);
    int error = ac_path_candidates(&context, length);
    return ac_finish(&context, error);
}

SEC("lsm/file_open")
int ac_file_open(__u64 *ctx) {
    int previous = (int)ctx[1];
    if (previous) return previous;
    return ac_file_path((struct file *)ctx[0], 1);
}

SEC("lsm/task_alloc")
int ac_task_alloc(__u64 *ctx) {
    int previous = (int)ctx[2];
    if (previous) return previous;
    __u64 trace = 0;
    int scoped = ac_current_trace(&trace);
    if (scoped <= 0) return scoped;
    struct task_struct *child = (struct task_struct *)ctx[0];
    __u64 *stored = ac_task_storage_get(&ac_task_scope, child, &trace, 1);
    if (!stored) return -12;
    *stored = trace;
    return 0;
}

SEC("lsm/path_mkdir")
int ac_path_mkdir(__u64 *ctx) {
    int previous = (int)ctx[3];
    if (previous) return previous;
    return ac_directory_path((struct path *)ctx[0], (struct dentry *)ctx[1], 2);
}

SEC("lsm/path_rmdir")
int ac_path_rmdir(__u64 *ctx) {
    int previous = (int)ctx[2];
    if (previous) return previous;
    return ac_directory_path((struct path *)ctx[0], (struct dentry *)ctx[1], 3);
}

SEC("lsm/bprm_check_security")
int ac_bprm_check(__u64 *ctx) {
    int previous = (int)ctx[1];
    if (previous) return previous;
    struct linux_binprm *bprm = (struct linux_binprm *)ctx[0];
    return ac_file_path(bprm->file, 4);
}

SEC("lsm/socket_connect")
int ac_socket_connect(__u64 *ctx) {
    int previous = (int)ctx[3];
    if (previous) return previous;
    struct ac_context context = {};
    int ready = ac_begin(5, &context);
    if (ready <= 0) return ready;
    void *address = (void *)ctx[1];
    int address_length = (int)ctx[2];
    __u16 family = 0;
    if (address_length < 2 || bpf_probe_read_kernel(&family, sizeof(family), address))
        return ac_finish(&context, -14);
    __u32 address_bytes = 0;
    if (family == 2) {
        if (address_length < 16) return ac_finish(&context, -22);
        context.work->key.resource[0] = 4;
        address_bytes = 5;
        if (bpf_probe_read_kernel(context.work->key.resource + 1, 4, address + 4) ||
            bpf_probe_read_kernel(context.work->key.resource + 5, 2, address + 2))
            return ac_finish(&context, -14);
    } else if (family == 10) {
        /* Linux also accepts sockaddr_in6 without the optional scope ID.
         * Matching reads only the port and the address ending at byte 24. */
        if (address_length < 24) return ac_finish(&context, -22);
        context.work->key.resource[0] = 6;
        address_bytes = 17;
        if (bpf_probe_read_kernel(context.work->key.resource + 1, 16, address + 8) ||
            bpf_probe_read_kernel(context.work->key.resource + 17, 2, address + 2))
            return ac_finish(&context, -14);
    } else {
        /* Network policy has IP address semantics; Unix and other socket
         * families are outside that configured resource domain. */
        return 0;
    }
    ac_candidate(&context, address_bytes + 2);
    ac_candidate(&context, address_bytes);
    return ac_finish(&context, 0);
}

char LICENSE[] SEC("license") = "GPL";
