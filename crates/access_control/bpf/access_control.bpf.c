/* Governance hooks share task scope; exec enrollment is independent of policy. */
#include "policy.h"
#include "directory.h"
#include "network.h"

static long (*ac_d_path)(struct path *path, char *buffer, __u32 size) =
    (void *)147;

struct linux_binprm {
    struct file *file;
} __attribute__((preserve_access_index));

/* Access-control fields remain local to this BPF object. */
struct file___ac_mode {
    unsigned int f_mode;
} __attribute__((preserve_access_index));

static __always_inline int ac_file_access(struct file *file, __u32 mask, int use_d_path) {
    if (!mask) return 0;
    struct ac_context context = {};
    __u32 first = mask & 1 ? 1 : (mask & 2 ? 6 : 7);
    int ready = ac_begin(first, &context);
    if (ready <= 0) return ready;
    long length = use_d_path
        ? ac_d_path(&file->f_path, (char *)context.work->key.resource, AC_RESOURCE_BYTES)
        : ac_path_resource(context.work, &file->f_path);
    if (!use_d_path && !length) return 0;
    /* Open, read and write share one path resolution and one policy snapshot. */
    #pragma unroll
    for (__u32 index = 0; index < 3; index++) {
        if (!(mask & (1 << index))) continue;
        __u32 domain = index == 0 ? 1 : index + 5;
        if (domain != first) {
            ready = ac_select_domain(domain, &context);
            if (ready < 0) return ready;
            if (!ready) continue;
        }
        int result = ac_finish(&context, ac_path_candidates(&context, length));
        if (result) return result;
    }
    return 0;
}

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

static __always_inline int ac_path_access(struct path *path, __u32 domain) {
    struct ac_context context = {};
    int ready = ac_begin(domain, &context);
    if (ready <= 0) return ready;
    int length = ac_path_resource(context.work, path);
    if (!length) return 0;
    return ac_finish(&context, ac_path_candidates(&context, length));
}

SEC("lsm/file_open")
int ac_file_open(__u64 *ctx) {
    int previous = (int)ctx[1];
    if (previous) return previous;
    struct file *file = (struct file *)ctx[0];
    struct file___ac_mode *mode = (void *)file;
    /* FMODE_READ/WRITE are bits 0/1. O_PATH has neither permission. */
    return ac_file_access(file, 1 | ((mode->f_mode & 3) << 1), 1);
}

SEC("lsm/file_permission")
int ac_file_permission(__u64 *ctx) {
    int previous = (int)ctx[2];
    if (previous) return previous;
    struct file *file = (struct file *)ctx[0];
    __u32 mask = ctx[1];
    return ac_file_access(file, (mask & 4 ? 2 : 0) | (mask & 10 ? 4 : 0), 0);
}

SEC("lsm/mmap_file")
int ac_mmap_file(__u64 *ctx) {
    int previous = (int)ctx[4];
    if (previous) return previous;
    struct file *file = (struct file *)ctx[0];
    /* Anonymous mappings have no file policy resource. */
    /* File-backed maps read file content. Only shared writes alter the file. */
    __u32 mask = ctx[2] & 7 ? 2 : 0;
    if ((ctx[2] & 2) && (ctx[3] & 3) != 2) mask |= 4;
    return file ? ac_file_access(file, mask, 1) : 0;
}

SEC("lsm/file_mprotect")
int ac_file_mprotect(__u64 *ctx) {
    int previous = (int)ctx[3];
    if (previous) return previous;
    struct vm_area_struct *area = (struct vm_area_struct *)ctx[0];
    /* Linux uses bits 0..2 for read/write/execute in both prot and vm_flags.
     * Reducing permissions remains possible after a path is denied. */
    if (!((ctx[2] & 7) & ~area->vm_flags)) return 0;
    struct file *file = area->vm_file;
    __u32 mask = 2;
    if ((ctx[2] & 2) && (area->vm_flags & 8)) mask |= 4;
    return file ? ac_file_access(file, mask, 0) : 0;
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

SEC("lsm/path_unlink")
int ac_path_unlink(__u64 *ctx) {
    int previous = (int)ctx[2];
    if (previous) return previous;
    /* Deletion is independent of opening or changing the file contents. */
    return ac_directory_path((struct path *)ctx[0], (struct dentry *)ctx[1], 8);
}

SEC("lsm/path_chmod")
int ac_path_chmod(__u64 *ctx) {
    int previous = (int)ctx[2];
    if (previous) return previous;
    return ac_path_access((struct path *)ctx[0], 9);
}

SEC("lsm/path_chown")
int ac_path_chown(__u64 *ctx) {
    int previous = (int)ctx[3];
    if (previous) return previous;
    return ac_path_access((struct path *)ctx[0], 9);
}

/* Truncation changes file contents even when no write syscall is issued. */
SEC("lsm/path_truncate")
int ac_path_truncate(__u64 *ctx) {
    int previous = (int)ctx[1];
    if (previous) return previous;
    return ac_path_access((struct path *)ctx[0], 7);
}

SEC("lsm/file_truncate")
int ac_file_truncate(__u64 *ctx) {
    int previous = (int)ctx[1];
    if (previous) return previous;
    return ac_file_access((struct file *)ctx[0], 4, 0);
}

/* Both names participate in rename/link policy. */
SEC("lsm/path_rename")
int ac_path_rename(__u64 *ctx) {
    int previous = (int)ctx[5];
    if (previous) return previous;
    struct path *old_dir = (void *)ctx[0], *new_dir = (void *)ctx[2];
    struct dentry *old_entry = (void *)ctx[1], *new_entry = (void *)ctx[3];
    int result = ac_directory_path(old_dir, old_entry, 10);
    if (result) return result;
    struct ac_context context = {};
    int ready = ac_begin(10, &context);
    if (ready <= 0) return ready;
    int length = ac_directory_resource(context.work, new_dir, new_entry);
    result = ac_finish(&context, ac_path_candidates(&context, length));
    if (result) return result;
    /* Exchange and same-inode renames keep both objects. Reuse the target
     * path and snapshot when ordinary replacement also requires delete. */
    struct inode *target = new_entry->d_inode;
    if ((ctx[4] & 2) || !target || target == old_entry->d_inode) return 0;
    ready = ac_select_domain(8, &context);
    if (ready <= 0) return ready;
    return ac_finish(&context, ac_path_candidates(&context, length));
}

SEC("lsm/path_link")
int ac_path_link(__u64 *ctx) {
    int previous = (int)ctx[3];
    if (previous) return previous;
    struct path *new_dir = (void *)ctx[1];
    /* VFS validates equal source/target mounts before this hook. */
    struct path source = { .mnt = new_dir->mnt, .dentry = (void *)ctx[0] };
    int result = ac_path_access(&source, 11);
    if (result) return result;
    return ac_directory_path(new_dir, (void *)ctx[2], 11);
}

SEC("lsm/bprm_check_security")
int ac_bprm_scope(__u64 *ctx) {
    int previous = (int)ctx[1];
    if (previous) return previous;
    /* Enroll before sched_process_exec consumes the pending launch binding.
     * A positive lookup means success, while LSM permits only a zero return. */
    __u64 trace = 0;
    int scoped = ac_current_trace(&trace);
    return scoped < 0 ? scoped : 0;
}

SEC("lsm/bprm_check_security")
int ac_bprm_check(__u64 *ctx) {
    int previous = (int)ctx[1];
    if (previous) return previous;
    struct linux_binprm *bprm = (struct linux_binprm *)ctx[0];
    return ac_file_path(bprm->file, 4);
}

/* Each action selects its own policy domain and actual socket protocol. */
SEC("lsm/socket_connect")
int ac_socket_connect(__u64 *ctx) {
    int previous = (int)ctx[3];
    if (previous) return previous;
    struct ac_context context = {};
    int ready = ac_begin(5, &context);
    if (ready <= 0) return ready;
    int protocol = ac_network_protocol(context.work, (void *)ctx[0]);
    if (protocol <= 0) return protocol ? ac_finish(&context, protocol) : 0;
    int bytes = ac_network_address(context.work, (void *)ctx[1], (int)ctx[2], 0);
    return ac_network_finish(&context, bytes);
}

SEC("lsm/socket_sendmsg")
int ac_socket_sendmsg(__u64 *ctx) {
    int previous = (int)ctx[3];
    if (previous) return previous;
    struct ac_context context = {};
    int ready = ac_begin(12, &context);
    if (ready <= 0) return ready;
    int protocol = ac_network_protocol(context.work, (void *)ctx[0]);
    if (protocol <= 0) return protocol ? ac_finish(&context, protocol) : 0;
    int bytes = ac_network_message(context.work, (void *)ctx[0], (void *)ctx[1]);
    return ac_network_finish(&context, bytes);
}

SEC("lsm/socket_bind")
int ac_socket_bind(__u64 *ctx) {
    int previous = (int)ctx[3];
    if (previous) return previous;
    struct ac_context context = {};
    int ready = ac_begin(13, &context);
    if (ready <= 0) return ready;
    int protocol = ac_network_protocol(context.work, (void *)ctx[0]);
    if (protocol <= 0) return protocol ? ac_finish(&context, protocol) : 0;
    int bytes = ac_network_address(context.work, (void *)ctx[1], (int)ctx[2], 0);
    return ac_network_finish(&context, bytes);
}

SEC("lsm/socket_listen")
int ac_socket_listen(__u64 *ctx) {
    int previous = (int)ctx[2];
    if (previous) return previous;
    struct ac_context context = {};
    int ready = ac_begin(14, &context);
    if (ready <= 0) return ready;
    int protocol = ac_network_protocol(context.work, (void *)ctx[0]);
    if (protocol <= 0) return protocol ? ac_finish(&context, protocol) : 0;
    return ac_network_finish(&context, ac_network_local(context.work, (void *)ctx[0]));
}

char LICENSE[] SEC("license") = "GPL";
