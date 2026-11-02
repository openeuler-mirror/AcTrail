#ifndef ACTRAIL_PROCESS_FILE_IDENTITY_H
#define ACTRAIL_PROCESS_FILE_IDENTITY_H

#include "../common/kernel_types.h"
#include "../abi/process.h"

/*
 * Linux 6.13+ multigrain timestamps keep a "ctime queried" flag in the top bit of
 * i_ctime_nsec. The kernel masks it before reporting ctime through stat(), and user
 * space compares this identity against stat(), so it is masked here too.
 */
#define ACTRAIL_INODE_CTIME_QUERIED (1U << 31)

static __always_inline void capture_file_identity(struct file *file, struct actrail_exec_file_identity *identity) {
    struct inode *inode = 0;
    struct super_block *superblock = 0;
    struct timespec64 modified = {}, changed = {};
    unsigned long inode_number = 0;
    __s64 size = 0;
    __u32 device = 0;
    __u32 nanoseconds = 0;

    if (!file || ACTRAIL_CORE_READ(&inode, file, f_inode) || !inode ||
        ACTRAIL_CORE_READ(&superblock, inode, i_sb) || !superblock ||
        ACTRAIL_CORE_READ(&device, superblock, s_dev) ||
        ACTRAIL_CORE_READ(&inode_number, inode, i_ino) ||
        ACTRAIL_CORE_READ(&size, inode, i_size) || size < 0) {
        return;
    }
    struct inode___legacy *legacy = (struct inode___legacy *)inode;
    struct inode___private *private_inode = (struct inode___private *)inode;
    struct inode___split_time *split_inode = (struct inode___split_time *)inode;
    if (ACTRAIL_CORE_FIELD_EXISTS(legacy->i_mtime)) {
        if (ACTRAIL_CORE_READ(&modified.tv_sec, legacy, i_mtime.tv_sec) ||
            ACTRAIL_CORE_READ(&modified.tv_nsec, legacy, i_mtime.tv_nsec)) return;
    } else if (ACTRAIL_CORE_FIELD_EXISTS(private_inode->__i_mtime)) {
        if (ACTRAIL_CORE_READ(&modified.tv_sec, private_inode, __i_mtime.tv_sec) ||
            ACTRAIL_CORE_READ(&modified.tv_nsec, private_inode, __i_mtime.tv_nsec)) return;
    } else if (ACTRAIL_CORE_FIELD_EXISTS(split_inode->i_mtime_sec)) {
        if (ACTRAIL_CORE_READ(&modified.tv_sec, split_inode, i_mtime_sec) ||
            ACTRAIL_CORE_READ(&nanoseconds, split_inode, i_mtime_nsec)) return;
        modified.tv_nsec = nanoseconds;
    } else {
        return;
    }
    if (ACTRAIL_CORE_FIELD_EXISTS(legacy->i_ctime)) {
        if (ACTRAIL_CORE_READ(&changed.tv_sec, legacy, i_ctime.tv_sec) ||
            ACTRAIL_CORE_READ(&changed.tv_nsec, legacy, i_ctime.tv_nsec)) return;
    } else if (ACTRAIL_CORE_FIELD_EXISTS(private_inode->__i_ctime)) {
        if (ACTRAIL_CORE_READ(&changed.tv_sec, private_inode, __i_ctime.tv_sec) ||
            ACTRAIL_CORE_READ(&changed.tv_nsec, private_inode, __i_ctime.tv_nsec)) return;
    } else if (ACTRAIL_CORE_FIELD_EXISTS(split_inode->i_ctime_sec)) {
        if (ACTRAIL_CORE_READ(&changed.tv_sec, split_inode, i_ctime_sec) ||
            ACTRAIL_CORE_READ(&nanoseconds, split_inode, i_ctime_nsec)) return;
        changed.tv_nsec = nanoseconds & ~ACTRAIL_INODE_CTIME_QUERIED;
    } else {
        return;
    }
    if (modified.tv_nsec < 0 || modified.tv_nsec >= 1000000000 ||
        changed.tv_nsec < 0 || changed.tv_nsec >= 1000000000) return;
    identity->device_major = device >> 20;
    identity->device_minor = device & ((1U << 20) - 1);
    identity->inode = inode_number;
    identity->size = (__u64)size;
    identity->mtime_seconds = modified.tv_sec;
    identity->ctime_seconds = changed.tv_sec;
    identity->mtime_nanoseconds = (__u32)modified.tv_nsec;
    identity->ctime_nanoseconds = (__u32)changed.tv_nsec;
    identity->valid = 1;
}

static __always_inline void capture_exec_file_identity(struct actrail_exec_file_identity *identity) {
    struct task_struct *task = actrail_bpf_get_current_task();
    struct mm_struct *mm = 0;
    struct file *file = 0;
    if (ACTRAIL_CORE_READ(&mm, task, mm) || !mm ||
        ACTRAIL_CORE_READ(&file, mm, exe_file) || !file) return;
    capture_file_identity(file, identity);
}

#endif
