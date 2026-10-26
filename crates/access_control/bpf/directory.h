#ifndef ACTRAIL_ACCESS_CONTROL_DIRECTORY_H
#define ACTRAIL_ACCESS_CONTROL_DIRECTORY_H

#include "policy.h"

struct qstr {
    unsigned int len;
    const unsigned char *name;
} __attribute__((preserve_access_index));
struct dentry {
    struct qstr d_name;
    struct dentry *d_parent;
} __attribute__((preserve_access_index));
struct vfsmount {
    struct dentry *mnt_root;
} __attribute__((preserve_access_index));
struct mount {
    struct mount *mnt_parent;
    struct dentry *mnt_mountpoint;
    struct vfsmount mnt;
} __attribute__((preserve_access_index));
struct seqcount {
    unsigned int sequence;
} __attribute__((preserve_access_index));
struct seqcount_spinlock {
    struct seqcount seqcount;
} __attribute__((preserve_access_index));
struct fs_struct {
    struct seqcount_spinlock seq;
    struct path root;
} __attribute__((preserve_access_index));
struct task_struct___ac_fs {
    struct fs_struct *fs;
} __attribute__((preserve_access_index));

/* seqlock_t starts with seqcount_spinlock_t, whose first member is the
 * seqcount_t sequence. Both symbols are mandatory when directory hooks load. */
extern const void rename_lock __attribute__((section(".ksyms")));
extern const void mount_lock __attribute__((section(".ksyms")));

static long (*ac_loop)(__u32 count, void *callback, void *context, __u64 flags) =
    (void *)181;

struct ac_directory_walk {
    struct ac_work *work;
    struct path root;
    struct path current;
    __u32 offset;
    int result;
};

static long ac_directory_step(__u32 index, void *opaque) {
    struct ac_directory_walk *walk = opaque;
    struct dentry *mount_root = 0, *parent = 0;
    struct vfsmount *current_mount = walk->current.mnt;
    struct dentry *current_entry = walk->current.dentry;
    (void)index;
    if (walk->current.mnt == walk->root.mnt &&
        walk->current.dentry == walk->root.dentry) {
        walk->result = 0;
        return 1;
    }
    walk->result = -14;
    if (!current_entry ||
        ACTRAIL_CORE_READ(&mount_root, current_mount, mnt_root)) return 1;
    if (walk->current.dentry == mount_root) {
        struct mount *mount = (void *)walk->current.mnt -
            __builtin_preserve_field_info(((struct mount *)0)->mnt, 0);
        struct mount *parent_mount = 0;
        walk->result = -18;
        if (ACTRAIL_CORE_READ(&parent_mount, mount, mnt_parent) || !parent_mount ||
            parent_mount == mount ||
            ACTRAIL_CORE_READ(&walk->current.dentry, mount, mnt_mountpoint)) return 1;
        walk->current.mnt = __builtin_preserve_access_index(&parent_mount->mnt);
        walk->result = -36;
        return 0;
    }
    const unsigned char *name = 0;
    __u32 length = 0;
    if (ACTRAIL_CORE_READ(&parent, current_entry, d_parent) || !parent ||
        parent == current_entry ||
        ACTRAIL_CORE_READ(&name, current_entry, d_name.name) || !name ||
        ACTRAIL_CORE_READ(&length, current_entry, d_name.len)) return 1;
    walk->result = -36;
    if (!length || length >= AC_RESOURCE_BYTES || length >= walk->offset) return 1;
    __u32 offset = walk->offset - length;
    if (offset >= AC_RESOURCE_BYTES) return 1;
    if (bpf_probe_read_kernel(walk->work->name + offset, length, name)) {
        walk->result = -14;
        return 1;
    }
    offset--;
    if (offset >= AC_RESOURCE_BYTES) return 1;
    walk->work->name[offset] = '/';
    walk->offset = offset;
    walk->current.dentry = parent;
    return 0;
}

static __always_inline int ac_directory_resource(
    struct ac_work *work, struct path *directory, struct dentry *entry
) {
    __u32 rename_before = 0, mount_before = 0, fs_before = 0;
    __u32 rename_after = 0, mount_after = 0, fs_after = 0;
    struct task_struct___ac_fs *task = (void *)actrail_bpf_get_current_task();
    struct fs_struct *fs = 0;
    struct path root = {}, current = {};
    if (bpf_probe_read_kernel(&rename_before, sizeof(rename_before), &rename_lock) ||
        bpf_probe_read_kernel(&mount_before, sizeof(mount_before), &mount_lock) ||
        ACTRAIL_CORE_READ(&fs, task, fs) || !fs ||
        ACTRAIL_CORE_READ(&fs_before, fs, seq.seqcount.sequence) ||
        ((rename_before | mount_before | fs_before) & 1)) return -11;
    if (ACTRAIL_CORE_READ(&root, fs, root) || !root.mnt || !root.dentry ||
        bpf_probe_read_kernel(&current, sizeof(current), directory) || !current.mnt)
        return -14;
    struct dentry *parent = 0;
    if (ACTRAIL_CORE_READ(&parent, entry, d_parent) || parent != current.dentry)
        return -11;
    current.dentry = entry;
    struct ac_directory_walk walk = {
        .work = work,
        .root = root,
        .current = current,
        .offset = AC_RESOURCE_BYTES - 1,
        .result = -36,
    };
    work->name[AC_RESOURCE_BYTES - 1] = 0;
    if (ac_loop(AC_RESOURCE_BYTES, ac_directory_step, &walk, 0) < 0) return -14;
    if (walk.result) return walk.result;
    __u32 offset = walk.offset;
    if (bpf_probe_read_kernel(&rename_after, sizeof(rename_after), &rename_lock) ||
        bpf_probe_read_kernel(&mount_after, sizeof(mount_after), &mount_lock) ||
        ACTRAIL_CORE_READ(&fs_after, fs, seq.seqcount.sequence) ||
        rename_before != rename_after || mount_before != mount_after ||
        fs_before != fs_after) return -11;
    if (offset == AC_RESOURCE_BYTES - 1) {
        offset--;
        work->name[offset] = '/';
    }
    actrail_barrier_var(offset);
    if (offset >= AC_RESOURCE_BYTES) return -36;
    for (__u32 i = 0; i < AC_RESOURCE_BYTES; i++) {
        __u32 source = offset + i;
        if (source >= AC_RESOURCE_BYTES) break;
        work->key.resource[i] = work->name[source];
    }
    return AC_RESOURCE_BYTES - offset;
}

#endif
